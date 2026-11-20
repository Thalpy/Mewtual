import assert from "node:assert/strict";
import test from "node:test";
import { StudioSharedPatches, STUDIO_SHARED_PATCH_LIMIT } from "./studio-shared-patches.ts";
import { JAM_PATCH_FILE_MAX_BYTES } from "./jam-contract.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { deferred } from "./studio-testkit.ts";
const encoded = btoa(JSON.stringify(legacyJamPatch("sine")));
const file = (id: number, binding = "verified") => ({ cid: id.toString(16).padStart(64, "0"), name: `sound-${id}.jampatch`, binding });
const settle = async () => { for (let i = 0; i < 100; i++) await Promise.resolve(); };

test("shared shelf retains only sixteen current candidates and requests the native patch cap", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [], sizes: number[] = [];
  let held: ReadonlyMap<string, unknown> = new Map();
  const shelf = new StudioSharedPatches(async <T>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args }); return (command === "download_file" ? encoded : undefined) as T;
  }, "test", values => { held = values; sizes.push(values.size); });
  shelf.update("A", 1, Array.from({ length: 20 }, (_, i) => file(i))); await settle();
  assert.equal(held.size, STUDIO_SHARED_PATCH_LIMIT); assert.ok(sizes.every(size => size <= 16));
  const reads = calls.filter(c => c.command === "download_file"); assert.equal(reads.length, 16);
  assert.ok(reads.every(c => c.args.maxBytes === JAM_PATCH_FILE_MAX_BYTES && typeof c.args.cancellation === "string"));
  shelf.update("A", 1, [file(30)]); await settle();
  assert.deepEqual([...held.keys()], [file(30).cid], "listing churn cannot grow a historical cache");
  shelf.update(null, null, []); assert.equal(held.size, 0);
});

test("ABA replaces stale shared-file reads serially and trust revocation suppresses completion", async () => {
  const first = deferred<string>(); let downloads = 0, active = 0, peak = 0, allowed = true;
  let held: ReadonlyMap<string, unknown> = new Map();
  const shelf = new StudioSharedPatches(async <T>(command: string) => {
    if (command !== "download_file") return undefined as T;
    active++; peak = Math.max(peak, active);
    try { return (++downloads === 1 ? await first.promise : encoded) as T; } finally { active--; }
  }, "test", values => { held = values; }, () => allowed);
  shelf.update("A", 1, [file(1)]); await settle();
  shelf.update("B", 2, [file(2)]); shelf.update("A", 1, [file(3)]);
  assert.equal(downloads, 1); first.resolve(encoded); await settle();
  assert.equal(peak, 1); assert.deepEqual([...held.keys()], [file(3).cid]);
  const blocked = deferred<string>();
  const second = new StudioSharedPatches(async <T>(command: string) => (command === "download_file" ? await blocked.promise : undefined) as T,
    "other", values => { held = values; }, () => allowed);
  second.update("A", 1, [file(4)]); await settle(); allowed = false; blocked.resolve(encoded); await settle();
  assert.equal(held.size, 0, "policy is rechecked even before a reactive update has run");
  shelf.update(null, null, []); second.update(null, null, []);
});

// Performance/save review (2026-10-30), finding 9: leaving the Studio cleared the shelf, so every
// return to the same channel downloaded all sixteen recipes again.
test("pausing the shelf keeps what it read for the same scope; resuming reads only what is missing", async () => {
  let downloads = 0, held: ReadonlyMap<string, unknown> = new Map();
  const shelf = new StudioSharedPatches(async <T>(command: string) => {
    if (command === "download_file") downloads++;
    return (command === "download_file" ? encoded : undefined) as T;
  }, "test", values => { held = values; });
  const candidates = [file(1), file(2), file(3)];
  shelf.update("A", 1, candidates); await settle();
  assert.equal(downloads, 3); assert.equal(held.size, 3);
  shelf.update("A", 1, candidates, false); await settle();
  assert.equal(held.size, 3, "leaving the surface pauses without forgetting");
  shelf.update("A", 1, candidates, true); await settle();
  assert.equal(downloads, 3, "returning to the same scope reads nothing again");
  shelf.update("A", 1, [...candidates, file(4)], true); await settle();
  assert.equal(downloads, 4, "only the new candidate is read");
  shelf.update("B", 1, candidates, false); await settle();
  assert.equal(held.size, 0, "a scope change still clears, paused or not");
  shelf.update(null, null, []);
});

test("pausing mid-read cancels that read and starts no further downloads", async () => {
  const first = deferred<string>(); let downloads = 0, held: ReadonlyMap<string, unknown> = new Map();
  const calls: string[] = [];
  const shelf = new StudioSharedPatches(async <T>(command: string) => {
    calls.push(command);
    if (command !== "download_file") return undefined as T;
    return (++downloads === 1 ? await first.promise : encoded) as T;
  }, "test", values => { held = values; });
  shelf.update("A", 1, [file(1), file(2)]); await settle();
  shelf.update("A", 1, [file(1), file(2)], false);
  first.resolve(encoded); await settle();
  assert.equal(downloads, 1); assert.equal(held.size, 0, "a read retired by the pause is not installed");
  assert.ok(calls.includes("cancel_inline_download"));
  shelf.update("A", 1, [file(1), file(2)], true); await settle();
  assert.equal(downloads, 3); assert.equal(held.size, 2);
  shelf.update(null, null, []);
});

test("closing during registration cancels after begin and starts no shared patch download", async () => {
  const begin = deferred<void>(), calls: string[] = [];
  const shelf = new StudioSharedPatches(async <T>(command: string) => {
    calls.push(command); if (command === "begin_inline_download") await begin.promise; return encoded as T;
  }, "test", () => {});
  shelf.update("A", 1, [file(1)]); shelf.update(null, null, []);
  begin.resolve(); await settle(); assert.equal(calls.includes("download_file"), false);
  assert.ok(calls.includes("cancel_inline_download"));
});
