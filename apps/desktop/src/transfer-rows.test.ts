import test from "node:test";
import assert from "node:assert/strict";
import { checkedPrivateDownloads, type PrivateDownload, type PrivateDownloadStatus, type SavedCheck } from "./private-downloads.ts";
import {
  SAVED_ROW_GROUPS, TRANSFER_ICONS, byteLabel, byteValueText, chunkLabel, compactControls, compactListControls, confirmWords,
  formatDecimalBytes, groupSavedRows, savedRowTransitions, savedRowView, type CompactControl, type SavedListRowAction,
  type SavedRowView,
} from "./transfer-rows.ts";
import { savedListRowView } from "./saved-files.ts";

const base: PrivateDownload = {
  id: "ab".repeat(32), cid: "cd".repeat(32), intent: "3", grant: "1", status: "incomplete",
  verifiedChunks: 12, savedChunks: 4, totalChunks: 40, chosenPlace: false,
  receivedBytes: 3_100_000, savedBytes: 1_000_000, totalBytes: 12_400_000, savedTo: null,
  savedEntry: null,
};
const row = (status: PrivateDownloadStatus, extra: Partial<PrivateDownload> = {}): PrivateDownload =>
  ({ ...base, status, savedTo: status === "saved" ? "downloads" : null, ...extra });
const idle = { busy: false, saving: false };
const view = (status: PrivateDownloadStatus, check: SavedCheck | null = null, extra: Partial<PrivateDownload> = {}) =>
  savedRowView(row(status, extra), check, "report.pdf", null, idle);

test("byte sizes are 1000-based with one decimal below 100 of a unit", () => {
  for (const [bytes, text] of [
    [0, "0 B"], [-5, "0 B"], [Number.NaN, "0 B"], [Number.POSITIVE_INFINITY, "0 B"],
    [1, "1 B"], [999, "999 B"], [999.9, "999 B"],
    [1000, "1.0 kB"], [1049, "1.0 kB"], [1050, "1.1 kB"], [99_949, "99.9 kB"],
    [99_950, "100 kB"], [100_000, "100 kB"], [999_499, "999 kB"],
    [999_500, "1.0 MB"], [1_000_000, "1.0 MB"], [3_100_000, "3.1 MB"], [12_400_000, "12.4 MB"],
    [152_000_000, "152 MB"], [999_999_999, "1.0 GB"], [1_260_000_000, "1.3 GB"],
    [1_000_000_000_000, "1.0 TB"], [5_000_000_000_000_000, "5000 TB"],
  ] as const) {
    assert.equal(formatDecimalBytes(bytes), text, String(bytes));
  }
  assert.equal(byteLabel(3_100_000, 12_400_000), "3.1 MB / 12.4 MB");
  assert.equal(byteValueText(3_100_000, 12_400_000), "3.1 MB of 12.4 MB");
  assert.equal(byteLabel(0, 0), "0 B / 0 B", "an empty file");
  assert.equal(chunkLabel(12, 40), "12 of 40 chunks");
  assert.doesNotMatch(byteLabel(1, 2), /%/, "no percentage");
});

test("every state sorts into its group with a tone, an icon and a word", () => {
  const expected: [PrivateDownloadStatus, SavedCheck | null, string, string, string, string][] = [
    ["incomplete", null, "needs", "warn", "interrupted", "Interrupted"],
    ["exhausted", null, "needs", "warn", "retry", "Retry needed"],
    ["blocked", null, "needs", "danger", "verify-failed", "Verification failed"],
    ["save-interrupted", null, "needs", "warn", "interrupted", "Saving interrupted"],
    ["destination-unavailable", null, "needs", "warn", "folder-unavailable", "Folder or file unavailable"],
    ["destination-changed", null, "needs", "danger", "file-changed", "Partly saved file changed"],
    ["saved", "missing", "needs", "warn", "saved-missing", "Saved file missing"],
    ["saved", "replaced", "needs", "warn", "saved-replaced", "Saved file replaced"],
    ["removing", null, "needs", "warn", "removing", "Removing"],
    ["receiving", null, "progress", "transfer", "receiving", "Receiving"],
    ["saving", null, "progress", "transfer", "saving", "Saving"],
    ["ready", null, "progress", "accent", "ready", "Ready to save"],
    ["paused", null, "progress", "muted", "paused", "Paused"],
    ["save-paused", null, "progress", "muted", "paused", "Saving paused"],
    ["saved", "present", "saved", "ok", "saved", "Saved"],
    ["saved", "unknown", "saved", "muted", "saved", "Saved"],
    ["saved", null, "saved", "ok", "saved", "Saved"],
  ];
  for (const [status, check, group, tone, icon, word] of expected) {
    const v = view(status, check);
    assert.deepEqual([v.group, v.tone, v.icon, v.word], [group, tone, icon, word], `${status} ${check}`);
    assert.ok(Object.hasOwn(TRANSFER_ICONS, v.icon));
  }
  // A check never relabels a row that is not saved.
  assert.equal(view("paused", "missing").check, null);
  assert.equal(view("paused", "missing").word, "Paused");
});

