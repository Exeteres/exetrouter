import { spawn } from "node:child_process"
import { homedir } from "node:os"
import { isAbsolute, join } from "node:path"
import type { Options } from "./options"
import type { Snapshot, Usage, UsageInput } from "./schemas"

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid exr report.")
  return value as Record<string, unknown>
}
function text(value: unknown, max = 512): string {
  if (typeof value !== "string" || value.length > max) throw new Error("Invalid exr report.")
  return value
}
function number(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) throw new Error("Invalid exr report.")
  return value
}
function nullable(value: unknown): number | null { return value == null ? null : number(value) }
function list(value: unknown, max: number): unknown[] {
  if (!Array.isArray(value) || value.length > max) throw new Error("Invalid exr report.")
  return value
}

export function snapshot(value: unknown): Snapshot {
  const root = record(value)
  const report = record("server" in root ? root.server : root)
  if (report.schema_version !== 1) throw new Error("Unsupported exr report.")
  const accounts = report.quota_accounts ?? (report.quota ? [{ label: "Account 1", quota: report.quota }] : [])
  return {
    version: text(report.version, 128),
    checkedAt: number(report.checked_at),
    configured: report.configuration_status === "configured",
    accounts: list(accounts, 4096).map((entry) => {
      const account = record(entry)
      const quota = record(account.quota)
      const preference = account.preference ? record(account.preference) : undefined
      return {
        id: number(account.id ?? 0),
        label: text(account.label),
        enabled: typeof preference?.enabled === "boolean" ? preference.enabled : null,
        locked: preference?.locked === true,
        windows: list(quota.windows, 16).map((entry) => {
          const window = record(entry)
          const remaining = number(window.remaining_percent)
          if (remaining > 100) throw new Error("Invalid exr report.")
          return {
            kind: text(window.kind, 64),
            remaining,
            minutes: nullable(window.window_minutes),
            resetAt: nullable(window.reset_at),
            observedAt: number(window.observed_at),
            status: text(window.status, 64),
          }
        }),
      }
    }),
  }
}

export function usage(value: unknown, by: UsageInput["by"]): Usage {
  const report = record(value)
  return {
    period: text(report.period, 16), timezone: text(report.timezone, 128),
    from: number(report.from_utc), to: number(report.to_utc),
    rows: list(report.rows, 16384).map((entry) => {
      const row = record(entry)
      return {
        // Never fall back to displaying a token id or ciphertext as a private token label.
        name: by === "token" ? text(row.display_name ?? "Label unavailable") : text(row.name),
        requests: number(row.requests), unknown: number(row.unknown_usage), partial: number(row.partial_usage),
        input: nullable(row.input_tokens), output: nullable(row.output_tokens), total: nullable(row.total_tokens),
      }
    }),
  }
}

/** Read-only subprocess boundary: no shell, stdin, stderr forwarding, secret output or unbounded buffering. */
export function execJSON(binary: string, args: readonly string[], signal: AbortSignal, allowNonzero = false,
  timeout = 50_000, maxBytes = 1_048_576): Promise<unknown> {
  if (signal.aborted) return Promise.reject(new Error("ExetRouter read cancelled."))
  return new Promise((resolve, reject) => {
    const env: NodeJS.ProcessEnv = { ...process.env, EXR_NO_UPDATE_CHECK: "1" }
    // Dashboard reads use native SSH/local identity, not the inference bearer.
    delete env.EXETROUTER_TOKEN
    const child = spawn(binary, [...args], {
      stdio: ["ignore", "pipe", "ignore"],
      shell: false,
      detached: process.platform !== "win32",
      env,
    })
    let size = 0
    let failed = false
    const chunks: Buffer[] = []
    const kill = () => {
      try {
        if (process.platform !== "win32" && child.pid) process.kill(-child.pid, "SIGKILL")
        else child.kill("SIGKILL")
      } catch { /* The process may already have exited. */ }
    }
    const fail = () => { failed = true; kill() }
    const timer = setTimeout(fail, timeout)
    signal.addEventListener("abort", fail, { once: true })
    // Account for cancellation between the initial check and listener registration.
    if (signal.aborted) fail()
    child.stdout!.on("data", (chunk: Buffer) => {
      size += chunk.length
      if (size > maxBytes) { fail(); return }
      if (!failed) chunks.push(chunk)
    })
    const clean = () => { clearTimeout(timer); signal.removeEventListener("abort", fail) }
    child.once("error", () => { clean(); reject(new Error("Cannot run exr. Check exrBinary in plugin options.")) })
    child.once("close", (code) => {
      clean()
      if (failed || code === null || (code !== 0 && !allowNonzero)) {
        reject(new Error("ExetRouter read failed. Check the saved exr profile and SSH key."))
        return
      }
      try { resolve(JSON.parse(Buffer.concat(chunks).toString("utf8"))) }
      catch { reject(new Error("exr did not return a supported JSON report.")) }
    })
  })
}

function expand(path: string): string { return path.startsWith("~/") ? join(homedir(), path.slice(2)) : path }

export class NativeClient {
  private active = 0
  constructor(private config: Options, private run = execJSON) {}

  private async read(args: string[], signal: AbortSignal, allowNonzero = false): Promise<unknown> {
    // Refuse excess work rather than retain an unbounded queue across desktop windows.
    if (this.active >= 2) throw new Error("ExetRouter is busy. Try again after the current read.")
    const profile = this.config.exrConfig && expand(this.config.exrConfig)
    if (profile && !isAbsolute(profile)) throw new Error("exrConfig must be an absolute path or start with ~/.")
    this.active++
    try {
      return await this.run(expand(this.config.exrBinary), [...(profile ? ["--config", profile] : []), ...args, "--json"], signal, allowNonzero)
    } finally { this.active-- }
  }

  async snapshot(signal: AbortSignal): Promise<Snapshot> {
    // Never use `limits`: it may submit a weekly activation request. Doctor is passive.
    return snapshot(await this.read(["doctor"], signal, true))
  }

  async usage(input: UsageInput, signal: AbortSignal): Promise<Usage> {
    if (!["day", "24h", "week", "month"].includes(input.period) || !["model", "user", "token"].includes(input.by)) {
      throw new Error("Invalid usage selection.")
    }
    return usage(await this.read(["usage", "--period", input.period, "--by", input.by], signal), input.by)
  }
}
