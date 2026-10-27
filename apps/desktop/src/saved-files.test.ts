import assert from "node:assert/strict";
import test from "node:test";
import type { PrivateDownload } from "./private-downloads.ts";
import {
  MAX_SAVED_FILES, REPLACE_OLDER_HELP, REPLACE_OLDER_LABEL, SAVED_LIMIT_CHOICES, SAVED_LIMIT_HELP, SAVED_LIMIT_LABEL,
  SAVED_MARK_TITLE, checkedSavedFileChecks, checkedSavedFiles, checkedSavedFilesLimit, checkedSavedLimitChange,
  liveTransferShown, replaceOlderFrom, savedCidSet, savedFileCheckKey, savedLimitDone, savedLimitLabel,
  savedLimitWarning, savedListNotice, savedListRowView, savedListRows, type SavedFile,
} from "./saved-files.ts";

const day = (ms: number) => `day ${ms}`;
const idle = { busy: false, saving: false };

function file(over: Partial<SavedFile> = {}): SavedFile {
  return {
    id: "a".repeat(32), cid: "c".repeat(64), name: "report.pdf", savedAt: 1000, size: 12,
    savedTo: "downloads", checkable: true, ...over,
  };
}

function privateRow(over: Partial<PrivateDownload> = {}): PrivateDownload {
  return {
    id: "d".repeat(64), cid: "c".repeat(64), intent: "3", grant: "1", status: "saved", verifiedChunks: 1,
    savedChunks: 1, totalChunks: 1, chosenPlace: false, receivedBytes: 1, savedBytes: 1, totalBytes: 1,
    savedTo: "downloads", savedEntry: null, ...over,
  };
}

test("the saved-files list is validated closed and keeps only known fields", () => {
  const good = { state: "ok", files: [{ ...file(), extra: "C:\\Users\\me\\secret" }] };
  const list = checkedSavedFiles(good);
  assert.deepEqual(list, { state: "ok", files: [file()] });
  assert.equal(Object.hasOwn(list.files[0], "extra"), false, "an unexpected property never survives");
  // Two copies of one file are two entries (different IDs, same CID).
  assert.equal(checkedSavedFiles({ state: "ok", files: [file(), file({ id: "b".repeat(32) })] }).files.length, 2);
  for (const state of ["newer", "damaged", "unavailable"] as const) {
    assert.deepEqual(checkedSavedFiles({ state, files: [] }), { state, files: [] });
  }
  const bad: unknown[] = [
    null, [], "x", { state: "yes", files: [] }, { state: "ok" }, { state: "damaged", files: [file()] },
    { readable: true, files: [] },
    { state: "ok", files: new Array(MAX_SAVED_FILES + 1).fill(file()) },
    { state: "ok", files: [file(), file()] },
    { state: "ok", files: [file({ id: "A".repeat(32) })] },
    { state: "ok", files: [file({ cid: "c".repeat(63) })] },
    { state: "ok", files: [file({ name: "" })] },
    { state: "ok", files: [file({ name: "C:\\Users\\me\\report.pdf" })] },
    { state: "ok", files: [file({ name: "folder/report.pdf" })] },
    { state: "ok", files: [file({ name: "bad\nname" })] },
    { state: "ok", files: [file({ name: "x".repeat(1025) })] },
    { state: "ok", files: [file({ savedAt: -1 })] },
    { state: "ok", files: [file({ savedAt: 1.5 })] },
    { state: "ok", files: [file({ size: 1024 ** 3 + 1 })] },
    { state: "ok", files: [{ ...file(), savedTo: "C:\\Downloads" }] },
    { state: "ok", files: [{ ...file(), checkable: "yes" }] },
  ];
  for (const value of bad) assert.throws(() => checkedSavedFiles(value), /could not be read/, JSON.stringify(value)?.slice(0, 80));
});

test("an unreadable list says why, and only a damaged one offers a new start", () => {
  assert.equal(savedListNotice("ok"), null);
  assert.match(savedListNotice("newer") ?? "", /newer version of Mewtual/);
  assert.match(savedListNotice("damaged") ?? "", /Starting a new list forgets the old one; the saved files stay where they are\.$/);
  assert.doesNotMatch(savedListNotice("newer") ?? "", /new list/);
  assert.match(savedListNotice("unavailable") ?? "", /right now/);
  for (const state of ["newer", "damaged", "unavailable"] as const) assert.doesNotMatch(savedListNotice(state) ?? "", /[·—]/);
});

test("saved file checks are validated closed", () => {
  const answer = [{ id: "a".repeat(32), name: "report.pdf", check: "missing", extra: 1 }];
  assert.deepEqual(checkedSavedFileChecks(answer), [{ id: "a".repeat(32), name: "report.pdf", check: "missing" }]);
  for (const value of [
    null, {}, [{ id: "a".repeat(32), name: "report.pdf", check: "gone" }],
    [{ id: "a".repeat(31), name: "report.pdf", check: "present" }],
    [{ id: "a".repeat(32), name: "x/y", check: "present" }],
    [{ id: "a".repeat(32), name: "a", check: "present" }, { id: "a".repeat(32), name: "b", check: "present" }],
  ]) {
    assert.throws(() => checkedSavedFileChecks(value), /could not be read/);
  }
  // An answer is tied to the entry's ID and its current name.
  assert.notEqual(savedFileCheckKey(file()), savedFileCheckKey(file({ name: "report (1).pdf" })));
});

