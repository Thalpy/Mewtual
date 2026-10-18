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
import { deferred } from "./studio-testkit.ts";

const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
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
// Accepted guards: the gate on the same line, or earlier in the enclosing function only in its
// checked forms (a result that is ignored is not a guard). The inline Save in switchView,
// switchServer and switchTo counts only inside those three functions. Known limit: a guard in a
// sibling branch earlier in the same function also satisfies the function-level check.
const LINE_GUARD = /leaveStudioThen\(/;
const FUNCTION_GUARD = /leaveStudioThen\(|if \(!\(await releaseStudio\(\)\)\) return/;
const INLINE_SAVERS = new Set(["switchView", "switchServer", "switchTo"]);
const FORCED = "studio-exit: forced edge";

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
    else if (fn && (FUNCTION_GUARD.test(body) || (INLINE_SAVERS.has(fn.name) && /saveStudioForNavigation\(/.test(body)))) how = "function";
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

test("forced edges are few, explicit and each carries its reason", () => {
  const forced = sites().filter(s => s.how === "forced");
  // Lock (group, clearServerView, unready, locked), leaving the last server (group,
  // clearServerView), that server closing natively (group, clearServerView), and the vault change
  // restarting its own first step after a failed attempt (already off the Studio).
  assert.equal(forced.length, 9, forced.map(s => `App.svelte:${s.line} ${s.text}`).join("\n"));
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
  assert.equal(FUNCTION_GUARD.test(patched.slice(fnStart, at)), false, "the probe's own function has no guard");
  // An ignored result is not a guard: a bare `await releaseStudio();` does not satisfy the audit.
  assert.equal(FUNCTION_GUARD.test("  async function f() {\n    await releaseStudio();\n    view = \"chat\";"), false);
});

test("switchServer and switchTo report a refused switch, and composite callers act only on success", () => {
  assert.match(source, /async function switchServer\(id: number, committed: \(\) => void = \(\) => \{\}\): Promise<boolean>/);
  // `committed` fires after the commit and before the batch read is awaited, in the same turn.
  const body = source.slice(source.indexOf("async function switchServer("), source.indexOf("async function leaveServer("));
  const commitAt = body.indexOf("activeServerId = id;"), hookAt = body.indexOf("committed();"), awaitAt = body.indexOf("await batch;");
  assert.ok(commitAt > 0 && hookAt > commitAt && awaitAt > hookAt && !/\bawait\b/.test(body.slice(commitAt, hookAt)), "no await between commit and the hook");
  assert.match(source, /async function switchTo\(id: string, keepSearch = false\): Promise<boolean>/);
  // No caller may await a switch as a bare statement and then carry on regardless of its outcome.
  // The one bare await left is goMention's final step: nothing runs after it.
  const bare = [...source.matchAll(/^[ \t]*await switch(?:Server|To)\([^;]*\);/gm)];
  assert.deepEqual(bare.map(m => m[0].trim()), ["await switchTo(channel);"], bare.map(m => `App.svelte:${lineOf(m.index!)}`).join(", "));
  const mention = source.slice(bare[0].index!, source.indexOf("\n  }\n", bare[0].index!));
  assert.match(mention, /^\s*await switchTo\(channel\);\s*\} finally \{\s*navStepEnd\(\);\s*\}$/, "goMention does nothing after its switch");
});

/** The production text of `names`, types stripped, evaluated with `state` as its scope. */
function production<T>(names: string[], state: object): T {
  const parts = names.map(name => {
    const start = source.search(new RegExp(String.raw`\n {2}(?:async )?function ${name}\(`)) + 1;
    assert.ok(start > 0, `${name} not found in App.svelte`);
    const text = stripTypeScriptTypes(source.slice(start, source.indexOf("\n  }\n", start) + 4), { mode: "strip" });
    return `const ${name} = (${text.trim()});`;
  });
  return new Function("state", `with (state) { ${parts.join("\n")} return { ${names.join(", ")} }; }`)(state) as T;
}

// Re-review L5: a refused Back/Forward step left the cursor on the entry it never reached, so the
// recording effect appended the place still on screen after it and dropped the forward trail.
test("a refused Back step leaves the history cursor on the place still on screen", async () => {
  const chat = { area: "group", server: 1, view: "chat", channel: "a", page: "", folder: "" };
  const studioPlace = { area: "group", server: 1, view: "studio", channel: "a", page: "", folder: "" };
  let release = false, cursorWhenRecordingResumed = -2;
  const views: string[] = [];
  const state = {
    navStack: [chat, studioPlace], navAt: 1, applying: false,
    get navApplying() { return state.applying; },
    set navApplying(on: boolean) { if (!on) cursorWhenRecordingResumed = state.navAt; state.applying = on; },
    sameLoc: (a: typeof chat, b: typeof chat) => JSON.stringify(a) === JSON.stringify(b),
    releaseStudio: async () => release, tick: async () => {}, arriveAtServer: async () => true, switchTo: async () => true,
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

// Review of 149e7321, finding 3: "Server settings" on server B, while the Studio saved first,
// opened and seeded the dialog from server A, whose Rename then renamed A. This runs the production
// openServerSettings text against a scripted switchServer with the contract App's has: it commits
// synchronously when the Studio holds nothing, else after Save, and resolves false on refusal.
function productionOpenServerSettings() {
  const start = source.indexOf("  function openServerSettings(");
  const text = source.slice(start, source.indexOf("\n  }\n", start) + 4);
  const js = stripTypeScriptTypes(text, { mode: "strip" });
  const servers = [{ id: 1, name: "One" }, { id: 2, name: "Two" }, { id: 3, name: "Three" }];
  let save: ReturnType<typeof deferred<boolean>> | null = null;
  const loaded = deferred<void>(); // switchServer resolves only after the new server's batch read
  const state = {
    activeServerId: 1 as number | null, communicationMode: "stale" as string | null, communicationModeError: "stale",
    serverNameDraft: "", sharedNameDraft: "", serverSettingsPage: "overview", setSearch: "x", showServerSettings: false,
    liveryLoaded: false, livery: { name: "" }, liveryDraft: null as unknown, liveryDraftFor: null as unknown, inviteRevealed: null as unknown,
    get cur() { return servers.find(s => s.id === state.activeServerId) ?? null; },
    serverLabel: (s: { name: string }) => s.name, emptyLivery: () => ({}), seedLiveryDraft: () => {}, precheckInviteRoutes: async () => {},
    studioSaves: false,
    // App's contract: commit (clearServerView closes the dialog), start the reads, run `committed`
    // in that same turn, and resolve only once the batch read is done.
    switchServer(id: number, committed: () => void = () => {}): Promise<boolean> {
      const commit = () => { state.activeServerId = id; state.showServerSettings = false; committed(); };
      if (!state.studioSaves) { commit(); return loaded.promise.then(() => true); }
      save = deferred<boolean>();
      return save.promise.then(ok => ok ? (commit(), loaded.promise.then(() => true)) : false);
    },
  };
  // The production text runs with App's state as its scope; nothing of the function is copied here.
  const open = new Function("state", `with (state) { return (${js.trim()}); }`)(state) as (id?: number | null, page?: string) => void;
  const flush = async () => { for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve)); };
  return { state, open, flush, saved: async (ok: boolean) => { save!.resolve(ok); await flush(); }, load: async () => { loaded.resolve(); await flush(); } };
}

test("server settings for another server open only once the switch to it committed, never on the server left", async () => {
  // Nothing for the Studio to save: the switch commits in the same turn and the dialog opens on B
  // at once, without waiting for B's reads (the existing behaviour).
  const now = productionOpenServerSettings(); now.open(2);
  assert.equal(now.state.activeServerId, 2); assert.equal(now.state.showServerSettings, true); assert.equal(now.state.serverNameDraft, "Two");

  // The Studio saves first and succeeds: nothing opens meanwhile, then B's dialog opens as the
  // switch commits, without waiting for B's reads.
  const later = productionOpenServerSettings(); later.state.studioSaves = true; later.open(2);
  assert.equal(later.state.showServerSettings, false, "no dialog while the switch is still waiting on Save");
  assert.equal(later.state.serverNameDraft, "", "nothing is seeded from the server being left");
  await later.saved(true);
  assert.equal(later.state.activeServerId, 2); assert.equal(later.state.showServerSettings, true); assert.equal(later.state.serverNameDraft, "Two");
  // B's load finishing later must not re-open or re-seed the dialog the member is now editing.
  later.state.serverNameDraft = "typed"; later.state.serverSettingsPage = "invites";
  await later.load();
  assert.equal(later.state.serverNameDraft, "typed"); assert.equal(later.state.serverSettingsPage, "invites");

  // Save refuses: the member stays on A and no dialog opens whose Rename would target A.
  const refused = productionOpenServerSettings(); refused.state.studioSaves = true; refused.open(2);
  await refused.saved(false);
  assert.equal(refused.state.activeServerId, 1); assert.equal(refused.state.showServerSettings, false); assert.equal(refused.state.serverNameDraft, "");

});
