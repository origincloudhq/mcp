//! A session against a fake API: a volume's life through the tools, the
//! waiting on operations, the errors an agent reads, and what a read-only
//! session or a choice of toolsets exposes.

mod common;

use common::{content, failed, Api};
use fake_server::{Received, Response};
use origincloud_core::CONSISTENCY_TOKEN_HEADER;
use origincloud_mcp::{toolsets, Defaults, Options, Server};
use serde_json::{json, Value};

const VOLUME: &str = "projects/p/zones/z/volumes/v";

fn operation(id: &str, done: bool, result: Value) -> String {
    let mut op = json!({ "name": format!("projects/p/zones/z/operations/{id}"), "done": done });
    if done {
        let key = if result.get("code").is_some() {
            "error"
        } else {
            "response"
        };
        op[key] = result;
    }
    op.to_string()
}

fn volume(size: i64) -> Value {
    json!({
        "@type": "type.googleapis.com/origincloud.pvs.v1beta1.Volume",
        "name": VOLUME,
        "sizeGib": size,
    })
}

/// The fake PVS: creates, grows and deletes the volume `v`, each operation
/// done at once, and answers each with a consistency token, which a get
/// of the volume must present.
fn pvs(r: &Received) -> Response {
    let token = |n: &str| (CONSISTENCY_TOKEN_HEADER, format!("tok-{n}"));
    let (header, value) = match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/v1beta1/projects/p/zones/z/volumes") => {
            return Response::json(operation("op-1", true, volume(8)))
                .with_header(token("1").0, token("1").1)
        }
        ("PATCH", "/v1beta1/projects/p/zones/z/volumes/v") => token("2"),
        ("DELETE", "/v1beta1/projects/p/zones/z/volumes/v") => {
            return Response::json(operation(
                "op-3",
                true,
                json!({ "@type": "type.googleapis.com/google.protobuf.Empty" }),
            ))
        }
        ("GET", "/v1beta1/projects/p/zones/z/volumes/v") => {
            return Response::json(
                json!({ "name": VOLUME, "sizeGib": 8, "state": "AVAILABLE" }).to_string(),
            )
        }
        _ => return Response::json("{}").with_status(404),
    };
    Response::json(operation("op-2", true, volume(20))).with_header(header, value)
}

#[tokio::test]
async fn a_volume_is_created_read_grown_and_deleted() {
    let api = Api::start(pvs).await;
    let agent = api.connect(Options::default()).await;

    let created = agent
        .call(
            "pvs_create_volume",
            json!({ "parent": "projects/p/zones/z", "volume": { "sizeGib": 8, "tags": { "origin:name": "data" } } }),
        )
        .await;
    assert!(!failed(&created), "{created:?}");
    assert_eq!(content(&created)["done"], true);
    assert_eq!(content(&created)["response"]["sizeGib"], 8);

    let read = agent
        .call("pvs_get_volume", json!({ "name": VOLUME }))
        .await;
    assert_eq!(content(&read)["state"], "AVAILABLE", "{read:?}");

    let grown = agent
        .call(
            "pvs_update_volume",
            json!({ "volume": { "name": VOLUME, "sizeGib": 20 }, "updateMask": "sizeGib" }),
        )
        .await;
    assert!(!failed(&grown), "{grown:?}");
    assert_eq!(content(&grown)["response"]["sizeGib"], 20);

    let deleted = agent
        .call("pvs_delete_volume", json!({ "name": VOLUME }))
        .await;
    assert!(!failed(&deleted), "{deleted:?}");
    assert_eq!(content(&deleted)["done"], true);

    let received = api.received();
    let calls: Vec<(&str, &str)> = received
        .iter()
        .map(|r| (r.method.as_str(), r.path.as_str()))
        .collect();
    assert_eq!(
        calls,
        [
            ("POST", "/v1beta1/projects/p/zones/z/volumes"),
            ("GET", "/v1beta1/projects/p/zones/z/volumes/v"),
            ("PATCH", "/v1beta1/projects/p/zones/z/volumes/v"),
            ("DELETE", "/v1beta1/projects/p/zones/z/volumes/v"),
        ]
    );
    let create: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(create["sizeGib"], 8, "{create}");
    assert_eq!(create["tags"]["origin:name"], "data");
    // The session keeps one client per service, so that a read shows the
    // session's own writes.
    assert_eq!(received[1].header(CONSISTENCY_TOKEN_HEADER), Some("tok-1"));
    assert_eq!(received[2].query_values("updateMask"), ["sizeGib"]);
    let update: Value = serde_json::from_slice(&received[2].body).unwrap();
    assert_eq!(update["sizeGib"], 20, "{update}");
}

