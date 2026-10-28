# Native Codex images and standalone search

These routes use **ChatGPT Codex OAuth only**, at the existing Codex backend. They do not use Platform keys, another provider, a conversational model to emulate image generation, or a router-hosted tool execution loop.

## Image API

`POST /v1/images/generations` accepts JSON with `model: "gpt-image-2"`, a nonempty `prompt`, and optional `background`, `quality`, `size`, and `n: 1`. `POST /v1/images/edits` additionally requires one to five `images: [{"image_url": "data:image/png;base64,..."}]`; HTTPS image URLs are also forwarded without fetching them. Unknown top-level fields, multipart uploads, streaming, multiple outputs and backend `file_id` references are not supported. Upstream validates image formats, dimensions and model options.

The unary JSON response contains `created` and one `data` entry with `b64_json` and optionally `generation_id`; other successful response metadata is preserved. The router validates canonical base64 without allocating a second decoded image. It never saves the image or prompt. Clients decode and save the image themselves. A malformed or incomplete upstream response is not successful generation.

Image selection respects active account state, operator policy, credential refresh and operational backoff. Known Free-plan credentials are excluded; unknown entitlement is validated upstream. Image models are not fabricated into the conversational model catalog. Image quota is not inferred from the ordinary text quota. A text quota cooldown alone does not block images, and an image rejection does not exhaust ordinary Codex or trigger reset-credit spending.

### Agent integration and the native client gate

**Use the [MCP endpoint](mcp.md) for agent discovery and invocation.** It exposes `generate_image` and `edit_image` with schemas/instructions and returns image content in unmodified MCP clients. No helper or skill is required for that path. The built-in Codex extension below is a separate client feature, not a blocker for MCP.

Codex 0.165.0's app-server image extension calls these same provider-relative endpoints and saves the returned image. However, its tool-planning gate requires actor authorization or a Codex-backed OpenAI authentication mode, in addition to vision capability and a non-Free plan. A custom provider's `env_key` router bearer does **not** satisfy this gate. Adding these routes alone therefore does not make `image_gen.imagegen` appear in an unmodified Codex client.

Do not fake OAuth state, copy `auth.json`, set `requires_openai_auth` to bypass the restriction, or claim that naming a provider `OpenAI` enables images. Enabling that specific built-in extension still needs an explicit upstream client capability for image-capable custom providers, followed by native-client validation. MCP tools work independently; no Codex client patch is installed by this change.

For an agent with shell access, the bundled [exr skill](../skills/exr/SKILL.md) includes a Python 3 helper. With an existing router bearer in `EXETROUTER_TOKEN` and the actual API URL in `EXETROUTER_BASE_URL` (including `/v1`), run from the repository:

```sh
python3 skills/exr/scripts/image.py --output ./generated.png <<'PROMPT'
A small red robot watering a garden, clean illustrated style.
PROMPT
```

Add `--reference ./input.png` for editing; repeat for up to five local PNG/JPEG/WebP files. The helper sends inline images without uploads, uses HTTPS except on loopback, refuses redirects and existing output files, writes only the decoded image with private permissions, and prints JSON containing its path and MIME type. It does not print the bearer, base64 or upstream error body. It never retries automatically. The agent can inspect the returned file with its normal image viewer. Installing/enabling the skill or changing another client's configuration remains a separate user-directed action.

## Standalone web search

`POST /v1/alpha/search` accepts a session `id`, catalog `model`, a `commands` object, and optional `reasoning`, `input`, `settings` and `max_output_tokens`. Commands/settings pass to the native backend; they are not translated into a tool emulation prompt. This is Codex's internal standalone web-search contract, not a public OpenAI Platform search API. Input may be a string or native item array; opaque context requires the existing user-owned router proofs. Files and saved backend conversations are not accepted.

The router forwards the plaintext `output` and optional structured `results`. It omits `encrypted_output`: the reviewed extension consumes plaintext, and this route does not issue a new opaque continuation proof. Literal HTTP/HTTPS URLs may start a session; backend `ref_id` values require an already-bound session.

Session IDs become user-scoped HMACs before forwarding. Only `(user, digest, account)` is persisted, never raw IDs, queries, URLs, backend references or results. Bindings survive bearer rotation and restart, including concurrent first requests. Search also respects normal model availability and text quota. Backend-owned search state is pinned: quota, deactivation, changed routing preferences or operational failures do **not** migrate an existing session. Start an independent session without old backend references on another eligible account instead. OpenAI may expire its own references.

The registry holds at most 4,096 sessions globally. It does not silently evict or rebind old sessions; exhaustion rejects new sessions with `search_session_capacity` before inference. This finite registry is a current limitation, not unlimited lifetime search history.

Codex's standalone-search path also depends on its feature/provider configuration (including `supports_standalone_web_search`); it is distinct from a Responses built-in web-search tool. This change does not modify a user's client configuration or claim that all clients automatically select the new route.

