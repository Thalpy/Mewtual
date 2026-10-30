/** Windows installers must carry the runtime selected by the actual build configuration. */
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";

type Config = {
  build?: { windows?: { staticVCRuntime?: boolean } };
  bundle?: { windows?: { bundleVCRuntime?: boolean } };
};
function config(name: string): Config {
  return JSON.parse(readFileSync(new URL(`../src-tauri/${name}`, import.meta.url), "utf8"));
}

const base = config("tauri.conf.json");
const platformPath = new URL("../src-tauri/tauri.windows.conf.json", import.meta.url);
const platform = existsSync(platformPath) ? config("tauri.windows.conf.json") : {};
const official = config("tauri.official.conf.json");

for (const [name, layers] of [
  ["source installer", [base, platform]],
  ["official release installer", [base, platform, official]],
] as const) {
  test(`${name} supplies its Microsoft VC runtime`, () => {
    // Tauri merges nested config fields in order. Its locked schema defaults to static
    // linkage and no DLL bundling; an omitted override preserves the earlier field.
    let staticallyLinked = true;
    let runtimeBundled = false;
    for (const layer of layers) {
      staticallyLinked = layer.build?.windows?.staticVCRuntime ?? staticallyLinked;
      runtimeBundled = layer.bundle?.windows?.bundleVCRuntime ?? runtimeBundled;
    }
    assert.ok(
      staticallyLinked || runtimeBundled,
      `${name} dynamically links the VC runtime but does not include it in the installer`,
    );
  });
}
