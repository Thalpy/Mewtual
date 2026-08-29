import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { storageRepairNotice } from "./storage-local.ts";

const app = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");

test("Storage lists complete encrypted local files and exports only by an explicit click", () => {
  assert.match(app, /local_files: Array</);
  assert.match(app, /\{#each storageHealth\.local_files as file/);
  assert.match(app, /onclick=\{\(\) => exportStoredFile\(file\.cid\)\}>Unlock copy/);
  assert.match(app, /writes a separate plaintext file to Downloads/);
  assert.match(app, /encrypted managed copy remains/);
  assert.match(app, /Partial downloads remain encrypted too/);
});

test("Storage states the media signature check without claiming decoder safety", () => {
  assert.match(app, /compare common image\/audio\/video container signatures/);
  assert.match(app, /does not prove the file is benign/);
  assert.match(app, /SVG and unrecognized media stay out of inline decoding/);
});

test("locking clears cached plaintext storage metadata in the webview", () => {
  const start = app.indexOf("function lockScreen(");
  const end = app.indexOf("\n  function ", start + 1);
  assert.ok(start >= 0 && end > start);
  assert.match(app.slice(start, end), /storageHealthCache\.clear\(\)/);
});

test("late storage reads and repairs are accepted only by the exact unlocked view", () => {
  const healthStart = app.indexOf("async function refreshStorageHealth(");
  const repairStart = app.indexOf("async function repairStorage(");
  const exportStart = app.indexOf("async function exportStoredFile(");
  assert.ok(healthStart >= 0 && repairStart > healthStart && exportStart > repairStart);
  const health = app.slice(healthStart, repairStart);
  const repair = app.slice(repairStart, exportStart);
  assert.match(health, /if \(!unlockedViewCurrent\(gen, server\)\) return;[\s\S]*storageHealthCache\.set/);
  assert.match(repair, /if \(!unlockedViewCurrent\(gen, server\)\) return;[\s\S]*storageHealthCache\.set/);
  assert.match(health, /if \(unlockedViewCurrent\(gen, server\)\) storageChecking = false/);
  assert.match(repair, /if \(unlockedViewCurrent\(gen, server\)\) storageRepairing = false/);
});

test("a zero-attempt repair never calls contradictory storage healthy", () => {
  assert.equal(storageRepairNotice({
    attempted_chunks: 0,
    recovered_chunks: 0,
    health: { missing_chunks: 0, unreadable_chunks: 1, invalid_manifests: 0 },
  }), "Nothing could be repaired automatically; 1 unreadable, missing, or invalid storage reference remains.");
  assert.equal(storageRepairNotice({
    attempted_chunks: 0,
    recovered_chunks: 0,
    health: { missing_chunks: 0, unreadable_chunks: 0, invalid_manifests: 0 },
  }), "Everything referenced by this server verifies.");
});

test("a repair that made attempts reports work without claiming all remaining content is safe", () => {
  assert.equal(storageRepairNotice({
    attempted_chunks: 3,
    recovered_chunks: 2,
    health: { missing_chunks: 1, unreadable_chunks: 0, invalid_manifests: 0 },
  }), "Checked 3 damaged or missing chunks; recovered 2.");
});
