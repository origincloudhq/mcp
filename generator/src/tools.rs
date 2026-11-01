//! An MCP tool per method, for `crates/mcp`: its name, description and
//! input schema, from the method's comments and request message, how it
//! affects the platform, and the code that decodes its arguments into the
//! request and calls the SDK.
//!
//! The scheme, which `crates/mcp`'s documentation describes for users:
//!
//! - The tool is `{service}_{method}` in snake case: `CreateVolume` of
//!   `pvs` is `pvs_create_volume`. The methods of the operations package
//!   (`origincloud.longrunning`) are tools of every service whose methods
//!   answer with operations, as they are commands of its group:
//!   `pvs_wait_operation`.
//! - Its arguments are the request message in proto3 JSON, the SDK's JSON.
//!   The input schema leaves out output-only fields, the fields
//!   `tools.toml` excludes from the method, a resource's identifier
//!   except in an update, which names the resource by it, and the immutable
//!   fields of the resource an update carries, which it can't change. In
//!   that resource only the identifier is required: an update changes the
//!   fields its mask names.
//! - A method bound to an HTTP GET, and every operation method, is
//!   read-only. A delete, and a method with a one-way change in
//!   `tools.toml`, is destructive. A read-only method, and one whose
//!   request has a `request_id`, is idempotent.
//! - A method answering with an operation waits for it a while and answers
//!   with the operation, done or not, as `crates/mcp`'s runtime says.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::Result;
use heck::ToSnakeCase;
use prost_reflect::{
    DescriptorPool, FieldDescriptor, Kind, MessageDescriptor, MethodDescriptor, ServiceDescriptor,
};
use serde_json::{json, Map, Value};

use crate::descriptors::{
    behaviors, client_name, message_extension, operation_result, rust_type_path, sdk_module,
    service_of, services_of, unparse, DURATION, EMPTY, FIELD_MASK, IDENTIFIER, IMMUTABLE,
    OPERATIONS, OUTPUT_ONLY, REQUIRED,
};
use crate::docs::{self, Comments};
use crate::policy::Policy;

const TIMESTAMP: &str = "google.protobuf.Timestamp";

/// How deep request messages may nest; deeper is taken for a message that
/// contains itself, whose schema has no end.
const DEPTH: usize = 8;

/// The generated tools of one service, and the methods left without one.
pub struct Generated {
    pub code: String,
    pub skipped: Vec<String>,
    /// Whether the service has the operation methods' tools.
    pub operations: bool,
}

/// What every method's generation shares.
struct Cx<'a> {
    pool: &'a DescriptorPool,
    comments: &'a Comments,
    policy: &'a Policy,
    /// The package whose methods these are.
    package: &'a str,
    /// The service the package names, as `tools.toml` does: `pvs`, or
    /// `longrunning` for the operation methods.
    own: &'a str,
    /// The service the tools belong to, their toolset: `pvs`.
    service: &'a str,
    /// The SDK module of the package: `::origincloud_pvs::v1beta1`.
    sdk: String,
    /// For the operation methods, the expression of the service's
    /// endpoint, which serves its operations: the `&Service` of one of its
    /// clients.
    host: Option<String>,
    /// The tool that waits for the service's operations, when it has one.
    wait: Option<String>,
    /// Every field name of the package, for the descriptions.
    fields: BTreeSet<String>,
}

/// One tool: its entry in the service's table, its arm in the service's
/// dispatch, and its function.
struct Tool {
    name: String,
    entry: String,
    function: String,
    code: String,
}

/// The name of the tool of `method` of `service`: `pvs_create_volume`.
fn tool_name(service: &str, method: &str) -> String {
    format!("{service}_{}", method.to_snake_case())
}

