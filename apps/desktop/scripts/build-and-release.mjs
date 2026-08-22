// Runs the maintainer checklist in docs/RELEASING.md end to end, instead of by hand:
//
//   0. npm test / npm run check            (the same two checks release.yml runs)
//   1. bump the version in all 5 places, move CHANGELOG.md's [Unreleased] entries under the
//      new heading, regenerate THIRD-PARTY-NOTICES.txt
//   2. a local, unsigned `tauri build` as a fail-fast check — the real signed installer only
//      comes out of the Windows CI runner, which has the minisign secrets this machine may not
//   3. commit, push, trigger .github/workflows/release.yml, watch it (~20 min), and print the
//      exact `gh api` command to publish the draft release it leaves
//
// It deliberately stops at the draft: docs/RELEASING.md keeps that a human step (review the
// release body, confirm .sig/latest.json are present) and this script doesn't second-guess that.
//
// Usage (from apps/desktop): npm run build-and-release -- [options]
//   --version=X.Y.Z-alpha.N   explicit new version (default: auto-increment the current alpha.N)
//   --ref=<branch>            branch to push and build (default: current branch)
//   --yes, -y                 skip the confirmation prompt before commit/push/trigger
//   --no-watch                trigger the workflow but don't wait for it
//   --skip-tests              skip step 0
//   --skip-build              skip the local build in step 2
//   --skip-notices            skip regenerating THIRD-PARTY-NOTICES.txt
//   --flows                   also run `npm run test:flows` in step 0 (needs local Edge; CI skips it)
//   --dry-run                 bump files and show the diff, then stop before committing
//   --help, -h                print this

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import readline from "node:readline/promises";

const DESKTOP = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const REPO_ROOT = resolve(DESKTOP, "../..");

const FILES = {
  tauriConf: join(DESKTOP, "src-tauri", "tauri.conf.json"),
  cargoToml: join(DESKTOP, "src-tauri", "Cargo.toml"),
  cargoLock: join(DESKTOP, "src-tauri", "Cargo.lock"),
  packageJson: join(DESKTOP, "package.json"),
  packageLock: join(DESKTOP, "package-lock.json"),
  changelog: join(REPO_ROOT, "CHANGELOG.md"),
  notices: join(DESKTOP, "public", "THIRD-PARTY-NOTICES.txt"),
};

const VERSION_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?$/;

// ---------- process helpers ----------

function banner(step, title) {
  console.log(`\n\x1b[1m[${step}] ${title}\x1b[0m`);
}

// git/gh/cargo are real .exe on PATH; npm on Windows is npm.cmd and needs a shell to resolve.
function run(cmd, args, opts = {}) {
  const result = spawnSync(cmd, args, { stdio: "inherit", cwd: REPO_ROOT, shell: false, ...opts });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${cmd} ${args.join(" ")} exited ${result.status}`);
  return result;
}

function runNpm(args, opts = {}) {
  return run("npm", args, { cwd: DESKTOP, shell: true, ...opts });
}

function capture(cmd, args, opts = {}) {
  const result = spawnSync(cmd, args, { encoding: "utf8", cwd: REPO_ROOT, shell: false, ...opts });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${cmd} ${args.join(" ")} exited ${result.status}: ${result.stderr}`);
  }
  return result.stdout.trim();
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function ask(question) {
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  try {
    return await rl.question(question);
  } finally {
    rl.close();
  }
}

function relToRepo(p) {
  return relative(REPO_ROOT, p).split(sep).join("/");
}

const readText = (path) => readFileSync(path, "utf8");
const writeText = (path, text) => writeFileSync(path, text);
const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// Replaces every match of a *global* pattern and blows up rather than silently doing the wrong
// thing if the match count isn't exactly what the caller expects. A version bump that touches
// zero, or the wrong number of, places is exactly the "quiet rather than loud" failure
// docs/RELEASING.md warns about for this step.
function replaceExact(text, pattern, replacement, expectedCount, label) {
  const matches = text.match(pattern) ?? [];
  if (matches.length !== expectedCount) {
    throw new Error(`${label}: expected ${expectedCount} match(es), found ${matches.length}`);
  }
  return text.replace(pattern, replacement);
}

