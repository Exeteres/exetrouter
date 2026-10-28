import assert from "node:assert/strict"
import { isBuiltin, createRequire } from "node:module"
import { mkdtemp, writeFile, rm, readFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { test } from "node:test"
import { runInNewContext } from "node:vm"
import type { Plugin } from "@opencode/plugin/gui"
import { Rpc } from "@opencode/plugin/rpc"
import * as Effect from "effect"
import { doctor, usage } from "./fixtures"

const require = createRequire(import.meta.url)
// Mirrors the host's inert placeholders: module-scope Solid templates in main,
// Node imports in windows. Components themselves are not rendered by this smoke test.
const inert: unknown = new Proxy(function () {}, {
  get: () => inert, apply: () => inert, construct: () => inert as object,
})

async function bundle(main: boolean) {
  const code = await readFile(new URL("../dist/gui-experimental.cjs", import.meta.url), "utf8")
  const tokens = Object.fromEntries(["Command", "MenuItem", "Panel", "SettingsPage", "ComposerPill", "Style"].map((id) => [id, { id }]))
  const sdk = {
    ...tokens,
    Plugin: { define: (value: unknown) => value },
    Ipc: { define: (spec: { id: string }) => ({ id: spec.id, spec }) },
    createLatest: () => { throw new Error("View should only fetch when rendered.") },
  }
  const exports: { default?: Plugin.Definition } = {}
  const timers = new Set<() => void>()
  runInNewContext(code, {
    exports,
    require(id: string) {
      if (id === "@opencode/plugin/gui") return sdk
      if (id === "solid-js" || id === "solid-js/web") return inert
      if (isBuiltin(id)) return main ? require(id) : inert
      if (id === "effect") return Effect
      if (id === "@opencode/plugin/rpc") return { Rpc }
      throw new Error("Unexpected external module in GUI bundle.")
    },
    process: main ? process : undefined,
    Buffer: main ? Buffer : undefined,
    URL, AbortSignal, AbortController, setTimeout, clearTimeout,
    setInterval(fn: () => void) { timers.add(fn); return fn },
    clearInterval(fn: () => void) { timers.delete(fn) },
  })
  assert.ok(exports.default)
  return { plugin: exports.default, timers }
}

test("the built GUI evaluates in a window with inert Node modules and registers only UI contributions", async () => {
  const { plugin } = await bundle(false)
  const contributions = new Map<string, unknown[]>()
  const opened: string[] = []
  const ctx = {
    id: "exetrouter", options: {},
    add(token: { id: string }, item: unknown) {
      contributions.set(token.id, [...(contributions.get(token.id) ?? []), typeof item === "function" ? item() : item])
      return () => {}
    },
    sessions: { current: () => undefined },
    layout: { settings: (id: string) => opened.push(id) },
    main: () => ({ state: () => undefined }),
  }
  plugin.setup!(ctx as unknown as Parameters<Plugin.Setup>[0])
  assert.deepEqual([...contributions.keys()].sort(), ["Command", "ComposerPill", "MenuItem", "Panel", "SettingsPage", "Style"].sort())
  const command = contributions.get("Command")![0] as { run(): void }
  command.run()
  assert.deepEqual(opened, ["exetrouter"])
})

test("the built main entry polls a synthetic exr once for all windows, publishes state, and cleans up", async () => {
  const root = await mkdtemp(join(tmpdir(), "exetrouter-gui-main-"))
  const controller = new AbortController()
  let cleanup: (() => void | Promise<void>) | void = undefined
  try {
    const binary = join(root, "exr-fixture")
    await writeFile(binary, `#!/usr/bin/env node\nprocess.stdout.write(JSON.stringify(process.argv.includes("doctor") ? ${JSON.stringify(doctor())} : ${JSON.stringify(usage())}));\n`, { mode: 0o700 })
    const { plugin, timers } = await bundle(true)
    let impl: { state(): { snapshot: { version: string } | null }; refresh(): Promise<void> } | undefined
    const ctx = {
      options: { exrBinary: binary }, signal: controller.signal,
      provide(_token: unknown, implementation: typeof impl) { impl = implementation; return { changed() {} } },
    }
    cleanup = await plugin.main!(ctx as unknown as Parameters<Plugin.MainSetup>[0])
    assert.equal(timers.size, 1)
    for (let i = 0; i < 100 && !impl?.state().snapshot; i++) await delay(10)
    assert.equal(impl?.state().snapshot?.version, "0.0.0-test")
    const first = impl!.refresh()
    assert.equal(impl!.refresh(), first)
    await first
    controller.abort()
    await cleanup?.()
    cleanup = undefined
    assert.equal(timers.size, 0)
  } finally {
    controller.abort()
    await cleanup?.()
    await rm(root, { recursive: true, force: true })
  }
})
