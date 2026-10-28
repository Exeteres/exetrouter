// V1 compaction summarizes plaintext independently, then replays the summary
// beside an older tail. The summary's hidden reasoning may belong to a different
// pool account; its visible text is the complete continuation summary already.
export const ExetRouterContext = async () => ({
  "experimental.chat.messages.transform": async (_input, output) => {
    for (const message of output.messages) {
      if (message.info.role !== "assistant" || message.info.summary !== true) continue
      message.parts = message.parts.filter((part) => {
        if (part.type !== "reasoning") return true
        return !Object.values(part.metadata ?? {}).some(
          (metadata) =>
            typeof metadata?.reasoningEncryptedContent === "string" &&
            metadata.reasoningEncryptedContent.startsWith("exrctx1."),
        )
      })
    }
  },
})