// ---------- version bump ----------

function currentVersions() {
  const conf = JSON.parse(readText(FILES.tauriConf)).version;
  const pkg = JSON.parse(readText(FILES.packageJson)).version;
  const lock = JSON.parse(readText(FILES.packageLock)).version;
  const crate = readText(FILES.cargoToml).match(/^version = "([^"]+)"/m)?.[1];
  const cargoLockText = readText(FILES.cargoLock);
  const idx = cargoLockText.indexOf('name = "mewtual-desktop"');
  const clock = idx === -1 ? undefined : cargoLockText.slice(idx).match(/version = "([^"]+)"/)?.[1];
  return { "tauri.conf.json": conf, "package.json": pkg, "package-lock.json": lock, "Cargo.toml": crate, "Cargo.lock": clock };
}

function assertVersionsConsistent() {
  const values = currentVersions();
  const conf = values["tauri.conf.json"];
  const mismatched = Object.entries(values).filter(([, v]) => v !== conf);
  if (mismatched.length) {
    throw new Error(
      `Version files disagree before bumping (tauri.conf.json=${conf} is the source of truth): ` +
        JSON.stringify(values),
    );
  }
  return conf;
}

function bumpAlpha(version) {
  const m = version.match(/^(\d+\.\d+\.\d+)-alpha\.(\d+)$/);
  if (!m) {
    throw new Error(
      `Can't auto-increment "${version}" (expected X.Y.Z-alpha.N). Pass one explicitly: ` +
        "npm run build-and-release -- --version=X.Y.Z-alpha.N",
    );
  }
  return `${m[1]}-alpha.${Number(m[2]) + 1}`;
}

function assertVersionUnclaimed(newVersion) {
  const probe = spawnSync("gh", ["release", "view", `v${newVersion}`], { encoding: "utf8", cwd: REPO_ROOT });
  if (probe.status === 0) {
    throw new Error(
      `v${newVersion} already exists as a GitHub release (draft or published). Pick a ` +
        "different --version, or delete that release first if it was a mistake.",
    );
  }
}

function bumpJsonVersion(path, oldVersion, newVersion, label, expectedCount = 1) {
  const pattern = new RegExp(`"version":\\s*"${escapeRegExp(oldVersion)}"`, "g");
  const updated = replaceExact(readText(path), pattern, `"version": "${newVersion}"`, expectedCount, label);
  writeText(path, updated);
}

function bumpCargoToml(oldVersion, newVersion) {
  const pattern = new RegExp(`^version = "${escapeRegExp(oldVersion)}"`, "gm");
  const updated = replaceExact(readText(FILES.cargoToml), pattern, `version = "${newVersion}"`, 1, "Cargo.toml");
  writeText(FILES.cargoToml, updated);
}

// Scoped to the mewtual-desktop [[package]] block so a dependency that happens to share the old
// version string is never at risk of being the one substituted.
function bumpCargoLock(oldVersion, newVersion) {
  const text = readText(FILES.cargoLock);
  const nameIdx = text.indexOf('name = "mewtual-desktop"');
  if (nameIdx === -1) throw new Error("Cargo.lock: no mewtual-desktop package entry found");
  const nextPkg = text.indexOf("\n[[package]]", nameIdx);
  const blockEnd = nextPkg === -1 ? text.length : nextPkg;
  const pattern = new RegExp(`version = "${escapeRegExp(oldVersion)}"`, "g");
  const block = replaceExact(
    text.slice(nameIdx, blockEnd),
    pattern,
    `version = "${newVersion}"`,
    1,
    "Cargo.lock (mewtual-desktop entry)",
  );
  writeText(FILES.cargoLock, text.slice(0, nameIdx) + block + text.slice(blockEnd));
}