#[tokio::test]
async fn an_operation_still_running_is_waited_for() {
    let api = Api::start(|r| match r.path.as_str() {
        "/v1beta1/projects/p/zones/z/volumes" => {
            Response::json(operation("op-1", false, Value::Null))
        }
        "/v1beta1/projects/p/zones/z/operations/op-1:wait" => {
            Response::json(operation("op-1", true, volume(8)))
        }
        _ => Response::json("{}").with_status(404),
    })
    .await;
    let agent = api.connect(Options::default()).await;

    let created = agent
        .call(
            "pvs_create_volume",
            json!({ "parent": "projects/p/zones/z", "volume": { "sizeGib": 8 } }),
        )
        .await;
    assert_eq!(content(&created)["done"], true, "{created:?}");
    assert_eq!(content(&created)["response"]["name"], VOLUME);
}

#[tokio::test]
async fn a_failed_operation_is_the_tools_error() {
    let api = Api::start(|_| {
        Response::json(operation(
            "op-1",
            true,
            json!({ "code": 9, "message": "the volume is attached" }),
        ))
    })
    .await;
    let agent = api.connect(Options::default()).await;

    let deleted = agent
        .call("pvs_delete_volume", json!({ "name": VOLUME }))
        .await;
    assert!(failed(&deleted), "{deleted:?}");
    assert_eq!(content(&deleted)["failed"], true);
    assert_eq!(
        content(&deleted)["error"]["message"],
        "the volume is attached"
    );
}

#[tokio::test]
async fn the_apis_refusal_reaches_the_agent() {
    let api = Api::start(|_| {
        Response::json(
            json!({ "error": {
                "code": 404,
                "status": "NOT_FOUND",
                "message": "volume v not found",
                "details": [{
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "reason": "NOT_FOUND",
                    "domain": "origincloudapis.eu",
                }],
            }})
            .to_string(),
        )
        .with_status(404)
    })
    .await;
    let agent = api.connect(Options::default()).await;

    let read = agent
        .call("pvs_get_volume", json!({ "name": VOLUME }))
        .await;
    assert!(failed(&read), "{read:?}");
    assert_eq!(content(&read)["reason"], "NOT_FOUND");
    assert_eq!(content(&read)["message"], "volume v not found");
}

#[tokio::test]
async fn arguments_the_schema_doesnt_describe_are_refused_unsent() {
    let api = Api::start(pvs).await;
    let agent = api.connect(Options::default()).await;

    for (tool, arguments, why) in [
        (
            "pvs_create_volume",
            json!({ "parent": "projects/p/zones/z", "volume": { "size_gib": 8 } }),
            "there is no field `volume.size_gib`",
        ),
        // The table leaves the field out of the method, which doesn't take
        // it, although its message has it.
        (
            "pvs_import_volume",
            json!({
                "parent": "projects/p/zones/z",
                "sourceUrl": "https://example.com/disk.img",
                "volume": { "sizeGib": 8, "sourceSnapshot": "projects/p/zones/z/snapshots/s" },
            }),
            "there is no field `volume.sourceSnapshot`",
        ),
        // Output only.
        (
            "pvs_create_volume",
            json!({ "parent": "projects/p/zones/z", "volume": { "sizeGib": 8, "state": "AVAILABLE" } }),
            "there is no field `volume.state`",
        ),
        (
            "pvs_create_volume",
            json!({ "volume": { "sizeGib": 8 } }),
            "`parent` is required",
        ),
    ] {
        let result = agent.call(tool, arguments).await;
        assert!(failed(&result), "{tool}: {result:?}");
        assert_eq!(content(&result)["reason"], "INVALID_ARGUMENTS");
        let message = content(&result)["message"].as_str().unwrap();
        assert!(message.starts_with(why), "{tool}: {message}");
    }
    assert!(api.received().is_empty());
}

