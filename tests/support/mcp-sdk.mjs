// Optional, isolated @modelcontextprotocol/sdk 1.32.2 fixture. Synthetic loopback only.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

let stage = "configuration";
try {
  const root = process.env.EXR_MCP_SDK_ROOT;
  const endpoint = new URL(process.env.EXR_MCP_TEST_URL);
  assert.equal(endpoint.protocol, "http:");
  assert.ok(["127.0.0.1", "[::1]"].includes(endpoint.hostname));
  const metadata = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
  assert.equal(metadata.version, "1.32.2");
  const { Client } = await import(pathToFileURL(join(root, "dist/esm/client/index.js")));
  const { StreamableHTTPClientTransport } = await import(pathToFileURL(join(root, "dist/esm/client/streamableHttp.js")));
  const client = new Client({ name: "exr-synthetic-fixture", version: "1" });
  const transport = new StreamableHTTPClientTransport(endpoint, {
    requestInit: { headers: { Authorization: `Bearer ${process.env.EXR_MCP_TEST_TOKEN}` } },
  });
  stage = "initialization";
  await client.connect(transport);
  try {
    stage = "tool discovery";
    const { tools } = await client.listTools();
    assert.deepEqual(tools.map(tool => tool.name), ["generate_image", "edit_image"]);
    stage = "generation";
    const generated = await client.callTool({ name: "generate_image", arguments: { prompt: "synthetic-private-sdk-prompt" } });
    assert.equal(generated.isError, false);
    const image = generated.content.find(item => item.type === "image");
    assert.equal(image.mimeType, "image/png");
    assert.ok(image.data.length > 16);
    stage = "editing";
    const edited = await client.callTool({ name: "edit_image", arguments: { prompt: "synthetic-private-sdk-edit", image_urls: [`data:image/png;base64,${image.data}`] } });
    assert.equal(edited.isError, false);
    assert.equal(edited.content.find(item => item.type === "image").data, image.data);
  } finally {
    await client.close();
  }
  console.log("MCP SDK 1.32.2: discovery, generation and editing passed (synthetic backend)");
} catch {
  // Never print SDK errors that might include headers, payloads or credentials.
  console.error(`MCP SDK fixture failed during ${stage}`);
  process.exitCode = 1;
}
