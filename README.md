# Origin Cloud MCP tools

`origincloud-mcp` is the [MCP](https://modelcontextprotocol.io) server of
[Origin Cloud](https://origincloud.eu)'s API as a Rust library: a tool per
method of the API, generated from its definitions in
[origincloudhq/api](https://github.com/origincloudhq/api), calling the
[Rust SDK](https://github.com/origincloudhq/sdk-rust).

The library has no transport and no source of credentials of its own: a
server picks both. The [`origin` CLI](https://github.com/origincloudhq/cli)
serves it to an agent on your machine, `origin mcp serve`, with your login.

## The tools

- **One per method.** `{service}_{method}` in snake case, such as
  `pvs_create_volume` or `location_list_zones`, in the toolset of its service
  (`pvs`, `location`). A service whose methods answer with operations also
  has `{service}_get_operation` and `{service}_wait_operation`.
- **Arguments.** The method's request in the API's JSON, as the tool's input
  schema describes it: without output-only fields, without a resource's name
  in a create, and without the fields the method doesn't take. Arguments the
  schema doesn't describe, such as a misspelt field, are refused before
  anything is sent, naming the fields there are.
- **Results.** The method's response in the API's JSON. A tool starting an
  operation waits for it up to 30 seconds and answers with the operation:
  done, with its `response` or its `error`, or not done yet, for the service's
  `_wait_operation` tool. A refusal of the API, a failed operation and
  arguments that don't make a request are the tool's error, with the API's
  reason and message.
- **Hints.** Read-only for reads and for the operation tools; destructive
  for deletes and for changes that can't be undone, such as growing a volume;
  idempotent for reads and for methods taking a request id. A read-only
  server exposes only the read-only tools, and refuses the others by name.
- **Instructions.** The server tells the agent what the tools take and
  answer, and the rules every Origin Cloud
  [agent skill](https://github.com/origincloudhq/skills) shares: use the
  login the user has and never print a credential, read before changing,
  ask before deletes and changes that can't be undone, wait on operations,
  name resources by their full names, and report errors with their code and
  correlation ID.
- **`current_context`** tells the agent the user's default project, zone and
  region, and what the server exposes.

## Use

```rust
use origincloud_mcp::{ClientSource, Options, Server};

/// Every call with the credentials of the environment, as the SDK reads them.
struct Environment;

impl ClientSource for Environment {
    fn client(&self, _service: &str) -> Result<origincloud_core::Client, origincloud_core::Error> {
        origincloud_core::Client::new()
    }
}

let server = Server::new(Environment, Options { read_only: true, ..Options::default() })?;
// Serve it with any rmcp transport, such as standard input and output:
// rmcp::ServiceExt::serve(server, rmcp::transport::stdio()).await?
```

The crate isn't published to crates.io yet; depend on it from this
repository at a commit:

```toml
origincloud-mcp = { git = "https://github.com/origincloudhq/mcp", rev = "<commit>" }
```

Pin `origincloud-core` and the service crates to the commit of
origincloudhq/sdk-rust that this repository's `Cargo.toml` names: the
library hands your `ClientSource` the SDK's `Client`, so both must be the
same crate.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), which describes how the tools are
generated.

## License

Apache License 2.0; see [LICENSE](LICENSE).
