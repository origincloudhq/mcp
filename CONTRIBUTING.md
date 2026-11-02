# Contributing

Thank you for helping improve Origin Cloud's MCP tools.

## Issues

Open an issue for anything that is wrong, unclear or missing: a tool that
behaves differently from its description, an input schema an agent can't
fill in, a description that doesn't explain enough. Say which tool it is
about, and include the call and its result where you can, with tokens and
account details removed.

To report a security vulnerability, don't open an issue: use **Report a
vulnerability** on this repository's Security tab, which reaches us
privately.

## Pull requests

Pull requests are reviewed and merged here. For anything larger than a small
fix, open an issue first, so that we can agree on the change before you spend
time on it. CI must pass: the checks below, run on every pull request.

## How the tools are built

- `crates/mcp` is the library, `origincloud-mcp`. `src/server.rs` is the MCP
  server of a session: the tools it exposes and the SDK clients their calls
  go through. `src/arguments.rs` checks a call's arguments against the tool's
  input schema, and `src/runtime.rs` holds what the generated tools share:
  decoding the request, answering, and waiting on operations.
- `crates/mcp/src/generated/` is generated from the API definitions in
  [origincloudhq/api](https://github.com/origincloudhq/api), at the commit
  `API_REVISION` names, by this repository's generator, `generator/`.

The generator downloads the API's dependencies from the Buf Schema Registry
at the commits its `buf.lock` pins, checks them against the digests it pins,
and compiles the definitions with [protox](https://docs.rs/protox). For each
service, the packages `origincloud.{service}.{version}`, it writes
`{service}.rs`, a tool per method of the newest version's services:

- its input schema, the request message in proto3 JSON, without
  output-only fields, the resource's name in a create, and the immutable
  fields of the resource an update carries, its `google.api.field_behavior`
  annotations deciding which fields are required;
- its description, from the definitions' comments;
- its hints, from the method: read-only for a GET binding, destructive for a
  delete, idempotent for a read or a request with a `request_id`;
- the code that decodes the arguments into the SDK's request and calls the
  SDK's client.

The operation methods, `origincloud.longrunning`'s `GetOperation` and
`WaitOperation`, are tools of each service whose methods answer with
long-running operations (`pvs_wait_operation`), because every service serves
the operations its methods return at its own host. The generator also writes
`tools.rs`, every service's tools and the dispatch to them, and the
descriptors of every package it read, `api.binpb`, from which the tests check
that every method has a tool and that the hints follow the methods.
`generator/src/tools.rs` implements the scheme.

What the API definitions don't express is in `generator/tools.toml`, by
service, method and request field (`pvs`, `UpdateVolume`, `volume.size_gib`):

- `[[no_tool]]` leaves a method without a tool, with the reason. The coverage
  test fails for any method of the API that has neither a tool nor an entry
  here, which is what keeps the tools at parity with the API;
- `[[exclude]]` leaves a field out of a method's input schema, when the
  method doesn't take it although its request message has it, with the
  reason;
- `[[require]]` makes a field required in a method's input schema, when
  its `google.api.field_behavior` doesn't, with the reason, which the
  tool's description gives;
- `[[one_way]]` marks a change that can't be undone, with its consequence:
  the method's tool is destructive, and its description says what can't be
  undone.

The generator warns when an entry names a service, a method or a field the
API doesn't have, and the coverage test fails for a `[[no_tool]]` entry
naming no method, so that an entry can't silently stop applying; regenerate
after changing the table.

Everything the generator runs is pinned by `Cargo.lock`, so generating from
the same API always gives the same code. The tools use the SDK's request and
response types, so the SDK commit `Cargo.toml` pins must have been generated
from the same API revision; the scripts check that.

So a change to what a tool offers starts in the API, and reaches the tools
through the SDK; a change to how every tool behaves is a change to the
generator or the library. Never edit the generated files.

The regeneration workflow (`.github/workflows/regenerate.yml`) moves the
tools to the newest SDK, and the API revision it was generated from, when the
SDK releases, and daily, and opens a pull request when that changes
anything. Once its CI passes, `merge-regeneration.yml` merges it, unless
someone added commits to it, which then wait for review, and
`downstream.yml` moves the CLI to it. The merge and the dispatch use a
GitHub App's token (the variable `REGENERATE_APP_ID` and the secret
`REGENERATE_APP_PRIVATE_KEY`); without it, the pull request is merged by
hand. If the API gained a
method the generator can't make a tool for, the generator says so on
standard error and the coverage test fails: extend the generator on that
pull request, or list the method under `[[no_tool]]` with the reason.

### A new service

When the API gains a service, regeneration writes its tools, and the
regeneration pull request fails to build until they are wired up, on that
pull request: add the service's SDK crate, `origincloud-{service}` from
`origincloudhq/sdk-rust` at the same `rev` as the others, to
`[workspace.dependencies]` in `Cargo.toml`, and
`origincloud-{service}.workspace = true` to `crates/mcp/Cargo.toml`, then run
`cargo update --workspace`, which adds it to `Cargo.lock` and moves nothing
else. `scripts/update.sh` keeps it at the SDK's head from then on.

Nothing else names a service: the scripts, the generator and the tests find
every service in the API. A file under `origincloud/` whose package isn't
shaped `origincloud.{service}.{version}` fails generation, rather than being
left out unnoticed.

## Checks

Rust (the version `rust-toolchain.toml` names), git and network access are all
you need.

```sh
scripts/generate.sh            # generate the tools of every service again
scripts/generate.sh --check    # check the checked-in tools are what the API generates
scripts/update.sh              # move to the newest SDK and its API revision, and generate
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI also checks the scripts with shellcheck.

## License

By contributing, you agree that your contribution is licensed under the
Apache License 2.0, like the rest of this repository.