// Moves [Unreleased]'s body under a new "## [version] - date" heading, leaving [Unreleased]
// empty. Returns false (and leaves the file untouched) if there was nothing to move — shipping
// a version heading with no entries would misrepresent the release, so that's a warn, not a fix.
function updateChangelog(newVersion) {
  const text = readText(FILES.changelog);
  const heading = "## [Unreleased]";
  const idx = text.indexOf(heading);
  if (idx === -1) throw new Error("CHANGELOG.md: no '## [Unreleased]' heading found");
  const afterHeading = idx + heading.length;
  const nextMatch = text.slice(afterHeading).match(/\n## \[/);
  if (!nextMatch) throw new Error("CHANGELOG.md: no version heading found after [Unreleased]");
  const nextIdx = afterHeading + nextMatch.index + 1; // start of the next "## [" line
  const body = text.slice(afterHeading, nextIdx);
  const trimmed = body.replace(/^\n+/, "").replace(/\n+$/, "");
  if (!trimmed.trim()) {
    console.warn(
      "CHANGELOG.md: [Unreleased] is empty — nothing to move under " +
        `[${newVersion}]. This release will ship with no changelog entry unless you add one by ` +
        "hand before publishing.",
    );
    return false;
  }
  const today = new Date().toISOString().slice(0, 10);
  const newSection = `${heading}\n\n## [${newVersion}] - ${today}\n\n${trimmed}\n\n`;
  writeText(FILES.changelog, text.slice(0, idx) + newSection + text.slice(nextIdx));
  return true;
}

function regenerateNotices() {
  banner("1b", "Regenerating THIRD-PARTY-NOTICES.txt");
  const probe = spawnSync("cargo", ["about", "--version"], { encoding: "utf8" });
  if (probe.status !== 0) {
    console.warn(
      "cargo-about not found (`cargo install cargo-about --locked --features cli`); skipping. " +
        "Regenerate by hand before publishing if Cargo.lock or package-lock.json changed.",
    );
    return false;
  }
  runNpm(["run", "notices"]);
  const result = spawnSync("git", ["diff", "--quiet", "--", relToRepo(FILES.notices)], { cwd: REPO_ROOT });
  return result.status !== 0; // non-zero means the regenerated file actually differs
}

// ---------- steps ----------

function runTests(opts) {
  banner(0, "Running tests");
  runNpm(["test"]);
  runNpm(["run", "check"]);
  if (opts.flows) runNpm(["run", "test:flows"]);
}

function localBuild() {
  banner(2, "Building the installer locally (fail-fast check, unsigned)");
  runNpm(["run", "tauri", "--", "build"]);
  console.log(`Installer (unsigned): ${join(DESKTOP, "src-tauri", "target", "release", "bundle", "nsis")}`);
}

function gitCommitAndPush(newVersion, changedFiles, ref) {
  const already = capture("git", ["diff", "--cached", "--name-only"]);
  if (already) {
    throw new Error(
      `Refusing to commit: something else is already staged:\n${already}\n` +
        "Commit or unstage it first so this commit contains only the version bump.",
    );
  }
  const relFiles = changedFiles.map(relToRepo);
  run("git", ["add", "--", ...relFiles]);
  const stagedNow = capture("git", ["diff", "--cached", "--name-only"])
    .split(/\r?\n/)
    .filter(Boolean)
    .sort();
  const expected = [...relFiles].sort();
  if (stagedNow.join("\n") !== expected.join("\n")) {
    throw new Error(
      `Staged set doesn't match the version files.\nExpected: ${expected.join(", ")}\n` +
        `Staged: ${stagedNow.join(", ")}`,
    );
  }
  run("git", ["commit", "-m", `Bump version to v${newVersion}`, "-m", "Co-Authored-By: Rosemary"]);
  const sha = capture("git", ["rev-parse", "HEAD"]);
  const hasUpstream = spawnSync("git", ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"], {
    cwd: REPO_ROOT,
  }).status === 0;
  run("git", hasUpstream ? ["push"] : ["push", "-u", "origin", ref]);
  console.log(`Pushed ${sha}`);
  return sha;
}

