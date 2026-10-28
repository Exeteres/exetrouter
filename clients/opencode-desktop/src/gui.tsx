import { Plugin } from "@opencode/plugin/gui"
import { main } from "./main"
import { setup } from "./window"

export default Plugin.define({ os: ["macos", "linux"], setup, main })