#[tokio::test]
async fn a_read_only_session_exposes_and_calls_nothing_that_changes() {
    let api = Api::start(pvs).await;
    let everything = api.connect(Options::default()).await.tools().await;
    let agent = api
        .connect(Options {
            read_only: true,
            ..Options::default()
        })
        .await;

    let exposed = agent.tools().await;
    assert!(exposed.iter().any(|t| t.name == "pvs_get_volume"));
    for tool in &exposed {
        let read_only = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
        assert_eq!(read_only, Some(true), "{} is exposed", tool.name);
    }
    let mutating: Vec<_> = everything
        .iter()
        .filter(|t| t.annotations.as_ref().and_then(|a| a.read_only_hint) == Some(false))
        .collect();
    assert!(mutating.iter().any(|t| t.name == "pvs_delete_volume"));
    for tool in mutating {
        let result = agent.call(&tool.name, json!({})).await;
        assert!(failed(&result), "{} was called", tool.name);
        let text = format!("{:?}", result.content);
        assert!(text.contains("read-only"), "{}: {text}", tool.name);
    }
    assert!(api.received().is_empty());
}

#[tokio::test]
async fn toolsets_select_what_is_exposed() {
    let api = Api::start(pvs).await;
    let agent = api
        .connect(Options {
            toolsets: Some(vec!["location".to_owned()]),
            ..Options::default()
        })
        .await;
    let names: Vec<String> = agent
        .tools()
        .await
        .into_iter()
        .map(|t| t.name.into())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("location_")),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n.starts_with("pvs_")), "{names:?}");
    let refused = agent
        .call("pvs_get_volume", json!({ "name": VOLUME }))
        .await;
    assert!(failed(&refused));
    assert!(format!("{:?}", refused.content).contains("toolset pvs"));

    let unknown = Server::new(
        NoClients,
        Options {
            toolsets: Some(vec!["no-such-toolset".to_owned()]),
            ..Options::default()
        },
    );
    let message = unknown
        .err()
        .expect("an unknown toolset is refused")
        .to_string();
    assert!(message.contains("\"no-such-toolset\""), "{message}");
    assert!(message.contains("pvs"), "{message}");
}

#[tokio::test]
async fn current_context_tells_the_defaults() {
    let api = Api::start(pvs).await;
    let agent = api
        .connect(Options {
            read_only: true,
            defaults: Defaults {
                project: Some("my-project-123".to_owned()),
                zone: Some("eu-central1-az1".to_owned()),
                region: None,
            },
            ..Options::default()
        })
        .await;
    let context = agent.call("current_context", json!({})).await;
    assert_eq!(
        content(&context),
        &json!({
            "defaults": { "project": "my-project-123", "zone": "eu-central1-az1" },
            // Every toolset, in order: each service has reads.
            "toolsets": toolsets().collect::<std::collections::BTreeSet<_>>(),
            "readOnly": true,
        })
    );
}

struct NoClients;

impl origincloud_mcp::ClientSource for NoClients {
    fn client(&self, _service: &str) -> Result<origincloud_core::Client, origincloud_core::Error> {
        Err(origincloud_core::Error::Config("no clients".to_owned()))
    }
}
