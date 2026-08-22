// Builds THIRD-PARTY-NOTICES.txt: the attribution file the installer ships.
//
// Nearly every licence in the tree (MIT, BSD, ISC, Apache-2.0, MPL-2.0, CDLA-Permissive-2.0)
// requires that its own text and copyright notices travel with the binary. Mewtual's own terms
// say the same thing from the other side: Part I requires preserving the notices supplied with
// third-party components. So this file is an obligation, not a courtesy, and it has to be
// regenerated whenever the dependency set moves.
//
// Two halves, one file:
//   - Rust: `cargo about generate` over the desktop crate's own workspace, which is the tree
//     that actually links into the shipped executable (the root workspace has test-only crates
//     the installer never sees).
//   - npm: the production dependency closure of apps/desktop. devDependencies (vite, esbuild,
//     typescript) build the bundle but ship nothing into it; svelte is the exception and is
//     pulled in explicitly because its compiled runtime does land in the output.
//
// Output goes to public/, so vite copies it into dist/ and the webview can fetch it on demand
// rather than inlining a megabyte of licence text into the JS bundle.
//
// Usage: npm run notices   (from apps/desktop)

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const ROOT = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const TAURI = join(ROOT, "src-tauri");
const OUT_DIR = join(ROOT, "public");
const OUT = join(OUT_DIR, "THIRD-PARTY-NOTICES.txt");

// Compiled into the bundle even though it is a devDependency: the svelte runtime ships.
const EXTRA_SHIPPED = ["svelte"];

const RULE = "=".repeat(80);

function rustNotices() {
  // cargo-about resolves the crate graph itself; it needs the manifest, not a lockfile path.
  //
  // Output goes to a temp file via -o rather than being captured off stdout: cargo-about
  // refuses to write generated output to a redirected stdout once it detects PowerShell
  // somewhere in its ancestry (a defensive check against PowerShell's own `>` operator
  // mangling non-ASCII redirected text), and PowerShell is this repo's primary shell. Writing
  // straight to a file sidesteps that check instead of fighting it.
  const tmpOut = join(tmpdir(), `mewtual-notices-${process.pid}.txt`);
  try {
    execFileSync(
      "cargo",
      ["about", "generate", "--manifest-path", join(TAURI, "Cargo.toml"), "-o", tmpOut, join(TAURI, "about.hbs")],
      { maxBuffer: 64 * 1024 * 1024, cwd: TAURI },
    );
    return readFileSync(tmpOut, "utf8");
  } finally {
    if (existsSync(tmpOut)) rmSync(tmpOut);
  }
}

// Walks node_modules for one package, following the flat layout npm produces. Returns null when
// a name cannot be resolved, which the caller reports rather than silently dropping.
function readPackage(name) {
  const dir = join(ROOT, "node_modules", ...name.split("/"));
  const manifest = join(dir, "package.json");
  if (!existsSync(manifest)) return null;
  const meta = JSON.parse(readFileSync(manifest, "utf8"));
  const license =
    typeof meta.license === "string" ? meta.license : meta.license?.type ?? "UNKNOWN";
  // Package authors are inconsistent about the filename, so try the ones that actually occur.
  const candidates = ["LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE", "LICENCE.md", "license", "LICENSE-MIT"];
  let text = null;
  for (const candidate of candidates) {
    const path = join(dir, candidate);
    if (existsSync(path)) {
      text = readFileSync(path, "utf8").trim();
      break;
    }
  }
  return { name, version: meta.version, license, text, deps: Object.keys(meta.dependencies ?? {}) };
}

// The production closure: direct dependencies plus everything they pull in transitively.
function npmClosure() {
  const manifest = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
  const queue = [...Object.keys(manifest.dependencies ?? {}), ...EXTRA_SHIPPED];
  const seen = new Map();
  const missing = [];
  while (queue.length) {
    const name = queue.shift();
    if (seen.has(name)) continue;
    const pkg = readPackage(name);
    if (!pkg) {
      missing.push(name);
      continue;
    }
    seen.set(name, pkg);
    queue.push(...pkg.deps);
  }
  return { packages: [...seen.values()].sort((a, b) => a.name.localeCompare(b.name)), missing };
}

function npmNotices() {
  const { packages, missing } = npmClosure();
  if (missing.length) {
    // Not fatal, but never silent: a package we cannot read is a notice we cannot ship.
    console.warn(`warning: could not resolve ${missing.join(", ")} in node_modules (run npm ci)`);
  }
  const blocks = packages.map((pkg) => {
    const head = `${RULE}\n${pkg.name} ${pkg.version}\nLicence: ${pkg.license}\n${RULE}\n`;
    // Some packages declare a licence but ship no text; naming the SPDX id is still the notice.
    const body = pkg.text ?? `(No licence file is distributed with this package. It declares ${pkg.license}.)`;
    return `${head}\n${body}\n`;
  });
  return blocks.join("\n");
}

const generated = `Mewtual: third-party notices

Mewtual itself is licensed under the Mewtual Combined Licence Terms 1.0 (see LICENSE in the
source distribution). This file is not that licence. It collects the licences and copyright
notices of the third-party components Mewtual is built from, each of which remains governed
solely by its own terms.

This file is generated. Do not edit it by hand: run "npm run notices" from apps/desktop.


${RULE}
RUST COMPONENTS
${RULE}

${rustNotices()}

${RULE}
JAVASCRIPT COMPONENTS
${RULE}

${npmNotices()}`;

mkdirSync(OUT_DIR, { recursive: true });
writeFileSync(OUT, generated.replaceAll("\r\n", "\n"), "utf8");
console.log(`wrote ${OUT} (${(generated.length / 1024).toFixed(0)} KiB)`);
process.exit(0);
