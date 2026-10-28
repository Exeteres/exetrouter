# ExetRouter for OpenCode Desktop (experimental MVP)

A local OpenCode V2 plugin with a dynamically discovered **ExetRouter** provider,
an Overview/Usage panel, a session status pill and a Settings page. Targets the
OpenCode **2.2.0** plugin contract; GUI APIs are experimental. This package is not
published to npm. See [the integration guide](../../docs/opencode-desktop.md).

## Build

Requires Node.js 22.12+ and npm. From this directory:

```sh
npm ci --ignore-scripts
npm run typecheck
npm test
npm run build
```

Configure the local package directory in the **global** `opencode.json(c)`.
Merge the entry into existing plugins; do not replace unrelated settings:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "plugins": [
    {
      "package": "/absolute/path/to/exetrouter/clients/opencode-desktop",
      "options": {
        "baseURL": "https://api.example.com/v1",
        "exrBinary": "/absolute/path/to/exr"
      }
    }
  ]
}
```

Use **Settings > ExetRouter > Connect token** or OpenCode's account connection
UI to save an existing router bearer in OpenCode's native credential store.
`EXETROUTER_TOKEN` in the **OpenCode server's** environment is an alternative.
Do not put a token in plugin options, configuration, command arguments or logs.
This plugin never issues/rotates a token automatically.

Select an `exetrouter/<model>` through the normal model picker. No default model
is forcibly selected. **Open ExetRouter** opens the panel, or Settings when no
session is open. The side panel's **+** menu also offers ExetRouter.

When moving from a static export, remove only the redundant
`providers.exetrouter` definition after reviewing it; its explicit model overrides
can otherwise reintroduce models absent from discovery. Preserve other providers
and settings. The plugin does not remove or rewrite configuration for you.

## Options

| Option | Default | Purpose |
| --- | --- | --- |
| `baseURL` | `http://127.0.0.1:8787/v1` | Router HTTP API; remote endpoints require HTTPS. |
| `transport` | `websocket` | Native `websocket` or `http` transport. |
| `refreshSeconds` | `60` | Catalog refresh interval, 30–3600 seconds. |
| `exrBinary` | `exr` | Executable or path; an absolute path is recommended in Desktop. |
| `exrConfig` | Native exr default | Optional absolute or `~/` path to an existing exr profile. |

The dashboard uses the saved native exr profile, including SSH host-key checks
and private-label decryption. Configure that profile with exr first. The HTTP
endpoint and SSH profile are independent; the plugin cannot prove they refer to
the same deployment. Settings displays them separately. Changing plugin options
reloads the plugin; no OpenCode configuration is rewritten by the plugin.

For standalone, keep `exr` or `exr serve` running and select its actual loopback
API address. This MVP does not start/stop services or perform OAuth onboarding.

## Safety and limits

- Generation goes directly through OpenCode's native OpenAI Responses provider,
  with `store=false` and native compaction. There are no inference hooks, retries,
  prompt/response logging, or JavaScript routing proxy.
- Catalog GETs use the native connection's bearer, refuse redirects, have a
  15-second deadline and a 2 MiB metadata bound. Only model limits, modalities,
  reasoning options and names are imported; remote executable/endpoint overlays
  are ignored. New connection inventories replace rather than merge old ones.
- Passive dashboard polling runs once per desktop app every 30 seconds. It uses
  `exr doctor --json`, **not** live `limits`, so it cannot request weekly activation
  or redeem credits. Cached quota may be stale/unknown. The router's independent
  automatic metadata/reset worker remains unchanged.
- Usage loads on demand. Model/user totals are shared; token grouping is scoped
  to the native authenticated user. Unknown counters stay unknown. Known token
  totals can be incomplete when unknown/partial requests exist.
- Native reads have a 50-second deadline, a 1 MiB output bound, at most two
  concurrent subprocesses and no shell or mutation interface. Cancellation kills
  the read's process group. Child stderr and arbitrary errors are never shown.
- The status pill describes the configured pool, **not** the upstream account of
  the current session. No quota percentages are added across subscriptions.
- The GUI and native bridge run only on the local desktop. For remote OpenCode
  sessions, install/configure the server plugin and credentials there separately;
  the local panel is explicitly marked as a local-profile view.

OAuth, issuing/rotating/revoking tokens, account mutations, service lifecycle and
operator administration are outside this MVP. Use the native client/operator
workflow. No public ExetRouter management HTTP API is added.

## Native package smoke test

After building, an optional isolated test packs and installs the package, then
loads it in an exact OpenCode 2.2.0 binary against a synthetic loopback catalog:

```sh
EXETROUTER_OPENCODE_PLUGIN_BIN=/absolute/path/to/opencode npm test
```

It does not use real accounts, the normal OpenCode profile, inference or reset
credits. npm installation in the test is offline and needs dependencies already
cached by `npm ci`. The regular tests use only synthetic data and loopback HTTP.
The native test does not verify the rendered Electron UI or renew ExetRouter's
full inference compatibility matrix.
