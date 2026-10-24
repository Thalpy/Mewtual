import test from "node:test";
import assert from "node:assert/strict";
import { checkedPrivateDownloads, type PrivateDownload, type PrivateDownloadStatus, type SavedCheck } from "./private-downloads.ts";
import {
  SAVED_ROW_GROUPS, TRANSFER_ICONS, byteLabel, byteValueText, chunkLabel, confirmWords, formatDecimalBytes,
  groupSavedRows, savedRowTransitions, savedRowView, type SavedRowView,
} from "./transfer-rows.ts";

const base: PrivateDownload = {
  id: "ab".repeat(32), cid: "cd".repeat(32), intent: "3", grant: "1", status: "incomplete",
  verifiedChunks: 12, savedChunks: 4, totalChunks: 40, chosenPlace: false,
  receivedBytes: 3_100_000, savedBytes: 1_000_000, totalBytes: 12_400_000, savedTo: null,
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
    for (const named of ["Resume download", "Resume saving", "Retry download", "Try again", "Save new copy", "Save again", "Remove", "Keep partly saved file"]) {
      if (v.why?.includes(named)) assert.ok(v.actions.some((a) => a.label === named), `${status}: ${named}`);
    }
  }
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
