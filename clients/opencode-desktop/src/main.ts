import type { Plugin } from "@opencode/plugin/gui"
import { Dashboard } from "./contract"
import { NativeClient } from "./native"
import { options } from "./options"
import type { DashboardState } from "./schemas"

export const main: Plugin.MainSetup = (ctx) => {
  const client = new NativeClient(options(ctx.options))
  let state: DashboardState = { snapshot: null, refreshing: false, error: null }
  let pending: Promise<void> | undefined
  const refresh = () => {
    pending ??= (async () => {
      state = { ...state, refreshing: true }
      bridge.changed()
      try {
        const snapshot = await client.snapshot(ctx.signal)
        state = { snapshot, refreshing: false, error: null }
      } catch {
        state = { ...state, refreshing: false, error: "Snapshot unavailable. Check exrBinary, the saved exr profile and SSH key." }
      }
      if (!ctx.signal.aborted) bridge.changed()
    })().finally(() => { pending = undefined })
    return pending
  }
  const bridge = ctx.provide(Dashboard, {
    state: () => state,
    refresh,
    usage: (input, caller) => client.usage(input, AbortSignal.any([ctx.signal, caller.signal])),
  })
  // One passive poll for the whole app, not one subprocess per window.
  const timer = setInterval(() => { void refresh() }, 30_000)
  void refresh()
  return async () => { clearInterval(timer); await pending }
}