/// The tools of `package`'s service: one per method of its services, and,
/// when one of them answers with an operation, one per method of the
/// operations package `operations`.
pub fn generate(
    pool: &DescriptorPool,
    comments: &Comments,
    policy: &Policy,
    package: &str,
    operations: Option<&str>,
) -> Result<Generated> {
    let service = service_of(package);
    let fields = |package: &str| -> BTreeSet<String> {
        pool.all_messages()
            .filter(|m| m.package_name() == package)
            .flat_map(|m| m.fields().map(|f| f.name().to_owned()).collect::<Vec<_>>())
            .collect()
    };
    let services = services_of(pool, package);
    let answers_with_operations = services
        .iter()
        .flat_map(|s| s.methods())
        .any(|m| operation_result(pool, &m).is_some());
    let operation_services = match (answers_with_operations, operations) {
        (true, Some(operations)) => services_of(pool, operations),
        _ => Vec::new(),
    };
    let wait = operation_services
        .iter()
        .flat_map(|s| s.methods())
        .find(|m| m.name() == "WaitOperation" && !policy.no_tool(OPERATIONS, m.name()))
        .map(|m| tool_name(service, m.name()));
    let cx = Cx {
        pool,
        comments,
        policy,
        package,
        own: service,
        service,
        sdk: sdk_module(package),
        host: None,
        wait,
        fields: fields(package),
    };

    let mut tools: Vec<Tool> = Vec::new();
    let mut skipped = Vec::new();
    for s in &services {
        for method in s.methods() {
            if policy.no_tool(service, method.name()) {
                continue;
            }
            match tool(&cx, s, &method) {
                Ok(tool) => tools.push(tool),
                Err(why) => skipped.push(format!("{}: {why}", method.full_name())),
            }
        }
    }

    // The operation methods, at the host of the service's first client.
    let mut with_operations = false;
    if let (Some(first), Some(operations)) = (services.first(), operations) {
        let ops = Cx {
            package: operations,
            own: OPERATIONS,
            sdk: sdk_module(operations),
            host: Some(format!(
                "&{}::{}::new(client).service().clone()",
                cx.sdk,
                client_name(first)
            )),
            fields: fields(operations),
            ..cx
        };
        for s in &operation_services {
            for method in s.methods() {
                if policy.no_tool(OPERATIONS, method.name()) {
                    continue;
                }
                let name = tool_name(service, method.name());
                if tools.iter().any(|t| t.name == name) {
                    skipped.push(format!(
                        "{}: the service has a method of its own named {}",
                        method.full_name(),
                        method.name()
                    ));
                    continue;
                }
                match tool(&ops, s, &method) {
                    Ok(tool) => {
                        tools.push(tool);
                        with_operations = true;
                    }
                    Err(why) => skipped.push(format!("{}: {why}", method.full_name())),
                }
            }
        }
    }

    let mut code = String::new();
    code.push_str(
        "#[allow(unused_imports)]\n\
         use crate::runtime::{finish, reply, request, Effect, ToolDef, ToolError};\n\n",
    );
    code.push_str(&docs::attributes(
        &format!("The tools of every method of `{package}`."),
        "",
    ));
    code.push_str("pub(crate) static TOOLS: &[ToolDef] = &[\n");
    for t in &tools {
        code.push_str(&t.entry);
    }
    code.push_str("];\n\n");
    code.push_str(&docs::attributes(
        "Calls the tool `name` with `arguments`; `None` when it isn't one of these.",
        "",
    ));
    code.push_str(
        "pub(crate) async fn call(\n\
             client: &::origincloud_core::Client,\n\
             name: &str,\n\
             arguments: ::serde_json::Map<String, ::serde_json::Value>,\n\
         ) -> Option<Result<::serde_json::Value, ToolError>> {\n\
             Some(match name {\n",
    );
    for t in &tools {
        writeln!(
            code,
            "        {:?} => {}(client, arguments).await,",
            t.name, t.function
        )?;
    }
    code.push_str("        _ => return None,\n    })\n}\n\n");
    for t in &tools {
        code.push_str(&t.code);
        code.push('\n');
    }
    Ok(Generated {
        code: unparse(&code)?,
        skipped,
        operations: with_operations,
    })
}

