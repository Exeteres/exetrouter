# Native protocol audit — October 2, 2026

This audit compares released client source with ExetRouter. It is a versioned engineering contract, not a promise that the private ChatGPT backend cannot change. A field appearing in a client proves that the client sends it; it does not by itself prove that the server requires it.

## Reviewed sources

| Project | Released revision | Scope |
| --- | --- | --- |
| [Codex](https://github.com/openai/codex/tree/a956835d020762cb2b570053af06f643a11c0ecc) | 0.160.0 | Native request construction, Lite, identity, compression and recovery. |
| [OpenCode V2](https://github.com/anomalyco/opencode/tree/527f0b931d1f9b3ebd34e106c51b31ce5db5b075) | 2.0.22 | Responses lowering, continuation, compaction and ChatGPT adapters. |
| [OpenCode V1](https://github.com/anomalyco/opencode/blob/aec0b9a6d8898f68f923aaf08b7306d931fd9d76/packages/opencode/src/plugin/openai/codex.ts) | 1.18.34 | Separate OAuth/transport implementation; October 3 mock HTTP tool/resume passed. Uses the built-in OpenAI provider and standard client retry policy. See [V1 setup](compatibility.md#v1-opencode-ai). |

Exact commits, selected file hashes and reviewed native test versions live in [protocol-sources.json](protocol-sources.json). Versions were resolved against npm stable tags or GitHub latest releases; release tags were resolved to commits. Development branches are not substituted for releases.

## Keep the three upstream contracts separate

1. Public OpenAI Platform `/v1/responses`: public API authentication, documented state/compaction/WS capabilities.
2. ChatGPT Codex `/backend-api/codex/responses`: Codex OAuth account, native fields and backend-specific transport. This is ExetRouter's upstream.
3. Registered ChatGPT token sharing: separate grant/scope and Platform resource. OpenCode V2's [chatgpt.ts](https://github.com/anomalyco/opencode/blob/527f0b931d1f9b3ebd34e106c51b31ce5db5b075/packages/core/src/plugin/provider/chatgpt.ts) implements this separately from its [Codex OAuth adapter](https://github.com/anomalyco/opencode/blob/527f0b931d1f9b3ebd34e106c51b31ce5db5b075/packages/core/src/plugin/provider/openai.ts). Its credentials and eligibility are not interchangeable with Codex OAuth.

Official [Responses request reference](https://developers.openai.com/api/reference/python/resources/responses/methods/create), [streaming](https://developers.openai.com/api/docs/guides/streaming-responses), [conversation state](https://developers.openai.com/api/docs/guides/conversation-state), [WebSocket mode](https://developers.openai.com/api/docs/guides/websocket-mode), [function calling](https://developers.openai.com/api/docs/guides/function-calling) and [compaction](https://developers.openai.com/api/docs/guides/compaction) explain the public API. They do not establish support for every public field in the private Codex backend. In particular, public WS named lanes and steering are outside our sequential native WS contract; public stored-response reconnect is outside our `store=false` contract.

## Header policy

Native identity construction is explicit in Codex [requests/headers.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/codex-api/src/requests/headers.rs). Lite, turn state and per-frame metadata are constructed in [core/client.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/client.rs). Our policy must distinguish authorization, functional continuation and optional client metadata.

| Header / metadata | ExetRouter policy | Current evidence / gap |
| --- | --- | --- |
| `Authorization`, `chatgpt-account-id` | Construct from the selected server-owned OAuth account; never forward the router bearer or client account selection. | Implemented for both transports. |
| `originator`, `User-Agent` | Identify ExetRouter honestly. | Native values differ. Synthetic direct comparisons with our originator/agent completed; exact native impersonation is not established as necessary. |
| `Content-Type`, `Accept` | HTTP JSON request and SSE upstream response. | Implemented. WS removes HTTP `stream`; upgrade headers belong to the transport. |
| `OpenAI-Beta: responses_websockets=2026-02-06` | Set for the selected native WS protocol. | Implemented. Do not equate this protocol with all current public WS extensions. |
| `session-id`, legacy `session_id` | Stable scoped identity; distinguish cache affinity from authorization/account ownership. | Session hints and cache keys are scoped independently when native children share a parent cache. Existing cache digests/account preference remain stable. |
| `thread-id` | Stable scoped thread identity. | HTTP/WS and per-frame hints now have a separate user/model HMAC domain. Parent/child/shared-cache, rotation, user isolation and restart fixtures pass. Impact on the observed stalls is not proven. |
| `x-codex-turn-state` | Preserve an issued opaque token transiently; validate owner before forwarding; persist only its keyed digest. | HTTP headers, WS handshake response.metadata after response.created and event metadata bind issued state before forwarding. WS-to-HTTP/new-WS/restart ownership fixture passes, including upgrade-header projection into a frame. |
| `x-openai-internal-codex-responses-lite` | Preserve the recognized native Lite mode independently of tool inventory. | Derived from `additional_tools`, native WS mode metadata or an explicit `true` header. See the targeted follow-up below; the original witness-only validation rejected valid prefix-free requests. |
| `x-codex-beta-features` | Advertise implemented capabilities. | `remote_compaction_v2` implemented; do not advertise arbitrary client features. |
| `x-client-request-id`, parent/subagent/turn metadata | Treat as bounded metadata, never as authorization or a reason to move accounts. | Not forwarded as arbitrary headers. Source presence alone is not proof that each is required for generation. |
| Response model/reasoning/rate-limit headers | Project a reviewed bounded subset when native client behavior needs it; exclude credentials and arbitrary payloads. | Router accounts for quotas; downstream projects bounded model/etag/reasoning metadata and validated turn state. Credentials/arbitrary headers are excluded; dedicated fixture passes. |

## Body and event policy

| Surface | Required handling for the supported contract |
| --- | --- |
| Model and generation options | Preserve the selected catalog model and forward backend options for upstream validation, including output caps. Never infer limits or silently substitute a model. Adapter structures still require validation. |
| HTTP framing | Decode supported content coding before JSON parsing. Keep wire and decoded-size bounds, decompression CPU/concurrency and error redaction independent of token windows. Authenticated identity/zstd decoding is implemented with wire/output, history-window, slot and cooperative CPU bounds; malformed/truncated/expansion fixtures pass. |
| HTTP Responses | Normalize string input to a user item and message `system` to `developer`; preserve item order/content. Default `store=false` without overriding an explicit value; request upstream SSE with `stream=true`; project JSON only when requested downstream. |
| Native standard Responses | Preserve tools, tool choice, parallel-tool flag, include, reasoning and text controls. Preserve unknown native item extensions transiently rather than rebuilding them as Chat messages. |
| Responses Lite | Preserve `additional_tools` and its ordered developer prefix, namespaced/custom tools and rebuilt instruction items. Do not flatten this into classic function tools. |
| Tool cycle | Preserve `call_id`, name/namespace, function `arguments`, custom tool `input`, matching outputs and ordering. `custom_tool_call` is not an empty-JSON `function_call`. Never invent successful tool results to repair a history. |
| Reasoning/history | Preserve complete output items, assistant `phase`, encrypted reasoning/compaction and `encrypted_function_args`. Bind opaque values to their owner before exposing them; never decode them or persist their payload. |
| Native WS | Sequential `response.create`, same-socket `previous_response_id`, generated-false warmup and account ownership. Remove HTTP `stream`. A new socket needs full context and validated opaque ownership. |
| Streaming | Preserve raw event bytes, order, IDs, deltas, item-done and terminal events. Fragments may split UTF-8/JSON or SSE delimiters. A tool-input delta or socket EOF is not completion. |
| Failure | Terminal failure/incomplete remains distinct from success. Interrupted usage remains unknown unless counters were confirmed. Keep bounded upstream deadlines even when downstream receives content-free heartbeats. |
| Compaction | Our compact endpoint adds `compaction_trigger` to one upstream Responses request and projects its checkpoint. OpenCode V2 supports this trigger separately from a standalone compact endpoint. Do not substitute the public compact algorithm or assume it returns only one item. |

Codex [common.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/codex-api/src/common.rs) and [client.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/client.rs) define these native fields; [http-client/request.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/http-client/src/request.rs) implements `zstd` body encoding. OpenCode's [Responses lowering](https://github.com/anomalyco/opencode/blob/527f0b931d1f9b3ebd34e106c51b31ce5db5b075/packages/ai/src/protocols/openai-responses.ts) and [continuation driver](https://github.com/anomalyco/opencode/blob/527f0b931d1f9b3ebd34e106c51b31ce5db5b075/packages/ai/src/protocols/open-responses-continuation.ts) preserve complete history and only send an incremental suffix after checking the preceding request/output.

The additional pinned [responses_metadata.rs](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/core/src/responses_metadata.rs) defines duplicate session/thread identities inside serialized turn metadata, alongside tool namespace inventory. ExetRouter now projects the same scoped identities there and in WS continuations; its earlier header/frame-only projection was inconsistent. This fixes a concrete contract mismatch. It does not independently prove the cause of an observed backend stall.

## Acceptance status

Bounded compression, scoped identities, turn-state ownership, Lite/metadata negotiation and broader native item fixtures are implemented. The [development plan](development-plan.md#sustained-sessions-and-failure-behavior) tracks remaining acceptance: full model windows, large tool output, multi-hour sessions, saturation and actual upstream outages.

## Targeted Lite follow-up — October 23, 2026

The current stable Codex npm release was resolved to **0.164.0**, tag `rust-v0.164.0`, commit `6aaac694803004a63c6f393d153c96d22521a6b9`. This is a targeted source review, not a replacement for the broader execution matrix above or verification of Codex Desktop's bundled runtime.

In that revision, [client.rs](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/core/src/client.rs) adds an `additional_tools` prefix only when the Lite prompt's tool list is nonempty. Its HTTP options still send the Lite header according to the model's mode, independently of that prefix. [V2 compaction](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/core/src/compact_remote_v2.rs) uses ordinary Responses streaming; background compaction uses the HTTP executor. A missing visible tool inventory therefore does not prove that an explicit Lite request is invalid.

The router accepts only the exact explicit mode `true` and constructs the corresponding upstream header, including replacement WS handshakes. It does not synthesize tools, copy arbitrary headers or bypass opaque-context/turn-state ownership. Offline fixtures cover prefix-free HTTP compaction through both Responses routes, subsequent checkpoint use, WS reconnection and quota transfer. They do not constitute a Desktop execution test.

The same `client.rs` keeps the first `x-codex-turn-state` in a turn-scoped `OnceLock` and replays it throughout that turn. A separate offline fixture reproduces the exact mixed-account error when a WS quota or soft-threshold transfer is followed by full input containing new-account output and that original token. The router previously combined both owners before applying its existing retired-token cleanup. It now authenticates the token and removes already-retired state first. Unknown state, unrelated account conflicts and ordinary affinity remain enforced; this does not prove that every observed context mismatch has that cause.

## Targeted WebSocket compression follow-up — October 26, 2026

Stable npm metadata still resolves Codex to **0.164.0**, tag `rust-v0.164.0`, commit `6aaac694803004a63c6f393d153c96d22521a6b9`. Its [Responses WS configuration](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/codex-api/src/endpoint/responses_websocket.rs) enables `ExtensionsConfig.permessage_deflate` with `DeflateConfig::default()`. Its [Cargo workspace](https://github.com/openai/codex/blob/6aaac694803004a63c6f393d153c96d22521a6b9/codex-rs/Cargo.toml) pins OpenAI's [tokio-tungstenite fork](https://github.com/openai-oss-forks/tokio-tungstenite/tree/0e5b2d73aa18dd9f0a50ee9ff199d5aef7594186) and [tungstenite fork](https://github.com/openai-oss-forks/tungstenite-rs/tree/4fffad30fe373adbdcffab9545e9e9bf4f2fc19f), with the `deflate` feature. The crates.io transport previously used by the router has no such implementation.

The router now pins those same revisions for its upstream transport only. Review covered negotiation parameters, message encoding and decoded-size enforcement. Raw `Message::Frame` writes bypass the fork's message compressor, so negotiated sockets use `Message::Text`; plain sockets retain continuation frames. Requests are never replayed or moved to another account for compression. The incoming upstream message limit remains 1 MiB after decompression.

Offline fixtures send an unchanged multibyte request larger than 16 MiB through a compression-negotiating peer whose individual wire-frame ceiling is 256 KiB, then continue a second turn with context takeover and compressed replies. A declining peer verifies plain fragmentation; a highly compressed response exceeding the decoded event bound is rejected. This is targeted transport evidence, not a full native-client matrix or proof that OpenAI accepts a particular large request. A provider can still impose decoded message/context limits.

## Targeted large-request continuation follow-up — October 26, 2026

The OpenCode V2 release tag **v2.1.4** resolves to `68bc26b7da1990ce10ffecfbc1e09eb13aa5d824`. Its [continuation driver](https://github.com/anomalyco/opencode/blob/68bc26b7da1990ce10ffecfbc1e09eb13aa5d824/packages/ai/src/protocols/open-responses-continuation.ts) classifies `previous_response_not_found` as rejected delivery with `retry-full`. Its [session transport](https://github.com/anomalyco/opencode/blob/68bc26b7da1990ce10ffecfbc1e09eb13aa5d824/packages/core/src/session/model-transport.ts) clears the checkpoint and closes the channel after an error. The [step](https://github.com/anomalyco/opencode/blob/68bc26b7da1990ce10ffecfbc1e09eb13aa5d824/packages/core/src/session/runner/step.ts) permits this recovery only before output, and the [runner](https://github.com/anomalyco/opencode/blob/68bc26b7da1990ce10ffecfbc1e09eb13aa5d824/packages/core/src/session/runner/llm.ts) permits one continuation recovery per step. This is a targeted source review, not a refreshed full native-client execution matrix or a compatibility claim about every client version.

For large complete generations, the router selects HTTP before upstream submission while retaining the native client-facing WS. It does not retry a submitted WS request after close 1009. HTTP cannot resolve socket-local response IDs. The initial implementation's 16 MiB uncompressed recovery window did not retain larger input; it refused unavailable HTTP continuations with `previous_response_not_found` before creating a usage row, relying on client-controlled full reconstruction. A normal full request/reconnect preserves ownership, selected-account eligibility and scoped identity. The follow-up below replaces repeated full reconstruction for large recoverable histories.

Offline fixtures cover unchanged multibyte input larger than 16 MiB over HTTP/compact/Chat/native WS; native warmup followed by HTTP selection; Lite tools, reasoning/encrypted arguments and tool-result forwarding; response-header metadata and known usage; local incremental refusal followed by full input on the same and a new client WS; HTTP 401/422/429/503, no size-driven account transfer/replay, and disconnect cancellation while awaiting HTTP headers. Real OpenAI acceptance of a particular large history remains unverified by these fixtures; HTTP has its own provider limits.

## Large-window recovery follow-up — October 27, 2026

The installed OpenCode runtime still reports **2.1.4**. Sanitized operational metadata showed successful large HTTP-backed WS generations followed by short incremental frames, channel closure and repeated full reconnections; eventually a local continuation refusal reached the client. This establishes repeated recovery, not the private reason the client's last recovery stopped. A source-level one-recovery allowance is not proof that every runtime path can tolerate a router requiring full reconstruction after every tool turn.

The router now compresses large completed latest windows in transient memory, retaining the same 16 MiB per-window/64 MiB shared representation budgets, with a separately bounded 64 MiB decoded window. This is not upstream wire compression and does not change input admission, account selection, payload persistence or inference retry policy. Older branches and exhausted optional recovery capacity still need client-carried full input.

Offline regression sends six consecutive native tool deltas after a full request larger than 16 MiB, requiring every turn to complete on the same client WS without continuation errors or full resends. It checks exact ordered UTF-8/tool/opaque history, tools, identities, Lite metadata and accounting, then reconnects and recovers a large incremental transfer after account deactivation. Unit fixtures verify compressed storage/decoded bounds, corruption and exact-size checks, global reservation release, and insufficient recovery capacity without modifying the original delta. These fixtures do not submit real inference or prove every native-client recovery path.

## Evidence and repeatable review

The October 2 offline matrix passed four native tool cycles, two HTTP rejection cases and six WS interruption cases, alongside 206 ordinary tests and static/publication checks. Authorized real tool/compaction/restart runs and failed probes are recorded once in [live testing](live-testing.md#measured-follow-up-2026-10-02). The later [public-ingress matrix](live-testing.md#measured-deployment-continuity-2026-10-02) passed two observed compaction cycles for both transports/clients, plus a separate 217,156-token Codex WS case. These results do not establish multi-hour reliability or prove the private-backend cause of earlier stalls.

Review source drift explicitly:

```sh
python3 scripts/check-protocol-sources.py --fetch --check-current
python3 scripts/check-protocol-sources.py
```

The first command downloads only pinned public reference files, verifies SHA-256 and compares current stable release versions/commits. The second verifies the cached references offline. Neither performs inference or executes downloaded code. A version or tag change fails the current check; inspect the changed source and update the manifest deliberately. The Rust native fixture reads its expected versions from that reviewed manifest, not registry `latest` at test time.

Then fetch verified client runtimes and run the isolated mock matrix as documented in [compatibility.md](compatibility.md). Live tests remain separately authorized and bounded under [live-testing.md](live-testing.md). Review both success and failure traces with synthetic inputs; never capture real conversations or credential-bearing traffic. Maintain the actual support contract in [openai-api.md](openai-api.md) and [upstream-contract.md](upstream-contract.md), and track completion in [development-plan.md](development-plan.md).
