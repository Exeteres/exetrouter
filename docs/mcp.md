# ExetRouter MCP server

Connect an MCP client once to **`https://api.example.com/mcp`**, or `http://127.0.0.1:8787/mcp` for standalone. Use the same existing router bearer as the API. Keep `exr serve`, the standalone dashboard, or the shared server running. The endpoint is `/mcp`, **not** `/v1/mcp`.

The client discovers tools and their JSON Schemas through `tools/list`; it also receives server instructions explaining when to generate images. No exr skill, helper process, provider-specific image-tool authorization, ChatGPT credentials on the client, or conversational host model is required. The agent still needs permission to call tools under its client's normal approval policy.

## Tools

| Tool | Arguments | Result |
| --- | --- | --- |
| `generate_image` | Required `prompt`; optional `quality`, `size`, `background` | One MCP image content block and a short text confirmation |
| `edit_image` | The same options plus `image_urls` containing one to five inline data-image or HTTPS URLs | One MCP image content block and a short text confirmation |

Both tools use **ChatGPT `gpt-image-2`** through the existing OAuth pool. `quality` accepts `auto`, `low`, `medium`, `high`; `background` accepts `auto`, `transparent`, `opaque`; size defaults to the backend default. They consume image quota and are not idempotent. The router never retries a submitted call automatically.

Images are returned as `{type: "image", mimeType: "image/png", data: "...base64..."}` (PNG, JPEG or WebP, detected from a short decoded prefix). They are not printed as base64 text or stored on the server. Display/save behavior depends on the client; MCP does not provide a universal client-local output path. For editing a local file, the client must supply its inline data URL or an accessible HTTPS URL. Server-local paths, arbitrary shell commands, backend file IDs and user-selected upstream accounts are not accepted.

This exposes ordinary MCP tools in unmodified Codex, independently of its built-in `image_gen` OAuth gate. It does not enable or impersonate that separate built-in extension. [Native routes and entitlement limits](native-tools.md) still apply, and a configured tool does not prove a particular subscription currently has image capacity.

## Codex CLI

Merge into the intended Codex configuration/profile, preserving unrelated settings:

```toml
[mcp_servers.exetrouter]
url = "https://api.example.com/mcp"
bearer_token_env_var = "EXETROUTER_TOKEN"
tool_timeout_sec = 900
```

The actual bearer must already be available to the Codex process through its secret environment. Do not put it in TOML, command arguments or chat. No OAuth login to the MCP server is needed. A read-only MCP context does not expose these quota-consuming tools; the server also rejects calls explicitly marked `openai/readOnly: true`.

Codex 0.165.0 source and an isolated app-server discovery fixture are recorded below. Use a fresh MCP connection/catalog after adding the entry; never patch Codex authentication state to make this work.

## OpenCode V2

