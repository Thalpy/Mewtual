import assert from "node:assert/strict";
import test from "node:test";
import { createVisualFileUploads } from "./visual-file-uploads.ts";
import { base64ToBytes, bytesToBase64 } from "./studio-native.ts";
import { uploadContract } from "./transfer-visual.ts";

test("visual Studio attachment uses the real bounded streaming contract and a distinct fileshare CID", () => {
  const bridge = createVisualFileUploads("a".repeat(64));
  const call = (command: string, args: Record<string, unknown>) => { const result = bridge.response(command, {server: 1, ...args}); assert.ok(result.handled); return result.value; };
  const bytes = new Uint8Array(70_000).fill(37);
  const ticket = uploadContract(call("begin_file_upload", {uploadId: "post", mime: "application/x-mewtual-pixa", size: bytes.length}));
  assert.equal(ticket.chunkTotal, 2);
  assert.throws(() => call("finish_file_upload", {token: ticket.token, name: "Test.pixa", path: "embed/me"}), /Incomplete/);
  assert.throws(() => call("push_file_chunk", {token: ticket.token, offset: 1, data: "AA=="}), /slice/);
  for (let offset = 0; offset < bytes.length; offset += ticket.sliceBytes) call("push_file_chunk", {token: ticket.token, offset, data: bytesToBase64(bytes.slice(offset, offset + ticket.sliceBytes))});
  const cid = call("finish_file_upload", {token: ticket.token, name: "Test.pixa", path: "embed/me"}) as string;
  assert.match(cid, /^[0-9a-f]{64}$/); assert.notEqual(cid, "e".repeat(64));
  assert.equal(bridge.files(1)[0].mime, "application/x-mewtual-pixa"); assert.deepEqual(bridge.files(2), []);
  assert.throws(() => call("download_file", {cid, maxBytes: bytes.length - 1}), /limit/);
  assert.throws(() => bridge.response("download_file", {server: 2, cid, maxBytes: bytes.length}), /scope/);
  assert.deepEqual(base64ToBytes(call("download_file", {cid, maxBytes: bytes.length}) as string), bytes);
  assert.equal(bridge.response("studio_apply", {}).handled, false);
});

test("visual upload refuses overlapping/oversized work and cancellation retires its reservation", () => {
  const bridge = createVisualFileUploads("a".repeat(64));
  const args = {server: 1, uploadId: "post", mime: "application/x-mewtual-pixa", size: 1};
  assert.throws(() => bridge.response("begin_file_upload", {...args, size: 9 * 1024 * 1024 + 1}), /Invalid/);
  const result = bridge.response("begin_file_upload", args); assert.ok(result.handled);
  const ticket = uploadContract(result.value);
  assert.throws(() => bridge.response("begin_file_upload", args), /Invalid/);
  bridge.response("cancel_file_upload", {server: 1, token: ticket.token});
  assert.throws(() => bridge.response("push_file_chunk", {server: 1, token: ticket.token, offset: 0, data: "AA=="}), /Unknown/);
  assert.ok(bridge.response("begin_file_upload", args).handled);
});