test("groups keep the list's order and come in the order Needs you, In progress, Saved", () => {
  const rows = [
    view("ready"), view("saved", "present"), view("incomplete"), view("receiving"), view("saved", "missing"),
  ].map((v, i) => ({ ...v, id: String(i) }));
  const groups = groupSavedRows(rows);
  assert.deepEqual(groups.needs.map((v) => v.id), ["2", "4"]);
  assert.deepEqual(groups.progress.map((v) => v.id), ["0", "3"]);
  assert.deepEqual(groups.saved.map((v) => v.id), ["1"]);
  assert.deepEqual(SAVED_ROW_GROUPS.map((g) => [g.id, g.heading]),
    [["needs", "Needs you"], ["progress", "In progress"], ["saved", "Saved"]]);
});

test("the bar counts received bytes before saving and saved bytes after", () => {
  const receiving = view("receiving").bar!;
  assert.equal(receiving.kind, "received");
  assert.equal(receiving.label, "3.1 MB / 12.4 MB");
  assert.equal(receiving.fraction, 3_100_000 / 12_400_000);
  assert.deepEqual([receiving.valueNow, receiving.valueMax, receiving.valueText],
    [3_100_000, 12_400_000, "3.1 MB of 12.4 MB"]);
  assert.equal(receiving.accessibleName, "Receiving report.pdf");
  assert.equal(view("receiving").chunks, "12 of 40 chunks");
  const saving = view("save-paused").bar!;
  assert.equal(saving.kind, "saved");
  assert.equal(saving.label, "1.0 MB / 12.4 MB");
  assert.equal(saving.accessibleName, "Saving report.pdf");
  assert.equal(view("save-paused").chunks, "4 of 40 chunks");
  for (const status of ["incomplete", "paused", "exhausted", "blocked"] as const) assert.equal(view(status).bar!.kind, "received");
  for (const status of ["saving", "save-interrupted", "destination-unavailable", "destination-changed"] as const) {
    assert.equal(view(status).bar!.kind, "saved");
  }
  // Ready, saved and removing rows have nothing in flight to show.
  for (const status of ["ready", "saved", "removing"] as const) {
    assert.equal(view(status).bar, null);
    assert.equal(view(status).chunks, null);
  }
});

test("unknown byte counts fall back to chunks, and an empty file reads 0 B", () => {
  const unknown = view("receiving", null, { receivedBytes: null, savedBytes: null, totalBytes: null }).bar!;
  assert.equal(unknown.label, null);
  assert.deepEqual([unknown.valueNow, unknown.valueMax, unknown.valueText], [12, 40, "12 of 40 chunks"]);
  assert.equal(unknown.fraction, 12 / 40);
  const empty = view("saving", null, { receivedBytes: 0, savedBytes: 0, totalBytes: 0, totalChunks: 1, savedChunks: 1 }).bar!;
  assert.equal(empty.label, "0 B / 0 B");
  assert.equal(empty.fraction, 1, "an empty file's one chunk is the whole file");
});