/// The tools registry: a module per service, the table of every service's
/// tools, and the dispatch to the service's own.
pub fn registry(services: &[&str]) -> Result<String> {
    let mut code = String::new();
    code.push_str("use crate::runtime::{Toolset, ToolError};\n");
    for service in services {
        code.push_str(&docs::attributes(&format!("The tools of `{service}`."), ""));
        writeln!(
            code,
            "pub(crate) mod {service} {{\n    include!(\"{service}.rs\");\n}}"
        )?;
    }
    code.push_str(&docs::attributes("Every service's tools, by service.", ""));
    code.push_str("pub(crate) static TOOLSETS: &[Toolset] = &[\n");
    for service in services {
        writeln!(
            code,
            "    Toolset {{ name: {service:?}, tools: {service}::TOOLS }},"
        )?;
    }
    code.push_str("];\n");
    code.push_str(&docs::attributes(
        "Calls the tool `name` of `service`; `None` when it has no such tool.",
        "",
    ));
    code.push_str(
        "pub(crate) async fn call(\n\
             service: &str,\n\
             client: &::origincloud_core::Client,\n\
             name: &str,\n\
             arguments: ::serde_json::Map<String, ::serde_json::Value>,\n\
         ) -> Option<Result<::serde_json::Value, ToolError>> {\n\
             match service {\n",
    );
    for service in services {
        writeln!(
            code,
            "        {service:?} => {service}::call(client, name, arguments).await,"
        )?;
    }
    code.push_str("        _ => None,\n    }\n}\n");
    unparse(&code)
}

/// The tool of method `m` of `s`.
fn tool(cx: &Cx<'_>, s: &ServiceDescriptor, m: &MethodDescriptor) -> Result<Tool, String> {
    if m.is_client_streaming() || m.is_server_streaming() {
        return Err("streaming methods aren't supported".to_owned());
    }
    let request = m.input();
    let update = request
        .get_field_by_name("update_mask")
        .is_some_and(|f| matches!(f.kind(), Kind::Message(ref k) if k.full_name() == FIELD_MASK));
    let schema = message_schema(cx, m, &request, "", update, 0)?;
    let operation = operation_result(cx.pool, m).is_some();

    let read = cx.own == OPERATIONS || bound_to_get(cx.pool, m);
    let one_way: Vec<_> = cx
        .policy
        .one_way
        .iter()
        .filter(|o| o.service == cx.own && o.method == m.name())
        .collect();
    let destructive = m.name().starts_with("Delete") || !one_way.is_empty();
    let idempotent = read || request.get_field_by_name("request_id").is_some();
    let effect = if read {
        "Effect::Read".to_owned()
    } else {
        format!("Effect::Write {{ destructive: {destructive}, idempotent: {idempotent} }}")
    };

    let name = tool_name(cx.service, m.name());
    let mut description = flow(&docs::help(
        cx.comments.leading(m.parent_file().name(), m.path()),
        &cx.fields,
    ));
    if description.is_empty() {
        description = format!("Calls `{}`.", m.name());
    }
    if cx.host.is_some() {
        let _ = write!(
            description,
            "\n\nThe operation is one a `{}_…` tool answered with.",
            cx.service
        );
    }
    if operation {
        description.push_str(
            "\n\nAnswers with the operation: once it is done, its `response` is the result, or \
             its `error` says why it failed.",
        );
        match &cx.wait {
            Some(wait) => {
                let _ = write!(
                    description,
                    " Waiting for it takes a while at most; while `done` is false, `{wait}` \
                     waits for it further."
                );
            }
            None => description.push_str(" Waiting for it takes a while at most."),
        }
    }
    for o in &one_way {
        let _ = write!(
            description,
            "\n\nSetting `{}` can't be undone. {}",
            json_path(&request, &o.field),
            o.consequence
        );
    }

    let function = m.name().to_snake_case();
    let entry = format!(
        "    ToolDef {{ name: {name:?}, title: {:?}, description: {description:?}, input_schema: {:?}, effect: {effect} }},\n",
        title(m.name()),
        serde_json::to_string(&schema).map_err(|e| e.to_string())?,
    );
    let request_type = format!(
        "{}::{}",
        cx.sdk,
        rust_type_path(cx.package, request.full_name())
    );
    let client = match &cx.host {
        Some(host) => format!("{}::OperationClient::new({host})", cx.sdk),
        None => format!("{}::{}::new(client)", cx.sdk, client_name(s)),
    };
    let answer = if operation {
        "finish(response).await"
    } else {
        "reply(&response)"
    };
    let code = format!(
        "async fn {function}(\n\
             client: &::origincloud_core::Client,\n\
             arguments: ::serde_json::Map<String, ::serde_json::Value>,\n\
         ) -> Result<::serde_json::Value, ToolError> {{\n\
             let request: {request_type} = request(arguments)?;\n\
             let response = {client}.{function}(request).await?;\n\
             {answer}\n\
         }}\n"
    );
    Ok(Tool {
        name,
        entry,
        function,
        code,
    })
}

