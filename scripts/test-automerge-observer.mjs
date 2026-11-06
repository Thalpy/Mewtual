// The patched dependency is deliberately excluded from workspace membership. Test its narrow
// observer units in an isolated source copy, without adding all upstream dev dependencies to the
// shipping lockfiles or running the whole upstream test suite. Works in nested worktrees too.
import {
  cpSync, existsSync, lstatSync, mkdirSync, readFileSync, realpathSync, readdirSync, rmSync, writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = join(root, "vendor", "automerge");
const target = join(root, "target");
const copy = join(target, "automerge-observer-unit-source");
const marker = ".mewtual-observer-unit-source";
const ownership = "Mewtual isolated Automerge observer unit source v1\n";
if (existsSync(join(source, marker))) throw new Error("Observer source uses the reserved test ownership marker");
mkdirSync(target, { recursive: true });
if (existsSync(copy)) {
  // Delete ONLY our exact marked source directory. Resolve containment and reject links before
  // recursive cleanup on Windows as well as Unix; never touch the Cargo artifact directory.
  if (lstatSync(copy).isSymbolicLink() || dirname(realpathSync(copy)) !== realpathSync(target)
    || readFileSync(join(copy, marker), "utf8") !== ownership) {
    throw new Error("Refusing to replace an unowned observer test source directory");
  }
  rmSync(copy, { recursive: true });
}
mkdirSync(copy, { recursive: true });
// Register ownership before copying so an interrupted first build can be retried safely.
writeFileSync(join(copy, marker), ownership);
cpSync(source, copy, {
  recursive: true,
  filter(path) {
    const rel = relative(source, path);
    // A developer's previous standalone Cargo build is not published source. In particular,
    // never copy gigabytes of cached build artifacts into another test-source directory.
    return rel !== "target" && !rel.startsWith(`target${sep}`);
  },
});
const digests = {};
function verify(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name), rel = relative(source, path);
    if (rel === "target") continue;
    if (entry.isSymbolicLink()) throw new Error("Observer source must not contain symlinks");
    if (entry.isDirectory()) verify(path);
    else if (entry.isFile()) {
      const original = readFileSync(path), copied = readFileSync(join(copy, rel));
      if (!original.equals(copied)) throw new Error(`Observer test copy differs: ${rel}`);
      digests[rel] = createHash("sha256").update(original).digest("hex");
    }
  }
}
verify(source);
// The ONLY changed copied source is workspace metadata. The checked-in published manifest,
// upstream lockfile, all patched runtime/test sources and both shipping lockfiles stay intact.
const manifest = join(copy, "Cargo.toml");
writeFileSync(manifest, `${readFileSync(manifest, "utf8")}\n[workspace]\n`);
writeFileSync(join(target, "automerge-observer-source-sha256.json"), `${JSON.stringify(digests, null, 2)}\n`);
const env = { ...process.env, CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR || target,
  CARGO_INCREMENTAL: process.env.CARGO_INCREMENTAL || "0", CARGO_PROFILE_DEV_DEBUG: process.env.CARGO_PROFILE_DEV_DEBUG || "0" };
for (const [selector, minimum] of [["history::tests", 6], ["value_metadata::tests", 9]]) {
  const result = spawnSync("cargo", ["test", "--locked", "--manifest-path", manifest, "--lib", "-j1", selector,
    "--", "--test-threads=2"], { cwd: root, env, encoding: "utf8", maxBuffer: 16 * 1024 * 1024, windowsHide: true });
  process.stdout.write(result.stdout ?? ""); process.stderr.write(result.stderr ?? "");
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  const plain = (result.stdout ?? "").replace(/\x1b\[[0-9;]*m/g, "");
  const passed = [...plain.matchAll(/test result: ok\. (\d+) passed;/g)].reduce((n, match) => n + Number(match[1]), 0);
  if (passed < minimum) throw new Error(`Observer selector ${selector} ran ${passed} tests; expected at least ${minimum}`);
}