test("a save in flight shows its live progress in its own row, as Saving", () => {
  const live = { done: 30, total: 40, bytesDone: 9_300_000, bytesTotal: 12_400_000, fresh: false };
  // Native's last state may still be ready, receiving or a halt (the list refreshes only on
  // journal steps, and the non-resumable writer has none): while bytes are written it is Saving.
  for (const status of ["ready", "receiving", "incomplete", "paused", "save-interrupted", "destination-unavailable"] as const) {
    const v = savedRowView(row(status), null, "report.pdf", live, { busy: false, saving: true });
    assert.deepEqual([v.state, v.word, v.group, v.icon, v.why], ["saving", "Saving", "progress", "saving", null], status);
    assert.equal(v.bar!.kind, "saved");
    assert.equal(v.bar!.label, "9.3 MB / 12.4 MB");
    assert.equal(v.chunks, "30 of 40 chunks");
  }
  // Removing keeps its own state.
  assert.equal(savedRowView(row("removing"), null, "report.pdf", live, idle).word, "Removing");
  // Never below what native already recorded durably for the same file.
  const behind = savedRowView(row("saving"), null, "report.pdf", { done: 1, total: 40, bytesDone: 10, bytesTotal: 12_400_000, fresh: false }, idle);
  assert.equal(behind.bar!.label, "1.0 MB / 12.4 MB");
  assert.equal(behind.chunks, "4 of 40 chunks");
  // A fresh copy starts at its own zero: the old file's counts are no floor, and a saved row
  // being saved again no longer offers Show in folder for the earlier copy meanwhile.
  const again = savedRowView(row("saved"), "present", "report.pdf", { done: 0, total: 40, bytesDone: 0, bytesTotal: 12_400_000, fresh: true }, idle);
  assert.deepEqual([again.word, again.bar!.label, again.chunks, again.canReveal, again.where],
    ["Saving", "0 B / 12.4 MB", "0 of 40 chunks", false, null]);
  const copy = savedRowView(row("destination-changed"), null, "report.pdf", { done: 2, total: 40, bytesDone: 200, bytesTotal: 12_400_000, fresh: true }, idle);
  assert.equal(copy.chunks, "2 of 40 chunks");
  // Out-of-range live numbers are clamped, never shown past the total.
  const wild = savedRowView(row("saving"), null, "report.pdf",
    { done: 99, total: 40, bytesDone: 99e9, bytesTotal: 12_400_000, fresh: false }, idle);
  assert.equal(wild.bar!.fraction, 1);
  assert.equal(wild.chunks, "40 of 40 chunks");
});

test("each state has its one-line explanation, plain and without separators", () => {
  const silent = new Set(["receiving", "saving", "saved:present", "saved:unknown", "saved:null"]);
  const cases: [PrivateDownloadStatus, SavedCheck | null][] = [
    ["receiving", null], ["saving", null], ["ready", null], ["paused", null], ["save-paused", null],
    ["incomplete", null], ["exhausted", null], ["blocked", null], ["save-interrupted", null],
    ["destination-unavailable", null], ["destination-changed", null], ["removing", null],
    ["saved", "present"], ["saved", "unknown"], ["saved", null], ["saved", "missing"], ["saved", "replaced"],
  ];
  for (const [status, check] of cases) {
    const key = status === "saved" ? `saved:${check}` : status;
    const why = view(status, check).why;
    if (silent.has(key)) assert.equal(why, null, key);
    else {
      assert.ok(why && why.length < 160, key);
      assert.doesNotMatch(why!, /[·—]|never|always|guarantee/i, key);
    }
  }
  // Removing explains whether it is running now or stuck from an earlier attempt.
  // Busy covers Remove, Clear and Keep alike, so the running sentence names none of them.
  assert.match(savedRowView(row("removing"), null, "a", null, { busy: true, saving: false }).why!, /^Working on it/);
  assert.match(view("removing").why!, /could not finish removing a file it created/);
  // Each explanation names a button that the row actually offers.
  for (const [status, check] of cases) {
    const v = view(status, check);
    for (const named of ["Resume download", "Resume saving", "Retry download", "Try again", "Save new copy", "Save again", "Remove", "Keep partly saved file", "Keep the file"]) {
      if (v.why?.includes(named)) assert.ok(v.actions.some((a) => a.label === named), `${status}: ${named}`);
    }
  }
  // Fix-the-lows round: a stuck Clear of a saved row (every part written) keeps a complete file,
  // and its sentence names the row's own Keep action. Compact rows: the actions are icons and a
  // menu now, so the sentences name them by verb ("Remove tries again", "try again").
  const stuckClear = view("removing", null, { savedChunks: 40, totalChunks: 40 });
  assert.match(stuckClear.why!, /Remove tries again, or keep the file\.$/);
  assert.ok(stuckClear.actions.some((a) => a.label === "Keep the file"));
  assert.match(view("removing").why!, /Remove tries again, or keep the partly saved file\.$/);
  assert.match(view("destination-unavailable").why!, /Reconnect it and try again, or save a new copy\.$/);
  assert.doesNotMatch(view("destination-unavailable").why!, /choose/i);
});