test("a list row says where the file went and offers only what applies", () => {
  const present = savedListRowView(file(), "present", true, day, idle);
  assert.equal(present.word, "Saved");
  assert.equal(present.tone, "ok");
  assert.equal(present.icon, "saved");
  assert.equal(present.where, "Saved to Downloads, day 1000");
  assert.equal(present.why, null);
  assert.equal(present.hint, null);
  assert.equal(present.canReveal, true);
  assert.deepEqual(present.actions.map((a) => a.label), ["Clear"]);
  assert.equal(present.actions[0].accessibleName, "Clear report.pdf");
  // Not checked yet reads the same as present.
  assert.equal(savedListRowView(file(), null, true, day, idle).word, "Saved");
  // Another folder, and a folder native could not tell.
  assert.equal(savedListRowView(file({ savedTo: "other" }), null, true, day, idle).where, "Saved to another folder, day 1000");
  assert.equal(savedListRowView(file({ savedTo: null }), null, true, day, idle).where, "Saved, day 1000");
  // Could not check: still Saved, with a quiet hint and Show in folder (native re-checks first).
  const unknown = savedListRowView(file(), "unknown", true, day, idle);
  assert.equal(unknown.tone, "muted");
  assert.equal(unknown.hint, "Could not check the file");
  assert.equal(unknown.canReveal, true);
});

test("a missing or replaced file says so and offers Save again while the file is still listed", () => {
  for (const [check, word, icon] of [["missing", "Saved file missing", "saved-missing"], ["replaced", "Saved file replaced", "saved-replaced"]] as const) {
    const row = savedListRowView(file(), check, true, day, idle);
    assert.equal(row.word, word);
    assert.equal(row.icon, icon);
    assert.equal(row.tone, "warn");
    assert.match(row.why ?? "", /Save again makes a fresh copy\.$/);
    assert.equal(row.where, "Was saved to Downloads, day 1000");
    assert.equal(row.canReveal, false, "nothing to show");
    assert.deepEqual(row.actions.map((a) => a.kind), ["again", "forget"]);
    // A file no longer listed in the server cannot be saved again from here.
    assert.deepEqual(savedListRowView(file(), check, false, day, idle).actions.map((a) => a.kind), ["forget"]);
  }
});

test("a file Mewtual cannot check is never called missing and offers no Show in folder", () => {
  const row = savedListRowView(file({ checkable: false }), "missing", true, day, idle);
  assert.equal(row.word, "Saved");
  assert.equal(row.hint, "Mewtual cannot check files in that folder");
  assert.equal(row.canReveal, false);
  assert.equal(row.why, null);
  assert.deepEqual(row.actions.map((a) => a.kind), ["forget"]);
});

test("a row's actions wait for its own running action and Save again for a running save", () => {
  const busy = savedListRowView(file(), "missing", true, day, { busy: true, saving: false });
  assert.ok(busy.actions.every((a) => a.disabled && a.disabledReason === "Another action on this file is running"));
  const saving = savedListRowView(file(), "missing", true, day, { busy: false, saving: true });
  assert.equal(saving.actions.find((a) => a.kind === "again")?.disabledReason, "A save of this file is running");
  assert.equal(saving.actions.find((a) => a.kind === "forget")?.disabled, false, "Clear never waits for a save");
});

test("list rows never duplicate a copy that has a Saved private row, and show newest first", () => {
  const older = file({ id: "1".repeat(32), cid: "1".repeat(64), savedAt: 10 });
  const newer = file({ id: "2".repeat(32), cid: "2".repeat(64), savedAt: 20 });
  const covered = file({ id: "3".repeat(32), cid: "3".repeat(64), savedAt: 30 });
  // Another copy of the covered file: it keeps its own row.
  const sibling = file({ id: "4".repeat(32), cid: "3".repeat(64), savedAt: 25 });
  const list = { state: "ok" as const, files: [older, newer, covered, sibling] };
  const rows = savedListRows(list, [
    privateRow({ cid: covered.cid, savedEntry: covered.id }),
    // A save in progress (no saved entry) never hides anything.
    privateRow({ id: "e".repeat(64), cid: older.cid, status: "saving", savedTo: null }),
  ]);
  assert.deepEqual(rows.map((r) => r.id), [sibling.id, newer.id, older.id]);
  // A Saved private row native could not match to an entry hides nothing either.
  assert.equal(savedListRows(list, [privateRow({ cid: covered.cid })]).length, 4);
  assert.deepEqual([...savedCidSet(list)].sort(), [older.cid, newer.cid, covered.cid].sort());
});

