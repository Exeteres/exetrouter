import { Ipc } from "@opencode/plugin/gui"
import { DashboardState, Usage, UsageInput } from "./schemas"

export const Dashboard = Ipc.define({
  id: "exetrouter.dashboard",
  state: DashboardState,
  methods: {
    refresh: {},
    usage: { input: UsageInput, output: Usage },
  },
})