For the reviewed Codex 0.165.0 profile, merge `standalone_web_search = true` into `[features]` and `supports_standalone_web_search = true` into `[model_providers.exetrouter]` if standalone search is wanted. Preserve the existing bearer, URLs, catalog and unrelated settings. Check the installed client's configuration contract before applying this to another version.

## Bounds, errors and accounting

Both routes require a valid router bearer and share user/global generation concurrency. Four global native-output slots remain held for the lifetime of retained downstream body bytes. Successful JSON output is bounded to 48 MiB for images and 8 MiB for search; error parsing is bounded to 64 KiB. These are output/resource bounds, not new inference input byte/token ceilings. Ordinary Responses/Chat event/output bounds are unchanged.

Every possible submission has a durable usage record. Only observed numeric token counters are recorded; omitted image/search usage stays unknown, not zero. Shutdown/client cancellation cancels the wait and finalizes an unknown outcome; restart recovers unfinished records normally. Requests are never automatically replayed or sent to a different account after submission. A disconnect cannot prove OpenAI cancelled work already received.

HTTP errors retain their status with fixed redacted bodies, except upstream 401 becomes `upstream_authentication_error` (502) and schedules credential refresh for a separate next request. A 429 creates a durable account/feature pause using validated `Retry-After`/error reset information, or a local 60-second fallback. Search may also use exhausted general-Codex window headers; no image-meter prefix or remaining percentage is invented. HTTP 5xx uses a feature pause (five seconds without retry/reset information). Concurrent success never clears a pause; credential refresh does not erase it. Existing authentication/operational protections still apply to all features. These new pauses do not spend credits or change the general text quota table.

## Source and validation scope

Targeted review: Codex **0.165.0**, commit [`360319d8dee62d87e2d59fc0c21805d79519cfed`](https://github.com/openai/codex/tree/360319d8dee62d87e2d59fc0c21805d79519cfed), released October 27, 2026:

- [`codex-api/src/images.rs`](https://github.com/openai/codex/blob/360319d8dee62d87e2d59fc0c21805d79519cfed/codex-rs/codex-api/src/images.rs) and [`endpoint/images.rs`](https://github.com/openai/codex/blob/360319d8dee62d87e2d59fc0c21805d79519cfed/codex-rs/codex-api/src/endpoint/images.rs): JSON generation/edit requests and unary image responses.
- [`codex-api/src/search.rs`](https://github.com/openai/codex/blob/360319d8dee62d87e2d59fc0c21805d79519cfed/codex-rs/codex-api/src/search.rs), [`endpoint/search.rs`](https://github.com/openai/codex/blob/360319d8dee62d87e2d59fc0c21805d79519cfed/codex-rs/codex-api/src/endpoint/search.rs) and [`app-server/tests/suite/v2/web_search.rs`](https://github.com/openai/codex/blob/360319d8dee62d87e2d59fc0c21805d79519cfed/codex-rs/app-server/tests/suite/v2/web_search.rs): standalone search, including a custom bearer provider.
- [`core/src/tools/spec_plan.rs`](https://github.com/openai/codex/blob/360319d8dee62d87e2d59fc0c21805d79519cfed/codex-rs/core/src/tools/spec_plan.rs): native image authorization gate.

`tests/native_tools.rs` uses synthetic loopback backends for forwarding, isolation, restart, feature pauses, separate-request refresh, cancellation, output bounds/concurrency and unknown usage. `scripts/test-native-tools.py` checks the helper without real credentials or inference. This is source-derived/offline evidence, not a live image-generation or complete Codex 0.165.0 compatibility claim. The existing broader [compatibility matrix](compatibility.md) retains its recorded versions. Voice/Realtime and transcription are not implemented.

## Schema 12 to 13: reviewed migration and rollback

Migration 013 adds `native_feature_pauses` (at most two rows per account) and `native_search_sessions` (bounded as above). Existing OAuth ciphertext/generations, users, tokens, activation policies, reset ledger, usage and keys remain unchanged. No deployment is performed automatically.

Obtain operator approval, inspect the actual host, stop the previous runtime, take and verify a consistent private backup including both keys, and keep the exact previous binaries/image and configuration. Deploy matching server/gateway/standalone versions under the normal reviewed migration procedure; routine Docker updates must refuse an unreviewed schema mismatch. Check schema 13, integrity, foreign keys and preserved authentication/policies with health/read-only diagnostics, without inference or deliberate credit spending.

For rollback, stop the new runtime and back up its current state/keys first. After confirming exactly the migration-013 structures and no later schema, retain both additive tables and their rows, and set `PRAGMA user_version=12` in a reviewed offline transaction. Restore the previous server/gateway binaries together; native image/search routes will no longer exist. Do not restore an old database merely to roll back code: that can discard usage or rotated OAuth credentials. Older code ignores the retained tables. Re-upgrade uses `CREATE TABLE IF NOT EXISTS` and preserves session ownership/pauses. Finish with integrity/foreign-key checks and read-only diagnostics.
