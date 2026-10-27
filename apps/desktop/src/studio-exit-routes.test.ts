// Structural audit for the review of 149e7321, finding 2. App.svelte cannot be mounted under
// node:test, so this pins the property studio-exit.test.ts proves behaviourally: every place that
// changes the main pane away from the Studio asks the Studio exit gate (or switchView, switchServer,
// switchTo, which run the same Save inline) BEFORE it changes the route flag. Ctrl+F and
// Ctrl+Shift+F used to assign `view = "chat"` directly, destroying a drawing the full draft queue
// had refused to stage.
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { fileURLToPath } from "node:url";
import { CHANNEL, ME, SERVER, deferred, fakeIpc, flipnoteContent, id32, indexContent, ordinaryView, settle } from "./studio-testkit.ts";
import { createStudioExit } from "./studio-exit.ts";
import { decodeStudioDraft } from "./studio-draft.ts";

// Git may check out CRLF on Windows. Audit logical source lines identically on both platforms;
// every guard, forced-edge count and negative probe below remains unchanged.
const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8").replaceAll("\r\n", "\n");
// Every flag write that can take the Studio off screen: the main pane's `view`, the inbox and DM
// home (which replace that pane), the active group, and the top-level screens that replace the
// whole layout (add server, vault change, the unready and locked states). Calls of clearServerView
// (which sets `view = "chat"`) are sites too, so each of its callers must be guarded in turn.
// Entering the Studio is not a route out of it.
const ROUTE = new RegExp([
  String.raw`\bview = "(?!studio")[a-z]+"`, String.raw`\bview = v;`, String.raw`\binboxView = true\b`, String.raw`\bdmHome = true\b`,
  String.raw`\bactiveServerId = null\b`, String.raw`\bshowAdd = true\b`, String.raw`\bvaultChangeStep = "current"`, String.raw`\buiStateReady = false\b`,
  String.raw`\blocked = true\b`, String.raw`(?<!function )\bclearServerView\(\)`,
].join("|"), "g");
// Accepted guards: the gate on the same line, or earlier in the enclosing function in a checked
// form (a result that is ignored is not a guard), with NO await between that guard and the site.
// Re-review of 13e338eb: openWikiPage asked the gate, then awaited a wiki read, then changed the
// view; a newer return to the Studio (and work created there) slipped between. A guard proves
// permission only up to the next await, so the last one before the site must follow every await.
// switchView's deferred `route.leave().then(...)` counts only inside switchView, which has no await.
// Known limit: a guard in a sibling branch earlier in the same function also satisfies this.
const LINE_GUARD = /leaveStudioThen\(/;
const FUNCTION_GUARD = /leaveStudioThen\(|if \([^\n]*!\(await (?:route\.(?:leave|scope)|releaseStudio)\(\)\)(?: \|\| !route\.current\(\) \|\| windowCloseInFlight)?\) return/g;
const FORCED = "studio-exit: forced edge";

/** The function-level verdict for a site at the end of `body`, which starts at the declaration. */
function guardedInBody(name: string, body: string): boolean {
  if (name === "switchView") return /route\.leave\(\)\.then/.test(body) && !/\bawait\b/.test(body);
  const guards = [...body.matchAll(FUNCTION_GUARD)];
  if (!guards.length) return false;
  const last = guards.at(-1)!;
  return !/\bawait\b/.test(body.slice(last.index! + last[0].length));
}

type Site = { at: number; line: number; text: string; how: "line" | "function" | "forced" | "clearServerView" | "unguarded" };

function lineOf(at: number): number { return source.slice(0, at).split("\n").length; }

/** The `//` comment block directly above the line starting at `lineStart`. */
function commentAbove(lineStart: number): string {
  const lines: string[] = [];
  for (let end = lineStart - 1; end > 0;) {
    const start = source.lastIndexOf("\n", end - 1) + 1, line = source.slice(start, end);
    if (!line.trim().startsWith("//")) break;
    lines.unshift(line); end = start - 1;
  }
  return lines.join("\n");
}

/** The 2-space-indented function whose body contains `at`, if any. */
function enclosingFunction(at: number): { name: string; start: number } | null {
  const declaration = /\n {2}(?:async )?function (\w+)\(/g;
  let found: { name: string; start: number } | null = null;
  for (let m = declaration.exec(source); m && m.index < at; m = declaration.exec(source)) found = { name: m[1], start: m.index };
  if (!found) return null;
  const end = source.indexOf("\n  }\n", found.start);
  return end > at ? found : null;
}

function sites(): Site[] {
  const out: Site[] = [];
  for (const m of source.matchAll(ROUTE)) {
    const at = m.index!, lineStart = source.lastIndexOf("\n", at - 1) + 1;
    const before = source.slice(lineStart, at);
    if (before.includes("//")) continue; // prose inside a comment
    const fn = enclosingFunction(at);
    let how: Site["how"] = "unguarded";
    const body = fn ? source.slice(fn.start, at) : "";
    if (LINE_GUARD.test(before)) how = "line";
    else if (source.slice(at, source.indexOf("\n", at)).includes(FORCED) || commentAbove(lineStart).includes(FORCED)) how = "forced";
    else if (fn?.name === "clearServerView") how = "clearServerView"; // its callers are sites themselves
    else if (fn && guardedInBody(fn.name, body)) how = "function";
    out.push({ at, line: lineOf(at), text: m[0], how });
  }
  return out;
}

test("every App route away from the Studio asks the exit gate before changing the main pane", () => {
  const all = sites();
  const unguarded = all.filter(s => s.how === "unguarded");
  assert.deepEqual(unguarded.map(s => `App.svelte:${s.line} ${s.text}`), [],
    "route through leaveStudioThen/releaseStudio (or switchView), or mark a genuine forced edge with a reason");
  // The keyboard shortcuts named by the review, by their own lines.
  for (const shortcut of ['view = "chat"; openSearch(true);', 'view = "chat"; openSearch(); });']) {
    const at = source.indexOf(shortcut);
    assert.ok(at > 0, `shortcut ${shortcut} moved; update this audit`);
    assert.ok(all.some(s => s.at === at && s.how === "line"), `${shortcut} must run inside leaveStudioThen`);
  }
});

// A navigation call without a route begins a NEW action. After an await in the same function that
// is the "fresh claim" a retired action must never take, so each such call is either given the
// function's route or named here with why it is a new action of its own. The callee's own guard is
// inside the callee, so the site audit above cannot see these.
const LATE_FRESH_ACTIONS: Record<string, string> = {
  "pairJoinAll:switchServer": "a join the member started has completed; showing the joined server is its result (switchServer saves the Studio first)",
  "leaveServer:serverGone": "the native leave ended the server's session; moving off it is forced, and its fresh switch has no Studio work of that server left to save",
  "refreshChannels:switchTo": "the active channel vanished from the listing; reconciliation is forced",
  "refreshRoles:switchView": "the moderation surface's permission was lost; leaving it is forced",
  "handleRichClick:openStatusRef": "the status-chip branch begins its own action; the earlier awaits are the wikilink branch's",
};

test("no navigation call after an await takes a fresh claim unless it is a stated new action", () => {
  const declarations = [...source.matchAll(/\n {2}(?:async )?function (\w+)\(/g)].map(m => ({ name: m[1], start: m.index!, end: source.indexOf("\n  }\n", m.index!) }));
  const found: string[] = [];
  // serverGone starts a fresh switchServer synchronously, so a call to it after an await is one too.
  for (const m of source.matchAll(/\b(switchView|switchServer|switchTo|openWikiPage|openStatusRef|serverGone)\(/g)) {
    if (/function\s*$/.test(source.slice(m.index! - 12, m.index!))) continue; // the declaration itself
    const fn = declarations.filter(f => f.start < m.index! && f.end > m.index!).at(-1);
    if (!fn) continue;
    // The argument list, with nested parentheses (callbacks) balanced.
    let depth = 0, end = m.index! + m[0].length - 1;
    for (; end < source.length; end++) { if (source[end] === "(") depth++; else if (source[end] === ")" && --depth === 0) break; }
    const args = source.slice(m.index! + m[0].length, end);
    const before = source.slice(fn.start, m.index!).replace(/(?:if \(!?\(?)?await\s*$/, ""); // the call's own await
    if (/\bawait\b/.test(before) && !/\broute\b/.test(args)) found.push(`${fn.name}:${m[1]}`);
  }
  assert.deepEqual([...new Set(found)].sort(), Object.keys(LATE_FRESH_ACTIONS).sort(),
    "pass the function's route, or name the call here with why it is a new action");
});

test("forced edges are few, explicit and each carries its reason", () => {
  const forced = sites().filter(s => s.how === "forced");
  // Lock (group, clearServerView, unready, locked), a server's native session ending (serverGone,
  // for a leave or a closure: group, clearServerView), and the vault change restarting its own
  // first step after a failed attempt (already off the Studio).
  assert.equal(forced.length, 7, forced.map(s => `App.svelte:${s.line} ${s.text}`).join("\n"));
  for (const s of forced) {
    const lineStart = source.lastIndexOf("\n", s.at - 1) + 1;
    const marked = commentAbove(lineStart) + source.slice(lineStart, source.indexOf("\n", s.at));
    assert.match(marked, /studio-exit: forced edge(?:\. | \()\S/, `App.svelte:${s.line} needs a stated reason`);
  }
});

test("the audit itself fails on a direct route assignment", () => {
  // A deliberately unguarded statement in a function with no guard must classify as unguarded:
  // otherwise the first test above could pass while protecting nothing.
  const probe = "\n  function probeRoute() {\n    view = \"chat\";\n  }\n";
  const at = source.length + probe.indexOf("view =");
  const patched = source + probe;
  const m = new RegExp(ROUTE.source, "g"); m.lastIndex = source.length;
  const hit = m.exec(patched); assert.ok(hit && hit.index === at);
  const before = patched.slice(patched.lastIndexOf("\n", at - 1) + 1, at);
  assert.equal(LINE_GUARD.test(before), false);
  const fnStart = patched.lastIndexOf("\n  function probeRoute(");
  assert.equal(guardedInBody("probeRoute", patched.slice(fnStart, at)), false, "the probe's own function has no guard");
  // An ignored result is not a guard: a bare `await releaseStudio();` does not satisfy the audit.
  assert.equal(guardedInBody("f", "  async function f() {\n    await releaseStudio();\n    view = \"chat\";"), false);
  // A guard does not survive an await: the openWikiPage shape the re-review reproduced fails.
  const stale = "  async function f() {\n    if (!(await route.leave())) return;\n    const body = await invoke(\"get_wiki_page\");\n    view = \"wiki\";";
  assert.equal(guardedInBody("f", stale), false);
  assert.equal(guardedInBody("f", stale.replace("    view", "    if (!(await route.leave())) return;\n    view")), true);
});

test("switchServer and switchTo report a refused switch, and composite callers act only on success", () => {
  assert.match(source, /async function switchServer\(id: number, committed: \(\) => void = \(\) => \{\}, route: StudioRoute = studioExit\.begin\(\)\): Promise<boolean>/);
  // `committed` fires after the commit and before the batch read is awaited, in the same turn.
  const body = source.slice(source.indexOf("async function switchServer("), source.indexOf("async function leaveServer("));
  const commitAt = body.indexOf("activeServerId = id;"), hookAt = body.indexOf("committed();"), awaitAt = body.indexOf("await batch;");
  assert.ok(commitAt > 0 && hookAt > commitAt && awaitAt > hookAt && !/\bawait\b/.test(body.slice(commitAt, hookAt)), "no await between commit and the hook");
  assert.match(source, /async function switchTo\(id: string, keepSearch = false, route: StudioRoute = studioExit\.begin\(\)\): Promise<boolean>/);
  // With no Studio work and a current route the switches await nothing before committing.
  for (const name of ["switchServer", "switchTo"]) {
    const fn = source.slice(source.indexOf(`async function ${name}(`));
    assert.match(fn.slice(0, 600), /if \(\(!route\.current\(\) \|\| studioHasWork\?\.\(\)\) && !\(await route\.scope\(\)\)\) return false;/, name);
  }
  // No caller may await a switch as a bare statement and then carry on regardless of its outcome.
  // The one bare await left is goMention's final step: nothing runs after it.
  const bare = [...source.matchAll(/^[ \t]*await switch(?:Server|To)\([^;]*\);/gm)];
  assert.deepEqual(bare.map(m => m[0].trim()), ["await switchTo(channel, false, route);"], bare.map(m => `App.svelte:${lineOf(m.index!)}`).join(", "));
  const mention = source.slice(bare[0].index!, source.indexOf("\n  }\n", bare[0].index!));
  assert.match(mention, /^\s*await switchTo\(channel, false, route\);\s*\} finally \{\s*navStepEnd\(\);\s*\}$/, "goMention does nothing after its switch");
  // Every arriveAtServer call carries its action's route: a fresh claim mid-action is the defect.
  assert.deepEqual([...source.matchAll(/arriveAtServer\(([^)]*)\)/g)].map(m => m[1]).filter(a => !/, route$/.test(a) && !a.includes(":")), []);
});

test("every awaited Studio route checks live ownership again before its continuation", () => {
  const leaves = [...source.matchAll(/^.*if \(!\(await route\.leave\(\)\).*$/gm)];
  assert.ok(leaves.length > 10);
  for (const line of leaves) assert.match(line[0], /\|\| !route\.current\(\) \|\| windowCloseInFlight/, `App.svelte:${lineOf(line.index!)}`);
  for (const name of ["switchServer", "switchTo"]) {
    const fn = source.slice(source.indexOf(`async function ${name}(`));
    assert.match(fn.slice(0, fn.indexOf("saveDraftFor(")), /await route\.scope\(\).*\n\s*if \(!route\.current\(\) \|\| windowCloseInFlight\) return false;/);
  }
  assert.equal(/await releaseStudio\(\)/.test(source), false, "awaiting callers retain their route for a mutation-time check");
});

/** The source of App's 2-space function `name`: its parameter list and body matched by bracket
 * depth, so one-line helpers and multi-line functions both come out whole. */
function functionText(name: string): string {
  const start = source.search(new RegExp(String.raw`\n {2}(?:async )?function ${name}\(`)) + 1;
  assert.ok(start > 0, `${name} not found in App.svelte`);
  const match = (open: string, close: string, from: number) => {
    let depth = 0;
    for (let i = from; i < source.length; i++) { if (source[i] === open) depth++; else if (source[i] === close && --depth === 0) return i; }
    assert.fail(`${name}: unbalanced ${open}${close}`);
  };
  const paramsEnd = match("(", ")", source.indexOf("(", start));
  return source.slice(start, match("{", "}", source.indexOf("{", paramsEnd)) + 1);
}

/** The production text of `names`, types stripped, evaluated with `state` as its scope. */
function production<T>(names: string[], state: object): T {
  const parts = names.map(name => `const ${name} = (${stripTypeScriptTypes(functionText(name), { mode: "strip" }).trim()});`);
  return new Function("state", `with (state) { ${parts.join("\n")} return { ${names.join(", ")} }; }`)(state) as T;
}

test("App's direct Inbox route stays put while window close is draining state", async () => {
  for (const view of ["studio", "chat"]) {
    let loads = 0;
    const state = { view, windowCloseInFlight: true, inboxView: false, dmHome: true,
      newsUnseen: false, inboxMode: "mentions", loadNews: () => { loads++; }, loadInbox: () => { loads++; },
      leaveStudioThen: null as unknown as ReturnType<typeof createStudioExit>["leaveThen"] };
    state.leaveStudioThen = createStudioExit({ onStudio: () => state.view === "studio", hasWork: () => false,
      closing: () => state.windowCloseInFlight, claim: () => () => true, save: async () => true }).leaveThen;
    const app = production<{ openInbox: () => void }>(["openInbox"], state);
    app.openInbox(); await flush();
    assert.equal(state.inboxView, false); assert.equal(state.dmHome, true); assert.equal(loads, 0);
    state.windowCloseInFlight = false; app.openInbox();
    assert.equal(state.inboxView, true); assert.equal(state.dmHome, false); assert.equal(loads, 1);
  }
});

test("a delayed wiki rename never relabels a replacement page or session", async () => {
  for (const replacement of ["server", "unlock", "page", "close"] as const) {
    const renamed = deferred<void>();
    let refreshes = 0;
    const state = {
      viewGeneration: 1, activeServerId: 7, locked: false, windowCloseInFlight: false,
      activeWikiPage: "Home", wikiPaneSerial: 1, wikiBody: "original body", wikiRenameTo: "Renamed", wikiRenaming: true,
      studioExit: createStudioExit({ onStudio: () => false, hasWork: () => false, closing: () => state.windowCloseInFlight,
        claim: () => () => false, save: async () => true }),
      unlockedViewCurrent: (gen: number, server: number) => !state.locked && gen === state.viewGeneration && server === state.activeServerId,
      invoke: (name: string, args: { server: number; from: string; to: string }) => {
        assert.equal(name, "rename_wiki_page"); assert.deepEqual(args, { server: 7, from: "Home", to: "Renamed" });
        return renamed.promise;
      },
      refreshWiki: async () => { refreshes++; }, openWikiPage: async () => false, toast: () => {},
    };
    const app = production<{ commitWikiRename: () => Promise<void> }>(["commitWikiRename"], state);
    const renaming = app.commitWikiRename();
    if (replacement === "server") { state.activeServerId = 8; state.viewGeneration++; }
    if (replacement === "unlock") state.viewGeneration += 2; // lock and authenticated reopen of the same server
    if (replacement === "page") state.wikiPaneSerial++; // same server/name opened again
    if (replacement === "close") state.windowCloseInFlight = true;
    state.wikiBody = "replacement body";
    renamed.resolve(); await renaming;
    assert.equal(state.activeWikiPage, "Home", replacement);
    assert.equal(state.wikiBody, "replacement body", replacement);
    if (replacement !== "page") assert.equal(refreshes, 0, "a retired session does not refresh another scope");
  }
});

test("a completed rename still updates its exact original displayed page after route retirement", async () => {
  const state = {
    viewGeneration: 1, activeServerId: 7, locked: false, windowCloseInFlight: false,
    activeWikiPage: "Home", wikiPaneSerial: 1, wikiBody: "kept local body", wikiRenameTo: "Renamed", wikiRenaming: true,
    studioExit: createStudioExit({ onStudio: () => false, hasWork: () => false, closing: () => false,
      claim: () => () => false, save: async () => true }),
    unlockedViewCurrent: (gen: number, server: number) => !state.locked && gen === state.viewGeneration && server === state.activeServerId,
    invoke: async () => {}, refreshWiki: async () => {}, openWikiPage: async () => false, toast: () => {},
  };
  const app = production<{ commitWikiRename: () => Promise<void> }>(["commitWikiRename"], state);
  await app.commitWikiRename();
  assert.equal(state.activeWikiPage, "Renamed"); assert.equal(state.wikiBody, "kept local body");
  assert.equal(state.wikiPaneSerial, 2, "reconciliation retires the old displayed-page identity");
});

test("wiki rename rechecks its original page after refresh and after the attempted open", async () => {
  for (const phase of ["refresh", "open"] as const) {
    const waiting = deferred<void>();
    let entered = false;
    const state = {
      viewGeneration: 1, activeServerId: 7, locked: false, windowCloseInFlight: false,
      activeWikiPage: "Home", wikiPaneSerial: 1, wikiBody: "old", wikiRenameTo: "Renamed", wikiRenaming: true,
      studioExit: createStudioExit({ onStudio: () => false, hasWork: () => false, closing: () => false,
        claim: () => () => false, save: async () => true }),
      unlockedViewCurrent: (gen: number, server: number) => !state.locked && gen === state.viewGeneration && server === state.activeServerId,
      invoke: async () => {},
      refreshWiki: async () => { if (phase === "refresh") { entered = true; await waiting.promise; } },
      openWikiPage: async () => { entered = true; await waiting.promise; return false; }, toast: () => {},
    };
    const app = production<{ commitWikiRename: () => Promise<void> }>(["commitWikiRename"], state);
    const renaming = app.commitWikiRename(); await flush(); assert.equal(entered, true);
    state.wikiPaneSerial++; state.wikiBody = "reopened Home";
    waiting.resolve(); await renaming;
    assert.equal(state.activeWikiPage, "Home", phase); assert.equal(state.wikiBody, "reopened Home", phase);
  }
});

test("a channel-open completion cannot add its channel to a replacement server", async () => {
  for (const replacement of ["server", "unlock", "route", "close"] as const) {
    const opened = deferred<string>();
    let current = true, switches = 0;
    const state = { newChannel: "new-channel", activeServerId: 7, viewGeneration: 1, locked: false, windowCloseInFlight: false, error: "",
      cur: { channels: [{ id: "original", name: "original" }] },
      studioExit: createStudioExit({ onStudio: () => false, hasWork: () => false, closing: () => false,
        claim: () => () => current, save: async () => true }),
      unlockedViewCurrent: (gen: number, server: number) => !state.locked && gen === state.viewGeneration && server === state.activeServerId,
      invoke: (name: string, args: { server: number; name: string }) => {
        assert.equal(name, "open_channel"); assert.deepEqual(args, { server: 7, name: "new-channel" }); return opened.promise;
      },
      switchTo: () => { switches++; },
    };
    const app = production<{ addChannel: () => Promise<void> }>(["addChannel"], state);
    const opening = app.addChannel();
    if (replacement === "server") state.activeServerId = 8;
    if (replacement === "server" || replacement === "unlock") state.viewGeneration++;
    if (replacement === "close") state.windowCloseInFlight = true;
    else current = false;
    state.cur = { channels: [{ id: "replacement", name: "replacement" }] };
    opened.resolve("created-in-original-server"); await opening;
    assert.deepEqual(state.cur.channels, [{ id: "replacement", name: "replacement" }], replacement);
    assert.equal(switches, 0);
  }
});

// Re-review L5: a refused Back/Forward step left the cursor on the entry it never reached, so the
// recording effect appended the place still on screen after it and dropped the forward trail.
test("a refused Back step leaves the history cursor on the place still on screen", async () => {
  const chat = { area: "group", server: 1, view: "chat", channel: "a", page: "", folder: "" };
  const studioPlace = { area: "group", server: 1, view: "studio", channel: "a", page: "", folder: "" };
  let release = false, work = true, cursorWhenRecordingResumed = -2;
  const views: string[] = [];
  const state = {
    navStack: [chat, studioPlace], navAt: 1, applying: false, windowCloseInFlight: false,
    get navApplying() { return state.applying; },
    set navApplying(on: boolean) { if (!on) cursorWhenRecordingResumed = state.navAt; state.applying = on; },
    sameLoc: (a: typeof chat, b: typeof chat) => JSON.stringify(a) === JSON.stringify(b),
    studioExit: createStudioExit({ onStudio: () => true, hasWork: () => work, closing: () => false, claim: () => () => true,
      save: async () => { if (release) work = false; return release; } }),
    tick: async () => {}, arriveAtServer: async () => true, switchTo: async () => true,
    switchView: (v: string) => { views.push(v); }, openInbox: () => {}, enterDmHome: () => {}, refreshWiki: async () => {}, openWikiPage: async () => {},
    inboxView: false, menu: null, view: "studio", folder: "", cur: { active: "a" },
  };
  const nav = production<{ navGo: (i: number) => void }>(["applyLoc", "navDrain", "navGo"], state);
  const flush = async () => { for (let i = 0; i < 6; i++) await new Promise(resolve => setImmediate(resolve)); };
  nav.navGo(0); await flush();
  assert.equal(cursorWhenRecordingResumed, 1, "recording resumed with the cursor back on the Studio entry");
  assert.equal(state.navAt, 1); assert.deepEqual(views, [], "nothing of the refused place was applied");
  release = true; nav.navGo(0); await flush();
  assert.equal(state.navAt, 0); assert.deepEqual(views, ["chat"]);
});

/** The real exit gate over a scripted Studio: `work` is retained edits, each Save waits on `saved`. */
function scriptedStudio(onStudio: () => boolean) {
  let serial = 0, pending: ReturnType<typeof deferred<boolean>> | null = null;
  const studio = { work: false, saves: 0 };
  const exit = createStudioExit({
    onStudio, hasWork: () => studio.work, closing: () => false,
    claim: () => { const mine = ++serial; return () => mine === serial; },
    save: () => { studio.saves++; pending = deferred<boolean>(); return pending.promise.then(ok => { if (ok) studio.work = false; return ok; }); },
  });
  return { studio, exit, saved: (ok: boolean) => pending!.resolve(ok) };
}
const flush = async () => { for (let i = 0; i < 6; i++) await new Promise(resolve => setImmediate(resolve)); };

// Adversarial review of the route fix: Back from the Studio to a chat entry in another channel moved
// the channel under the still-mounted Studio. The Studio's scope effect ends every navigation lease
// (the session generation), so the action retired itself and switchView did nothing.
test("Back from the Studio to a chat in another channel shows that chat in one step", async () => {
  const chat = { area: "group", server: 1, view: "chat", channel: "Y", page: "", folder: "" };
  const studioPlace = { area: "group", server: 1, view: "studio", channel: "X", page: "", folder: "" };
  let serial = 0;
  const views: string[] = [];
  const state = {
    navStack: [chat, studioPlace], navAt: 1, navApplying: false, view: "studio", inboxView: false, menu: null, folder: "", windowCloseInFlight: false,
    cur: { active: "X" },
    sameLoc: (a: typeof chat, b: typeof chat) => JSON.stringify(a) === JSON.stringify(b),
    studioExit: createStudioExit({ onStudio: () => state.view === "studio", hasWork: () => false, closing: () => false,
      claim: () => { const mine = ++serial; return () => mine === serial; }, save: async () => true }),
    tick: async () => {}, arriveAtServer: async () => true, openInbox: () => {}, enterDmHome: () => {}, refreshWiki: async () => {}, openWikiPage: async () => true,
    // App's switchView with a route: a retired route changes nothing.
    switchView: (v: string, route: { current: () => boolean }) => { if (!route.current()) return; views.push(v); state.view = v; },
    // The channel moves; a Studio still mounted then changes scope, which ends every lease.
    switchTo: async (id: string) => { state.cur.active = id; if (state.view === "studio") serial++; await flush(); return true; },
  };
  const nav = production<{ navGo: (i: number) => void }>(["applyLoc", "navDrain", "navGo"], state);
  nav.navGo(0); await flush();
  assert.deepEqual(views, ["chat"], "the chat surface was shown"); assert.equal(state.cur.active, "Y");
  assert.equal(state.navAt, 0, "the cursor landed on the chat entry");
});

// Adversarial review of the route fix: "Rename page…"/"Delete page…" chained their arming onto
// openWikiPage, so a retired request armed Delete on the page still shown, not the one chosen.
test("a wiki open retired by a newer action reports it, so Delete cannot arm on the page still shown", async () => {
  const retired = productionOpenWikiPage();
  retired.state.activeWikiPage = "Shown";
  const opening = retired.open("Chosen"); await flush();
  retired.studio.exit.begin(); // the wrench, a tab, a quick switch: any newer action
  await retired.respond();
  assert.equal(await opening, false); assert.equal(retired.state.activeWikiPage, "Shown");
  const plain = productionOpenWikiPage(); const opened = plain.open("Page"); await plain.respond();
  assert.equal(await opened, true); assert.equal(plain.state.activeWikiPage, "Page");
  // The menu continuations act only on the page their own request opened.
  for (const act of ["startWikiRename()", "armWikiDelete()"]) {
    assert.match(source, new RegExp(String.raw`openWikiPage\(p\)\.then\(\(opened\) => \{ if \(opened && activeWikiPage === p\) ${act.replace(/[()]/g, "\\$&")}; \}\)`), act);
  }
  assert.match(source, /if \(\(await openWikiPage\(name, \{\}, route\)\) && activeWikiPage === name\) wikiEdit = true;/);
});

// Re-review of 22e4ed77, finding 1: a rename on server A that completed after the member switched to
// server B (and opened B's own "Shared") retargeted B's pane to "Renamed" by name alone, so B's next
// Save wrote B's "Shared" body over B's "Renamed". This runs the production commitWikiRename.
function productionRename(listing: Promise<void> = Promise.resolve()) {
  const renamed = deferred<void>();
  const studio = scriptedStudio(() => false);
  const state = {
    wikiRenameTo: "Renamed", wikiRenaming: true, activeWikiPage: "Shared", activeServerId: 1 as number | null, viewGeneration: 1, wikiPaneSerial: 0, refreshes: 0, windowCloseInFlight: false,
    viewCurrent: (gen: number, server: number | null) => gen === state.viewGeneration && server === state.activeServerId,
    unlockedViewCurrent: (gen: number, server: number | null): boolean => state.viewCurrent(gen, server),
    invoke: async (cmd: string) => { assert.equal(cmd, "rename_wiki_page"); await renamed.promise; },
    refreshWiki: async () => { state.refreshes++; await listing; },
    // openWikiPage's contract: true only when it opened the page (which bumps the pane serial), and
    // a retired route never does.
    openWikiPage: async (name: string, _opts: unknown, route: { current: () => boolean }) => { if (!route.current()) return false; state.activeWikiPage = name; state.wikiPaneSerial++; return true; },
    toast: () => {}, studioExit: studio.exit,
  };
  const app = production<{ commitWikiRename: () => Promise<void> }>(["wikiPane", "wikiPaneCurrent", "commitWikiRename"], state);
  return { state, studio, commit: app.commitWikiRename, land: async () => { renamed.resolve(); await flush(); } };
}

test("a wiki rename that lands after a server switch never retargets the other server's page", async () => {
  const across = productionRename();
  const renaming = across.commit(); await flush();
  // Switch to server B (switchServer claims and moves the generation) and open B's own "Shared".
  across.studio.exit.begin(); across.state.activeServerId = 2; across.state.viewGeneration++; across.state.activeWikiPage = "Shared";
  await across.land(); await renaming;
  assert.equal(across.state.activeWikiPage, "Shared", "B's page keeps its identity, so B's Save keeps its target");
  assert.equal(across.state.refreshes, 0, "A's page list is not reloaded into B's pane");

  // Same server, but a newer action retired the open: the pane still shows A's renamed page, under
  // the new name (the intended fallback).
  const retired = productionRename();
  const pending = retired.commit(); await flush();
  retired.studio.exit.begin();
  await retired.land(); await pending;
  assert.equal(retired.state.activeWikiPage, "Renamed");

  // Same server and still current: the renamed page opens.
  const plain = productionRename();
  const done = plain.commit(); await plain.land(); await done;
  assert.equal(plain.state.activeWikiPage, "Renamed"); assert.equal(plain.state.refreshes, 1);

  // Adversarial review: while the page list reloads, the member opens "Shared" again (still in the
  // tree). Reads run in order, so that pane holds the post-rename, empty "Shared". The rename's
  // fallback must not rename that pane to "Renamed", whose next Save would empty the real page.
  const list = deferred<void>(), reopened = productionRename(list.promise);
  const committing = reopened.commit(); await reopened.land();
  reopened.studio.exit.begin(); reopened.state.activeWikiPage = "Shared"; reopened.state.wikiPaneSerial++; // the newer open
  list.resolve(); await flush(); await committing;
  assert.equal(reopened.state.activeWikiPage, "Shared", "the newer pane keeps its own identity");
});

// Adversarial review: the older wiki actions wrote to whatever pane or server was current when they
// resumed. These run their production text across a pane change and a server switch.
function productionWikiActions() {
  const pending = deferred<boolean>(), history = deferred<unknown[]>();
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  const state = {
    activeServerId: 1 as number | null, viewGeneration: 1, wikiPaneSerial: 0, locked: false,
    activeWikiPage: "X", wikiBody: "sent text", wikiDirty: true, wikiEdit: true, wikiDrafts: new Map<string, string>(),
    wikiPages: ["X"], wikiMeta: {} as Record<string, string>, wikiFormat: "md", wikiReviewDays: 3, wikiDeleteArmed: true, wikiRedirectedFrom: "",
    wikiHistory: [] as unknown[], wikiHistorySel: "", showWikiHistory: false,
    viewCurrent: (gen: number, server: number | null) => gen === state.viewGeneration && server === state.activeServerId,
    unlockedViewCurrent: (gen: number, server: number | null) => !state.locked && gen === state.viewGeneration && server === state.activeServerId,
    invokeDebugged: async (cmd: string, args: Record<string, unknown>) => { calls.push({ cmd, args }); return { value: await pending.promise }; },
    invoke: async (cmd: string, args: Record<string, unknown>) => { calls.push({ cmd, args }); if (cmd === "get_wiki_history") return history.promise; await pending.promise; },
    refreshWiki: async () => {}, toast: (_text: string) => {},
  };
  const app = production<{ saveWikiPage: () => Promise<void>; deleteWikiPage: (name: string) => Promise<void>; setWikiPageFormat: (fmt: string) => Promise<void>; openWikiHistory: () => Promise<void>; restoreWikiRev: (rev: string) => Promise<void> }>(
    ["wikiPane", "wikiPaneCurrent", "saveWikiPage", "deleteWikiPage", "setWikiPageFormat", "openWikiHistory", "restoreWikiRev"], state);
  const openPage = (page: string, body: string) => { state.activeWikiPage = page; state.wikiPaneSerial++; state.wikiBody = body; state.wikiDirty = true; };
  const switchServer = (id: number) => { state.activeServerId = id; state.viewGeneration++; };
  return { state, app, calls, openPage, switchServer, answer: async (ok: boolean) => { pending.resolve(ok); await flush(); }, history };
}

test("a wiki Save receipt keeps newer typing stashed by a page open that is still reading", async () => {
  const saved = deferred<boolean>(), readY = deferred<string>();
  const state = {
    view: "wiki", activeServerId: 1 as number | null, viewGeneration: 1, wikiPaneSerial: 0, windowCloseInFlight: false, error: "",
    activeWikiPage: "X", wikiBody: "saved C", wikiDirty: true, wikiEdit: true, wikiDrafts: new Map<string, string>(), wikiPages: ["X", "Y"],
    wikiRenaming: false, wikiDeleteArmed: false, wikiRedirectedFrom: "", wikiReviewOpen: false, wikiHistorySel: "", showWikiHistory: false, wikiReviewDays: 3,
    showInsert: false, insertTarget: "", closeInsert: () => {}, parseRedirect: () => null, revealWikiPage: () => {},
    viewCurrent: (gen: number, server: number | null): boolean => gen === state.viewGeneration && server === state.activeServerId,
    invokeDebugged: async () => ({ value: await saved.promise }),
    invoke: async (_cmd: string, args: { name: string }) => args.name === "Y" ? readY.promise : "saved C",
    refreshWiki: async () => {}, toast: () => {}, studioExit: scriptedStudio(() => false).exit,
  };
  const app = production<{ saveWikiPage: () => Promise<void>; openWikiPage: (name: string) => Promise<boolean> }>(
    ["wikiPane", "wikiPaneCurrent", "saveWikiPage", "openWikiPage"], state);
  const saving = app.saveWikiPage();
  state.wikiBody = "newer N";
  const openingY = app.openWikiPage("Y"); await flush();
  assert.equal(state.wikiDrafts.get("X"), "newer N", "opening Y stashes the newer text while its read waits");
  saved.resolve(false); await saving;
  assert.equal(state.wikiDirty, true);
  assert.equal(state.wikiDrafts.get("X"), "newer N", "saving C cannot delete the already stashed N");
  readY.resolve("Y body"); assert.equal(await openingY, true);
  assert.equal(await app.openWikiPage("X"), true);
  assert.equal(state.wikiBody, "newer N"); assert.equal(state.wikiDirty, true); assert.equal(state.wikiEdit, true);
});

test("wiki save, delete, format and history act only on the page and server they began on", async () => {
  // Save X, then open Y and type before the receipt: Y stays dirty (its text was never sent).
  const save = productionWikiActions(); const saving = save.app.saveWikiPage();
  save.openPage("Y", "unsent text for Y"); await save.answer(false); await saving;
  assert.equal(save.state.wikiDirty, true, "Y's unsent text stays unsaved");
  // More typing in X itself during the save also keeps X dirty.
  const typed = productionWikiActions(); const savingX = typed.app.saveWikiPage();
  typed.state.wikiBody = "sent text, then more"; await typed.answer(false); await savingX;
  assert.equal(typed.state.wikiDirty, true);
  // X was restored from an older stash and edited further: saving the pane retires that stash, or
  // reopening X would bring the older draft back over the saved text.
  const stale = productionWikiActions(); stale.state.wikiDrafts.set("X", "older draft");
  const savingStale = stale.app.saveWikiPage(); await stale.answer(false); await savingStale;
  assert.equal(stale.state.wikiDrafts.has("X"), false); assert.equal(stale.state.wikiDirty, false);

  // Delete A's "X", switch to B and open B's own "X" meanwhile: B's pane keeps its unsaved text.
  const del = productionWikiActions(); const deleting = del.app.deleteWikiPage("X");
  del.switchServer(2); del.openPage("X", "B's unsaved X");
  await del.answer(true); await deleting;
  assert.equal(del.state.activeWikiPage, "X"); assert.equal(del.state.wikiBody, "B's unsaved X"); assert.equal(del.state.wikiDirty, true);
  assert.equal(del.calls[0].args.server, 1);

  // Format chosen for A's new page "N" (unsaved, so it saves first); a switch to B during that save
  // must not set the shared format on B's "N".
  const fmt = productionWikiActions(); fmt.state.activeWikiPage = "N"; fmt.state.wikiPages = [];
  const formatting = fmt.app.setWikiPageFormat("wiki"); await flush();
  fmt.switchServer(2); fmt.state.activeWikiPage = "N"; fmt.state.wikiPaneSerial++;
  await fmt.answer(false); await formatting;
  const set = fmt.calls.find(c => c.cmd === "set_wiki_format")!;
  assert.deepEqual([set.args.server, set.args.name], [1, "N"]); assert.deepEqual(fmt.state.wikiMeta, {}, "B's page metadata is untouched");

  // Restoring X's revision, then opening Y before the reply: the toast names X, Y's history stays.
  const restore = productionWikiActions(); const toasts: string[] = []; restore.state.toast = (text: string) => { toasts.push(text); };
  restore.state.showWikiHistory = true;
  const restoring = restore.app.restoreWikiRev("r1"); restore.openPage("Y", "y"); restore.state.showWikiHistory = true;
  await restore.answer(false); await restoring;
  assert.deepEqual(restore.calls[0].args, { server: 1, page: "X", rev: "r1" });
  assert.match(toasts[0], /Restored "X"/); assert.equal(restore.state.showWikiHistory, true, "Y's open history is not closed by X's restore");

  // History read for X lands nowhere once the pane moved on, or behind a lock.
  for (const move of ["page", "lock"] as const) {
    const h = productionWikiActions(); const opening = h.app.openWikiHistory();
    if (move === "page") h.openPage("Y", "y"); else h.state.locked = true;
    h.history.resolve([{ id: "r1", body: "X's old body" }]); await flush(); await opening;
    assert.deepEqual(h.state.wikiHistory, [], `${move}: revisions never reach another pane or the lock screen`);
    assert.equal(h.state.showWikiHistory, false);
  }
});

// Review of 149e7321, finding 3, and its re-review: "Server settings" on server B, while the Studio
// saved first, opened and seeded the dialog from server A (whose Rename then renamed A); and then an
// older B request that was still waiting replaced a NEWER settings dialog for A. This runs the
// production openServerSettings/seedServerSettings text over the real exit gate and a scripted
// switchServer with App's contract: it saves through `route.scope()` when the Studio holds work,
// commits (clearServerView closes the dialog), runs `committed` in that same turn, and resolves
// only once the new server's batch read is done.
function productionOpenServerSettings() {
  const servers = [{ id: 1, name: "One" }, { id: 2, name: "Two" }, { id: 3, name: "Three" }];
  const studio = scriptedStudio(() => true);
  const loaded = deferred<void>();
  const state = {
    activeServerId: 1 as number | null, communicationMode: "stale" as string | null, communicationModeError: "stale",
    serverNameDraft: "", sharedNameDraft: "", serverSettingsPage: "overview", setSearch: "x", showServerSettings: false,
    liveryLoaded: false, livery: { name: "" }, liveryDraft: null as unknown, liveryDraftFor: null as unknown, inviteRevealed: null as unknown,
    get cur() { return servers.find(s => s.id === state.activeServerId) ?? null; },
    serverLabel: (s: { name: string }) => s.name, emptyLivery: () => ({}), seedLiveryDraft: () => {}, precheckInviteRoutes: async () => {},
    studioExit: studio.exit, studioHasWork: () => studio.studio.work,
    async switchServer(id: number, committed: () => void = () => {}, route = studio.exit.begin()): Promise<boolean> {
      if ((!route.current() || studio.studio.work) && !(await route.scope())) return false;
      state.activeServerId = id; state.showServerSettings = false; committed();
      await loaded.promise;
      return true;
    },
  };
  const app = production<{ openServerSettings: (id?: number | null, page?: string) => void }>(["openServerSettings", "seedServerSettings"], state);
  return { state, open: app.openServerSettings, studio, saved: async (ok: boolean) => { studio.saved(ok); await flush(); }, load: async () => { loaded.resolve(); await flush(); } };
}

test("server settings for another server open only once the switch to it committed, never on the server left", async () => {
  // Nothing for the Studio to save: the switch commits in the same turn and the dialog opens on B
  // at once, without waiting for B's reads (the existing behaviour).
  const now = productionOpenServerSettings(); now.open(2);
  assert.equal(now.state.activeServerId, 2); assert.equal(now.state.showServerSettings, true); assert.equal(now.state.serverNameDraft, "Two");

  // The Studio saves first and succeeds: nothing opens meanwhile, then B's dialog opens as the
  // switch commits, without waiting for B's reads.
  const later = productionOpenServerSettings(); later.studio.studio.work = true; later.open(2);
  assert.equal(later.state.showServerSettings, false, "no dialog while the switch is still waiting on Save");
  assert.equal(later.state.serverNameDraft, "", "nothing is seeded from the server being left");
  await later.saved(true);
  assert.equal(later.state.activeServerId, 2); assert.equal(later.state.showServerSettings, true); assert.equal(later.state.serverNameDraft, "Two");
  // B's load finishing later must not re-open or re-seed the dialog the member is now editing.
  later.state.serverNameDraft = "typed"; later.state.serverSettingsPage = "invites";
  await later.load();
  assert.equal(later.state.serverNameDraft, "typed"); assert.equal(later.state.serverSettingsPage, "invites");

  // Save refuses: the member stays on A and no dialog opens whose Rename would target A.
  const refused = productionOpenServerSettings(); refused.studio.studio.work = true; refused.open(2);
  await refused.saved(false);
  assert.equal(refused.state.activeServerId, 1); assert.equal(refused.state.showServerSettings, false); assert.equal(refused.state.serverNameDraft, "");

  // Re-review: while B's request waits on the Save, the member opens A's settings and types. The
  // newer request owns navigation, so when the Save confirms, the older B switch never commits: it
  // neither closes A's dialog nor seeds B's draft over the one being edited.
  const newer = productionOpenServerSettings(); newer.studio.studio.work = true; newer.open(2);
  newer.open(1);
  assert.equal(newer.state.showServerSettings, true); assert.equal(newer.state.serverNameDraft, "One");
  newer.state.serverNameDraft = "typed for A";
  await newer.saved(true); await newer.load();
  assert.equal(newer.state.activeServerId, 1, "the retired switch to B never happened");
  assert.equal(newer.state.showServerSettings, true); assert.equal(newer.state.serverNameDraft, "typed for A");
});

// Re-review of 13e338eb, finding 1: openWikiPage asked the exit gate, then awaited the wiki read, and
// then changed the view on the strength of that earlier permission. Returning to the Studio in
// between (and drawing what the full draft queue refuses) lost the drawing when the old response
// arrived. This runs the production openWikiPage over the real exit gate.
function productionOpenWikiPage() {
  const read = deferred<string>();
  const state = {
    view: "chat", viewGeneration: 1, activeServerId: 7 as number | null, error: "", windowCloseInFlight: false,
    wikiDirty: false, activeWikiPage: "", wikiPaneSerial: 0, wikiDrafts: new Map<string, string>(), wikiBody: "studio was here", wikiPages: ["Page"],
    showInsert: false, insertTarget: "", closeInsert: () => {}, parseRedirect: () => null, revealWikiPage: () => {},
    wikiRedirectedFrom: "", wikiRenaming: false, wikiDeleteArmed: false, showWikiHistory: false, wikiReviewOpen: false, wikiHistorySel: "", wikiEdit: false,
    viewCurrent: (gen: number, server: number | null) => gen === state.viewGeneration && server === state.activeServerId,
    invoke: () => read.promise,
    studioExit: null as unknown as ReturnType<typeof createStudioExit>,
  };
  const studio = scriptedStudio(() => state.view === "studio");
  state.studioExit = studio.exit;
  const app = production<{ openWikiPage: (name: string) => Promise<void> }>(["openWikiPage"], state);
  return { state, studio, open: app.openWikiPage, respond: async () => { read.resolve("# Page"); await flush(); } };
}

test("a wiki response that arrives after the member went back to the Studio never replaces the Studio", async () => {
  // Superseded: returning to the Studio (switchView claims a newer route) with unstageable work.
  const back = productionOpenWikiPage();
  const opening = back.open("Page"); await flush();
  back.studio.exit.begin(); back.state.view = "studio"; back.studio.studio.work = true;
  await back.respond(); await opening;
  assert.equal(back.state.view, "studio", "the old response was discarded");
  assert.equal(back.state.wikiBody, "studio was here"); assert.equal(back.studio.studio.saves, 0, "nothing was asked of the newer Studio route");

  // Still current, but the Studio stayed on screen during the read and gained work there: the same
  // route must save it before leaving, and a refusal keeps the Studio.
  const stayed = productionOpenWikiPage(); stayed.state.view = "studio";
  const waiting = stayed.open("Page"); await flush();
  stayed.studio.studio.work = true;
  await stayed.respond(); assert.equal(stayed.studio.studio.saves, 1, "the route saved the new work before leaving");
  stayed.studio.saved(false); await flush(); await waiting;
  assert.equal(stayed.state.view, "studio"); assert.equal(stayed.state.wikiBody, "studio was here");

  // Still current and nothing to save: the page opens.
  const plain = productionOpenWikiPage();
  const opened = plain.open("Page"); await plain.respond(); await opened;
  assert.equal(plain.state.view, "wiki"); assert.equal(plain.state.wikiBody, "# Page");
  assert.equal(plain.state.wikiPaneSerial, 1, "each actual page open owns a fresh identity");
});

test("a wiki open loses permission if a newer action or close lands after its final Save decision", async () => {
  for (const interrupt of ["supersede", "close"] as const) {
    const f = productionOpenWikiPage();
    let current = true, reads = 0;
    f.state.studioExit = createStudioExit({ onStudio: () => true, hasWork: () => {
      if (++reads === 2) queueMicrotask(() => { if (interrupt === "close") f.state.windowCloseInFlight = true; else current = false; });
      return false;
    }, closing: () => f.state.windowCloseInFlight, claim: () => () => current, save: async () => true });
    const opened = f.open("Page"); await f.respond();
    assert.equal(await opened, false, interrupt);
    assert.equal(f.state.view, "chat"); assert.equal(f.state.wikiBody, "studio was here");
    assert.equal(f.state.activeWikiPage, ""); assert.equal(f.state.wikiPaneSerial, 0);
  }
});

/** A `with` scope for a large App function whose other dependencies do not matter to the test: the
 * named state wins, then real globals, and any other name is an inert stub that can be called,
 * read through or assigned to. A stub is not thenable and converts to 0 (`x += 1` stays numeric). */
function inertScope(state: Record<string, unknown>): Record<string, unknown> {
  const stub: unknown = new Proxy(function () {}, {
    get: (_t, p) => p === Symbol.toPrimitive ? () => 0 : p === "then" ? undefined : stub,
    apply: () => stub, set: () => true,
  });
  return new Proxy(state, {
    has: () => true,
    get: (t, p) => typeof p === "string" && p in t ? t[p] : p === Symbol.unscopables ? undefined
      : p in globalThis ? (globalThis as Record<PropertyKey, unknown>)[p] : stub,
    set: (t, p, v) => { if (typeof p === "string") t[p] = v; return true; },
  });
}

// Review of d2548a37, P3: lock assigned the "latest Studio edits could not be saved" warning, then
// its own later `error = ""` erased it, so the member was never told. This runs the whole production
// lockScreen with a capture that fails, as a full draft queue makes it.
test("lock keeps its warning that the latest Studio edits could not be captured", async () => {
  const { captureLockContinuity } = await import("./window-close.ts");
  for (const failing of [true, false]) {
    let submitted: unknown = "unset", resets = 0;
    const state = {
      locked: false, error: "an earlier message", uiStateReady: true, inboxIdle: undefined,
      uiStateSaveTimer: undefined, inboxTimer: undefined, newsAggregateTimer: undefined, inCall: false,
      captureLockContinuity,
      continuityJson: () => { if (failing) throw new Error("Too many unsaved edits"); return "{}"; },
      nativeVaultLock: { begin: (json: unknown) => { submitted = json; return Promise.resolve({}); } },
      studioReset: () => { resets++; },
    };
    const scope = inertScope(state);
    production<{ lockScreen: () => void }>(["lockScreen"], scope).lockScreen();
    assert.equal(state.locked, true); assert.equal(resets, 1, "the Studio was cleared");
    if (failing) {
      assert.equal(submitted, null, "the lock went ahead without the snapshot");
      assert.match(state.error, /^Locked, but the latest Studio edits could not be saved: .*Too many unsaved edits/);
    } else {
      assert.equal(submitted, "{}"); assert.equal(state.error, "", "an ordinary lock clears the old message");
    }
  }
});

// Review of d2548a37, P2 1: leaving a server, or that server closing natively, moved off it through
// switchServer's Save gate after its actor had stopped. The Save could not land, so the app stayed
// selected on the removed server and the Studio stayed scoped to it. This runs the production
// leaveServer and serverGone over the real Studio session, studio-state and exit gate; the native
// leave stops the actor and emits `server-closed` before it returns, as lib.rs does.
const OTHER = SERVER + 1, DOC = id32(1);
async function productionServerRemoval(opts: { applyFails?: boolean; whileLeaving?: () => void; leaveFailsAfterStop?: boolean; reply?: Promise<void> } = {}) {
  const st = await import("./studio-state.svelte.ts");
  const ipc = fakeIpc(), calls: string[] = [], toasts: string[] = [];
  let stopped = false;
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent()), group: "deadbeef" }));
  ipc.on("studio_read", () => ({ ...ordinaryView(flipnoteContent({ frames: [] })), group: "deadbeef" }));
  ipc.on("studio_apply", () => {
    calls.push("studio_apply");
    if (stopped) throw "server is not running"; // the stopped actor: nothing can land
    if (opts.applyFails) throw "Studio transaction expired, cancelled or server stopped";
    return ordinaryView(flipnoteContent({ fps: 7, frames: [] }));
  });
  st.useStudioIpc(ipc, 5000);
  const session = st.ensureStudio(ME);
  st.setStudioScope(SERVER, CHANNEL); session.flush(); session.open(DOC); await settle(8);
  const state = {
    servers: [{ id: SERVER, name: "Alpha" }, { id: OTHER, name: "Beta" }], activeServerId: SERVER as number | null, view: "studio",
    windowCloseInFlight: false, locked: false, uiStateReady: true, uiStateLoadGeneration: 1, inCall: false, callServer: null, error: "", fileTrustPolicies: {},
    restoredStudioDraft: null as string | null, leavingServers: new Map<number, { session: number; closed: boolean }>(),
    leaveVoice: () => {}, saveUiStateImmediately: async () => {}, beginViewSwitch: () => {},
    toast: (text: string) => { toasts.push(text); },
    clearServerView: () => { state.view = "chat"; },
    studioWork: st.studioWork, retireStudioServer: st.retireStudioServer, studioHasWork: st.studioHasPending,
    studioExit: createStudioExit({ onStudio: () => state.view === "studio", hasWork: st.studioHasPending, closing: () => false,
      claim: st.claimStudioNavigation, save: async () => { try { await st.saveStudio(); return true; } catch (e) { toasts.push(String(e)); return false; } } }),
    // switchServer's production gate (pinned by the switchServer test above), then its commit.
    switchServer: async (id: number) => {
      if (state.windowCloseInFlight) return false;
      const route = state.studioExit.begin();
      if ((!route.current() || st.studioHasPending()) && !(await route.scope())) return false;
      state.activeServerId = id; state.view = "chat"; return true;
    },
    invoke: async (cmd: string) => {
      calls.push(cmd); assert.equal(cmd, "leave_server");
      opts.whileLeaving?.();
      stopped = true; app.serverClosed(SERVER); // the stopping actor's event, before leave_server returns
      if (opts.reply) await opts.reply;
      if (opts.leaveFailsAfterStop) throw "could not remove the server's registry entry";
    },
  };
  const app = production<{ serverGone: (id: number, left: boolean) => void; serverClosed: (id: number) => void; leaveServer: (id: number) => Promise<void> }>(
    ["restoredDraftServer", "serverGone", "serverClosed", "leaveServer"], state);
  return { st, session, state, calls, toasts, app, stop: () => { stopped = true; } };
}

test("a removed server is moved off without a Save to its stopped actor, and the Studio scope follows", async () => {
  assert.match(source, /listen<\{ server: number \}>\("server-closed", \(e\) => serverClosed\(e\.payload\.server\)\)/,
    "the event takes the same path; a leave in flight decides it");
  const { disposeStudio } = await import("./studio-state.svelte.ts");
  try {
    // A voluntary leave saves the unsent edit while the actor can still take it, then leaves.
    const leave = await productionServerRemoval();
    leave.session.setFps(DOC, 7);
    await leave.app.leaveServer(SERVER); await settle(8);
    assert.deepEqual(leave.calls, ["studio_apply", "leave_server"], "saved first, and nothing sent after the stop");
    assert.equal(leave.state.activeServerId, OTHER); assert.deepEqual(leave.state.servers.map(s => s.id), [OTHER]);
    assert.deepEqual([leave.session.scope, leave.session.saves.length, leave.st.studioDraftSnapshot()], [null, 0, null]);
    assert.deepEqual(leave.toasts, []);
    leave.st.setStudioScope(OTHER, CHANNEL); // the Studio's scope effect on the next server: no refusal
    assert.equal(leave.session.scope?.server, OTHER);

    // An edit made while the native leave runs cannot be saved anywhere: it is dropped and said so.
    const during = await productionServerRemoval({ whileLeaving: () => during.session.setFps(DOC, 9) });
    await during.app.leaveServer(SERVER); await settle(8);
    assert.deepEqual(during.calls, ["leave_server"]);
    assert.equal(during.state.activeServerId, OTHER);
    assert.deepEqual([during.session.scope, during.session.saves.length, during.st.studioDraftSnapshot()], [null, 0, null]);
    assert.equal(during.toasts.length, 1); assert.match(during.toasts[0], /^You left Alpha, .*discarded/);

    // A server closed under the session: no Save is attempted, and the edit is kept for its channel.
    const closed = await productionServerRemoval();
    closed.session.setFps(DOC, 7); closed.stop();
    closed.app.serverClosed(SERVER); await settle(8);
    assert.deepEqual(closed.calls, [], "nothing was sent to the stopped actor");
    assert.equal(closed.state.activeServerId, OTHER); assert.equal(closed.state.view, "chat");
    assert.deepEqual([closed.session.scope, closed.session.saves.length], [null, 0]);
    const kept = decodeStudioDraft(closed.st.studioDraftSnapshot()!);
    assert.deepEqual([kept.scope.server, kept.scope.channel, kept.records.length], [SERVER, CHANNEL, 1]);
    assert.match(closed.toasts.join("; "), /^Alpha stopped\. Its unsaved Studio edits are kept and come back when you open their channel after Mewtual restarts/);
    closed.st.setStudioScope(OTHER, CHANNEL); await settle(8); // the next server's Studio is not refused
    assert.equal(closed.session.scope?.server, OTHER);
    closed.st.setStudioScope(SERVER, CHANNEL); await settle(8); // the server is back (Mewtual restarted)
    const [held] = closed.session.saves;
    assert.equal(closed.session.saves.length, 1); assert.equal(closed.session.awaitsExplicitSave(held.id), true, "back, held for an explicit Save");

    // A Save that cannot land before the leave refuses the leave: nothing is left, nothing moves.
    const refused = await productionServerRemoval({ applyFails: true });
    refused.session.setFps(DOC, 7);
    await refused.app.leaveServer(SERVER); await settle(8);
    assert.deepEqual(refused.calls, ["studio_apply"]); assert.equal(refused.state.activeServerId, SERVER);
    assert.equal(refused.state.servers.length, 2); assert.equal(refused.session.saves.length, 1);

    // A restored draft for that server waits for its channel; the leave refuses until it is settled.
    const restored = await productionServerRemoval();
    restored.session.setFps(DOC, 7);
    const json = restored.st.studioDraftSnapshot()!;
    restored.session.reset(); restored.st.restoreStudioDraft(json); restored.st.setStudioScope(OTHER, CHANNEL); await settle(8);
    restored.state.activeServerId = OTHER;
    await restored.app.leaveServer(SERVER); await settle(8);
    assert.deepEqual(restored.calls, [], "not left"); assert.equal(restored.state.servers.length, 2);
    assert.match(restored.toasts.join("; "), /restored Studio edits/);
    assert.equal(restored.st.studioDraftSnapshot(), json, "the draft is untouched");
  } finally { disposeStudio(); }
});

test("an old leave reply cannot retire replacement Studio state or another session's leave operation", async () => {
  const { disposeStudio } = await import("./studio-state.svelte.ts");
  try {
    for (const failed of [false, true]) {
      for (const replacement of [false, true]) {
        const reply = deferred<void>();
        const h = await productionServerRemoval({ reply: reply.promise, leaveFailsAfterStop: failed });
        const leaving = h.app.leaveServer(SERVER);
        await settle(4);
        assert.deepEqual(h.calls, ["leave_server"]);
        h.state.locked = true; h.state.uiStateLoadGeneration++;
        h.session.reset();
        h.state.locked = false; h.state.uiStateLoadGeneration++;
        h.st.setStudioScope(SERVER, CHANNEL); h.session.open(DOC); await settle(8);
        h.session.setFps(DOC, 11);
        h.state.error = "replacement session";
        h.state.fileTrustPolicies = { [SERVER]: "replacement policy" };
        const draft = h.st.studioDraftSnapshot();
        const next = { session: h.state.uiStateLoadGeneration, closed: false };
        if (replacement) h.state.leavingServers.set(SERVER, next);
        reply.resolve(); await leaving; await settle(4);
        assert.equal(h.state.activeServerId, SERVER, "old completion cannot force navigation");
        assert.deepEqual(h.state.servers.map(s => s.id), [SERVER, OTHER]);
        assert.equal(h.state.error, "replacement session");
        assert.deepEqual(h.state.fileTrustPolicies, { [SERVER]: "replacement policy" });
        assert.equal(h.st.studioDraftSnapshot(), draft, "new unsaved work is untouched");
        assert.deepEqual(h.toasts, []);
        if (replacement) assert.equal(h.state.leavingServers.get(SERVER), next, "original completion cannot delete a replacement operation");
        else assert.equal(h.state.leavingServers.has(SERVER), false, "its own bookkeeping can retire");
      }
    }
  } finally { disposeStudio(); }
});

test("an old pending leave cannot hide a server closure in the replacement unlocked session", async () => {
  const { disposeStudio } = await import("./studio-state.svelte.ts");
  const reply = deferred<void>();
  try {
    const h = await productionServerRemoval({ reply: reply.promise });
    const leaving = h.app.leaveServer(SERVER); await settle(4);
    h.state.uiStateLoadGeneration += 2; // lock, then unlock; old reply is still held
    h.session.reset(); h.st.setStudioScope(SERVER, CHANNEL); h.session.open(DOC); await settle(8);
    h.session.setFps(DOC, 13);
    h.app.serverClosed(SERVER); await settle(4);
    assert.equal(h.state.activeServerId, OTHER, "current closure is handled immediately");
    assert.equal(h.session.scope, null);
    const draft = h.st.studioDraftSnapshot();
    assert.equal(decodeStudioDraft(draft!).scope.server, SERVER, "current unsaved work is retained");
    const toasts = [...h.toasts];
    reply.resolve(); await leaving; await settle(4);
    assert.equal(h.st.studioDraftSnapshot(), draft); assert.deepEqual(h.toasts, toasts, "old reply remains inert");
  } finally { reply.resolve(); disposeStudio(); }
});

// Re-review of the fix: the event's timing, a failed leave, a lock and a window close.
test("a removed server stays coherent when the leave fails after the stop, the event is late, or a lock or close races it", async () => {
  const { disposeStudio } = await import("./studio-state.svelte.ts");
  try {
    // leave_server failed after the actor stopped: still a member, so a closure, and the edit drawn
    // during it is kept rather than discarded under a "You left" message.
    const failed = await productionServerRemoval({ leaveFailsAfterStop: true, whileLeaving: () => failed.session.setFps(DOC, 9) });
    await failed.app.leaveServer(SERVER); await settle(8);
    assert.equal(failed.state.error, "could not remove the server's registry entry");
    assert.equal(failed.state.activeServerId, OTHER); assert.deepEqual(failed.session.saves.length, 0);
    assert.equal(decodeStudioDraft(failed.st.studioDraftSnapshot()!).scope.server, SERVER, "kept");
    assert.equal(failed.toasts.length, 1); assert.match(failed.toasts[0], /^Alpha stopped\./);

    // The event arriving after leave_server's response finds the server already gone.
    const late = await productionServerRemoval();
    late.state.invoke = async (cmd: string) => { late.calls.push(cmd); late.stop(); };
    await late.app.leaveServer(SERVER); late.app.serverClosed(SERVER); await settle(8);
    assert.equal(late.state.activeServerId, OTHER); assert.deepEqual(late.toasts, []);

    // A lock during the native leave keeps the lock screen's own message, and the session alone.
    const locking = await productionServerRemoval({ leaveFailsAfterStop: true });
    locking.session.setFps(DOC, 7);
    locking.state.invoke = async (cmd: string) => {
      locking.calls.push(cmd); locking.stop();
      locking.state.locked = true; locking.state.error = "Locked, but the latest Studio edits could not be saved: full";
      locking.app.serverClosed(SERVER); throw "leave failed";
    };
    await locking.app.leaveServer(SERVER); await settle(8);
    assert.equal(locking.state.error, "Locked, but the latest Studio edits could not be saved: full");
    assert.deepEqual(locking.toasts, []);

    // A server closing during a window close: the session's scope is retired and its edit kept for
    // the close's snapshot; the app is on no server, never on the removed one, if the close fails.
    const closing = await productionServerRemoval();
    closing.session.setFps(DOC, 7); closing.stop(); closing.state.windowCloseInFlight = true;
    closing.app.serverClosed(SERVER); await settle(8);
    assert.equal(closing.state.activeServerId, null); assert.equal(closing.session.scope, null);
    assert.equal(decodeStudioDraft(closing.st.studioDraftSnapshot()!).records.length, 1);

    // App's own copy of a draft restored at unlock (the Studio not loaded yet) refuses only a leave
    // of the server it belongs to.
    const unloaded = await productionServerRemoval();
    unloaded.session.setFps(DOC, 7);
    const json = unloaded.st.studioDraftSnapshot()!; unloaded.session.reset();
    unloaded.state.restoredStudioDraft = json;
    await unloaded.app.leaveServer(SERVER); assert.deepEqual(unloaded.calls, [], "its server: refused");
    unloaded.state.restoredStudioDraft = json.replace(`"server":${SERVER}`, `"server":${OTHER}`);
    await unloaded.app.leaveServer(SERVER); assert.deepEqual(unloaded.calls, ["leave_server"], "another server's: left");
  } finally { disposeStudio(); }
});
