import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { exportStudioGif, publishStudioPixa } from "./studio-media.ts";
import { fakeIpc, id64 } from "./studio-testkit.ts";

const fixture = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
const packageBytes = () => new Uint8Array(Buffer.from(fixture.pixaHex, "hex"));

test("pixa publication checks exact native byte count without claiming an export record was saved", async () => {
  const ipc = fakeIpc(), bytes = packageBytes();
  ipc.on("publish_pixa", () => ({ cid: id64(1), bytes: bytes.length }));
  assert.deepEqual(await publishStudioPixa(ipc, 7, bytes), { cid: id64(1), bytes: bytes.length });
  assert.deepEqual(ipc.calls.map((call) => call.cmd), ["publish_pixa"]);
  assert.equal(ipc.calls[0].args.server, 7);
  ipc.on("publish_pixa", () => ({ cid: id64(1), bytes: bytes.length + 1 }));
  await assert.rejects(publishStudioPixa(ipc, 7, bytes), /submitted bytes/);
});

test("native GIF reveal failure remains an acknowledged save at its real path", async () => {
  const ipc = fakeIpc();
  ipc.on("export_studio_gif", () => ({ path: "Downloads/Mewtual-flipnote (1).gif", displayed: false, warning: "could not reveal", contentValidation: "matched" }));
  const result = await exportStudioGif(ipc, 7, packageBytes());
  assert.equal(result.displayed, false); assert.equal(result.warning, "could not reveal");
  assert.equal(result.path, "Downloads/Mewtual-flipnote (1).gif");
  assert.equal("name" in ipc.calls[0].args, false); assert.equal("path" in ipc.calls[0].args, false);
});

test("media input is validated before IPC and malformed GIF acknowledgements refuse", async () => {
  const ipc = fakeIpc();
  await assert.rejects(publishStudioPixa(ipc, 7, new Uint8Array([1])));
  await assert.rejects(exportStudioGif(ipc, 7, new Uint8Array([1])));
  assert.equal(ipc.calls.length, 0);
  ipc.on("export_studio_gif", () => ({ path: "x", displayed: true, warning: null, contentValidation: "unchecked" }));
  await assert.rejects(exportStudioGif(ipc, 7, packageBytes()), /validated native/);
});
