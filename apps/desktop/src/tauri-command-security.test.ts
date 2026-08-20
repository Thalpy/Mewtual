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

/** Extract only literal `invoke("name")` calls, tolerating nested TypeScript generic types. */
function invokedCommands(source: string): string[] {
  const names: string[] = [];
  const marker = /\binvoke\b/g;
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

function frontendSources(dir: string): string[] {
  const sources: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) sources.push(...frontendSources(path));
    else if ([".ts", ".svelte"].includes(extname(entry.name)) && !entry.name.endsWith(".test.ts")) {
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
  const bootstrap = new Set(["vault_exists", "unlock", "resume_session", "lock_session"]);
  for (const [command, segment] of segments) {
    if (bootstrap.has(command)) continue;
    assert.match(
      segment,
      /(?:actor_of|server_actor_of|require_unlocked_session)\s*\(/,
      `${command} does not visibly cross the native session gate`,
    );
  }
});
