import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

// Transfers UX (PR #51): the real App.svelte handlers for the merged row, the inline remove
// confirmation and Show in folder, executed against stand-ins for the state they read. Function
// text is taken from the production script, so a change to a handler is what these tests run.
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["liveSaveFor", "confirmingRemove", "askRemove", "cancelRemove", "runSavedRowAction",
  "rowButtonClick", "clearSavedRow", "revealSavedRow", "rowFocusTarget", "pressRowControl"]);
const handlers = tree.statements.filter((n) => ts.isFunctionDeclaration(n) && n.name && names.has(n.name.text));
assert.equal(handlers.length, names.size);
const constant = (name: string) => {
  const found = tree.statements.find((n) => ts.isVariableStatement(n) &&
    n.declarationList.declarations.some((d) => ts.isIdentifier(d.name) && d.name.text === name));
  assert.ok(found, name);
  return found!;
};
const production = ts.transpileModule([constant("confirmToken"), constant("focusLost"), constant("pressedBy"), ...handlers]
  .map((n) => n.getFullText(tree)).join("\n"), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

const row = { id: "d".repeat(64), cid: "c".repeat(64), intent: "3", status: "paused" };
const next = { id: "e".repeat(64), cid: "f".repeat(64), intent: "1", status: "incomplete" };
const remove = { kind: "remove", label: "Remove", confirm: true, disabled: false, disabledReason: null };

function app() {
  return new Function("rowValue", "nextValue", `
    let activeServerId = 1, viewGeneration = 1, locked = false, savedCheckVisit = 4;
    let downloads = {}, activeSaves = {}, confirmRemove = {}, revealingSaved = {}, savedChecks = {}, clearingSaved = {};
    let pressedControl = {}, pressCount = 0;
    let privateDownloads = [rowValue, nextValue];
    // The real validator accepts exactly these answers; anything else is "unknown".
    const checkedSavedCheck = (value) => ["present", "missing", "replaced", "unknown"].includes(value) ? value : "unknown";
    let present = new Set([rowValue.id, nextValue.id]);
    const ran = [], focused = [], calls = [], notices = [], checks = [];
    let replies = new Map();
    const savedRowGroups = { needs: [{ id: rowValue.id }, { id: nextValue.id }], progress: [], saved: [] };
    const dlKey = (server, cid) => server + ":" + cid;
    const tick = async () => {};
    // A tiny DOM: rows by ID (each with a Remove and a first action), the headings of groups that
    // still have rows (an emptied group's section unmounts), and the tab's own heading. Focusing
    // an element makes it active; an element taken away leaves focus on <body>.
    class HTMLElement {
      constructor(key, connected = () => true) { this.key = key; this.connected = connected; }
      get isConnected() { return this.connected(); }
      focus() { focused.push(this.key); document.activeElement = this; }
    }
    const element = (key) => {
      const row = key.match(/data-sdl-row="([0-9a-f]+)"/);
      return new HTMLElement(key, () => !row || present.has(row[1]));
    };
    // The Confirm remove button the user pressed: it unmounts once the action has run.
    let confirmShown = true;
    const confirmButton = new HTMLElement("confirm", () => confirmShown);
    const document = {
      body: new HTMLElement("body"), activeElement: null,
      querySelector(selector) {
        const row = selector.match(/data-sdl-row="([0-9a-f]+)"/);
        if (row) return present.has(row[1]) ? element(selector) : null;
        if (selector === ".sdl-row :is(.sdl-ico-btn, .sdl-act)") return present.size ? element(selector) : null;
        return element(selector);
      },
      getElementById(id) {
        if (id === "sdl-needs") return present.size ? element("#" + id) : null;
        return element("#" + id);
      },
    };
    const persist = new Set();
    let hold = null;
    const runPrivateDownloadAction = async (target, kind) => {
      ran.push([target.id, kind]);
      if (hold) await hold;
      // The control that ran it (Confirm remove) is gone after the action, whatever happened; a
      // control that had focus and is taken away leaves focus on <body>.
      confirmShown = false;
      if (!persist.has(target.id)) present.delete(target.id);
      if (document.activeElement && !document.activeElement.isConnected) document.activeElement = document.body;
    };
    // A reply can be a value, an Error to throw, or a function run when the command is answered
    // (a test models what changes while native works).
    const invoke = async (command, args) => {
      calls.push([command, args]);
      const reply = replies.get(command);
      if (reply instanceof Error) throw reply;
      return (typeof reply === "function" ? reply(args) : reply) ?? null;
    };
    const unlockedViewCurrent = (gen, server) => !locked && gen === viewGeneration && server === activeServerId;
    const toast = (...args) => notices.push(args);
    const checkSavedRows = async () => { checks.push("check"); };
    ${production}
    document.activeElement = confirmButton;
    return {
      ran, focused, calls, notices, checks, replies,
      blur() { document.activeElement = document.body; },
      focusElsewhere() { document.activeElement = new HTMLElement("composer"); },
      set downloads(value) { downloads = value; }, set activeSaves(value) { activeSaves = value; },
      set server(value) { activeServerId = value; },
      set savedChecks(value) { savedChecks = value; }, get savedChecks() { return savedChecks; },
      set privateDownloads(value) { privateDownloads = value; },
      get clearingSaved() { return clearingSaved; },
      get pressedControl() { return pressedControl; },
      holdRuns(value) { hold = value; },
      press(id, slot) { pressRowControl(id, slot); },
      set generation(value) { viewGeneration = value; },
      get confirmRemove() { return confirmRemove; },
      keep(id) { persist.add(id); },
      gone(id) { present.delete(id); },
      liveSaveFor, confirmingRemove, askRemove, cancelRemove, rowButtonClick, clearSavedRow, runSavedRowAction,
      revealSavedRow, rowFocusTarget,
    };
  `)(row, next);
}

test("a save in flight feeds its own row only while it is the active invocation", () => {
  const a = app();
  const key = `1:${row.cid}`;
  // `done` and `bytesDone` include chunks held before the save; the row counts only what this
  // save wrote (`savedDone`, `savedBytes`), and nothing before its first write.
  a.downloads = { [key]: { operation: "save-1", done: 7, total: 9, bytesDone: 700, bytesTotal: 900 } };
  assert.equal(a.liveSaveFor(row), null, "no active save of this file");
  a.activeSaves = { [key]: "save-1" };
  assert.equal(a.liveSaveFor(row), null, "nothing written yet: the row keeps native's state");
  a.downloads = { [key]: { operation: "save-1", done: 7, total: 9, bytesDone: 700, bytesTotal: 900, savedDone: 5, savedBytes: 500 } };
  assert.deepEqual(a.liveSaveFor(row), { done: 5, total: 9, bytesDone: 500, bytesTotal: 900, fresh: false });
  a.downloads = { [key]: { operation: "save-1", done: 7, total: 9, bytesDone: 700, bytesTotal: 900, savedDone: 0, fresh: true } };
  assert.deepEqual(a.liveSaveFor(row), { done: 0, total: 9, bytesDone: 0, bytesTotal: 900, fresh: true });
  a.activeSaves = { [key]: "save-2" };
  assert.equal(a.liveSaveFor(row), null, "a stale record of an older save");
  a.activeSaves = { [key]: "save-1" };
  a.server = 2;
  assert.equal(a.liveSaveFor(row), null, "another server's transfer");
});

test("Remove asks inline first, confirms on the second press, and Cancel puts it back", async () => {
  const a = app();
  a.rowButtonClick(row, remove, "needs");
  await Promise.resolve();
  assert.deepEqual(a.ran, [], "the first press only asks");
  assert.equal(a.confirmingRemove(row), true);
  assert.ok(a.focused.some((f: string) => f.includes(`data-sdl-confirm="${row.id}"`)), "focus moves to Confirm remove");
  await a.cancelRemove(row);
  assert.equal(a.confirmingRemove(row), false);
  assert.ok(a.focused.at(-1).includes(`data-sdl-remove="${row.id}"`), "Cancel returns focus to Remove");
  await a.askRemove(row);
  a.rowButtonClick(row, remove, "needs");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(a.ran, [[row.id, "remove"]]);
  assert.equal(a.confirmingRemove(row), false, "the confirmation is spent");
});

test("a confirmation never carries over to a row that changed meanwhile", async () => {
  const a = app();
  await a.askRemove(row);
  const changed = { ...row, intent: "4" };
  assert.equal(a.confirmingRemove(changed), false);
  a.rowButtonClick(changed, remove, "needs");
  await Promise.resolve();
  assert.deepEqual(a.ran, [], "it asks again instead of removing");
  const otherState = { ...row, status: "incomplete" };
  assert.equal(a.confirmingRemove(otherState), false);
});

test("only the control that started an action shows it running, until it settles", async () => {
  // Compact rows: the pressed icon (the primary or the x) carries the busy ring, not its siblings.
  const a = app();
  const resume = { kind: "resume", label: "Resume download", confirm: false, disabled: false, disabledReason: null };
  a.rowButtonClick(row, resume, "progress");
  assert.equal(a.pressedControl[row.id], "primary", "set as the action starts");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(a.pressedControl[row.id], undefined, "cleared once it settled");
  const b = app();
  await b.askRemove(row);
  b.rowButtonClick(row, remove, "needs");
  assert.equal(b.pressedControl[row.id], "remove");
  // A press that only asks for confirmation marks nothing.
  const c = app();
  c.rowButtonClick(row, remove, "needs");
  assert.equal(c.pressedControl[row.id], undefined);
  // Review LOW 4: an earlier action settling does not clear a later press's marker (Remove while
  // a resumed save still runs keeps the x's ring).
  const d = app();
  d.keep(row.id);
  let finish: () => void = () => {};
  d.holdRuns(new Promise<void>((resolve) => { finish = resolve; }));
  d.rowButtonClick(row, resume, "progress");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(d.pressedControl[row.id], "primary", "the resume is still running");
  d.press(row.id, "remove"); // the user pressed the x meanwhile; that removal is still running
  finish();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(d.pressedControl[row.id], "remove", "the resume settling later leaves the newer marker");
});

test("actions that need no confirmation run at once, and disabled ones do nothing", async () => {
  const a = app();
  a.rowButtonClick(row, { kind: "resume", label: "Resume download", confirm: false, disabled: false, disabledReason: null }, "progress");
  a.rowButtonClick(row, { ...remove, disabled: true, disabledReason: "Another action on this file is running" }, "needs");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(a.ran, [[row.id, "resume"]]);
});

test("focus never falls to <body> when a confirmation or a row goes away", async () => {
  // The row goes: the next row of the group.
  const a = app();
  await a.runSavedRowAction(row, remove, "needs");
  assert.ok(a.focused.at(-1).includes(`data-sdl-row="${next.id}"`), "the next row's first action");
  // The last row of a group that still has rows: its heading.
  const b = app();
  await b.runSavedRowAction(next, remove, "needs");
  assert.equal(b.focused.at(-1), "#sdl-needs");
  // The group empties, so its section and heading unmount: the tab's own heading.
  const d = app();
  d.gone(next.id);
  await d.runSavedRowAction(row, remove, "needs");
  assert.equal(d.focused.at(-1), "#transfers-title");
  // The row stays (a stuck removal, an error): the confirmation that had focus is gone, so focus
  // goes back to the row's Remove.
  const c = app();
  c.keep(row.id);
  await c.runSavedRowAction(row, remove, "needs");
  assert.equal(c.focused.at(-1), `[data-sdl-row="${row.id}"] [data-sdl-remove]`);
  // Focus the user moved elsewhere meanwhile (a long save finishing later) is never taken back.
  const e = app();
  e.focusElsewhere();
  await e.runSavedRowAction(row, remove, "needs");
  assert.deepEqual(e.focused, []);
  // Away from a confirmation, a row that stays gets its first action, not Remove.
  const f = app();
  f.keep(row.id);
  // Compact rows: a row's controls are icon buttons (or the confirmation pair).
  assert.equal(f.rowFocusTarget(row.id, "needs", undefined, false).key, `[data-sdl-row="${row.id}"] :is(.sdl-ico-btn, .sdl-act)`);
  assert.equal(f.rowFocusTarget(row.id, "needs", undefined, true).key, `[data-sdl-row="${row.id}"] [data-sdl-remove]`);
});

test("Show in folder sends only the Download ID and revision, and a refusal re-checks the row", async () => {
  const a = app();
  await a.revealSavedRow({ ...row, status: "saved" });
  assert.deepEqual(a.calls, [["reveal_saved_download", { server: 1, download: row.id, expectedIntent: "3" }]]);
  assert.deepEqual(a.notices, []);
  a.replies.set("reveal_saved_download", new Error("The saved file is no longer there."));
  a.savedChecks = { [row.id]: { intent: "3", result: "present", visit: 4 } };
  await a.revealSavedRow({ ...row, status: "saved" });
  assert.deepEqual(a.notices.at(-1), ["Error: The saved file is no longer there.", "warn", 6000]);
  assert.equal(a.savedChecks[row.id].visit, -1, "the row is checked again");
  assert.deepEqual(a.checks, ["check"]);
});

test("Clear of a Saved row checks the file again and asks unless it is present right now", async () => {
  // Review L9: the row skips the confirmation on an earlier "present", but Clear releases the
  // private copy, the only copy left once the saved file is gone.
  const saved = { ...row, status: "saved" };
  const clear = { kind: "remove", label: "Clear", confirm: false, disabled: false, disabledReason: null };
  const check = ["check_saved_download", { server: 1, download: row.id, expectedIntent: "3" }];
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  // Deleted since the visit's check: the press checks again and asks instead of clearing.
  for (const answer of ["missing", "replaced", "unknown", "nonsense", new Error("Mewtual is locked.")]) {
    const a = app();
    a.privateDownloads = [saved, next];
    a.savedChecks = { [row.id]: { intent: "3", result: "present", visit: 4 } };
    a.replies.set("check_saved_download", answer);
    a.rowButtonClick(saved, clear, "saved");
    await settle();
    assert.deepEqual(a.calls, [check], `${answer}: checked again first`);
    assert.deepEqual(a.ran, [], `${answer}: nothing released`);
    assert.equal(a.confirmingRemove(saved), true, `${answer}: it asks inline`);
    assert.equal(a.savedChecks[row.id].result, typeof answer === "string" && answer !== "nonsense" ? answer : "unknown",
      `${answer}: the row shows what was found`);
  }
  // Still present: it clears at once, after the fresh check.
  const b = app();
  b.privateDownloads = [saved, next];
  b.replies.set("check_saved_download", "present");
  b.rowButtonClick(saved, clear, "saved");
  await settle();
  assert.deepEqual(b.calls, [check]);
  assert.deepEqual(b.ran, [[row.id, "remove"]]);
  // A confirmed Clear (the row already asked) runs without another check.
  const c = app();
  c.privateDownloads = [saved, next];
  await c.askRemove(saved);
  c.rowButtonClick(saved, { ...clear, confirm: true }, "saved");
  await settle();
  assert.deepEqual(c.calls, []);
  assert.deepEqual(c.ran, [[row.id, "remove"]]);
  // The row changed, or the view moved on, while the check ran: nothing runs and nothing asks.
  for (const change of ["row", "view"]) {
    const d = app();
    d.privateDownloads = [saved, next];
    d.replies.set("check_saved_download", () => {
      if (change === "row") d.privateDownloads = [{ ...saved, intent: "4" }, next];
      else d.generation = 2;
      return "present";
    });
    d.rowButtonClick(saved, clear, "saved");
    await settle();
    assert.deepEqual(d.ran, [], `${change}: nothing released on stale terms`);
    assert.equal(d.confirmingRemove(saved), false, `${change}: nothing asked`);
  }
  // A second press while the check runs does not start another check or a second Clear.
  const e = app();
  e.privateDownloads = [saved, next];
  let answer: (value: string) => void = () => {};
  e.replies.set("check_saved_download", () => new Promise<string>((resolve) => { answer = resolve; }));
  e.rowButtonClick(saved, clear, "saved");
  e.rowButtonClick(saved, clear, "saved");
  await settle();
  assert.equal(e.calls.length, 1);
  // Review LOW-6: the button shows the check running, as Show in folder does.
  assert.equal(e.clearingSaved[row.id], true, "Clear is marked busy while its check runs");
  answer("present");
  await settle();
  assert.deepEqual(e.ran, [[row.id, "remove"]]);
  assert.equal(e.clearingSaved[row.id], undefined, "and no longer once it answered");
});

test("the Transfers markup keeps one row per file and the new contracts", () => {
  // The live list leaves out a download whose private row carries its save, and (PR #56) a
  // finished download whose file the saved-files list already shows in Saved.
  assert.match(source, /let liveTransferList = \$derived\(\s*transferList\.filter\(\(t\) => \(t\.direction !== "download" \|\| !privateDownloadCids\.has\(t\.cid\)\) &&\s*liveTransferShown\(t, savedCids\)\)/);
  assert.match(source, /\{#each liveTransferList as d \(d\.key\)\}/);
  assert.doesNotMatch(source, /\{#each transferList as d/);
  // Rows come from the pure view, grouped and labelled by their headings.
  assert.match(source, /<section class="sdl" aria-labelledby=\{`sdl-\$\{group\.id\}`\}>/);
  assert.match(source, /<ul class="sdl-list" aria-labelledby=\{`sdl-\$\{group\.id\}`\}>/);
  assert.match(source, /data-state=\{saved\.state\} data-tone=\{saved\.tone\}/);
  // One polite live region for the tab.
  assert.equal(source.match(/class="sdl-sr" aria-live="polite"/g)?.length, 1);
  // The old dense paragraph is gone; its one-line replacements live in transfer-rows.ts.
  assert.doesNotMatch(source, /Resume continues a partly saved file only if it is still the file Mewtual created/);
  // The replace setting, deferred from PR #51, arrived with PR #56: a device preference shown in
  // Chat & Media and passed to every save as a flag (native decides what may be replaced).
  assert.match(source, /<span>\{REPLACE_OLDER_LABEL\}<\/span>/);
  assert.match(source, /saveGroupFile\(invoke, server, f\.cid, f\.name, \{ operation, saveAs, replaceOlder \}\)/);
  assert.match(source, /resumeGroupFile\([^)]*saveAs, replaceOlder\)/);
  // Compact rows (design compact.md): the old text-button row and the inline link are gone; every
  // control is an icon button with a verb as its tooltip and the file in its accessible name.
  assert.doesNotMatch(source, /class="sdl-actions"|class="sdl-link"/);
  assert.match(source, /\{@const controls = compactControls\(saved\)\}/);
  assert.match(source, /<span class="sdl-status" title=\{saved\.statusTitle\}>/);
  assert.match(source, /\{#if !saved\.showWhy\}<span class="sdl-sr">\{saved\.statusTitle\}<\/span>\{\/if\}/);
  assert.match(source, /\{#if saved\.showWhy\}<p class="sdl-why">\{saved\.why\}<\/p>\{\/if\}/);
  assert.match(source, /<span class="sdl-chip">\{saved\.word\}<\/span>/, "the chip is word-only");
  // The primary: an action (with its label as tooltip, or the disabled reason) or Show in folder.
  assert.match(source, /title=\{primary\.action\.disabledReason \?\? primary\.action\.label\}\s*aria-label=\{primary\.action\.accessibleName\}/);
  assert.match(source, /aria-label=\{primary\.accessibleName\}/);
  // More: a menu button naming the file; the menu is keyboard operable and mounted at the top.
  assert.match(source, /aria-haspopup="menu" aria-expanded=\{rowMenu\?\.rowId === saved\.id \? "true" : "false"\}/);
  assert.match(source, /aria-label=\{`More actions for \$\{saved\.name\}`\}/);
  assert.match(source, /<div class="sdl-menu" class:up=\{rowMenu\.up\} id=\{`sdl-menu-\$\{rowMenu\.rowId\}`\} role="menu"/);
  assert.equal(source.match(/class="sdl-menu-item" role="menuitem"/g)?.length, 2);
  assert.match(source, /tabindex=\{rowMenuFocus === index \? 0 : -1\}/, "roving tabindex");
  // The x: Remove or Clear, busy while Clear re-checks, with the disabled reason described.
  assert.match(source, /<button class="sdl-ico-btn danger" type="button" data-sdl-remove=\{saved\.id\}/);
  assert.match(source, /title=\{clearingSaved\[saved\.id\] \? "Checking the saved file" : remove\.disabledReason \?\? remove\.label\}\s*aria-label=\{remove\.accessibleName\}/);
  assert.match(source, /\{@const removeBusy = clearingSaved\[saved\.id\] \|\| \(saved\.busy && pressedControl\[saved\.id\] === "remove"\)\}/);
  // While confirming, only the chip and the pair show.
  assert.match(source, /\{#if !confirming && controls\.primary\}/);
  assert.match(source, /\{#if confirming && remove\}/);
  // Review MEDIUM 3: the global button hover (accent) must not show through a disabled or busy
  // icon button or menu item; the guard comes before the looks that keep their own backgrounds.
  const css = readFileSync(new URL("./app.css", import.meta.url), "utf8");
  const guard = css.indexOf(".sdl-ico-btn:hover,\n.sdl-menu-item:hover { background: transparent; }");
  assert.ok(guard > 0, "the hover guard");
  assert.ok(guard < css.indexOf(".sdl-ico-btn.go {") && guard < css.indexOf(".sdl-more[aria-expanded=\"true\"] {"));
  // The listeners closing the menu on an outside press, a scroll or a resize.
  assert.match(source, /window\.addEventListener\("pointerdown", rowMenuPointerDown, true\)/);
  assert.match(source, /document\.addEventListener\("scroll", rowMenuMoved, true\)/);
  assert.match(source, /window\.addEventListener\("resize", rowMenuMoved\)/);
});

test("the menu, image viewer and file pane save items take their label from the file's row", () => {
  // Fix-the-lows round (picker review residual): the file menu, the inline embed menu, the image
  // viewer and the file pane say "Save to chosen place" while a remembered Save as choice would
  // be used (`privateDownloadChatLabel`), not a fixed "Download". The attachment chip (the file's
  // name), the .pixa viewer's button and Unlock copy are not labelled (a recorded residual).
  assert.match(source, /function downloadLabel\(f: UiFile\): string \{\s*return privateDownloadChatLabel\(privateDownloads\.find\(\(row\) => row\.cid === f\.cid\)\);/);
  assert.match(source, /\{ label: downloadLabel\(f\), icon: "down", onSelect: \(\) => downloadFile\(f\) \}/);
  assert.match(source, /\{ label: downloadLabel\(embedded\), icon: "down", onSelect: \(\) => downloadFile\(embedded\) \}/);
  assert.match(source, /downloadFile\(lightboxFile\)\}>\{lightboxFile \? downloadLabel\(lightboxFile\) : "Download"\}</);
  assert.match(source, /downloadFile\(fileInfo\)\}>↓ \{fileInfo \? downloadLabel\(fileInfo\) : "Download"\}</);
  assert.doesNotMatch(source, /label: "Download", icon: "down", onSelect: \(\) => downloadFile/);
  assert.doesNotMatch(source, /downloadFile\((lightboxFile|fileInfo)\)\}>(↓ )?Download</);
});