test("compact rows show the sentence on NEEDS YOU rows only and title every status symbol", () => {
  const cases: [PrivateDownloadStatus, SavedCheck | null][] = [
    ["receiving", null], ["saving", null], ["ready", null], ["paused", null], ["save-paused", null],
    ["incomplete", null], ["exhausted", null], ["blocked", null], ["save-interrupted", null],
    ["destination-unavailable", null], ["destination-changed", null], ["removing", null],
    ["saved", "present"], ["saved", "unknown"], ["saved", null], ["saved", "missing"], ["saved", "replaced"],
  ];
  for (const [status, check] of cases) {
    const v = view(status, check);
    assert.equal(v.showWhy, v.group === "needs", `${status}/${check}: the sentence shows on NEEDS YOU rows only`);
    assert.ok(v.statusTitle.length > 0 && v.statusTitle.length < 160, `${status}/${check}`);
    assert.doesNotMatch(v.statusTitle, /[·—]/);
    if (v.group === "needs") assert.equal(v.statusTitle, v.why, `${status}: the visible sentence is the tooltip`);
  }
  assert.equal(view("receiving").statusTitle, "Receiving the encrypted copy.");
  assert.equal(view("saving").statusTitle, "Writing the readable file.");
  assert.equal(view("paused").statusTitle, view("paused").why, "a progress row's sentence moves to the tooltip");
  assert.equal(view("saved", "present").statusTitle, "Saved. The file was there when Mewtual last checked.");
  assert.equal(view("saved", "unknown").statusTitle, "Saved. Mewtual could not check the file just now.");
});

