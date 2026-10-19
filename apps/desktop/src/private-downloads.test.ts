import test from "node:test";
import assert from "node:assert/strict";
import { acceptSaveProgress, checkedPrivateDownloads, privateDownloadLabel } from "./private-downloads.ts";
const row = { id: "ab".repeat(32), cid: "cd".repeat(32), intent: "18446744073709551615", grant: "2",
  status: "incomplete", verifiedChunks: 1, totalChunks: 2 };
test("saved download status uses exact revisions and a closed renderer projection", () => {
  assert.deepEqual(checkedPrivateDownloads([{ ...row, manifest: "never copy", path: "never copy" }]), [row]);
  assert.equal(privateDownloadLabel(checkedPrivateDownloads([row])[0]), "Interrupted");
  assert.deepEqual(checkedPrivateDownloads([]), []);
});
test("invalid saved owners, revisions, phases and bounds are refused", () => {
  for (const change of [ { id: "AB".repeat(32) }, { cid: null }, { intent: 1 }, { intent: "01" },
    { intent: "18446744073709551616" }, { grant: "0" }, { grant: -1 }, { status: "__proto__" },
    { verifiedChunks: -1 }, { verifiedChunks: 3 }, { verifiedChunks: 1.5 }, { totalChunks: 129 } ]) {
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
