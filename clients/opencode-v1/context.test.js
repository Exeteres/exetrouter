import assert from "node:assert/strict"
import test from "node:test"
import { ExetRouterContext } from "./context.js"

const reasoning = (value, provider = "openai") => ({
  type: "reasoning",
  text: "",
  metadata: { [provider]: { itemId: "rs_fixture", reasoningEncryptedContent: value } },
})
const message = (parts, info = {}) => ({ info: { role: "assistant", ...info }, parts })
const transform = async (messages) => {
  const hooks = await ExetRouterContext()
  await hooks["experimental.chat.messages.transform"]({}, { messages })
  return messages
}

test("only plaintext compaction summary reasoning is omitted; saved history and tail are unchanged", async () => {
  const summary = { type: "text", text: "Synthetic continuation summary" }
  const saved = [
    message([reasoning("exrctx1.summary-account"), summary], { summary: true }),
    message([
      reasoning("exrctx1.tail-account"),
      { type: "tool", callID: "fixture", state: { status: "completed", output: "synthetic result" } },
    ]),
  ]
  const output = await transform(structuredClone(saved))
  assert.deepEqual(output[0], message([summary], { summary: true }))
  assert.deepEqual(output[1], saved[1])
  assert.equal(saved[0].parts.length, 2)
  assert.deepEqual(await transform(output), output)
})

test("router namespace aliases work without changing other providers or user messages", async () => {
  const foreign = reasoning("provider-owned-reasoning")
  const signed = { type: "reasoning", text: "Synthetic thinking", metadata: { anthropic: { signature: "fixture" } } }
  const messages = [
    message([reasoning("exrctx1.fixture", "custom-router"), foreign, signed], { summary: true }),
    message([reasoning("exrctx1.user")], { role: "user", summary: true }),
  ]
  const output = await transform(structuredClone(messages))
  assert.deepEqual(output[0].parts, [foreign, signed])
  assert.deepEqual(output[1], messages[1])
})

test("ordinary opaque history and portability receipts are never stripped", async () => {
  const receipts = [reasoning("exrtr1.fixture"), reasoning(null)]
  const messages = [
    message([reasoning("exrctx1.normal"), ...receipts]),
    message(receipts, { summary: true }),
  ]
  assert.deepEqual(await transform(structuredClone(messages)), messages)
})
