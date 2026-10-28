export function catalog() {
  return { providers: { exetrouter: {
    name: "ExetRouter", package: "@opencode/ai/providers/openai/responses",
    models: { "test-reasoner": {
      name: "Synthetic Reasoner", limit: { context: 100000, input: 80000, output: 0 },
      capabilities: { tools: true, input: ["text", "image"], output: ["text"] },
      settings: { reasoningEffort: "low", reasoningSummary: "none", textVerbosity: "low" },
      variants: ["low", "high", "future"].map((id) => ({ id, settings: { reasoningEffort: id } })),
    } },
  } } }
}

export function doctor() {
  return {
    schema_version: 1, version: "0.0.0-test", checked_at: 1800000000, configuration_status: "configured",
    quota_accounts: [{ id: 1, label: "Synthetic account", preference: { enabled: true, locked: false }, quota: {
      status: "current", windows: [{ kind: "primary", remaining_percent: 0, window_minutes: 480,
        reset_at: 1800003600, observed_at: 1800000000, status: "current" }],
    } }],
  }
}

export function usage() {
  return { period: "day", timezone: "UTC", from_utc: 1800000000, to_utc: 1800086400, rows: [
    { name: "test-reasoner", requests: 2, unknown_usage: 1, known_usage: 1, partial_usage: 0,
      input_tokens: 0, output_tokens: null, total_tokens: null },
  ], timeline: [] }
}

export function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