/// Whether `m`'s HTTP binding is a GET.
fn bound_to_get(pool: &DescriptorPool, m: &MethodDescriptor) -> bool {
    message_extension(pool, &m.options(), "google.api.http")
        .is_some_and(|rule| rule.has_field_by_name("get"))
}

/// A method's name in words, the first capitalized: `CreateVolume` is
/// `Create volume`.
fn title(method: &str) -> String {
    let words = method.to_snake_case().replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A field path from `message`, such as `volume.size_gib`, in JSON names:
/// `volume.sizeGib`.
fn json_path(message: &MessageDescriptor, path: &str) -> String {
    let mut names = Vec::new();
    let mut current = Some(message.clone());
    for part in path.split('.') {
        let Some(field) = current.as_ref().and_then(|m| m.get_field_by_name(part)) else {
            names.push(part.to_owned());
            current = None;
            continue;
        };
        names.push(field.json_name().to_owned());
        current = match field.kind() {
            Kind::Message(m) => Some(m),
            _ => None,
        };
    }
    names.join(".")
}

/// The JSON schema of `message`, the request of `m` or a message inside it
/// at `path`.
fn message_schema(
    cx: &Cx<'_>,
    m: &MethodDescriptor,
    message: &MessageDescriptor,
    path: &str,
    update: bool,
    depth: usize,
) -> Result<Value, String> {
    if depth > DEPTH {
        return Err(format!("{} nests too deep", message.full_name()));
    }
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in message.fields() {
        let field_path = if path.is_empty() {
            field.name().to_owned()
        } else {
            format!("{path}.{}", field.name())
        };
        if cx.policy.excluded(cx.own, m.name(), &field_path) {
            continue;
        }
        // A oneof needs its members out of `required`, and at most (or
        // exactly) one of them given, which the schema and the arguments'
        // check don't say yet: the method has no tool, and the coverage test
        // says so, rather than a tool that can't be called.
        if let Some(oneof) = field.containing_oneof().filter(|o| !o.is_synthetic()) {
            return Err(format!(
                "{field_path} is in the oneof {}, which tools don't support yet",
                oneof.name()
            ));
        }
        let b = behaviors(cx.pool, &field);
        let identifier = b.contains(&IDENTIFIER);
        if b.contains(&OUTPUT_ONLY)
            || (identifier && !update)
            || (update && depth > 0 && b.contains(&IMMUTABLE))
        {
            continue;
        }
        let mut schema = field_schema(cx, m, &field, &field_path, update, depth)?;
        let comment = flow(&docs::help(
            cx.comments
                .leading(field.parent_file().name(), field.path()),
            &cx.fields,
        ));
        describe(&mut schema, &comment);
        if (b.contains(&REQUIRED) && !(update && depth > 0)) || identifier {
            required.push(Value::String(field.json_name().to_owned()));
        }
        properties.insert(field.json_name().to_owned(), schema);
    }
    let mut schema = json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false,
    });
    if !required.is_empty() {
        schema["required"] = Value::Array(required);
    }
    Ok(schema)
}

