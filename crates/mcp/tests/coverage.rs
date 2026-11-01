//! Every method of the API has a tool, or an entry in the generator's
//! table saying why not, and every tool is a method's: the MCP tools stay
//! at parity with the API. Read from the descriptors the generator checked
//! in beside the tools (`src/generated/api.binpb`), and its table
//! (`generator/tools.toml`).

mod common;

use std::collections::BTreeSet;

use common::connect;
use origincloud_mcp::{ClientSource, Options, Server};
use prost_reflect::{DescriptorPool, MethodDescriptor, Value};
use serde::Deserialize;

/// The package of the operation methods, tools of every service whose
/// methods answer with operations.
const OPERATIONS: &str = "longrunning";

#[derive(Deserialize)]
struct Table {
    #[serde(default)]
    no_tool: Vec<NoTool>,
    #[serde(default)]
    one_way: Vec<Entry>,
}

#[derive(Deserialize)]
struct NoTool {
    service: String,
    method: String,
}

#[derive(Deserialize)]
struct Entry {
    service: String,
    method: String,
}

fn pool() -> DescriptorPool {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/generated/api.binpb"
    ))
    .expect("the generator wrote the descriptors");
    DescriptorPool::decode(bytes.as_slice()).expect("descriptors")
}

fn table() -> Table {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../generator/tools.toml"
    ))
    .expect("the generator's table");
    toml::from_str(&text).expect("the table parses")
}

/// The service a package of the API names: `pvs` for
/// `origincloud.pvs.v1beta1`; `None` for a package that isn't the API's,
/// such as `google.longrunning`, which the descriptors hold as a
/// dependency.
fn service_of(package: &str) -> Option<&str> {
    package.strip_prefix("origincloud.")?.split('.').next()
}

/// Every method of the API as `(the toolset it belongs to, the service
/// that names it in the table, the method)`: each service's own, and the
/// operation methods of each service answering with operations.
fn methods(pool: &DescriptorPool) -> Vec<(String, String, MethodDescriptor)> {
    let operations: Vec<MethodDescriptor> = pool
        .services()
        .filter(|s| service_of(s.package_name()) == Some(OPERATIONS))
        .flat_map(|s| s.methods().collect::<Vec<_>>())
        .collect();
    let mut methods = Vec::new();
    for service in pool.services() {
        let Some(name) = service_of(service.package_name()).filter(|s| *s != OPERATIONS) else {
            continue;
        };
        let name = name.to_owned();
        for method in service.methods() {
            methods.push((name.clone(), name.clone(), method));
        }
    }
    let with_operations: BTreeSet<String> = methods
        .iter()
        .filter(|(_, _, m)| has_extension(pool, m, "google.longrunning.operation_info"))
        .map(|(s, _, _)| s.clone())
        .collect();
    for service in with_operations {
        for method in &operations {
            methods.push((service.clone(), OPERATIONS.to_owned(), method.clone()));
        }
    }
    methods
}

fn has_extension(pool: &DescriptorPool, m: &MethodDescriptor, name: &str) -> bool {
    pool.get_extension_by_name(name)
        .is_some_and(|ext| m.options().has_extension(&ext))
}

fn bound_to_get(pool: &DescriptorPool, m: &MethodDescriptor) -> bool {
    let Some(ext) = pool.get_extension_by_name("google.api.http") else {
        return false;
    };
    match m.options().get_extension(&ext).as_ref() {
        Value::Message(rule) => rule.has_field_by_name("get"),
        _ => false,
    }
}

/// `CreateVolume` of `pvs` is `pvs_create_volume`.
fn tool_name(service: &str, method: &str) -> String {
    let mut snake = String::new();
    for (i, c) in method.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            snake.push('_');
        }
        snake.push(c.to_ascii_lowercase());
    }
    format!("{service}_{snake}")
}

struct NoClients;

impl ClientSource for NoClients {
    fn client(&self, _service: &str) -> Result<origincloud_core::Client, origincloud_core::Error> {
        Err(origincloud_core::Error::Config("no clients".to_owned()))
    }
}

async fn tools() -> Vec<rmcp::model::Tool> {
    let server = Server::new(NoClients, Options::default()).unwrap();
    connect(server).await.tools().await
}

#[tokio::test]
async fn every_method_has_a_tool_or_a_reason() {
    let pool = pool();
    let table = table();
    let listed: BTreeSet<String> = tools()
        .await
        .into_iter()
        .map(|t| t.name.into_owned())
        .filter(|n| n != "current_context")
        .collect();

    let mut expected = BTreeSet::new();
    let mut missing = Vec::new();
    for (toolset, service, method) in methods(&pool) {
        let name = tool_name(&toolset, method.name());
        let reason = table
            .no_tool
            .iter()
            .any(|n| n.service == service && n.method == method.name());
        if reason {
            assert!(
                !listed.contains(&name),
                "{name} has a tool and a reason not to"
            );
            continue;
        }
        if !listed.contains(&name) {
            missing.push(name.clone());
        }
        expected.insert(name);
    }
    assert!(
        missing.is_empty(),
        "these methods have no tool, and the table no reason: {missing:?}"
    );
    let stray: Vec<_> = listed.difference(&expected).collect();
    assert!(stray.is_empty(), "these tools are no method's: {stray:?}");

    for entry in &table.no_tool {
        assert!(
            methods(&pool)
                .iter()
                .any(|(_, s, m)| *s == entry.service && m.name() == entry.method),
            "the table leaves {} {} without a tool, which the API doesn't have",
            entry.service,
            entry.method
        );
    }
}

#[tokio::test]
async fn the_hints_follow_the_methods() {
    let pool = pool();
    let table = table();
    let tools = tools().await;
    let mut checked = 0;
    for (toolset, service, method) in methods(&pool) {
        let name = tool_name(&toolset, method.name());
        let Some(tool) = tools.iter().find(|t| t.name == name) else {
            continue;
        };
        let hints = tool.annotations.clone().unwrap_or_default();
        let read = service == OPERATIONS || bound_to_get(&pool, &method);
        assert_eq!(hints.read_only_hint, Some(read), "{name}");
        let one_way = table
            .one_way
            .iter()
            .any(|o| o.service == service && o.method == method.name());
        if !read {
            let destructive = method.name().starts_with("Delete") || one_way;
            assert_eq!(hints.destructive_hint, Some(destructive), "{name}");
        }
        checked += 1;
    }
    assert!(checked > 0);
    assert!(tools.iter().any(|t| t.name == "pvs_update_volume"
        && t.annotations.as_ref().and_then(|a| a.destructive_hint) == Some(true)));
}
