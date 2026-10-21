import test from "node:test";
import assert from "node:assert/strict";
import {
  acceptSaveProgress, checkedPrivateDownloads, privateDownloadActions, privateDownloadChatSave,
  privateDownloadDetail, privateDownloadLabel, privateDownloadOffers, type PrivateDownload,
} from "./private-downloads.ts";
const row = { id: "ab".repeat(32), cid: "cd".repeat(32), intent: "18446744073709551615", grant: "2",
  status: "incomplete", verifiedChunks: 1, savedChunks: 0, totalChunks: 2 };
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
    { totalChunks: 129 }, { savedChunks: undefined }, { savedChunks: -1 }, { savedChunks: 3 }, { savedChunks: 0.5 } ]) {
    assert.throws(() => checkedPrivateDownloads([{ ...row, ...change }]), /progress could not be read/);
  }
  assert.throws(() => checkedPrivateDownloads([row, row]));
  assert.throws(() => checkedPrivateDownloads(Array(33).fill(row)));
  assert.throws(() => checkedPrivateDownloads(null));
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
  assert.deepEqual(kinds("ready"), ["resume", "remove"]);
  assert.deepEqual(kinds("save-interrupted"), ["resume", "remove"]);
  assert.deepEqual(kinds("save-paused"), ["resume", "remove"]);
  assert.deepEqual(kinds("destination-unavailable"), ["resume", "fresh", "remove"]);
  assert.deepEqual(kinds("destination-changed"), ["fresh", "remove"]);
  assert.deepEqual(kinds("saved"), ["fresh", "remove"]);
  assert.deepEqual(kinds("removing"), ["remove", "keep"]);
  assert.equal(privateDownloadActions(as("saved")).find((a) => a.kind === "remove")?.label, "Clear");
  assert.equal(privateDownloadActions(as("saved")).find((a) => a.kind === "fresh")?.label, "Save another copy");
  assert.equal(privateDownloadActions(as("exhausted"))[0].label, "Retry download");
  assert.equal(privateDownloadOffers(as("destination-changed"), "resume"), false);
  assert.equal(privateDownloadOffers(as("destination-changed"), "fresh"), true);
  for (const status of statuses) {
    for (const action of privateDownloadActions(as(status))) {
      assert.equal(action.needsIdle, action.kind === "resume" || action.kind === "fresh", `${status} ${action.kind}`);
      assert.doesNotMatch(action.label, /[·—–]/);
    }
  }
});
test("saving from the file list only continues obvious states and otherwise opens Transfers", () => {
  const resumes = new Set(["ready", "incomplete", "paused", "blocked", "save-interrupted", "save-paused"]);
  for (const status of statuses) {
    assert.equal(privateDownloadChatSave(as(status)), resumes.has(status) ? "resume" : "transfers", status);
  }
});
test("detail lines count the right progress and use no separators or dashes", () => {
  assert.equal(privateDownloadDetail(as("incomplete")), "Interrupted, 1 of 2 chunks received");
  assert.equal(privateDownloadDetail(as("save-interrupted", { savedChunks: 1 })), "Saving interrupted, 1 of 2 chunks saved");
  assert.equal(privateDownloadDetail(as("destination-changed")), "Partly saved file changed, 0 of 2 chunks saved");
  assert.equal(privateDownloadDetail(as("saved")), "Saved");
  for (const status of statuses) assert.doesNotMatch(privateDownloadDetail(as(status)), /[·—–]/);
});
