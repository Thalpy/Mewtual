import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { extname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { REVIEWED_TAURI_COMMANDS } from "./tauri-command-security.ts";

const sourceDir = fileURLToPath(new URL(".", import.meta.url));
const bridgePath = fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url));

function registeredCommands(source: string): string[] {
  const body = /tauri::generate_handler!\[([\s\S]*?)\]\)/.exec(source)?.[1];
  assert.ok(body, "the Tauri generate_handler list must remain statically enumerable");
  return [...body.matchAll(/^\s*([a-z][a-z0-9_]*)\s*,?\s*$/gm)].map((match) => match[1]);
}

function commandFunctions(source: string): string[] {
  return [...source.matchAll(/#\[tauri::command\]\s*(?:#\[[^\]]+\]\s*)*(?:pub\s+)?(?:async\s+)?fn\s+([a-z][a-z0-9_]*)/g)]
    .map((match) => match[1]);
}

function commandSegments(source: string): Map<string, string> {
  const attributes = [...source.matchAll(/#\[tauri::command\]/g)];
  const segments = new Map<string, string>();
  for (let index = 0; index < attributes.length; index += 1) {
    const start = attributes[index].index;
    const end = attributes[index + 1]?.index ?? source.indexOf("pub fn run()", start);
    const segment = source.slice(start, end > start ? end : undefined);
    const name = /(?:async\s+)?fn\s+([a-z][a-z0-9_]*)/.exec(segment)?.[1];
    assert.ok(name, "every Tauri command attribute must precede a named function");
    segments.set(name, segment);
  }
  return segments;
}

/**
 * Extract literal `invoke("name")` calls, tolerating nested TypeScript generic types.
 *
 * `invokeDebugged` counts too. It is the instrumented wrapper, and its callers name their command
 * exactly as a direct caller would; leaving it out would mean every migrated call site silently
 * stopped being audited, which would make the instrumentation a way to bypass this check.
 */
function invokedCommands(source: string): string[] {
  const names: string[] = [];
  const marker = /\binvoke(?:Debugged)?\b/g;
  for (let match = marker.exec(source); match; match = marker.exec(source)) {
    let cursor = marker.lastIndex;
    while (/\s/.test(source[cursor] ?? "")) cursor += 1;
    if (source[cursor] === "<") {
      let depth = 0;
      do {
        if (source[cursor] === "<") depth += 1;
        else if (source[cursor] === ">") depth -= 1;
        cursor += 1;
      } while (cursor < source.length && depth > 0);
      while (/\s/.test(source[cursor] ?? "")) cursor += 1;
    }
    if (source[cursor] !== "(") continue; // an import/reference, not a call
    cursor += 1;
    while (/\s/.test(source[cursor] ?? "")) cursor += 1;
    const quote = source[cursor];
    assert.ok(quote === '"' || quote === "'", "Tauri command names must be static string literals");
    const end = source.indexOf(quote, cursor + 1);
    assert.ok(end > cursor, "unterminated Tauri command literal");
    names.push(source.slice(cursor + 1, end));
  }
  return names;
}

/**
 * The one module allowed to invoke a command it was handed rather than one it names.
 *
 * `diagnostics.ts` is the instrumented-invoke wrapper: it takes a command name from its caller,
 * records the trace around the call, and forwards it. Every *caller* of it still passes a literal,
 * so the guarantee this file exists for is unchanged, and the check below still sees those
 * literals. Widening this list is a security decision: an indirect invoke is a command name that
 * cannot be audited by reading the frontend.
 */
const INVOKE_WRAPPERS = ["diagnostics.ts"];

function frontendSources(dir: string): string[] {
  const sources: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) sources.push(...frontendSources(path));
    else if (
      [".ts", ".svelte"].includes(extname(entry.name)) &&
      !entry.name.endsWith(".test.ts") &&
      !INVOKE_WRAPPERS.includes(entry.name)
    ) {
      sources.push(readFileSync(path, "utf8"));
    }
  }
  return sources;
}

test("every native command is registered exactly once and classified for security review", () => {
  const bridge = readFileSync(bridgePath, "utf8");
  const registered = registeredCommands(bridge);
  const functions = commandFunctions(bridge);
  assert.equal(new Set(registered).size, registered.length, "duplicate command registration");
  assert.equal(new Set(REVIEWED_TAURI_COMMANDS).size, REVIEWED_TAURI_COMMANDS.length, "duplicate ledger entry");
  assert.deepEqual([...registered].sort(), [...functions].sort(), "command functions and handler list diverged");
  assert.deepEqual([...registered].sort(), [...REVIEWED_TAURI_COMMANDS].sort(), "update the security ledger");
});

test("the frontend invokes only registered, security-classified literal commands", () => {
  const registered = new Set(registeredCommands(readFileSync(bridgePath, "utf8")));
  const reviewed = new Set<string>(REVIEWED_TAURI_COMMANDS);
  const invoked = frontendSources(sourceDir).flatMap(invokedCommands);
  assert.ok(invoked.length > 0, "expected to find frontend IPC calls");
  for (const command of invoked) {
    assert.ok(registered.has(command), `frontend invokes unregistered command: ${command}`);
    assert.ok(reviewed.has(command), `frontend invokes unreviewed command: ${command}`);
  }
});

test("every non-bootstrap native command visibly crosses the unlocked-session gate", () => {
  const segments = commandSegments(readFileSync(bridgePath, "utf8"));
  // Commands that must work before, or without, an unlocked session. The log_ui pair is here for
  // the same reason the vault ones are: a log that only works once you are unlocked cannot record
  // unlock failing, and cannot record the startup errors that leave a user with a blank window,
  // which is exactly when they most need something to send. Both write into the local diagnostics
  // pipeline and nothing else, both are bounded, and both are rate-limited natively.
  const bootstrap = new Set([
    "vault_exists",
    "unlock",
    "resume_session",
    "lock_session",
    "log_ui",
    "log_ui_batch",
    "record_ui_events",
  ]);
  // Helpers that cross the gate on a command's behalf. Each one is verified to do so by the test
  // below, so recognising it here extends the guarantee transitively rather than punching a hole
  // in it. A helper added to this list without that proof would silently exempt every command
  // that calls it, which is the failure mode this whole test exists to prevent.
  const gatekeepers = "actor_of|server_actor_of|require_unlocked_session|channel_target";
  for (const [command, segment] of segments) {
    if (bootstrap.has(command)) continue;
    assert.match(
      segment,
      new RegExp(`(?:${gatekeepers})\\s*\\(`),
      `${command} does not visibly cross the native session gate`,
    );
  }
});

/**
 * The gate-crossing helpers have to actually cross the gate.
 *
 * The test above trusts them on a command's behalf, so if one of them ever stopped calling
 * `require_unlocked_session` every command that delegates to it would silently become
 * unauthenticated while the audit kept passing. That is a worse outcome than having no audit,
 * because it looks like one.
 */
test("every helper the session-gate audit trusts does the checking itself", () => {
  const bridge = readFileSync(bridgePath, "utf8");
  for (const helper of ["actor_of", "channel_target"]) {
    const start = bridge.indexOf(`async fn ${helper}(`);
    assert.ok(start > 0, `${helper} is trusted by the audit but does not exist`);
    // The body runs to the next top-level item; enough to see what it calls.
    const end = bridge.indexOf("\n}", start);
    const body = bridge.slice(start, end);
    assert.match(
      body,
      /require_unlocked_session\s*\(/,
      `${helper} is trusted to gate commands but never checks the session`,
    );
  }
});
