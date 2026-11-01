//! What the tests share: a fake API that records every request, and an MCP
//! client connected to a server in the same process.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use fake_server::{FakeServer, Received, Response};
use origincloud_core::{AccessToken, Client};
use origincloud_mcp::{ClientSource, Options, Server};
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

/// A fake API answering with `answer`, recording what it received.
pub struct Api {
    pub server: FakeServer,
    pub received: Arc<Mutex<Vec<Received>>>,
}

impl Api {
    pub async fn start(answer: impl Fn(&Received) -> Response + Send + Sync + 'static) -> Self {
        let received = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&received);
        let server = FakeServer::start(move |request| {
            let response = answer(&request);
            seen.lock().unwrap().push(request);
            async move { response }
        })
        .await;
        Self { server, received }
    }

    /// The requests received so far.
    pub fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }

    /// An MCP client of a server with `options`, whose calls go to this
    /// API.
    pub async fn connect(&self, options: Options) -> Agent {
        let server = Server::new(Endpoint(self.server.url()), options).unwrap();
        connect(server).await
    }
}

/// Every service's client at one endpoint, with a fixed token.
struct Endpoint(String);

impl ClientSource for Endpoint {
    fn client(&self, _service: &str) -> Result<Client, origincloud_core::Error> {
        Client::builder()
            .endpoint(self.0.clone())
            .credentials(AccessToken::new("token"))
            .build()
    }
}

/// An MCP client, the agent's side of a session.
pub struct Agent(RunningService<RoleClient, ()>);

/// An MCP client of `server`, over an in-process pipe.
pub async fn connect(server: Server) -> Agent {
    let (server_side, client_side) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(session) = server.serve(server_side).await {
            let _ = session.waiting().await;
        }
    });
    Agent(().serve(client_side).await.expect("the client connects"))
}

impl Agent {
    pub async fn tools(&self) -> Vec<Tool> {
        self.0.list_all_tools().await.expect("the tools are listed")
    }

    /// Calls `tool` with `arguments`, a JSON object.
    pub async fn call(&self, tool: &str, arguments: Value) -> CallToolResult {
        let Value::Object(arguments) = arguments else {
            panic!("arguments are an object")
        };
        self.0
            .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
            .await
            .expect("the call is answered")
    }
}

/// A result's structured content.
pub fn content(result: &CallToolResult) -> &Value {
    result
        .structured_content
        .as_ref()
        .expect("a structured result")
}

/// Whether `result` is an error.
pub fn failed(result: &CallToolResult) -> bool {
    result.is_error == Some(true)
}
