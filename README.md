# op-registry

Declare operations once; derive CLI, tool-list and call adapters from the descriptors. MIT OR Apache-2.0.

The `op-listen` crate (`op-listen/`) adds a shared streaming listener on top of op-registry, with cursors, frames, a per-app gate and a reconnecting follower.

## Quick start

Define parameter and output types, then register the operation. Parameter types must
also implement `Serialize`: CLI parsing is converted to JSON before dispatch.

```rust,ignore
use op_registry::{registry, Audience, CallOptions, OpParams};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

#[derive(OpParams, Serialize, Deserialize, JsonSchema)]
struct ListParams {
    #[op(long = "limit", default = 10)]
    limit: u32,
}

#[derive(Serialize, JsonSchema)]
struct Item {
    name: String,
}

async fn list(_: &Context, _: ListParams) -> anyhow::Result<Vec<Item>> {
    Ok(Vec::new())
}

let registry = registry! {
    Ctx = Context;
    entries: [{
        noun: "item", verb: "list", description: "List items",
        access: reads, audience: Agent, params: ListParams, output: Vec<Item>,
        op: list,
        output_schema: serde_json::json!({"type": "array"}),
    }]
};

let command = op_registry::cli::command(&registry, "app", "Example");
let matches = command.get_matches();
let result = op_registry::cli::dispatch(
    &registry, &context, &matches, CallOptions::default(),
).await?;
let tools = op_registry::mcp::tool_list(&registry, Audience::Agent);
let value = op_registry::mcp::call_tool(
    &registry, &context, Audience::Agent, "item_list",
    serde_json::json!({}), CallOptions::default(),
).await?;
```

The `output_schema:` override comes after `op:` in a `registry!` entry. The
parameter derive includes `Serialize` because CLI arguments pass through JSON.

## Output contract

For the CLI, stdout carries exactly one JSON value per call. A streaming
operation prints one JSON value per line. Failures are formatted as
`Code: message` on stderr by `cli::error_line`; the process exit code comes from the
coded-error class table supplied to `cli::exit_code_for` (for example, a
`coded_error::cli::ExitTable`). The library does not print or exit; the host does.

For MCP, `mcp::tool_error` produces a transport-safe failure with its code,
class, and fingerprint; messages of non-logical failures are redacted to
`request failed`. Streaming operations are not exposed as MCP tools.

## Pagination convention

List-like operations conventionally take `--limit N` and `--all`. Their output
carries a `truncated` boolean, which is `true` when more results exist than
were returned. `--cursor` exists only on streaming operations. This is a
convention for the operation author; the library does not enforce it.

## Declared but not acted on

Descriptors record and expose these fields, but the library does not itself act
on them: `access`, `renders_text`, `exit_code_on_ok`, `requires`, `hidden`,
`since`, `deprecated`, and `aliases`. The host decides what to do with this
metadata.

## Parity and composition

Use `cli::command` and `mcp::tool_list` to build the two frontend surfaces.
`parity::cli_surface` collects the CLI names, and `parity::assert_parity`
checks them against the MCP tools and registry. It supports explicit
`allow_cli_only` and `allow_mcp_only` lists, applies the selected audience
rule, and treats streaming operations as CLI-only.

Another crate can contribute entries with `Registry::merge`, or directly with
`push_unary` and `push_stream` (including `CliHooks`). These operations panic
when an operation name is duplicated.