test("compact controls put every action of every state in exactly one place", () => {
  type Expect = { primary: string | null; look?: string; icon?: string; x: string | null; menu: string[] };
  const expected: [PrivateDownloadStatus, SavedCheck | null, Expect][] = [
    ["receiving", null, { primary: "Pause", look: "stop", icon: "pause", x: "Remove", menu: [] }],
    ["saving", null, { primary: "Pause", look: "stop", icon: "pause", x: "Remove", menu: [] }],
    ["ready", null, { primary: "Save to Downloads", look: "go", icon: "saving", x: "Remove", menu: ["Save as"] }],
    ["paused", null, { primary: "Resume download", look: "go", icon: "play", x: "Remove", menu: ["Save as"] }],
    ["save-paused", null, { primary: "Resume saving", look: "go", icon: "play", x: "Remove", menu: [] }],
    ["incomplete", null, { primary: "Resume download", look: "go", icon: "play", x: "Remove", menu: ["Save as"] }],
    ["exhausted", null, { primary: "Retry download", look: "go", icon: "retry", x: "Remove", menu: [] }],
    ["blocked", null, { primary: "Resume download", look: "go", icon: "play", x: "Remove", menu: [] }],
    ["save-interrupted", null, { primary: "Resume saving", look: "go", icon: "play", x: "Remove", menu: [] }],
    ["destination-unavailable", null, { primary: "Try again", look: "go", icon: "retry", x: "Remove", menu: ["Save new copy", "Save as"] }],
    ["destination-changed", null, { primary: "Save new copy", look: "go", icon: "save-copy", x: "Remove", menu: ["Save as"] }],
    ["saved", "missing", { primary: "Save again", look: "go", icon: "saving", x: "Clear", menu: ["Save as"] }],
    ["saved", "replaced", { primary: "Save again", look: "go", icon: "saving", x: "Clear", menu: ["Save as"] }],
    ["removing", null, { primary: null, x: "Remove", menu: ["Keep partly saved file"] }],
    ["saved", "present", { primary: "Show in folder", look: "", icon: "show-in-folder", x: "Clear", menu: ["Save another copy", "Save as"] }],
    ["saved", "unknown", { primary: "Show in folder", look: "", icon: "show-in-folder", x: "Clear", menu: ["Save another copy", "Save as"] }],
    ["saved", null, { primary: "Show in folder", look: "", icon: "show-in-folder", x: "Clear", menu: ["Save another copy", "Save as"] }],
  ];
  for (const [status, check, want] of expected) {
    const v = view(status, check);
    const c = compactControls(v);
    const key = `${status}/${check}`;
    const label = (item: CompactControl | { kind: "separator" }) =>
      item.kind === "action" ? item.action.label : item.kind === "reveal" ? item.label : "---";
    assert.equal(c.primary ? label(c.primary) : null, want.primary, `${key}: primary`);
    if (c.primary) {
      assert.equal(c.primary.look, want.look, `${key}: look`);
      assert.equal(c.primary.icon, want.icon, `${key}: icon`);
      assert.ok(c.primary.icon in TRANSFER_ICONS);
    }
    assert.equal(c.remove?.label ?? null, want.x, `${key}: the x`);
    assert.deepEqual(c.menu.map(label), want.menu, `${key}: menu`);
    // No action is lost or shown twice: primary, x and menu together are the view's actions.
    const shown = [c.primary, c.remove ? { kind: "action" as const, action: c.remove } : null, ...c.menu]
      .filter((item): item is CompactControl => !!item && item.kind === "action").map((item) => item.action.kind);
    assert.deepEqual([...shown].sort(), v.actions.map((a) => a.kind).sort(), `${key}: every action exactly once`);
    for (const item of c.menu) if (item.kind !== "separator") assert.ok(item.icon in TRANSFER_ICONS, `${key}: menu icon`);
  }
  // Rows shown as Saving while native still reports another state (a save being written): the
  // actions are native's, so each still appears exactly once, the save starters disabled.
  const live = { done: 2, total: 40, bytesDone: 200, bytesTotal: 4000, fresh: false };
  for (const status of ["ready", "receiving", "destination-unavailable", "saved"] as const) {
    const v = savedRowView(row(status), status === "saved" ? "present" : null, "report.pdf", live, { busy: false, saving: true });
    assert.equal(v.state, "saving", status);
    const c = compactControls(v);
    const kinds = [c.primary, ...c.menu].filter((item): item is CompactControl => !!item && item.kind === "action")
      .map((item) => item.action.kind).concat(c.remove ? [c.remove.kind] : []);
    assert.deepEqual([...kinds].sort(), v.actions.map((a) => a.kind).sort(), `${status} shown as saving`);
    for (const item of [c.primary, ...c.menu]) {
      if (item?.kind === "action" && item.action.needsIdle) assert.equal(item.action.disabledReason, "A save of this file is running", status);
    }
  }
  // The x confirms as before, except a retry on a Removing row (already confirmed) and Clear of a
  // file just found present (it re-checks first).
  assert.equal(compactControls(view("removing")).remove!.confirm, false);
  assert.equal(compactControls(view("saved", "present")).remove!.confirm, false);
  assert.equal(compactControls(view("paused")).remove!.confirm, true);
  assert.equal(compactControls(view("saved", "missing")).remove!.confirm, true);
  // A remembered Save as choice is the primary's tooltip on a ready row.
  assert.equal(compactControls(view("ready", null, { chosenPlace: true })).primary!.kind === "action"
    && (compactControls(view("ready", null, { chosenPlace: true })).primary as { action: { label: string } }).action.label,
  "Save to chosen place");
  // The reveal names the file; a Keep on a complete file says so in the menu.
  const reveal = compactControls(view("saved", "present")).primary!;
  assert.equal(reveal.kind === "reveal" && reveal.accessibleName, "Show report.pdf in folder");
  const keep = compactControls(view("removing", null, { savedChunks: 40, totalChunks: 40 })).menu[0];
  assert.equal(keep.kind === "action" && keep.action.label, "Keep the file");
  assert.equal(keep.kind === "action" && keep.icon, "keep");
});