function triggerWorkflow(ref) {
  banner("3", `Triggering release.yml on ${ref}`);
  run("gh", ["workflow", "run", "release.yml", "--ref", ref]);
}

async function findRun(ref, sha, { retries = 20, delayMs = 3000 } = {}) {
  for (let i = 0; i < retries; i++) {
    const out = capture("gh", [
      "run", "list", "--workflow=release.yml", "--branch", ref, "--limit", "5",
      "--json", "databaseId,headSha,url",
    ]);
    const match = JSON.parse(out).find((r) => r.headSha === sha);
    if (match) return match;
    await sleep(delayMs);
  }
  throw new Error(
    `No release.yml run appeared for ${sha} on ${ref} after ${(retries * delayMs) / 1000}s. ` +
      "Check the Actions tab by hand.",
  );
}

function watchRun(runId) {
  banner("4", "Watching the run (the signed Windows build takes roughly 20 minutes)");
  run("gh", ["run", "watch", String(runId), "--exit-status"]);
}

function reportDraft(newVersion, sha) {
  banner("5", "Build finished — the release is a DRAFT, review before publishing");
  const rel = JSON.parse(
    capture("gh", ["release", "view", `v${newVersion}`, "--json", "databaseId,url,isDraft,assets"]),
  );
  console.log(`Draft release: ${rel.url}`);
  const names = rel.assets.map((a) => a.name);
  if (!names.some((n) => n.endsWith(".sig")) || !names.some((n) => n === "latest.json")) {
    console.warn(
      "WARNING: the draft has no .sig and/or no latest.json — signing secrets may not be " +
        "configured. Per docs/RELEASING.md, do NOT publish an unsigned draft.",
    );
  }
  console.log(
    "\nReview in the web UI (tick 'Set as the latest release', leave 'Set as a pre-release' " +
      "unticked), or publish from the terminal:\n",
  );
  console.log(
    `  gh api -X PATCH repos/{owner}/{repo}/releases/${rel.databaseId} \\\n` +
      `    -f tag_name=v${newVersion} \\\n` +
      `    -f target_commitish=${sha} \\\n` +
      "    -F draft=false -F prerelease=false -f make_latest=true\n",
  );
}

// ---------- cli ----------

function printHelp() {
  console.log(`Usage: npm run build-and-release -- [options]

Runs the docs/RELEASING.md checklist end to end:
  0. npm test / npm run check
  1. bump the version in all 5 places, move CHANGELOG.md's [Unreleased] entries, regen notices
  2. a local, unsigned "tauri build" as a fail-fast check before waiting on CI
  3. commit, push, trigger .github/workflows/release.yml, watch it, and report the draft release
     it leaves (this never auto-publishes — see docs/RELEASING.md's reasoning)

Options:
  --version=X.Y.Z-alpha.N   explicit new version (default: auto-increment the current alpha.N)
  --ref=<branch>            branch to push and build (default: current branch)
  --yes, -y                 skip the confirmation prompt before commit/push/trigger
  --no-watch                trigger the workflow but don't wait for it
  --skip-tests              skip step 0
  --skip-build              skip the local build in step 2
  --skip-notices            skip regenerating THIRD-PARTY-NOTICES.txt
  --flows                   also run \`npm run test:flows\` in step 0 (needs local Edge; CI skips it)
  --dry-run                 bump files and show the diff, then stop before committing
  --help, -h                this message
`);
}

