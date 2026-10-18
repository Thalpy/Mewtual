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
import { createStudioExit } from "./studio-exit.ts";

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
// Accepted guards: the gate on the same line, or earlier in the enclosing function in a checked
// form (a result that is ignored is not a guard), with NO await between that guard and the site.
// Re-review of 13e338eb: openWikiPage asked the gate, then awaited a wiki read, then changed the
// view; a newer return to the Studio (and work created there) slipped between. A guard proves
// permission only up to the next await, so the last one before the site must follow every await.
// switchView's deferred `route.leave().then(...)` counts only inside switchView, which has no await.
// Known limit: a guard in a sibling branch earlier in the same function also satisfies this.
const LINE_GUARD = /leaveStudioThen\(/;
const FUNCTION_GUARD = /leaveStudioThen\(|if \([^\n]*!\(await (?:route\.(?:leave|scope)|releaseStudio)\(\)\)\) return/g;
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
  "leaveServer:switchServer": "membership already ended; moving off the removed server is forced",
  "refreshChannels:switchTo": "the active channel vanished from the listing; reconciliation is forced",
  "refreshRoles:switchView": "the moderation surface's permission was lost; leaving it is forced",
  "handleRichClick:openStatusRef": "the status-chip branch begins its own action; the earlier awaits are the wikilink branch's",
};

test("no navigation call after an await takes a fresh claim unless it is a stated new action", () => {
  const declarations = [...source.matchAll(/\n {2}(?:async )?function (\w+)\(/g)].map(m => ({ name: m[1], start: m.index!, end: source.indexOf("\n  }\n", m.index!) }));
  const found: string[] = [];
  for (const m of source.matchAll(/\b(switchView|switchServer|switchTo|openWikiPage|openStatusRef)\(/g)) {
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
  let release = false, work = true, cursorWhenRecordingResumed = -2;
  const views: string[] = [];
  const state = {
    navStack: [chat, studioPlace], navAt: 1, applying: false,
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
    navStack: [chat, studioPlace], navAt: 1, navApplying: false, view: "studio", inboxView: false, menu: null, folder: "",
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
    view: "chat", viewGeneration: 1, activeServerId: 7 as number | null, error: "",
    wikiDirty: false, activeWikiPage: "", wikiDrafts: new Map<string, string>(), wikiBody: "studio was here", wikiPages: ["Page"],
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
});