test("the live list leaves out only finished downloads of files the list already shows", () => {
  const saved = new Set(["c".repeat(64)]);
  const cid = "c".repeat(64);
  assert.equal(liveTransferShown({ direction: "download", cid, status: "done" }, saved), false);
  for (const status of ["active", "waiting", "failed"]) {
    assert.equal(liveTransferShown({ direction: "download", cid, status }, saved), true, status);
  }
  assert.equal(liveTransferShown({ direction: "upload", cid, status: "done" }, saved), true);
  assert.equal(liveTransferShown({ direction: "download", cid: "f".repeat(64), status: "done" }, saved), true);
});

test("the replace preference is on only for its exact word, and its copy follows the house rules", () => {
  assert.equal(replaceOlderFrom("on"), true);
  for (const stored of [null, "", "off", "true", "ON", "1"]) assert.equal(replaceOlderFrom(stored), false, String(stored));
  for (const text of [REPLACE_OLDER_LABEL, REPLACE_OLDER_HELP, SAVED_MARK_TITLE]) {
    assert.doesNotMatch(text, /[·—]/, text);
  }
  assert.equal(REPLACE_OLDER_LABEL, "Replace my older saved copy");
  assert.match(REPLACE_OLDER_HELP, /^When you save a file again, Mewtual replaces the copy it saved before, if that copy is still unchanged\. It replaces only a copy it can prove it saved\./);
  // No absolute claim (design review M3(g)).
  assert.doesNotMatch(REPLACE_OLDER_HELP, /\bnever\b/);
});

test("the Remember saved files setting is validated closed, within native's range", () => {
  const good = { state: "ok", limit: 256, listed: 212, min: 16, max: 4096, default: 256 };
  assert.deepEqual(checkedSavedFilesLimit({ ...good, extra: "C:\\secret" }), good);
  assert.deepEqual(checkedSavedFilesLimit({ ...good, state: "unreadable", limit: null, listed: null }),
    { ...good, state: "unreadable", limit: null, listed: null });
  const bad: unknown[] = [
    null, [], { ...good, state: "maybe" }, { ...good, limit: null }, { ...good, state: "unreadable" },
    { ...good, limit: 8 }, { ...good, limit: 5000 }, { ...good, limit: 300.5 }, { ...good, listed: -1 },
    { ...good, listed: "212" }, { ...good, min: 0 }, { ...good, default: 8000 }, { ...good, max: 5000 },
  ];
  for (const value of bad) assert.throws(() => checkedSavedFilesLimit(value), /could not be read/, JSON.stringify(value));
  assert.deepEqual(checkedSavedLimitChange({ limit: 16, forgets: 196, applied: true, path: "x" }),
    { limit: 16, forgets: 196, applied: true });
  for (const value of [null, { limit: 16, forgets: -1, applied: true }, { limit: 16, forgets: 1, applied: "yes" },
    { limit: 0, forgets: 0, applied: false }, { limit: 16, forgets: 5000, applied: false }]) {
    assert.throws(() => checkedSavedLimitChange(value), /could not be read/, JSON.stringify(value));
  }
});

test("the setting's choices, labels and warnings say what happens to entries and files", () => {
  assert.equal(SAVED_LIMIT_CHOICES[0], 16);
  assert.equal(SAVED_LIMIT_CHOICES.at(-1), 4096);
  assert.ok(SAVED_LIMIT_CHOICES.includes(256));
  assert.equal(SAVED_LIMIT_LABEL, "Remember saved files");
  assert.equal(savedLimitLabel(256, 256), "256 files (default)");
  assert.equal(savedLimitLabel(1024, 256), "1024 files");
  // Lowering forgets entries, never files, and the page says so before the user confirms.
  assert.match(SAVED_LIMIT_HELP, /Lowering it forgets the oldest entries from the list; the saved files stay where they are\./);
  assert.equal(savedLimitWarning(1), "This forgets the oldest saved file from the list. The files themselves stay where they are.");
  assert.equal(savedLimitWarning(84), "This forgets the 84 oldest saved files from the list. The files themselves stay where they are.");
  assert.equal(savedLimitDone({ limit: 512, forgets: 0, applied: true }), "Remembering 512 saved files on this device.");
  assert.equal(savedLimitDone({ limit: 16, forgets: 1, applied: true }), "Remembering 16 saved files on this device. Forgot the oldest entry.");
  for (const text of [SAVED_LIMIT_LABEL, SAVED_LIMIT_HELP, savedLimitWarning(2), savedLimitDone({ limit: 16, forgets: 3, applied: true })]) {
    assert.doesNotMatch(text, /[·—]/, text);
    assert.doesNotMatch(text, /\bnever\b/, text);
  }
});

test("no list row text uses a middle dot or an em dash", () => {
  for (const check of [null, "present", "missing", "replaced", "unknown"] as const) {
    for (const checkable of [true, false]) {
      const row = savedListRowView(file({ checkable }), check, true, day, idle);
      for (const text of [row.word, row.why ?? "", row.where, row.hint ?? "", ...row.actions.map((a) => a.label)]) {
        assert.doesNotMatch(text, /[·—]/, text);
      }
    }
  }
});