test("a saved-files list row is arranged by the same rules as a Saved private row", () => {
  // PR #56 merged with the compact rows: one model for both kinds of row in SAVED.
  type Want = { primary: string | null; look?: string; icon?: string; menu: string[] };
  const file = { id: "a".repeat(32), cid: "c".repeat(64), name: "report.pdf", savedAt: 1, size: 1,
    savedTo: "downloads" as const, checkable: true };
  const cases: [string, SavedCheck | null, boolean, boolean, Want][] = [
    // Still there (or not checked yet, or could not check): Show in folder, Save another copy in More.
    ["present", "present", true, true, { primary: "Show in folder", look: "", icon: "show-in-folder", menu: ["Save another copy"] }],
    ["not checked", null, true, true, { primary: "Show in folder", look: "", icon: "show-in-folder", menu: ["Save another copy"] }],
    ["unknown", "unknown", true, true, { primary: "Show in folder", look: "", icon: "show-in-folder", menu: ["Save another copy"] }],
    // Gone: Save again is the primary, as on a Saved private row whose file is missing.
    ["missing", "missing", true, true, { primary: "Save again", look: "go", icon: "saving", menu: [] }],
    ["replaced", "replaced", true, true, { primary: "Save again", look: "go", icon: "saving", menu: [] }],
    // Gone and no longer shared: nothing to save or show, only Clear.
    ["missing, not shared", "missing", false, true, { primary: null, menu: [] }],
    // Present but no longer shared: Show in folder only.
    ["present, not shared", "present", false, true, { primary: "Show in folder", look: "", icon: "show-in-folder", menu: [] }],
    // Native cannot check it: never Show in folder.
    ["not checkable", "missing", true, false, { primary: null, menu: ["Save another copy"] }],
  ];
  for (const [key, check, listed, checkable, want] of cases) {
    const v = savedListRowView({ ...file, checkable }, check, listed, () => "today", idle);
    const c = compactListControls(v);
    const label = (item: CompactControl<SavedListRowAction> | { kind: "separator" }) =>
      item.kind === "action" ? item.action.label : item.kind === "reveal" ? item.label : "---";
    assert.equal(c.primary ? label(c.primary) : null, want.primary, `${key}: primary`);
    if (c.primary) {
      assert.equal(c.primary.look, want.look, `${key}: look`);
      assert.equal(c.primary.icon, want.icon, `${key}: icon`);
      assert.ok(c.primary.icon in TRANSFER_ICONS);
    }
    assert.equal(c.remove?.label, "Clear", `${key}: the x is Clear`);
    assert.deepEqual(c.menu.map(label), want.menu, `${key}: menu`);
    for (const item of c.menu) if (item.kind !== "separator") assert.ok(item.icon in TRANSFER_ICONS, `${key}: menu icon`);
    // Every action exactly once.
    const shown = [c.primary, c.remove ? { kind: "action" as const, action: c.remove } : null, ...c.menu]
      .filter((item): item is CompactControl<SavedListRowAction> => !!item && item.kind === "action").map((item) => item.action.kind);
    assert.deepEqual([...shown].sort(), v.actions.map((a) => a.kind).sort(), `${key}: every action exactly once`);
  }
  // The reveal names the file; the actions carry their file-specific names.
  const present = compactListControls(savedListRowView(file, "present", true, () => "today", idle));
  assert.equal(present.primary?.kind === "reveal" && present.primary.accessibleName, "Show report.pdf in folder");
  assert.equal(present.remove?.accessibleName, "Clear report.pdf");
});

test("saved rows say where the file went without naming a folder native did not prove", () => {
  assert.equal(view("saved", "present").where, "Saved to Downloads");
  assert.equal(view("saved", "present", { savedTo: "other" }).where, "Saved to another folder");
  assert.equal(view("saved", "missing").where, "Was saved to Downloads");
  assert.equal(view("saved", "replaced", { savedTo: "other" }).where, "Was saved to another folder");
  // Neither proven (no Downloads folder, an unreadable journal, an older native): no folder named,
  // Show in folder still offered.
  assert.equal(view("saved", "present", { savedTo: null }).where, null);
  assert.equal(view("saved", "present", { savedTo: null }).canReveal, true);
  assert.equal(view("saved", "unknown").hint, "Could not check the file");
  assert.equal(view("saved", "present").hint, null);
  // Show in folder only where the file may still be there.
  assert.equal(view("saved", "present").canReveal, true);
  assert.equal(view("saved", "unknown").canReveal, true);
  assert.equal(view("saved", null).canReveal, true);
  assert.equal(view("saved", "missing").canReveal, false);
  assert.equal(view("saved", "replaced").canReveal, false);
  for (const status of ["destination-unavailable", "ready", "saving"] as const) {
    assert.equal(view(status).where, null);
    assert.equal(view(status).canReveal, false);
  }
});

