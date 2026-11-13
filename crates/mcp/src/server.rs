//! The MCP server of one session: the tools it exposes, and the SDK
//! clients their calls go through.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use origincloud_core::Client;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::arguments;
use crate::generated;
use crate::runtime::{Effect, ToolDef, ToolError};

/// The tool that tells an agent the session's defaults.
const CURRENT_CONTEXT: &str = "current_context";

/// What the server tells the agent about its tools.
const TOOLS: &str = "Origin Cloud's API, a toolset per service. The tools take and \
    answer the API's messages in JSON. Resources are named by their whole resource names, \
    such as `projects/{project}/zones/{zone}/volumes/{volume}`: `current_context` says which \
    project and zone the user works in, and `location_list_zones` lists the zones. A tool \
    that starts an operation answers with it once it is done, or, when that takes longer, \
    with `done` false: the service's `_wait_operation` tool waits for it further.";

/// The rules every Origin Cloud skill shares, origincloudhq/skills'
/// `shared/rules.md`, which `scripts/generate.sh` copies at the commit
/// `SKILLS_REVISION` names.
const RULES: &str = include_str!("generated/rules.md");

/// The server's instructions: its tools, then the shared rules.
fn instructions() -> String {
    format!("{TOOLS}\n\n{RULES}")
}

/// Where the server gets the SDK clients its tool calls go through: the
/// host's credentials and endpoints. The server asks once per service and
/// session, and keeps the client, so that its reads show the session's own
/// writes.
pub trait ClientSource: Send + Sync + 'static {
    /// A client for the calls of `service`'s tools, such as `pvs`.
    fn client(&self, service: &str) -> Result<Client, origincloud_core::Error>;
}

/// What a session exposes.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The toolsets to expose, by service, such as `pvs`; every one when
    /// `None`. [`toolsets`] lists them.
    pub toolsets: Option<Vec<String>>,
    /// Exposes only the tools that change nothing.
    pub read_only: bool,
    /// What `current_context` tells the agent.
    pub defaults: Defaults,
}

/// The defaults of the user the session acts for, which `current_context`
/// tells the agent.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Defaults {
    /// The project, such as `my-project-123`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The zone, such as `eu-central1-az1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// The region whose front door the calls go to; the regionless one
    /// when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// An [`Options::toolsets`] entry naming no toolset.
#[derive(Debug, thiserror::Error)]
#[error("there is no toolset {name:?}; the toolsets are {}", toolsets().collect::<Vec<_>>().join(", "))]
pub struct UnknownToolset {
    pub name: String,
}

/// The toolsets there are, one per service: `location`, `pvs`, ...
pub fn toolsets() -> impl Iterator<Item = &'static str> {
    generated::TOOLSETS.iter().map(|t| t.name)
}

/// One generated tool the session exposes.
struct Exposed {
    toolset: &'static str,
    def: &'static ToolDef,
    tool: Tool,
}

/// The MCP server of one session.
pub struct Server {
    source: Box<dyn ClientSource>,
    options: Options,
    tools: Vec<Exposed>,
    context: Tool,
    clients: Mutex<BTreeMap<&'static str, Client>>,
}

impl Server {
    /// A session exposing what `options` selects, its calls going through
    /// the clients of `source`.
    pub fn new(source: impl ClientSource, options: Options) -> Result<Self, UnknownToolset> {
        if let Some(selected) = &options.toolsets {
            if let Some(name) = selected
                .iter()
                .find(|n| !toolsets().any(|t| t == n.as_str()))
            {
                return Err(UnknownToolset { name: name.clone() });
            }
        }
        let tools = generated::TOOLSETS
            .iter()
            .filter(|t| {
                options
                    .toolsets
                    .as_ref()
                    .is_none_or(|selected| selected.iter().any(|s| s == t.name))
            })
            .flat_map(|t| t.tools.iter().map(move |def| (t.name, def)))
            .filter(|(_, def)| !options.read_only || def.effect == Effect::Read)
            .map(|(toolset, def)| Exposed {
                toolset,
                def,
                tool: tool(def),
            })
            .collect();
        Ok(Self {
            source: Box::new(source),
            options,
            tools,
            context: context_tool(),
            clients: Mutex::new(BTreeMap::new()),
        })
    }

