import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

// Compact Transfers rows (design compact.md, section 5.5): the real App.svelte handlers of a row's
// "more" menu, run against a small stand-in for the DOM they touch. Function text is taken from
// the production script, so these tests run what ships.
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["rowMenuDisabled", "rowMenuPlace", "openRowMenu", "closeRowMenu", "moreClick", "moreKeydown",
  "rowMenuKeydown", "runRowMenuItem", "runListMenuItem", "rowMenuPointerDown", "rowMenuMoved", "rowMenuStale",
  "settleRowMenu", "pressRowControl"]);
const handlers = tree.statements.filter((n) => ts.isFunctionDeclaration(n) && n.name && names.has(n.name.text));
assert.equal(handlers.length, names.size);
const constant = (name: string) => {
  const found = tree.statements.find((n) => ts.isVariableStatement(n) &&
    n.declarationList.declarations.some((d) => ts.isIdentifier(d.name) && d.name.text === name));
  assert.ok(found, name);
  return found!;
};
const production = ts.transpileModule(
  [constant("menuKey"), constant("rowMenuItems"), constant("rowMoreButton"), constant("focusInRowMenu"),
    constant("focusLost"), constant("pressedBy"), ...handlers]
    .map((n) => n.getFullText(tree)).join("\n"),
  { compilerOptions: { target: ts.ScriptTarget.ES2022 } },
).outputText;

const ROW = "d".repeat(64);
const OTHER = "e".repeat(64);
/** A saved-files list entry (PR #56); its row id is `saved-<entry id>`. */
const LIST = "f".repeat(32);
const action = (kind: string, label: string, disabled = false) =>
  ({ kind, label, accessibleName: `${label} a.bin`, confirm: false, disabled, disabledReason: disabled ? "Another action on this file is running" : null });
const saveAs = { kind: "action", action: action("saveAs", "Save as"), icon: "save-as", look: "" };
const fresh = { kind: "action", action: action("fresh", "Save another copy"), icon: "save-copy", look: "" };
const reveal = { kind: "reveal", icon: "show-in-folder", look: "", label: "Show in folder", accessibleName: "Show a.bin in folder" };
const controls = (menu: unknown[]) => ({ primary: null, remove: null, menu });

/** The handlers plus a stand-in DOM: one More button per row, the open menu with one element per
 * item, `document.activeElement`, and a window 900 x 640. Every focus is recorded. */
