# OpenCode V1 context compatibility

OpenCode V1 1.18.30 summarizes plaintext in an independent inference request, then
combines its summary with retained original messages. A pool can route that
summary to another account. Replaying its hidden encrypted reasoning alongside
the original tail then fails with `context_account_mismatch`; elapsed idle time
is not the cause.

`context.js` omits only ExetRouter-enveloped reasoning on V1's **plaintext
compaction summaries** from the outgoing message copy. It preserves the visible
summary, ordinary reasoning, transfer receipts, messages and tool history.
It makes no requests, writes no state and logs no content. It does not disable
server ownership checks or change Codex/V2 native compaction checkpoints.

## Install on the V1 client

From this checkout, with the affected OpenCode V1 process closed:

```sh
install -d "${XDG_CONFIG_HOME:-$HOME/.config}/opencode/plugins"
install -m 600 clients/opencode-v1/context.js "${XDG_CONFIG_HOME:-$HOME/.config}/opencode/plugins/exetrouter-context.js"
```

If `OPENCODE_CONFIG_DIR` selects a separate V1 configuration directory, use its
`plugins/` directory instead. Keep other plugins/configuration unchanged. Open
the existing session again; no database editing, token rotation or new chat is
needed for this compaction conflict. Unrelated mixed-account history without
transfer proofs remains an error.

## Offline checks

```sh
node --test clients/opencode-v1/context.test.js
EXETROUTER_OPENCODE_V1_BIN=/absolute/path/to/opencode-1.18.30 cargo test --locked --test responses opencode_v1_compaction_preserves_tail_across_independent_summary_account -- --ignored --nocapture
```

The native fixture uses isolated configuration, synthetic accounts and a loopback
backend. It first reproduces the unmodified failure with a retained tool/reasoning
tail and a summary on another account, then installs the plugin and recovers that
same session across router restart and client reopening.
It does not use real OAuth, inference or reset credits.
