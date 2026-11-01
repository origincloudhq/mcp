//! Origin Cloud's MCP tools: a tool per method of the API, generated from
//! its definitions, calling the [Rust SDK](https://github.com/origincloudhq/sdk-rust).
//!
//! The library is the tools and the protocol, without a transport or a
//! source of credentials: each server picks its own. `origin mcp serve`
//! serves it over standard input and output with the CLI's login; a server
//! on the network serves the same [`Server`] per session, with the
//! caller's.
//!
//! ```no_run
//! use origincloud_mcp::{ClientSource, Options, Server};
//!
//! struct Environment;
//!
//! impl ClientSource for Environment {
//!     fn client(&self, _service: &str) -> Result<origincloud_core::Client, origincloud_core::Error> {
//!         origincloud_core::Client::new()
//!     }
//! }
//!
//! # fn main() -> Result<(), origincloud_mcp::UnknownToolset> {
//! let server = Server::new(Environment, Options { read_only: true, ..Options::default() })?;
//! // rmcp::ServiceExt::serve(server, transport)
//! # Ok(())
//! # }
//! ```
//!
//! # The tools
//!
//! - A tool per method, `{service}_{method}` in snake case
//!   (`pvs_create_volume`), in the toolset of its service (`pvs`). A
//!   service whose methods answer with operations also has the operation
//!   methods' tools, `pvs_get_operation` and `pvs_wait_operation`, which
//!   its host serves for its operations.
//! - A tool's arguments are the method's request message in proto3 JSON,
//!   less what the method doesn't take: output-only fields, a resource's
//!   name in a create, the immutable fields in an update, and the fields
//!   the generator's table leaves out. Arguments the schema doesn't
//!   describe, such as a misspelt field, are refused, not dropped.
//! - Its result is the response message in proto3 JSON. A tool starting an
//!   operation waits for it up to 30 seconds and answers with the
//!   operation: `done`, with its `response` or `error`, or not done yet,
//!   for `{service}_wait_operation`.
//! - Read-only for a method bound to an HTTP GET and for the operation
//!   methods; destructive for a delete and for a change the generator's
//!   table calls one-way (growing a volume); idempotent for a read and for
//!   a method taking a request id. A read-only session exposes only the
//!   read-only tools, and refuses the others by name.
//! - `current_context` tells the agent the user's default project, zone
//!   and region, and what the session exposes.
//!
//! The generator, `generator/`, writes the tools from the API definitions;
//! its table, `generator/tools.toml`, lists the methods without a tool.

mod arguments;
mod runtime;
mod server;

// Included rather than declared as modules, so that rustfmt leaves the
// generated code as the generator wrote it.
mod generated {
    include!("generated/tools.rs");
}

pub use server::{toolsets, ClientSource, Defaults, Options, Server, UnknownToolset};
