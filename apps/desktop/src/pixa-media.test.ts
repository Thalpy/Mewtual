import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { decodePixa, encodePixa, PIXA_MAX_BYTES } from "./pixa.ts";
import { decodePixaBase64, mayFetchPixa, pixaFrameAt, PixaMediaCache, PixaFileReader } from "./pixa-media.ts";
import { mediaKind, mediaChoices, deckSurface } from "./jukebox.ts";
import { deferred } from "./studio-testkit.ts";
const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
const bytes = Buffer.from(vector.pixaHex, "hex"), encoded = bytes.toString("base64");
const packageValue = () => decodePixa(bytes);

test("PIXA hints reach file picker and picture surfaces; complete bytes remain mandatory", () => {
  assert.equal(mediaKind("a.PIXA"), "pixa"); assert.equal(mediaKind("a", "application/x-mewtual-pixa"), "pixa");
  assert.equal(mediaChoices([{ cid: "a", name: "a.pixa", mime: "application/octet-stream" }], "pixa").length, 1);
  assert.equal(deckSurface("pixa", true, false, true), "dock"); assert.equal(deckSurface("pixa", true, true, true), "focus");
  assert.equal(deckSurface("pixa", false, true, true), "none");
  assert.deepEqual(decodePixaBase64(encoded), packageValue());
  for (const size of [0, -1, NaN, Infinity, 1.1, PIXA_MAX_BYTES + 1]) assert.equal(mayFetchPixa(size), false);
  assert.equal(mayFetchPixa(PIXA_MAX_BYTES), true);
  assert.throws(() => decodePixaBase64("A".repeat(Math.ceil(PIXA_MAX_BYTES / 3) * 4 + 1)), /encoded size/);
  assert.throws(() => decodePixaBase64(btoa("PIXA")));
});

test("animation selects frame boundaries once and holds final picture through sound tails", () => {
  const p = { ...packageValue(), fps: 10, frames: [bytes, bytes, bytes] };
  assert.equal(pixaFrameAt(p, -1), 0); assert.equal(pixaFrameAt(p, 99), 0); assert.equal(pixaFrameAt(p, 100), 1);
  assert.equal(pixaFrameAt(p, 200), 2); assert.equal(pixaFrameAt(p, 600000), 2);
});

test("package cache retains only active/replacement encoded packages and evicts least recent", () => {
  const cache = new PixaMediaCache(), p = packageValue();
  cache.set("a", p); cache.set("b", p); cache.get("a"); cache.set("c", p);
  assert.equal(cache.get("b"), undefined); assert.equal(cache.get("a"), p); assert.equal(cache.get("c"), p);
  cache.clear(); assert.equal(cache.get("a"), undefined);
});

test("viewer close during native registration cancels after registration and starts no read", async () => {
  const gate = deferred<void>(), started = deferred<void>(), calls: string[] = [];
  const reader = new PixaFileReader(async <T>(command: string) => {
    calls.push(command); if (command === "begin_inline_download") { started.resolve(); await gate.promise; }
    return encoded as T;
  }, "a".repeat(16));
  const pending = reader.read(1, "b".repeat(64), bytes.length, () => true);
  await started.promise; reader.cancel(); assert.deepEqual(calls, ["begin_inline_download"]);
  gate.resolve(); assert.equal(await pending, null);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls.includes("download_file"), false); assert.equal(calls.includes("cancel_inline_download"), true);
});

test("viewer rejects oversized listings before IPC, caps native read and discards late scope", async () => {
  let live = true; const gate = deferred<string>(), started = deferred<void>(), calls: { command: string; args: Record<string, unknown> }[] = [];
  const reader = new PixaFileReader(async <T>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args }); if (command === "download_file") { started.resolve(); return await gate.promise as T; } return undefined as T;
  }, "a".repeat(16));
  await assert.rejects(reader.read(1, "b".repeat(64), PIXA_MAX_BYTES + 1, () => true)); assert.equal(calls.length, 0);
  const pending = reader.read(1, "b".repeat(64), bytes.length, () => live);
  await started.promise; live = false; gate.resolve(encoded); assert.equal(await pending, null);
  assert.equal(calls.find(c => c.command === "download_file")!.args.maxBytes, PIXA_MAX_BYTES);
  const good = await reader.read(1, "b".repeat(64), bytes.length, () => true);
  assert.deepEqual(encodePixa(good!), new Uint8Array(bytes));
});
