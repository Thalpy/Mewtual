import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { extname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { REVIEWED_TAURI_COMMANDS } from "./tauri-command-security.ts";

const sourceDir = fileURLToPath(new URL(".", import.meta.url));
const nativeDir = fileURLToPath(new URL("../src-tauri/src/", import.meta.url));
const bridgePath = join(nativeDir, "lib.rs");

/**
 * A floor on how many registered commands the extraction must find.
 *
 * This is the assertion the rest of this file was missing, and its absence is why fourteen
 * commands went unreviewed for the whole life of the Studio feature. A regex that matches nothing,
 * or that matches only part of the handler list, produces exactly the same green run as one that
 * matches everything: the ledger below is written from whatever the extractor happened to see, so
 * parity between two views of the same blind spot proves nothing at all. The old pattern could not
 * match a path-qualified registration and quietly reported 154 of 168; nothing noticed.
 *
 * The number is deliberately well under the real count, so it needs no maintenance as commands come
 * and go. Lower it only when the exposed surface has genuinely shrunk, never to get a run green.
 */
const MINIMUM_REGISTERED_COMMANDS = 150;

/**
 * A floor on how many frontend call sites the invoke scan must find, for the same reason.
 *
 * `invoked.length > 0` is not a floor: one surviving call site in one file would satisfy it while
 * every other file had silently stopped being audited.
 */
const MINIMUM_FRONTEND_INVOCATIONS = 120;

/**
 * Every native source that can define a command, keyed by its path below `src-tauri/src`.
 *
 * Commands are not all in `lib.rs`, and every helper here used to read `lib.rs` alone.
 * `creative_blobs.rs`, `studio.rs` and `studio/recovery.rs` define fourteen of them between them,
 * so those fourteen were invisible to this entire file: not classified, not checked for the session
 * gate, not even counted. Walking the tree instead of naming one file is what stops the next module
 * from disappearing the same way.
 */
function nativeSources(): Map<string, string> {
  const sources = new Map<string, string>();
  const walk = (dir: string, prefix: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path, `${prefix}${entry.name}/`);
      else if (extname(entry.name) === ".rs") sources.set(`${prefix}${entry.name}`, readFileSync(path, "utf8"));
    }
  };
  walk(nativeDir, "");
  assert.ok(sources.has("lib.rs"), "the native bridge source must remain readable from here");
  return sources;
}

/**
 * The IPC names in the `generate_handler!` list.
 *
 * A registration is either a bare function name or a module path, and Tauri derives the IPC name
 * from the function either way: `studio::recovery::studio_recovery_read` is invoked from the
 * webview as `studio_recovery_read`. The module prefix is therefore stripped rather than kept.
 * That holds only while no command configures its attribute, which the test below enforces instead
 * of assuming.
 */
function registeredCommands(source: string): string[] {
  const body = /tauri::generate_handler!\[([\s\S]*?)\]\)/.exec(source)?.[1];
  assert.ok(body, "the Tauri generate_handler list must remain statically enumerable");
  return [...body.matchAll(/^\s*(?:[a-z][a-z0-9_]*::)*([a-z][a-z0-9_]*)\s*,?\s*$/gm)].map((match) => match[1]);
}

/**
 * Command functions, wherever they are defined and whatever their visibility.
 *
 * The module commands are `pub(crate)`, which the old `(?:pub\s+)?` could not match. That is the
 * same bug as the handler-list one wearing different clothes: two independent reasons the Studio
 * surface stayed invisible, either of which was enough on its own.
 */
function commandFunctions(source: string): string[] {
  return [
    ...source.matchAll(
      /#\[tauri::command\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+([a-z][a-z0-9_]*)/g,
    ),
  ].map((match) => match[1]);
}

/**
 * Where the last command in a file stops.
 *
 * Boundaries here are deliberately generous: a segment runs to the next command attribute and so
 * may contain the helpers written between them. What it must not swallow is a `#[cfg(test)]`
 * module, because test code calls the session-gate helpers freely and would make the last command
 * in a file look gated when it is not.
 */
function endOfCommands(source: string, start: number): number {
  const stops = ["\npub fn run()", "\n#[cfg(test)]"]
    .map((marker) => source.indexOf(marker, start))
    .filter((index) => index > start);
  return stops.length > 0 ? Math.min(...stops) : source.length;
}