/// `text` with each paragraph on one line: the comments' line breaks only
/// wrap them.
fn flow(text: &str) -> String {
    text.split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `schema`'s description: `comment`, before what the schema already says
/// of its form.
fn describe(schema: &mut Value, comment: &str) {
    let form = schema
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let text = match (comment.is_empty(), form.is_empty()) {
        (true, _) => form,
        (false, true) => comment.to_owned(),
        (false, false) => format!("{comment}\n\n{form}"),
    };
    if !text.is_empty() {
        schema["description"] = Value::String(text);
    }
}

/// The JSON schema of a field: a map, a list, or one value.
fn field_schema(
    cx: &Cx<'_>,
    m: &MethodDescriptor,
    field: &FieldDescriptor,
    path: &str,
    update: bool,
    depth: usize,
) -> Result<Value, String> {
    if field.is_map() {
        let Kind::Message(entry) = field.kind() else {
            return Err(format!("the map {path} has no entry message"));
        };
        let value = entry.map_entry_value_field();
        let values = value_schema(cx, m, &value.kind(), path, update, depth)?;
        return Ok(json!({ "type": "object", "additionalProperties": values }));
    }
    let value = value_schema(cx, m, &field.kind(), path, update, depth)?;
    if field.is_list() {
        Ok(json!({ "type": "array", "items": value }))
    } else {
        Ok(value)
    }
}

/// The JSON schema of one value of `kind`, in proto3 JSON.
fn value_schema(
    cx: &Cx<'_>,
    m: &MethodDescriptor,
    kind: &Kind,
    path: &str,
    update: bool,
    depth: usize,
) -> Result<Value, String> {
    Ok(match kind {
        Kind::String => json!({ "type": "string" }),
        Kind::Bytes => json!({ "type": "string", "contentEncoding": "base64" }),
        Kind::Bool => json!({ "type": "boolean" }),
        Kind::Int32
        | Kind::Sint32
        | Kind::Sfixed32
        | Kind::Uint32
        | Kind::Fixed32
        | Kind::Int64
        | Kind::Sint64
        | Kind::Sfixed64
        | Kind::Uint64
        | Kind::Fixed64 => json!({ "type": "integer" }),
        Kind::Float | Kind::Double => json!({ "type": "number" }),
        Kind::Enum(e) => {
            let values: Vec<String> = e
                .values()
                .filter(|v| !(v.number() == 0 && v.name().ends_with("_UNSPECIFIED")))
                .map(|v| v.name().to_owned())
                .collect();
            json!({ "type": "string", "enum": values })
        }
        Kind::Message(message) => match message.full_name() {
            TIMESTAMP => json!({
                "type": "string",
                "format": "date-time",
                "description": "In RFC 3339, such as `2026-11-01T12:00:00Z`.",
            }),
            DURATION => json!({
                "type": "string",
                "description": "Seconds with an `s`, such as `30s`.",
            }),
            FIELD_MASK => json!({
                "type": "string",
                "description": "Field names in camel case, separated by commas, such as `sizeGib,tags`.",
            }),
            EMPTY => json!({ "type": "object", "additionalProperties": false }),
            name if name.starts_with("google.protobuf.") => {
                return Err(format!("{path}: the type {name} isn't supported"));
            }
            _ => message_schema(cx, m, message, path, update, depth + 1)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_titles() {
        assert_eq!(tool_name("pvs", "CreateVolume"), "pvs_create_volume");
        assert_eq!(tool_name("pvs", "WaitOperation"), "pvs_wait_operation");
        assert_eq!(title("ListStorageClasses"), "List storage classes");
    }

    #[test]
    fn paragraphs_flow() {
        assert_eq!(
            flow("A larger size\ngrows it.\n\n\nIt never\n  shrinks."),
            "A larger size grows it.\n\nIt never shrinks."
        );
    }

    #[test]
    fn a_comment_comes_before_what_the_form_says() {
        let mut schema = json!({ "type": "string", "description": "Such as `30s`." });
        describe(&mut schema, "How long to wait.");
        assert_eq!(schema["description"], "How long to wait.\n\nSuch as `30s`.");
        let mut schema = json!({ "type": "string" });
        describe(&mut schema, "");
        assert!(schema.get("description").is_none());
    }
}