    /// The session's client for `service`'s calls.
    fn client(&self, service: &'static str) -> Result<Client, origincloud_core::Error> {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(client) = clients.get(service) {
            return Ok(client.clone());
        }
        let client = self.source.client(service)?;
        clients.insert(service, client.clone());
        Ok(client)
    }

    /// Calls the tool `name`, which `call_tool` answers with.
    async fn call(
        &self,
        name: &str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, McpError> {
        if name == CURRENT_CONTEXT {
            return Ok(CallToolResult::structured(json!({
                "defaults": self.options.defaults,
                "toolsets": self.tools.iter().map(|t| t.toolset).collect::<std::collections::BTreeSet<_>>(),
                "readOnly": self.options.read_only,
            })));
        }
        let Some(exposed) = self.tools.iter().find(|t| t.def.name == name) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                self.unavailable(name),
            )]));
        };
        if let Err(message) = arguments::check(&exposed.tool.schema_as_json_value(), &arguments) {
            return Ok(failure(&ToolError::Arguments(message)));
        }
        let client = match self.client(exposed.toolset) {
            Ok(client) => client,
            Err(error) => return Ok(failure(&ToolError::Call(error))),
        };
        match generated::call(exposed.toolset, &client, name, arguments).await {
            Some(Ok(value)) => Ok(CallToolResult::structured(value)),
            Some(Err(error)) => Ok(failure(&error)),
            None => Err(McpError::internal_error(
                format!("the tool {name} has no call"),
                None,
            )),
        }
    }

    /// Why the session has no tool `name`.
    fn unavailable(&self, name: &str) -> String {
        let def = generated::TOOLSETS
            .iter()
            .find_map(|t| t.tools.iter().find(|d| d.name == name).map(|d| (t.name, d)));
        match def {
            Some((_, d)) if self.options.read_only && d.effect != Effect::Read => format!(
                "{name} changes the platform, and this server is read-only: it exposes only the \
                 tools that change nothing"
            ),
            Some((toolset, _)) => {
                format!("{name} is in the toolset {toolset}, which this server doesn't expose")
            }
            None => format!("there is no tool {name}"),
        }
    }
}

/// A failed call's result, which the agent reads.
fn failure(error: &ToolError) -> CallToolResult {
    let mut value = error.to_json();
    if let (ToolError::Failed(_), Value::Object(fields)) = (error, &mut value) {
        fields.insert("failed".to_owned(), Value::Bool(true));
    }
    CallToolResult::structured_error(value)
}

/// The MCP tool of a generated one.
fn tool(def: &'static ToolDef) -> Tool {
    let schema: JsonObject = serde_json::from_str(def.input_schema)
        .expect("the generator writes each input schema as a JSON object");
    let annotations = match def.effect {
        Effect::Read => ToolAnnotations::new().read_only(true),
        Effect::Write {
            destructive,
            idempotent,
        } => ToolAnnotations::new()
            .read_only(false)
            .destructive(destructive)
            .idempotent(idempotent),
    };
    Tool::new(def.name, def.description, Arc::new(schema))
        .with_title(def.title)
        .with_annotations(annotations.open_world(false))
}

fn context_tool() -> Tool {
    let schema = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    let Value::Object(schema) = schema else {
        unreachable!("a JSON object")
    };
    Tool::new(
        CURRENT_CONTEXT,
        "The project, zone and region the user works in, as they configured them, and the \
         toolsets this server exposes, and whether it is read-only. Build resource names from \
         these defaults, such as `projects/{project}/zones/{zone}/volumes/{volume}`, unless the \
         user names others.",
        Arc::new(schema),
    )
    .with_title("Current context")
    .with_annotations(ToolAnnotations::new().read_only(true).open_world(false))
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("origincloud", env!("CARGO_PKG_VERSION"))
                    .with_title("Origin Cloud"),
            )
            .with_instructions(instructions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut tools = vec![self.context.clone()];
        tools.extend(self.tools.iter().map(|t| t.tool.clone()));
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let arguments = request.arguments.unwrap_or_default();
        Ok(self.call(&request.name, arguments).await?.into())
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if name == CURRENT_CONTEXT {
            return Some(self.context.clone());
        }
        self.tools
            .iter()
            .find(|t| t.def.name == name)
            .map(|t| t.tool.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instructions describe the tools, then give the shared rules
    /// whole, as origincloudhq/skills holds them.
    #[test]
    fn the_instructions_are_the_tools_then_the_shared_rules() {
        let text = instructions();
        assert!(text.starts_with("Origin Cloud's API, a toolset per service."));
        assert!(text.ends_with(RULES));
        assert!(
            RULES.contains("## Ask before deletes and one-way changes"),
            "{RULES}"
        );
    }
}