function commandSegments(sources: Map<string, string>): Map<string, { file: string; segment: string }> {
  const segments = new Map<string, { file: string; segment: string }>();
  for (const [file, source] of sources) {
    const attributes = [...source.matchAll(/#\[tauri::command\]/g)];
    for (let index = 0; index < attributes.length; index += 1) {
      const start = attributes[index].index;
      const end = attributes[index + 1]?.index ?? endOfCommands(source, start);
      const segment = source.slice(start, end > start ? end : undefined);
      const name = /(?:async\s+)?fn\s+([a-z][a-z0-9_]*)/.exec(segment)?.[1];
      assert.ok(name, "every Tauri command attribute must precede a named function");
      assert.ok(!segments.has(name), `two native sources define a command named ${name}`);
      segments.set(name, { file, segment });
    }
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
  const sources = nativeSources();
  const registered = registeredCommands(sources.get("lib.rs") ?? "");
  const functions = [...sources.values()].flatMap(commandFunctions);
  // The IPC name is the function name, which is what makes stripping the module path the right
  // answer above. A configured attribute (`#[tauri::command(rename = "...")]`, or an async-runtime
  // choice) would break that quietly and put a name in this ledger the webview never calls, so it
  // is refused here until whoever adds one teaches the extractor what the IPC name became.
  for (const [file, source] of sources) {
    assert.doesNotMatch(
      source,
      /#\[tauri::command\s*\(/,
      `${file} configures a tauri::command attribute; teach the extractor which IPC name it produces`,
    );
  }
  assert.ok(
    registered.length >= MINIMUM_REGISTERED_COMMANDS,
    `extraction found only ${registered.length} registered commands, below the ${MINIMUM_REGISTERED_COMMANDS} floor: the handler list is being under-read, and a ledger written from an under-read list reviews nothing`,
  );
  assert.equal(new Set(registered).size, registered.length, "duplicate command registration");
  assert.equal(new Set(REVIEWED_TAURI_COMMANDS).size, REVIEWED_TAURI_COMMANDS.length, "duplicate ledger entry");
  assert.deepEqual([...registered].sort(), [...functions].sort(), "command functions and handler list diverged");
  assert.deepEqual([...registered].sort(), [...REVIEWED_TAURI_COMMANDS].sort(), "update the security ledger");
});

test("the frontend invokes only registered, security-classified literal commands", () => {
  const registered = new Set(registeredCommands(readFileSync(bridgePath, "utf8")));
  const reviewed = new Set<string>(REVIEWED_TAURI_COMMANDS);
  const sources = frontendSources(sourceDir);
  // An aliased import would rename every call through it past the marker below, which is this
  // file's own blind spot one level down: a scan that finds nothing looks exactly like a scan that
  // finds everything. The wrapper in `diagnostics.ts` is the one sanctioned indirection.
  for (const source of sources) {
    assert.doesNotMatch(
      source,
      /import\s*\{[^}]*\binvoke\s+as\b/,
      "an aliased invoke import hides its call sites from this audit",
    );
  }
  const invoked = sources.flatMap(invokedCommands);
  assert.ok(
    invoked.length >= MINIMUM_FRONTEND_INVOCATIONS,
    `scan found only ${invoked.length} frontend IPC calls, below the ${MINIMUM_FRONTEND_INVOCATIONS} floor: call sites are being missed rather than audited`,
  );
  for (const command of invoked) {
    assert.ok(registered.has(command), `frontend invokes unregistered command: ${command}`);
    assert.ok(reviewed.has(command), `frontend invokes unreviewed command: ${command}`);
  }
});

test("every non-bootstrap native command visibly crosses the unlocked-session gate", () => {
  const segments = commandSegments(nativeSources());
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
    // This command is the lock boundary for OS close, including an already-visible vault gate. Its
    // focused Rust tests prove native locking precedes either deferral or acknowledged destroy.
    "close_vault_window",
    // Idempotent, reveals no token-existence bit, and only removes bounded native work.
    "cancel_inline_download",
    "log_ui",
    "log_ui_batch",
    "record_ui_events",
  ]);
  // Helpers that cross the gate on a command's behalf. Each one is verified to do so by the test
  // below, so recognising it here extends the guarantee transitively rather than punching a hole
  // in it. A helper added to this list without that proof would silently exempt every command
  // that calls it, which is the failure mode this whole test exists to prevent.
  //
  // `op.actor` is the diagnostics-aware wrapper: it fetches the actor and binds it to the calling
  // operation's trace in one act, so a command cannot get one without the other. It reaches the
  // gate through `actor_of`, and the test below checks that it does rather than assuming it.
  //
  // `unlocked_ui_session_generation` is the gate the module commands reach first: it takes the
  // commit lock, checks the session and returns the generation the rest of the operation is fenced
  // against. It reaches `require_unlocked_session` directly, and the test below checks that rather
  // than assuming it.
  const gatekeepers =
    "actor_of|actor_instance_of|server_actor_of|require_unlocked_session|require_ui_session_generation|unlocked_ui_session_generation|channel_target|op\\.actor";
  // Helpers trusted only inside the module that defines them.
  //
  // `invoke` is far too bare a name to trust everywhere: `lib.rs` already contains the token
  // `invoke(` in prose, and one doc comment landing inside a segment would exempt a command that
  // never gates at all. The Studio commands do gate, through their module's single custody fence,
  // and the test below proves that fence reaches the shared helper. Scoping the name to the file
  // that defines it keeps the audit honest in both directions.
  const moduleGatekeepers = new Map([
    ["studio.rs", "invoke|invoke_custody"],
    ["studio/recovery.rs", "invoke_control|invoke_custody"],
  ]);
  for (const [command, { file, segment }] of segments) {
    if (bootstrap.has(command)) continue;
    const local = moduleGatekeepers.get(file);
    assert.match(
      segment,
      new RegExp(`(?:${local ? `${gatekeepers}|${local}` : gatekeepers})\\s*\\(`),
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
  for (const helper of [
    "actor_of",
    "actor_instance_of",
    "channel_target",
    "require_ui_session_generation",
    "unlocked_ui_session_generation",
  ]) {
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

  // `op.actor` is a method rather than a free function, so it is found and bounded differently.
  // It gates transitively, through `actor_of`, which the loop above has just proved does the
  // checking. Trusting it in the audit without proving this link is what would turn a passing
  // audit into a false one.
  const method = bridge.indexOf("    async fn actor(&self, state: &AppState");
  assert.ok(method > 0, "op.actor is trusted by the audit but does not exist");
  const body = bridge.slice(method, bridge.indexOf("\n    }", method));
  assert.match(
    body,
    /actor_of\s*\(/,
    "op.actor is trusted to gate commands but never reaches a helper that checks the session",
  );

  // The Studio modules gate transitively too, and the chain is one link longer. `invoke_custody`
  // is the single Ready/lease/session fence both Studio entry points funnel through, and `invoke`
  // and `invoke_control` are one-call wrappers around it. Trusting those three names in the audit
  // above without proving each link is exactly what would turn a passing audit into a false one:
  // the fourteen Studio commands would then be classified, counted, and checked against nothing.
  const studio = readFileSync(join(nativeDir, "studio.rs"), "utf8");
  const recovery = readFileSync(join(nativeDir, "studio", "recovery.rs"), "utf8");
  const chain = [
    ["studio.rs", studio, "invoke_custody", "unlocked_ui_session_generation"],
    ["studio.rs", studio, "invoke", "invoke_custody"],
    ["studio/recovery.rs", recovery, "invoke_control", "invoke_custody"],
  ] as const;
  for (const [file, source, helper, reaches] of chain) {
    const start = source.indexOf(`async fn ${helper}`);
    assert.ok(start > 0, `${file}: ${helper} is trusted by the audit but does not exist`);
    const end = source.indexOf("\n}", start);
    assert.match(
      source.slice(start, end),
      new RegExp(`${reaches}\\s*\\(`),
      `${file}: ${helper} is trusted to gate Studio commands but never reaches ${reaches}`,
    );
  }
});

test("instrumented actor handles can only be trace-bound through Operation", () => {
  const bridge = readFileSync(bridgePath, "utf8");
  const calls = [...bridge.matchAll(/\.with_trace\s*\(/g)];
  assert.equal(
    calls.length,
    1,
    "a direct with_trace call can pass the canonical H(raw) instead of the actor token and split correlation",
  );
  const method = bridge.indexOf("    fn bind_actor(&self, actor: ServerActor)");
  assert.ok(method > 0, "Operation.bind_actor is the single trace-binding gateway");
  const body = bridge.slice(method, bridge.indexOf("\n    }", method));
  assert.match(body, /actor\.with_trace\(self\.actor_trace\.0\)/);
  assert.match(
    bridge,
    /op\.bind_actor\(entry\.actor\.clone\(\)\)/,
    "multi-server backup snapshots must bind each actor to the operation",
  );
});