Merge this server into the existing configuration. Explicitly disabling OAuth is appropriate here because ExetRouter authenticates clients with router bearers, not a public MCP OAuth issuer:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "exetrouter": {
        "type": "remote",
        "url": "https://api.example.com/mcp",
        "oauth": false,
        "headers": {"Authorization": "Bearer {env:EXETROUTER_TOKEN}"},
        "codemode": false,
        "timeout": {"execution": 900000}
      }
    }
  }
}
```

`codemode: false` puts the tools directly on the agent's native tool list; OpenCode names them `exetrouter_generate_image` and `exetrouter_edit_image`. Code Mode is also a client option, not a server requirement. Check the connection with `opencode mcp list` or `/mcps`; configuration changes reload automatically. Existing permissions can still hide/deny tools. This example follows the [V2 MCP documentation](https://opencode.ai/v2/docs/mcp-servers), not V1 configuration syntax.

Other clients can use the same Streamable HTTP URL and bearer if they support authenticated remote MCP servers. Clients that only support stdio or OAuth-only remote servers need a separate adapter; no universal compatibility claim is made.

## Adding tools later

The curated registry at the top of `src/server/mcp.rs` is the single source for tool names, descriptions, schemas, validation and dispatch. Add a supported operation there, reuse its existing authorization/accounting path, and test its contract. Do not expose arbitrary backend URLs, administrative commands or a remote shell through this registry.

Clients obtain new definitions from `tools/list` without installing another skill or hard-coding each tool. Modern discovery/tool catalogs declare `ttlMs: 0` and `cacheScope: "private"` so clients can re-fetch on next use without sharing authorization-sensitive results. This is a freshness hint, not an instruction to poll. An already-open client may cache its tool list until refresh/reconnect. This implementation deliberately advertises **`listChanged: false`**: the registry is immutable during a running server process, and there is no notification/subscription service. A server upgrade alone does not promise instantaneous changes in every open client. Client allowlists/permissions may also need review for newly added tools.

## Protocol, safety and bounds

Supported Streamable HTTP revisions: `2026-07-28`, `2025-11-25`, `2025-06-18`, `2025-03-26`. Modern clients use per-request metadata and `server/discover`; legacy clients use `initialize` and an issued `Mcp-Session-Id`. Modern header/body method, tool name and protocol version must agree, including encoded `Mcp-Name` values. Discovery/list/ping does not submit inference or refresh/spend reset credits.

Legacy session handles are user-bound authenticated random correlations, not bearer credentials or backend conversations. They retain no database/session registry and survive process restart with the existing key. Every HTTP request still authenticates its current router bearer. Only active legacy calls retain a keyed request digest and cancellation signal, bounded by concurrency; raw client IDs/reasons and arguments are not persisted. Different clients' numeric request IDs cannot cancel each other's work. `notifications/cancelled` cancels only the matching active user/session call. Modern HTTP cancellation closes the request's response stream.

Metadata returns JSON; image calls return request-scoped SSE with content-free keepalives and exactly one final JSON-RPC response. Disconnect, legacy cancellation or shutdown drops the native wait and preserves observed counters or an unknown outcome. Cancellation cannot prove OpenAI stopped work already received. Never blindly retry an interrupted generation. Backend failures become fixed tool results with `isError: true`, not false successful image blocks or echoed upstream payloads. Unsupported methods/tools/parameters fail before submission. Sessions are not resumable, `GET`/`DELETE` return 405, and no tasks, sampling, elicitation, resources, prompts or public administration are exposed.

The binding adds at most 64 concurrent MCP response streams globally, with the configured generation-per-user bound, independently of ordinary HTTP/WS slots. Image calls additionally share the existing native generation and four output-buffer slots with REST; retained response bytes keep their output permit through downstream delivery. The native 48 MiB output budget remains in force. MCP bodies have no new router-defined inference input byte/token ceiling; body reads have a ten-second deadline. Input JSON is held in memory.

Every browser `Origin` is rejected with 403. This endpoint targets native/desktop/server clients, not browser JavaScript; arbitrary reverse-proxy headers do not establish an allowed origin. Use TLS remotely and retain the normal trusted ingress Host policy. Bundled nginx templates expose exactly `/mcp` with request/response buffering, caching and payload logging disabled. Existing ingress configurations must deliberately add that location; do not expose management routes.

Calls reuse the native account pool, activation/locks, credential refresh, feature cooldowns and durable accounting, with fixed usage surfaces `mcp/generate_image` and `mcp/edit_image`. Prompts, image data, arguments/results, arbitrary client metadata and credentials are never logged or persisted by ExetRouter. Clients and OpenAI have their own retention policies. MCP adds no further schema change beyond the [reviewed schema-13 migration](native-tools.md#schema-12-to-13-reviewed-migration-and-rollback).

## Verification scope

- [MCP 2026-07-28 transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) and [tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools), with explicit legacy support rather than assuming all clients already use the latest revision.
- Codex 0.165.0, revision [`360319d8dee62d87e2d59fc0c21805d79519cfed`](https://github.com/openai/codex/tree/360319d8dee62d87e2d59fc0c21805d79519cfed): [`config/src/mcp_types.rs`](https://github.com/openai/codex/blob/360319d8dee62d87e2d59fc0c21805d79519cfed/codex-rs/config/src/mcp_types.rs), [`rmcp-client/src/rmcp_client.rs`](https://github.com/openai/codex/blob/360319d8dee62d87e2d59fc0c21805d79519cfed/codex-rs/rmcp-client/src/rmcp_client.rs), and its exact official Rust MCP SDK revision `98a54789b80110ad210495c4ad845c89e1980471`.
- Official TypeScript MCP SDK **1.32.2**, revision [`3b73021a0a2272c0c04c3917bcdb03398b3ec63f`](https://github.com/modelcontextprotocol/typescript-sdk/tree/3b73021a0a2272c0c04c3917bcdb03398b3ec63f): an isolated client discovers tools and completes generation/editing against synthetic image responses.

`cargo test --locked --test native_tools` checks discovery, protocol errors, headers/origins, authorization, image projection, read-only contexts, cancellation, isolation, restart and accounting with synthetic loopback backends. Two optional fixtures run only when explicitly supplied isolated runtimes: `EXR_MCP_SDK_ROOT` names the installed SDK package directory; `EXR_MCP_CODEX_BIN` names Codex 0.165.0 for app-server tool discovery without a model turn, in both default legacy and `mcp_2026_07_28` modes. Neither reads real client profiles or credentials. CI runs the SDK fixture with the pinned version. These checks do not prove live ChatGPT image entitlement, live generation quality, every client's image rendering, or the full newer Codex compatibility matrix. No deployment or live inference is part of these checks.