function app(moreBottom = 128) {
  return new Function("ROW", "OTHER", "LIST", "moreBottom", `
    let rowMenu = null, rowMenuFocus = 0, rowMenuHadFocus = false, view = "downloads", pressCount = 0;
    let pressedControl = {}, confirmRemove = {};
    let savedRowViews = [
      { id: ROW, state: "saved", group: "saved", menuItems: "fresh,saveAs" },
      { id: OTHER, state: "saved", group: "saved", menuItems: "saveAs" },
    ];
    let privateDownloads = [{ id: ROW, status: "saved", intent: "3" }, { id: OTHER, status: "saved", intent: "1" }];
    const focused = [], ran = [], revealed = [];
    const tick = async () => {};
    const window = { innerWidth: 900, innerHeight: 640 };
    const confirmingRemove = (row) => confirmRemove[row.id] === row.status + ":" + row.intent;
    // The arranged controls of a row, for the stale check: the menu's kinds as listed above.
    const compactControls = (view) => ({ menu: view.menuItems.split(",").filter(Boolean).map((kind) => ({ kind: "action", action: { kind } })) });
    const runSavedRowAction = async (row, action, group) => { ran.push([row.id, action.kind, group, pressedControl[row.id]]); delete pressedControl[row.id]; };
    const revealSavedRow = async (row) => { revealed.push(row.id); };
    // PR #56: saved-files list rows (row id "saved-<entry id>") share the menu.
    let savedListViews = [{ id: LIST, menuItems: "again" }];
    let savedListFiles = [{ id: LIST }];
    const compactListControls = compactControls;
    const runSavedListAction = async (file, action) => { ran.push(["list", file.id, action.kind]); };
    const revealSavedListRow = async (file) => { revealed.push("list:" + file.id); };
    // The rows of each group, from the views; the focus fallback names where it would go.
    const savedRowGroups = new Proxy({}, { get: (_, group) => savedRowViews.filter((v) => v.group === group) });
    const rowFocusTarget = (rowId, group, nextId) => new Element(nextId ? "next:" + nextId : "heading:" + group);
    class Element {
      constructor(key) { this.key = key; }
      focus() { focused.push(this.key); document.activeElement = this; }
      closest(selector) { return selector === ".sdl-menu" && this.key.startsWith("item") ? menu : null; }
      contains(node) { return node === this; }
    }
    const buttons = new Map([[ROW, new Element("more:" + ROW)], [OTHER, new Element("more:" + OTHER)],
      ["saved-" + LIST, new Element("more:saved-" + LIST)]]);
    let moreTop = moreBottom - 28;
    for (const button of buttons.values()) button.getBoundingClientRect = () => ({ top: moreTop, bottom: moreTop + 28, right: 300 });
    let items = [];
    const menu = new Element("menu");
    menu.contains = (node) => node === menu || items.includes(node);
    const elsewhere = new Element("elsewhere");
    // The Transfers panel holds every More button; the chat pane holds none.
    const panel = new Element("panel");
    panel.contains = (node) => [...buttons.values()].includes(node);
    const body = new Element("body");
    const document = {
      body,
      activeElement: null,
      querySelectorAll(selector) {
        if (selector !== ".sdl-menu [role=menuitem]") throw new Error("unexpected " + selector);
        return rowMenu ? items : [];
      },
      querySelector(selector) {
        if (selector === ".sdl-menu") return rowMenu ? menu : null;
        const more = selector.match(/data-sdl-more="([0-9a-z-]+)"/);
        if (more) return buttons.get(more[1]) ?? null;
        throw new Error("unexpected " + selector);
      },
    };
    ${production}
    const menuFor = (count) => { items = Array.from({ length: count }, (_, i) => new Element("item" + i)); };
    const key = (k) => ({ key: k, defaults: 0, preventDefault() { this.defaults++; }, stops: 0, stopPropagation() { this.stops++; } });
    return {
      focused, ran, revealed, buttons, elsewhere, panel, menuFor, key,
      get rowMenu() { return rowMenu; }, get rowMenuFocus() { return rowMenuFocus; },
      get active() { return document.activeElement; },
      get pressedControl() { return pressedControl; },
      set view(value) { view = value; }, set savedRowViews(value) { savedRowViews = value; },
      set savedListViews(value) { savedListViews = value; }, set savedListFiles(value) { savedListFiles = value; },
      set moreTop(value) { moreTop = value; },
      // What the menu's focusin does, and what a removed menu leaves: focus on <body>.
      menuFocused() { rowMenuHadFocus = true; },
      dropFocus() { document.activeElement = body; },
      confirm(id) { const row = privateDownloads.find((r) => r.id === id); confirmRemove[id] = row.status + ":" + row.intent; },
      rowMenuDisabled, openRowMenu, closeRowMenu, moreClick, moreKeydown, rowMenuKeydown, runRowMenuItem,
      rowMenuPointerDown, rowMenuMoved, rowMenuStale,
      stale() { return rowMenuStale(savedRowViews, view === "downloads"); },
      settle() { settleRowMenu(savedRowViews, view === "downloads"); },
    };
  `)(ROW, OTHER, LIST, moreBottom);
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("More opens below its button and focuses the first item; ArrowUp opens on the last", async () => {
  const a = app();
  const c = controls([fresh, saveAs]);
  a.menuFor(2);
  a.moreClick(ROW, c, { currentTarget: a.buttons.get(ROW) });
  await settle();
  assert.deepEqual(
    { rowId: a.rowMenu.rowId, top: a.rowMenu.top, bottom: a.rowMenu.bottom, right: a.rowMenu.right, up: a.rowMenu.up, key: a.rowMenu.key },
    { rowId: ROW, top: 132, bottom: null, right: 600, up: false, key: "saved:saved:fresh,saveAs" },
  );
  assert.equal(a.focused.at(-1), "item0", "a click focuses the first item too");
  assert.equal(a.rowMenuFocus, 0);
  // Clicking More again closes it and returns focus to the button.
  a.moreClick(ROW, c, { currentTarget: a.buttons.get(ROW) });
  assert.equal(a.rowMenu, null);
  assert.equal(a.focused.at(-1), `more:${ROW}`);
  // ArrowUp on the button opens on the last item; ArrowDown on the first. Other keys do nothing.
  const up = a.key("ArrowUp");
  a.moreKeydown(ROW, c, { ...up, currentTarget: a.buttons.get(ROW) });
  await settle();
  assert.equal(a.focused.at(-1), "item1");
  assert.equal(a.rowMenuFocus, 1);
  a.closeRowMenu(false);
  const enter = a.key("Enter");
  a.moreKeydown(ROW, c, { ...enter, currentTarget: a.buttons.get(ROW) });
  assert.equal(a.rowMenu, null, "Enter is the button's own click");
});

test("near the bottom of the window the menu opens upwards", async () => {
  const a = app(620);
  a.menuFor(2);
  await a.openRowMenu(ROW, controls([fresh, saveAs]), a.buttons.get(ROW));
  assert.equal(a.rowMenu.up, true);
  assert.equal(a.rowMenu.top, null);
  assert.equal(a.rowMenu.bottom, 640 - 592 + 4);
});

test("inside the menu the arrows wrap, Home and End jump, Escape and Tab close", async () => {
  const a = app();
  a.menuFor(3);
  await a.openRowMenu(ROW, controls([fresh, saveAs, reveal]), a.buttons.get(ROW));
  const press = (k: string) => { const event = a.key(k); a.rowMenuKeydown(event); return event; };
  press("ArrowUp");
  assert.equal(a.focused.at(-1), "item2", "up from the first wraps to the last");
  press("ArrowDown");
  assert.equal(a.focused.at(-1), "item0", "down from the last wraps to the first");
  press("End");
  assert.equal(a.focused.at(-1), "item2");
  press("Home");
  assert.equal(a.focused.at(-1), "item0");
  const escape = press("Escape");
  assert.equal(a.rowMenu, null);
  assert.equal(a.focused.at(-1), `more:${ROW}`, "Escape returns focus to More");
  assert.equal(escape.stops, 1, "Escape does not also reach the tab's own handlers");
  await a.openRowMenu(ROW, controls([fresh, saveAs, reveal]), a.buttons.get(ROW));
  const tab = press("Tab");
  assert.equal(a.rowMenu, null, "Tab closes it");
  // Review HIGH 1: the menu is mounted at the end of the page, so Tab from it would leave the
  // page. Focus goes back to More first and the browser's own Tab moves on from there.
  assert.equal(a.focused.at(-1), `more:${ROW}`);
  assert.equal(tab.defaults, 0, "the Tab itself is not prevented");
});

test("an item closes the menu, focuses More, then runs; a disabled item does nothing", async () => {
  const a = app();
  a.menuFor(2);
  await a.openRowMenu(ROW, controls([fresh, saveAs]), a.buttons.get(ROW));
  await a.runRowMenuItem(ROW, { kind: "action", action: action("saveAs", "Save as", true), icon: "save-as", look: "" });
  assert.notEqual(a.rowMenu, null, "a disabled item leaves the menu open");
  assert.deepEqual(a.ran, []);
  await a.runRowMenuItem(ROW, saveAs);
  assert.equal(a.rowMenu, null);
  assert.equal(a.focused.at(-1), `more:${ROW}`);
  assert.deepEqual(a.ran, [[ROW, "saveAs", "saved", "more"]], "the More button carries the busy state while it runs");
  assert.equal(a.pressedControl[ROW], undefined);
  await a.openRowMenu(ROW, controls([fresh, saveAs, reveal]), a.buttons.get(ROW));
  await a.runRowMenuItem(ROW, reveal);
  assert.deepEqual(a.revealed, [ROW]);
});

test("More with nothing to run is unavailable with the busy reason, and does not open", async () => {
  const a = app();
  const busy = controls([{ ...saveAs, action: action("saveAs", "Save as", true) }]);
  assert.equal(a.rowMenuDisabled(busy), "Another action on this file is running");
  assert.equal(a.rowMenuDisabled(controls([{ ...saveAs, action: action("saveAs", "Save as", true) }, fresh])), null);
  assert.equal(a.rowMenuDisabled(controls([reveal])), null, "Show in folder is never disabled by a busy row");
  a.menuFor(1);
  a.moreClick(ROW, busy, { currentTarget: a.buttons.get(ROW) });
  await settle();
  assert.equal(a.rowMenu, null);
});

test("a press outside, a scroll or a resize closes the menu; one menu at a time", async () => {
  const a = app();
  a.menuFor(2);
  await a.openRowMenu(ROW, controls([fresh, saveAs]), a.buttons.get(ROW));
  a.rowMenuPointerDown({ target: a.buttons.get(ROW) });
  assert.notEqual(a.rowMenu, null, "the More button's own press is its click");
  a.rowMenuPointerDown({ target: a.elsewhere });
  assert.equal(a.rowMenu, null);
  assert.notEqual(a.focused.at(-1), `more:${ROW}`, "focus stays where the pointer put it");
  // Opening another row's menu replaces the first: there is only one open menu.
  await a.openRowMenu(ROW, controls([fresh, saveAs]), a.buttons.get(ROW));
  await a.openRowMenu(OTHER, controls([saveAs]), a.buttons.get(OTHER));
  assert.equal(a.rowMenu.rowId, OTHER);
  // A scroll inside the menu, or of something not holding its More button (the chat), is
  // ignored; a scroll of the Transfers panel or a resize closes it, and focus that was in the
  // menu returns to More (review LOW 5).
  a.rowMenuMoved({ type: "scroll", target: a.active });
  assert.notEqual(a.rowMenu, null);
  a.rowMenuMoved({ type: "scroll", target: a.elsewhere });
  assert.notEqual(a.rowMenu, null, "another pane scrolled");
  a.rowMenuMoved({ type: "scroll", target: a.panel });
  assert.equal(a.rowMenu, null);
  assert.equal(a.focused.at(-1), `more:${OTHER}`);
  await a.openRowMenu(ROW, controls([fresh, saveAs]), a.buttons.get(ROW));
  a.rowMenuMoved({ type: "resize", target: null });
  assert.equal(a.rowMenu, null);
});

test("leaving Transfers, a changed menu, a confirmation or a removed row make the menu stale", async () => {
  const a = app();
  a.menuFor(2);
  await a.openRowMenu(ROW, controls([fresh, saveAs]), a.buttons.get(ROW));
  assert.equal(a.stale(), false, "the same row and items");
  a.view = "chat";
  assert.equal(a.stale(), true);
  a.view = "downloads";
  a.savedRowViews = [{ id: ROW, state: "saved", group: "saved", menuItems: "saveAs" }];
  assert.equal(a.stale(), true, "the row's menu changed");
  // Review LOW 6: a changed state closes it even with the same items (Save as may now mean a new
  // copy).
  a.savedRowViews = [{ id: ROW, state: "destination-changed", group: "needs", menuItems: "fresh,saveAs" }];
  assert.equal(a.stale(), true, "the row's state changed");
  a.savedRowViews = [{ id: ROW, state: "saved", group: "saved", menuItems: "fresh,saveAs" }];
  assert.equal(a.stale(), false);
  a.confirm(ROW);
  assert.equal(a.stale(), true, "a confirmation shows only the pair");
  const b = app();
  b.menuFor(2);
  await b.openRowMenu(ROW, controls([fresh, saveAs]), b.buttons.get(ROW));
  b.savedRowViews = [];
  assert.equal(b.stale(), true, "the row left the list");
});

test("a saved-files list row uses the same menu: it opens, runs its item, and goes stale with its entry", async () => {
  // PR #56 merged with the compact rows: a list row in SAVED opens the one shared menu under its
  // row id, an item runs the list's own action (never a private download's), and the menu closes
  // once the entry leaves the list or its items change.
  const a = app();
  const listId = `saved-${LIST}`;
  const again = { kind: "action", action: action("again", "Save another copy"), icon: "save-copy", look: "" };
  a.menuFor(1);
  await a.openRowMenu(listId, controls([again]), a.buttons.get(listId));
  assert.equal(a.rowMenu?.rowId, listId);
  assert.equal(a.rowMenu?.group, "saved");
  assert.equal(a.focused.at(-1), "item0");
  assert.equal(a.stale(), false);
  await a.runRowMenuItem(listId, again);
  assert.equal(a.rowMenu, null);
  assert.equal(a.focused.at(-1), `more:${listId}`);
  assert.deepEqual(a.ran, [["list", LIST, "again"]], "the list's own action, not a private download's");
  await a.openRowMenu(listId, controls([again, reveal]), a.buttons.get(listId));
  await a.runRowMenuItem(listId, reveal);
  assert.deepEqual(a.revealed, [`list:${LIST}`]);
  // Stale: the items changed, or the entry left the list; a cleared entry runs nothing.
  await a.openRowMenu(listId, controls([again]), a.buttons.get(listId));
  a.savedListViews = [{ id: LIST, menuItems: "" }];
  assert.equal(a.stale(), true, "the row's menu changed");
  a.savedListViews = [];
  assert.equal(a.stale(), true, "the entry left the list");
  a.savedListFiles = [];
  await a.runRowMenuItem(listId, again);
  assert.equal(a.ran.length, 1, "nothing runs for an entry that is gone");
  // A row id the menu does not know opens nothing.
  await a.openRowMenu(`saved-${"0".repeat(32)}`, controls([again]), a.buttons.get(listId));
  assert.equal(a.rowMenu, null);
});

test("a stale menu that had focus puts it back on More, or on the row's neighbours", async () => {
  // Review MEDIUM 2: by the time a refresh removes the menu its focused item is gone, so focus
  // would fall to <body>.
  const views = [
    { id: ROW, state: "saved", group: "saved", menuItems: "fresh,saveAs" },
    { id: OTHER, state: "saved", group: "saved", menuItems: "saveAs" },
  ];
  // Its More is still there (the menu changed): focus goes back to it.
  const a = app();
  a.menuFor(2);
  await a.openRowMenu(ROW, controls([fresh, saveAs]), a.buttons.get(ROW));
  a.menuFocused();
  a.savedRowViews = [{ ...views[0], menuItems: "saveAs" }, views[1]];
  a.dropFocus();
  a.settle();
  await settle();
  assert.equal(a.rowMenu, null);
  assert.equal(a.focused.at(-1), `more:${ROW}`);
  // The row went, and its More with it: the next row of the group takes focus.
  const b = app();
  b.menuFor(2);
  await b.openRowMenu(ROW, controls([fresh, saveAs]), b.buttons.get(ROW));
  b.menuFocused();
  b.buttons.delete(ROW);
  b.savedRowViews = [views[1]];
  b.dropFocus();
  b.settle();
  await settle();
  assert.equal(b.focused.at(-1), `next:${OTHER}`);
  // Focus the user had moved elsewhere is left alone; a menu that never had focus moves none.
  const c = app();
  c.menuFor(2);
  await c.openRowMenu(ROW, controls([fresh, saveAs]), c.buttons.get(ROW));
  c.menuFocused();
  c.elsewhere.focus();
  c.savedRowViews = [views[1]];
  c.settle();
  await settle();
  assert.equal(c.focused.at(-1), "elsewhere");
  // A menu that stays open follows its button when the list reflows without a scroll.
  const d = app();
  d.menuFor(2);
  await d.openRowMenu(ROW, controls([fresh, saveAs]), d.buttons.get(ROW));
  d.moreTop = 300;
  d.savedRowViews = views;
  d.settle();
  await settle();
  assert.equal(d.rowMenu.top, 332, "moved with its button");
});
