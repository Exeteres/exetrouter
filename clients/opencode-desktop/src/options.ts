export interface Options {
  baseURL: string
  transport: "websocket" | "http"
  refreshSeconds: number
  exrBinary: string
  exrConfig?: string
}

export function apiURL(value: string): string {
  try {
    const url = new URL(value)
    const loopback = url.hostname === "localhost" || url.hostname === "127.0.0.1" || url.hostname === "[::1]"
    if ((url.protocol !== "https:" && !(url.protocol === "http:" && loopback)) ||
      url.username || url.password || url.search || url.hash ||
      /[\s\\\u0000-\u001f\u007f]/.test(value) || !url.pathname.replace(/\/+$/, "").endsWith("/v1")) throw 0
    return url.href.replace(/\/+$/, "")
  } catch {
    throw new Error("Set baseURL to an HTTPS /v1 endpoint, or a loopback HTTP /v1 endpoint, without credentials, query or fragment.")
  }
}

export function options(input: Readonly<Record<string, unknown>>): Options {
  // Secrets belong to OpenCode connections, never plugin options or GUI stores.
  if (Object.keys(input).some((key) => !["baseURL", "transport", "refreshSeconds", "exrBinary", "exrConfig"].includes(key))) {
    throw new Error("Unsupported ExetRouter option. Store the router token in an OpenCode connection.")
  }
  const text = (key: string, fallback?: string): string | undefined => {
    const value = input[key] ?? fallback
    if (value === undefined) return undefined
    if (typeof value !== "string" || !value || /[\u0000-\u001f\u007f]/.test(value)) throw new Error("Invalid ExetRouter option.")
    return value
  }
  const transport = input.transport ?? "websocket"
  const refreshSeconds = input.refreshSeconds ?? 60
  if (transport !== "websocket" && transport !== "http") throw new Error("transport must be websocket or http.")
  if (typeof refreshSeconds !== "number" || !Number.isInteger(refreshSeconds) || refreshSeconds < 30 || refreshSeconds > 3600) {
    throw new Error("refreshSeconds must be between 30 and 3600.")
  }
  return {
    baseURL: apiURL(text("baseURL", "http://127.0.0.1:8787/v1")!),
    transport,
    refreshSeconds,
    exrBinary: text("exrBinary", "exr")!,
    exrConfig: text("exrConfig"),
  }
}