function parseArgs(argv) {
  const opts = {
    version: null, ref: null, yes: false, watch: true,
    tests: true, build: true, notices: true, flows: false, dryRun: false,
  };
  for (const arg of argv) {
    if (arg === "--yes" || arg === "-y") opts.yes = true;
    else if (arg === "--no-watch") opts.watch = false;
    else if (arg === "--skip-tests") opts.tests = false;
    else if (arg === "--skip-build") opts.build = false;
    else if (arg === "--skip-notices") opts.notices = false;
    else if (arg === "--flows") opts.flows = true;
    else if (arg === "--dry-run") opts.dryRun = true;
    else if (arg.startsWith("--version=")) opts.version = arg.slice("--version=".length);
    else if (arg.startsWith("--ref=")) opts.ref = arg.slice("--ref=".length);
    else if (arg === "--help" || arg === "-h") { printHelp(); process.exit(0); }
    else { console.error(`Unknown argument: ${arg}\n`); printHelp(); process.exit(1); }
  }
  if (opts.version && !VERSION_RE.test(opts.version)) {
    console.error(`--version="${opts.version}" doesn't look like X.Y.Z or X.Y.Z-something`);
    process.exit(1);
  }
  return opts;
}

// ---------- main ----------

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const ref = opts.ref ?? capture("git", ["rev-parse", "--abbrev-ref", "HEAD"]);

  if (opts.tests) runTests(opts);
  else console.log("Skipping tests (--skip-tests)");

  banner(1, "Bumping the version");
  const oldVersion = assertVersionsConsistent();
  const newVersion = opts.version ?? bumpAlpha(oldVersion);
  console.log(`${oldVersion} -> ${newVersion}`);
  assertVersionUnclaimed(newVersion);

  bumpJsonVersion(FILES.tauriConf, oldVersion, newVersion, "tauri.conf.json");
  bumpJsonVersion(FILES.packageJson, oldVersion, newVersion, "package.json");
  bumpJsonVersion(FILES.packageLock, oldVersion, newVersion, "package-lock.json", 2);
  bumpCargoToml(oldVersion, newVersion);
  bumpCargoLock(oldVersion, newVersion);
  const changelogChanged = updateChangelog(newVersion);

  let noticesChanged = false;
  if (opts.notices) noticesChanged = regenerateNotices();
  else console.log("Skipping notices (--skip-notices)");

  if (opts.build) localBuild();
  else console.log("Skipping local build (--skip-build)");

  const changedFiles = [FILES.tauriConf, FILES.packageJson, FILES.packageLock, FILES.cargoToml, FILES.cargoLock];
  if (changelogChanged) changedFiles.push(FILES.changelog);
  if (noticesChanged) changedFiles.push(FILES.notices);

  banner("review", "Diff to be committed");
  run("git", ["diff", "--stat", "--", ...changedFiles.map(relToRepo)]);

  if (opts.dryRun) {
    console.log("\nDry run: stopping before commit. Files above are modified but NOT committed.");
    console.log(`Discard with: git checkout -- ${changedFiles.map(relToRepo).join(" ")}`);
    return;
  }

  if (!opts.yes) {
    const answer = await ask(`\nCommit, push to "${ref}", and trigger release.yml? [y/N] `);
    if (!/^y(es)?$/i.test(answer.trim())) {
      console.log("Aborted. Version bump left uncommitted in the working tree.");
      return;
    }
  }

  const sha = gitCommitAndPush(newVersion, changedFiles, ref);
  triggerWorkflow(ref);
  console.log("Waiting for the run to appear...");
  const match = await findRun(ref, sha);
  console.log(`Run: ${match.url}`);

  if (!opts.watch) {
    console.log(
      "--no-watch given: not waiting. Once it finishes, run " +
        `\`gh release view v${newVersion}\` to get the publish command.`,
    );
    return;
  }

  watchRun(match.databaseId);
  reportDraft(newVersion, sha);
}

main().catch((err) => {
  console.error(`\n${err.message}`);
  process.exit(1);
});
