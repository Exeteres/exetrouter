# OpenCode Desktop integration

The optional [experimental plugin](../clients/opencode-desktop/README.md) adds a
native Overview/Usage panel, Settings page, status pill and dynamic ExetRouter
model discovery. A plugin is **not** required for inference: the existing
[configuration export](compatibility.md#opencode-setup) remains supported.

## Architecture

```text
OpenCode server plugin → authenticated /v1/models/opencode-v2
                      → provider transform → native Responses provider
                                            → ExetRouter HTTP/WS inference

Desktop window → typed IPC → Electron main → exr doctor/usage --json
                                           → standalone / restricted SSH
```

The server entry registers only the ExetRouter integration, provider and a
content-free catalog-status/refresh RPC. It neither edits global configuration
nor intercepts model requests. Account-specific inventory is bound to its
OpenCode connection, refreshed after credential changes and atomically replaced.
A temporary same-connection discovery failure keeps the last inventory marked
stale. A connection change cannot retain the previous inventory.

The desktop main process owns the bounded native-client adapter. It reuses
exr's existing profile, SSH identity/host-key verification and private metadata
handling. It never reads SQLite or OAuth credentials directly. Dashboard state
is in memory, not persisted to GUI stores. The native bridge is IPC-only: it
does not expose account management through the OpenCode HTTP RPC surface.

## Current scope

- Configure the plugin in global `opencode.json(c)`; install/build instructions
  and options are in the package README. The package is source-only, not on npm.
- Save an existing router token through OpenCode's native connection flow, or
  supply `EXETROUTER_TOKEN` to its server. The plugin does not create tokens.
- Select discovered models in the normal picker. Router metadata supplies limits,
  modalities and reasoning variants without inheriting a bundled OpenAI catalog.
- Overview reads cached diagnostics and quota observations. Usage is loaded on
  demand. No diagnostic request submits inference or activates a weekly window.
- Settings shows the HTTP endpoint and the selected native-client profile, and
  can connect a token/refresh models. Edit connection options in configuration;
  profile creation/editing remains in exr.
- Standalone requires an existing running `exr`/`exr serve`. Remote OpenCode
  execution requires its own server-side plugin configuration and credentials.

The MVP has no token issuance/rotation, OAuth flow, account mutations, installer,
service supervisor or operator administration. These require a reviewed native
bridge rather than disabling the CLI's terminal and secret-output safeguards.

## Reviewed contract and verification

Targets OpenCode **2.2.0**, source revision
`1097e2668b1e08ad63bdf0b1c4860b9bc01e420c`. Review covered GUI registries/main
process APIs, provider/integration schema and provider transforms. The SDK is
pinned to that version because the GUI API is experimental. Relevant references:
[GUI plugins](https://opencode.ai/v2/docs/build/plugins/gui/),
[server plugins](https://opencode.ai/v2/docs/build/plugins/server),
[RPC](https://opencode.ai/v2/docs/build/plugins/rpc).

Package checks cover TypeScript, the single CommonJS GUI bundle, synthetic
catalog projection, redirect/body bounds, credential-switch races, cancellation,
passive native command selection, nullable usage and stale quota formatting.
The built bundle is also evaluated with the host's two-process placeholder
model, checking window contributions and a synthetic main-process native read.
The optional native test loads a packed/installed package in an isolated 2.2.0
server against a loopback catalog without generation. It does not prove the
rendered Electron UI, live upstream inference, long sessions or compaction;
the [existing compatibility matrix](compatibility.md) retains its recorded scope.
