import test from "node:test";
import assert from "node:assert/strict";
import {
  acceptSaveProgress, checkedPrivateDownloads, checkedSavedCheck, privateDownloadActions, privateDownloadChatLabel,
  privateDownloadChatSave,
  privateDownloadDetail, privateDownloadLabel, privateDownloadOffers, privateDownloadSaveAsIsNewCopy,
  type PrivateDownload, type SavedCheck,
} from "./private-downloads.ts";
const row = { id: "ab".repeat(32), cid: "cd".repeat(32), intent: "18446744073709551615", grant: "2",
  status: "incomplete", verifiedChunks: 1, savedChunks: 0, totalChunks: 2, chosenPlace: false,
  receivedBytes: 8, savedBytes: 0, totalBytes: 16, savedTo: null, savedEntry: null };
const statuses = ["receiving", "incomplete", "ready", "paused", "exhausted", "blocked", "removing",
  "saving", "save-interrupted", "save-paused", "destination-unavailable", "destination-changed", "saved"] as const;
const as = (status: PrivateDownload["status"], extra: Partial<PrivateDownload> = {}): PrivateDownload =>
  ({ ...row, status, ...extra }) as PrivateDownload;

test("saved download status uses exact revisions and a closed renderer projection", () => {
  assert.deepEqual(checkedPrivateDownloads([{ ...row, manifest: "never copy", path: "never copy", folder: "x" }]), [row]);
  assert.equal(privateDownloadLabel(checkedPrivateDownloads([row])[0]), "Interrupted");
  assert.deepEqual(checkedPrivateDownloads([]), []);
  for (const status of statuses) assert.equal(checkedPrivateDownloads([{ ...row, status }])[0].status, status);
});
test("invalid saved owners, revisions, phases and bounds are refused", () => {
  for (const change of [ { id: "AB".repeat(32) }, { cid: null }, { intent: 1 }, { intent: "01" },
    { intent: "18446744073709551616" }, { grant: "0" }, { grant: -1 }, { status: "__proto__" },
    { status: "published" }, { verifiedChunks: -1 }, { verifiedChunks: 3 }, { verifiedChunks: 1.5 },
    { totalChunks: 129 }, { savedChunks: undefined }, { savedChunks: -1 }, { savedChunks: 3 }, { savedChunks: 0.5 },
    { chosenPlace: undefined }, { chosenPlace: "C:\\Users\\me" }, { chosenPlace: 1 } ]) {
    assert.throws(() => checkedPrivateDownloads([{ ...row, ...change }]), /progress could not be read/);
  }
  assert.throws(() => checkedPrivateDownloads([row, row]));
  assert.throws(() => checkedPrivateDownloads(Array(33).fill(row)));
  assert.throws(() => checkedPrivateDownloads(null));
});
test("a saved row may name the saved-files entry it stands for, and only a saved row", () => {
  // PR #56: an entry ID on a saved row; nothing on any other row, and nothing that is not an ID.
  const saved = { ...row, status: "saved", savedTo: "downloads", savedEntry: "ef".repeat(16) };
  assert.equal(checkedPrivateDownloads([saved])[0].savedEntry, "ef".repeat(16));
  assert.equal(checkedPrivateDownloads([{ ...saved, savedEntry: undefined }])[0].savedEntry, null);
  for (const change of [
    { status: "incomplete", savedTo: null }, { savedEntry: "EF".repeat(16) }, { savedEntry: "ef".repeat(17) },
    { savedEntry: 7 }, { savedEntry: "C:\\Users\\me\\report.pdf" },
  ]) {
    assert.throws(() => checkedPrivateDownloads([{ ...saved, ...change }]), /progress could not be read/);
  }
});
test("save progress requires the exact active invocation and cannot revive a terminal row", () => {
  const row = { operation: "save-current", status: "downloading" };
  assert.equal(acceptSaveProgress(row, { cancellation: "save-current" }), true);
  for (const cancellation of [null, "preview", "save-old"]) assert.equal(acceptSaveProgress(row, { cancellation }), false);
  for (const status of ["done", "failed"]) assert.equal(acceptSaveProgress({ ...row, status }, { cancellation: "save-current" }), false);
});
test("every typed state offers truthful actions: no resume for a changed file, no implicit re-save", () => {
  const kinds = (status: PrivateDownload["status"]) => privateDownloadActions(as(status)).map((a) => a.kind);
  assert.deepEqual(kinds("receiving"), ["pause", "remove"]);
  assert.deepEqual(kinds("saving"), ["pause", "remove"]);
  assert.deepEqual(kinds("ready"), ["resume", "saveAs", "remove"]);
  assert.deepEqual(kinds("save-interrupted"), ["resume", "remove"]);
  assert.deepEqual(kinds("save-paused"), ["resume", "remove"]);
  assert.deepEqual(kinds("destination-unavailable"), ["resume", "fresh", "saveAs", "remove"]);
  assert.deepEqual(kinds("destination-changed"), ["fresh", "saveAs", "remove"]);
  assert.deepEqual(kinds("saved"), ["fresh", "saveAs", "remove"]);
  assert.deepEqual(kinds("removing"), ["remove", "keep"]);
  assert.equal(privateDownloadActions(as("saved")).find((a) => a.kind === "remove")?.label, "Clear");
  assert.equal(privateDownloadActions(as("saved")).find((a) => a.kind === "fresh")?.label, "Save another copy");
  assert.equal(privateDownloadActions(as("exhausted"))[0].label, "Retry download");
  assert.equal(privateDownloadOffers(as("destination-changed"), "resume"), false);
  assert.equal(privateDownloadOffers(as("destination-changed"), "fresh"), true);
  for (const status of statuses) {
    for (const action of privateDownloadActions(as(status))) {
      assert.equal(action.needsIdle, ["resume", "fresh", "saveAs"].includes(action.kind), `${status} ${action.kind}`);
      assert.doesNotMatch(action.label, /[·—–]/);
    }
  }
});
test("Save as is offered only where a new file would start, and halted or saved rows mean a new copy", () => {
  const offers = new Set(["ready", "incomplete", "paused", "destination-unavailable", "destination-changed", "saved"]);
  for (const status of statuses) {
    assert.equal(privateDownloadOffers(as(status), "saveAs"), offers.has(status), status);
    const action = privateDownloadActions(as(status)).find((a) => a.kind === "saveAs");
    if (action) assert.equal(action.label, "Save as");
  }
  const newCopy = new Set(["destination-unavailable", "destination-changed", "saved"]);
  for (const status of statuses) assert.equal(privateDownloadSaveAsIsNewCopy(as(status)), newCopy.has(status), status);
});
test("a remembered choice is named in the label and detail without any place", () => {
  const chosen = as("ready", { chosenPlace: true });
  assert.equal(privateDownloadActions(chosen)[0].label, "Save to chosen place");
  assert.equal(privateDownloadActions(as("ready"))[0].label, "Save to Downloads");
  assert.equal(privateDownloadDetail(as("incomplete", { chosenPlace: true })),
    "Interrupted, 1 of 2 chunks received, saves to the chosen place");
  assert.equal(privateDownloadDetail(as("incomplete")), "Interrupted, 1 of 2 chunks received");
  assert.deepEqual(checkedPrivateDownloads([{ ...row, chosenPlace: true }])[0].chosenPlace, true);
});
test("saving from the file list only continues obvious states and otherwise opens Transfers", () => {
  const resumes = new Set(["ready", "incomplete", "paused", "blocked", "save-interrupted", "save-paused"]);
  for (const status of statuses) {
    assert.equal(privateDownloadChatSave(as(status)), resumes.has(status) ? "resume" : "transfers", status);
  }
});
test("the plain save item names a remembered choice it would use instead of a plain Download", () => {
  // Fix-the-lows round (picker review residual): the item used to say "Download" while a
  // remembered Save as choice redirected it.
  const resumes = new Set(["ready", "incomplete", "paused", "blocked", "save-interrupted", "save-paused"]);
  for (const status of statuses) {
    assert.equal(privateDownloadChatLabel(as(status, { chosenPlace: true })),
      resumes.has(status) ? "Save to chosen place" : "Download", status);
    assert.equal(privateDownloadChatLabel(as(status)), "Download", `${status}: no choice remembered`);
  }
  assert.equal(privateDownloadChatLabel(undefined), "Download", "a file with no row");
});
test("Keep on a Removing row calls a complete file complete", () => {
  // Fix-the-lows round (follow-up "Clear on a saved row with an unplugged folder").
  const keep = (extra: Partial<PrivateDownload>) =>
    privateDownloadActions(as("removing", extra)).find((a) => a.kind === "keep")!.label;
  assert.equal(keep({ savedChunks: 2, totalChunks: 2 }), "Keep the file");
  assert.equal(keep({ savedChunks: 1, totalChunks: 2 }), "Keep partly saved file");
  assert.equal(keep({ savedChunks: 0, totalChunks: 2 }), "Keep partly saved file");
});
test("detail lines count the right progress and use no separators or dashes", () => {
  assert.equal(privateDownloadDetail(as("incomplete")), "Interrupted, 1 of 2 chunks received");
  assert.equal(privateDownloadDetail(as("save-interrupted", { savedChunks: 1 })), "Saving interrupted, 1 of 2 chunks saved");
  assert.equal(privateDownloadDetail(as("destination-changed")), "Partly saved file changed, 0 of 2 chunks saved");
  assert.equal(privateDownloadDetail(as("saved")), "Saved");
  for (const status of statuses) assert.doesNotMatch(privateDownloadDetail(as(status)), /[·—–]/);
});
test("a saved file check says what it found, offers Save again only when the file is gone or changed", () => {
  // User decision, 23 October: a Saved row reports a missing or replaced file instead of reading
  // "Saved" until cleared; an unchecked or uncheckable file keeps "Saved" without claiming a check.
  const saved = as("saved");
  const detail = (check: SavedCheck | null) => privateDownloadDetail(saved, check);
  assert.equal(detail(null), "Saved");
  assert.equal(detail("present"), "Saved");
  assert.equal(detail("missing"), "The saved file is no longer there");
  assert.equal(detail("replaced"), "The saved file was changed or replaced");
  assert.equal(detail("unknown"), "Saved, could not check the file");
  const fresh = (check: SavedCheck | null) => privateDownloadActions(saved, check).find((a) => a.kind === "fresh");
  for (const check of ["missing", "replaced"] as const) {
    assert.equal(fresh(check)?.label, "Save again", check);
    assert.equal(fresh(check)?.needsIdle, true, "an explicit save that waits for any save of the file");
    assert.deepEqual(privateDownloadActions(saved, check).map((a) => a.kind), ["fresh", "saveAs", "remove"]);
    assert.equal(privateDownloadActions(saved, check).find((a) => a.kind === "remove")?.label, "Clear", "Clear is unchanged");
  }
  for (const check of [null, "present", "unknown"] as const) assert.equal(fresh(check)?.label, "Save another copy");
  // A check never changes any other row's line or actions.
  for (const status of statuses.filter((s) => s !== "saved")) {
    for (const check of ["missing", "replaced", "unknown"] as const) {
      assert.equal(privateDownloadDetail(as(status), check), privateDownloadDetail(as(status)), status);
      assert.deepEqual(privateDownloadActions(as(status), check), privateDownloadActions(as(status)), status);
    }
  }
  for (const check of ["present", "missing", "replaced", "unknown"] as const) {
    assert.doesNotMatch(detail(check), /[·—–]/);
  }
});
test("a saved file check answer is a closed word, never a path or a status", () => {
  for (const word of ["present", "missing", "replaced", "unknown"]) assert.equal(checkedSavedCheck(word), word);
  for (const value of ["saved", "C:\\Users\\me\\cat.bin", "", null, 1, { result: "present" }, "__proto__"]) {
    assert.throws(() => checkedSavedCheck(value), /check could not be read/);
  }
});