test("actions carry file-specific names, disabled reasons and the remove confirmation", () => {
  const v = view("incomplete");
  assert.deepEqual(v.actions.map((a) => a.accessibleName),
    ["Resume download report.pdf", "Save as report.pdf", "Remove report.pdf"]);
  const remove = v.actions.find((a) => a.kind === "remove")!;
  assert.equal(remove.confirm, true, "Remove deletes saved progress");
  assert.deepEqual(confirmWords(remove, "report.pdf"),
    { confirm: "Confirm remove", confirmName: "Confirm remove report.pdf", cancelName: "Cancel removing report.pdf", group: "Confirm removal" });
  // Clear on a saved row whose file was just found where it was saved deletes no copy of the
  // file the user lacks, so it does not ask; otherwise it asks.
  const clear = (check: SavedCheck | null) => view("saved", check).actions.find((a) => a.label === "Clear")!;
  assert.equal(clear("present").confirm, false);
  for (const check of ["missing", "replaced", "unknown", null] as const) assert.equal(clear(check).confirm, true, String(check));
  assert.equal(confirmWords(clear(null), "a.bin").confirm, "Confirm clear");
  // Keep partly saved file never asks.
  assert.equal(view("removing").actions.find((a) => a.kind === "keep")!.confirm, false);
  // A pending action disables everything with the reason; a running save only the save starters.
  const busy = savedRowView(row("paused"), null, "report.pdf", null, { busy: true, saving: false });
  assert.ok(busy.actions.every((a) => a.disabled && a.disabledReason === "Another action on this file is running"));
  const saving = savedRowView(row("paused"), null, "report.pdf", null, { busy: false, saving: true });
  assert.deepEqual(saving.actions.map((a) => [a.label, a.disabledReason]), [
    ["Resume download", "A save of this file is running"], ["Save as", "A save of this file is running"], ["Remove", null],
  ]);
});

test("only chip word changes are announced, never chunk ticks or the first load", () => {
  const one = (status: PrivateDownloadStatus, done = 1): SavedRowView =>
    savedRowView(row(status, { verifiedChunks: done }), null, "report.pdf", null, idle);
  const first = savedRowTransitions(null, [one("receiving")]);
  assert.equal(first.announcement, "");
  const tick = savedRowTransitions(first.words, [one("receiving", 5)]);
  assert.equal(tick.announcement, "", "progress alone is not announced");
  const changed = savedRowTransitions(tick.words, [one("ready")]);
  assert.equal(changed.announcement, "report.pdf: Ready to save.");
  // A row that appears is not a transition (the list itself shows it).
  const added = savedRowTransitions(changed.words, [one("ready"), { ...one("paused"), id: "other" }]);
  assert.equal(added.announcement, "");
});

test("the renderer accepts the byte counts and where a saved file went, closed and bounded", () => {
  const native = (extra: Record<string, unknown>) =>
    checkedPrivateDownloads([{ ...base, ...extra }])[0];
  assert.equal(native({}).totalBytes, 12_400_000);
  // An older native without the fields: no byte counts, not zero.
  const older = { ...base } as Record<string, unknown>;
  delete older.receivedBytes; delete older.savedBytes; delete older.totalBytes; delete older.savedTo;
  assert.deepEqual(
    [checkedPrivateDownloads([older])[0].totalBytes, checkedPrivateDownloads([older])[0].savedTo], [null, null]);
  assert.equal(native({ status: "saved", savedTo: "other" }).savedTo, "other");
  for (const bad of [
    { receivedBytes: 13_000_000 }, { savedBytes: -1 }, { totalBytes: 1.5 }, { totalBytes: null },
    { receivedBytes: "3100000" }, { savedTo: "downloads" }, { status: "saved", savedTo: "C:/Users" },
  ]) {
    assert.throws(() => native(bad), /could not be read/, JSON.stringify(bad));
  }
  // Unexpected properties never reach the renderer.
  assert.equal(Object.hasOwn(native({ path: "C:/secret" }), "path"), false);
});
