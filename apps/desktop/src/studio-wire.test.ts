// Shared byte vectors pin the Rust domain codec to the real frontend serializers. This does
// not connect the fixture StudioStore to the backend or change any UI surface.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { canonicalJson } from "./studio-contract.ts";
import { validateJamPatch } from "./jam-patch.ts";

const vectors = JSON.parse(readFileSync(new URL(
  "../../../crates/catcoms-replication/tests/fixtures/studio-ops-v1.json", import.meta.url,
), "utf8")) as { version: number; patch_id: string; cases: { name: string; kind: string; body: string }[] };

test("Studio Rust wire vectors match the actual canonicalJson serializer", () => {
  assert.equal(vectors.version, 1);
  assert.equal(vectors.cases.length, 24);
  for (const row of vectors.cases) {
    const body = JSON.parse(row.body);
    assert.equal(canonicalJson(body), row.body, row.name);
    assert.deepEqual(new TextEncoder().encode(canonicalJson(body)), new TextEncoder().encode(row.body));
  }
});

test("Studio patch identity uses jam declaration order inside sorted Studio JSON", () => {
  const row = vectors.cases.find((row) => row.name === "set_patch")!;
  const body = JSON.parse(row.body);
  const patch = validateJamPatch(body.descriptor);
  assert.equal(patch.ok, true);
  if (!patch.ok) return;
  assert.equal(createHash("sha256").update(patch.canonical).digest("hex"), vectors.patch_id);
  assert.equal(body.patch, vectors.patch_id);
  assert.notEqual(patch.canonical, canonicalJson(body.descriptor));
  assert.notEqual(createHash("sha256").update(canonicalJson(body.descriptor)).digest("hex"), vectors.patch_id);
});

test("Studio expiry vectors preserve absent, null and timestamp zero distinctly", () => {
  const read = (name: string) => JSON.parse(vectors.cases.find((row) => row.name === name)!.body);
  assert.equal(Object.hasOwn(read("expiry_unrecorded"), "expiry"), false);
  assert.equal(read("expiry_never").expiry, null);
  assert.equal(read("expiry_zero").expiry, 0);
  assert.equal(read("expiry_max").expiry, Number.MAX_SAFE_INTEGER);
  // The current in-memory UI fixture's numeric expiry shape is not the production protocol;
  // gate-2 save/load must map all three states without using 0 as a Never sentinel.
});

test("Studio Rust patch ranges match the actual frontend validator at each boundary", () => {
  const ranges = JSON.parse(readFileSync(new URL(
    "../../../crates/catcoms-replication/tests/fixtures/studio-patch-ranges-v1.json", import.meta.url,
  ), "utf8")) as { path: string; min: number; max: number }[];
  assert.equal(ranges.length, 18);
  const original = JSON.parse(vectors.cases.find((row) => row.name === "set_patch")!.body).descriptor;
  for (const { path, min, max } of ranges) {
    for (const [value, accepted] of [[min - 1, false], [min, true], [max, true], [max + 1, false]] as const) {
      const recipe = structuredClone(original);
      const keys = path.slice(1).split("/");
      let target = recipe;
      for (const key of keys.slice(0, -1)) target = target[key];
      target[keys.at(-1)!] = value;
      assert.equal(validateJamPatch(recipe).ok, accepted, `${path}: ${value}`);
    }
  }
});
