import { Rpc } from "@opencode/plugin/rpc"
import { Schema } from "effect"
import { CatalogStatus } from "./schemas"

export const ExetRouter = Rpc.define({
  id: "exetrouter.catalog",
  methods: {
    status: { input: Schema.toStandardSchemaV1(Schema.Struct({})), output: Schema.toStandardSchemaV1(CatalogStatus) },
    refresh: { input: Schema.toStandardSchemaV1(Schema.Struct({})), output: Schema.toStandardSchemaV1(CatalogStatus) },
  },
  events: {},
})
