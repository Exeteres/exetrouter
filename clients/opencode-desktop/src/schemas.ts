import { Schema } from "effect"

export const CatalogStatus = Schema.Struct({
  state: Schema.Literals(["needs_token", "ready", "stale", "error"]),
  baseURL: Schema.String,
  models: Schema.Number,
  fetchedAt: Schema.NullOr(Schema.Number),
})
export type CatalogStatus = typeof CatalogStatus.Type

export const Window = Schema.Struct({
  kind: Schema.String,
  remaining: Schema.Number,
  minutes: Schema.NullOr(Schema.Number),
  resetAt: Schema.NullOr(Schema.Number),
  observedAt: Schema.Number,
  status: Schema.String,
})
export type Window = typeof Window.Type

export const Snapshot = Schema.Struct({
  version: Schema.String,
  checkedAt: Schema.Number,
  configured: Schema.Boolean,
  accounts: Schema.Array(Schema.Struct({
    id: Schema.Number,
    label: Schema.String,
    enabled: Schema.NullOr(Schema.Boolean),
    locked: Schema.Boolean,
    windows: Schema.Array(Window),
  })),
})
export type Snapshot = typeof Snapshot.Type

export const DashboardState = Schema.Struct({
  snapshot: Schema.NullOr(Snapshot),
  refreshing: Schema.Boolean,
  error: Schema.NullOr(Schema.String),
})
export type DashboardState = typeof DashboardState.Type

export const UsageInput = Schema.Struct({
  period: Schema.Literals(["day", "24h", "week", "month"]),
  by: Schema.Literals(["model", "user", "token"]),
})
export type UsageInput = typeof UsageInput.Type

export const Usage = Schema.Struct({
  period: Schema.String,
  timezone: Schema.String,
  from: Schema.Number,
  to: Schema.Number,
  rows: Schema.Array(Schema.Struct({
    name: Schema.String,
    requests: Schema.Number,
    unknown: Schema.Number,
    partial: Schema.Number,
    input: Schema.NullOr(Schema.Number),
    output: Schema.NullOr(Schema.Number),
    total: Schema.NullOr(Schema.Number),
  })),
})
export type Usage = typeof Usage.Type
