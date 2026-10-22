import test from "node:test";
import assert from "node:assert/strict";
import {
  completedDownload,
  downloadSavedNotice,
  guideSavedNotice,
  isSaveAsCancelled,
  savedFileName,
  saveGroupFile,
  resumeGroupFile,
  saveSpaceGuide,
  type NativeInvoker,
  type SavedFileResult,
} from "./native-download.ts";

function mockInvoker(response: unknown) {
  const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
  const invoke: NativeInvoker = async <T>(command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    return response as T;
  };
  return { invoke, calls };
}

test("saving a shared file addresses it natively and never carries its bytes", async () => {
  const response: SavedFileResult = {
    path: String.raw`C:\Users\cat\Downloads\notes (2).txt`,
    displayed: true,
  };
  const mock = mockInvoker(response);

  assert.deepEqual(await saveGroupFile(mock.invoke, 3, "ab12", "notes.txt"), response);
  assert.deepEqual(mock.calls, [{
    command: "save_group_file",
    args: { server: 3, cid: "ab12", name: "notes.txt" },
  }]);
  // The point of the command: the file is addressed, not carried. A bytes-shaped argument here
  // would mean the plaintext had come back through the webview to be saved.
  assert.deepEqual(Object.keys(mock.calls[0].args ?? {}).sort(), ["cid", "name", "server"]);
});

test("the image guide uses its constrained native command", async () => {
  const response: SavedFileResult = { path: "/home/cat/Downloads/guide.png", displayed: true };
  const mock = mockInvoker(response);

  assert.deepEqual(await saveSpaceGuide(mock.invoke, "iVBORw0KGgo="), response);
  assert.deepEqual(mock.calls, [{
    command: "save_and_open_space_guide",
    args: { pngBase64: "iVBORw0KGgo=" },
  }]);
});

test("explicit resume sends only saved identity and revision with a fresh native destination name", async () => {
  const saved = { path: "/Downloads/resumed.bin", displayed: true };
  const mock = mockInvoker(saved);
  assert.deepEqual(await resumeGroupFile(mock.invoke, 2, "c", "resumed.bin", "d", "7"), saved);
  assert.deepEqual(mock.calls, [{ command: "save_group_file", args: {
    server: 2, cid: "c", name: "resumed.bin", resume: "d", expectedIntent: "7",
  } }]);
});
test("Save as sends only a flag, never a path, and its cancel is recognized exactly", async () => {
  const saved = { path: "/Chosen/report.bin", displayed: true };
  const mock = mockInvoker(saved);
  assert.deepEqual(await saveGroupFile(mock.invoke, 2, "c", "report.bin", { saveAs: true }), saved);
  assert.deepEqual(mock.calls[0].args, { server: 2, cid: "c", name: "report.bin", saveAs: true });
  await resumeGroupFile(mock.invoke, 2, "c", "r.bin", "d", "7", "op", true, true);
  assert.deepEqual(mock.calls[1].args, {
    server: 2, cid: "c", name: "r.bin", resume: "d", expectedIntent: "7", freshCopy: true, saveAs: true, operation: "op",
  });
  for (const args of mock.calls.map((call) => call.args ?? {})) {
    for (const key of Object.keys(args)) assert.doesNotMatch(key, /path|folder|dir|destination/i);
  }
  await saveGroupFile(mock.invoke, 2, "c", "plain.bin");
  assert.equal("saveAs" in (mock.calls[2].args ?? {}), false, "the one-click default sends no flag");
  assert.equal(isSaveAsCancelled("Save as was cancelled."), true);
  assert.equal(isSaveAsCancelled(new Error("Save as was cancelled.")), true);
  assert.equal(isSaveAsCancelled("Save as was cancelled. Also something else"), false);
  assert.equal(isSaveAsCancelled("A Save as window is already open."), false);
});
test("a save that may not be in Downloads names no folder", () => {
  assert.deepEqual(
    downloadSavedNotice("report.bin", { path: "/Chosen/report.bin", displayed: true }, false),
    { text: "Saved report.bin.", kind: "ok", ms: 4_000 },
  );
  assert.deepEqual(
    downloadSavedNotice("report.bin", { path: "/Chosen/report.bin", displayed: false }, false),
    { text: "Saved to /Chosen/report.bin.", kind: "info", ms: 8_000 },
  );
  const warned = downloadSavedNotice("r.bin", { path: "/Chosen/r.bin", displayed: true, warning: "Note." }, false);
  assert.equal(warned.kind, "info");
  assert.doesNotMatch(warned.text, /Downloads/);
  for (const notice of [warned, downloadSavedNotice("r.bin", { path: "/x/r.bin", displayed: true }, false)]) {
    assert.doesNotMatch(notice.text, /[·—–]/);
  }
  // A media file whose bytes matched its declared type is still outside Downloads when it went
  // to a chosen place; the matched verdict must not fall through to the Downloads wording.
  for (const displayed of [true, false]) {
    for (const warning of [undefined, "Note."]) {
      const matched = downloadSavedNotice("cat.png",
        { path: "/Chosen/cat.png", displayed, warning, contentValidation: "matched" }, false);
      assert.doesNotMatch(matched.text, /Downloads/, `displayed=${displayed} warning=${warning}`);
      assert.match(matched.text, /^Saved /);
      if (warning) assert.ok(matched.text.includes(warning));
    }
  }
  // Mismatch and unrecognized verdicts keep their untrusted-content warning at a chosen place.
  assert.match(downloadSavedNotice("cat.png",
    { path: "/Chosen/cat.png", displayed: true, contentValidation: "mismatch" }, false).text, /untrusted/);
});
test("successful reveal does not hide a committed save's encrypted-cleanup warning", () => {
  const warning = "The encrypted download copy could not be removed; remove it from Transfers.";
  const notice = downloadSavedNotice("cat.bin", { path: "/Downloads/cat.bin", displayed: true, warning });
  assert.match(notice.text, /Saved cat.bin/);
  assert.ok(notice.text.includes(warning));
  assert.equal(notice.kind, "info");
  assert.ok(downloadSavedNotice("cat.png", { path: "/Downloads/cat.png", displayed: true, warning, contentValidation: "mismatch" }).text.includes(warning));
});

test("native completion turns the UI transfer into a fully saved row", () => {
  assert.deepEqual(
    completedDownload(
      { total: 7, bytesTotal: 65_536 },
      { path: "/Downloads/cat.png", displayed: true },
      123_456,
    ),
    {
      status: "done",
      progress: 1,
      done: 7,
      bytesDone: 65_536,
      savedPath: "/Downloads/cat.png",
      error: undefined,
      updatedAt: 123_456,
    },
  );
});

test("saved notices distinguish a visible folder from a reveal warning", () => {
  assert.deepEqual(
    downloadSavedNotice("cat.png", { path: "/Downloads/cat.png", displayed: true, inDownloads: true }),
    { text: "Saved cat.png to Downloads", kind: "ok", ms: 4_000 },
  );
  assert.deepEqual(
    downloadSavedNotice("cat.png", {
      path: "/Downloads/cat.png",
      displayed: false,
      warning: "file manager unavailable",
      inDownloads: true,
    }),
    {
      text: "Saved to /Downloads/cat.png. file manager unavailable",
      kind: "info",
      ms: 8_000,
    },
  );
  assert.equal(
    guideSavedNotice({ path: "/Downloads/guide.png", displayed: true }).note,
    "Saved to /Downloads/guide.png and opened in your image viewer.",
  );
  const mismatch = downloadSavedNotice("cat.png", {
    path: "/Downloads/cat.png",
    displayed: true,
    contentValidation: "mismatch",
  });
  assert.equal(mismatch.kind, "warn");
  assert.match(mismatch.text, /bytes do not match/i);
  assert.match(mismatch.text, /untrusted/i);
});

test("malformed native responses cannot falsely mark a download done", async () => {
  const missingPath = mockInvoker({ displayed: true });
  await assert.rejects(
    saveGroupFile(missingPath.invoke, 1, "ab12", "cat.png"),
    /invalid save result/,
  );

  const badWarning = mockInvoker({ path: "/Downloads/cat.png", displayed: false, warning: 7 });
  await assert.rejects(
    saveGroupFile(badWarning.invoke, 1, "ab12", "cat.png"),
    /invalid save result/,
  );
  const badValidation = mockInvoker({
    path: "/Downloads/cat.png",
    displayed: true,
    contentValidation: "safe",
  });
  await assert.rejects(
    saveGroupFile(badValidation.invoke, 1, "ab12", "cat.png"),
    /invalid save result/,
  );
  const badPlace = mockInvoker({ path: "/Chosen/cat.png", displayed: true, inDownloads: "yes" });
  await assert.rejects(saveGroupFile(badPlace.invoke, 1, "ab12", "cat.png"), /invalid save result/);
});

test("a notice names the file that was written, not the listed name", () => {
  // Re-review 2 LOW 5: a name typed in Save as, or a numbered name when the listed one was taken.
  assert.equal(
    downloadSavedNotice("report.bin", { path: String.raw`D:\Chosen\Quarterly report.bin`, displayed: true }, false).text,
    "Saved Quarterly report.bin.",
  );
  assert.equal(
    downloadSavedNotice("cat.png", { path: "/home/u/Downloads/cat (1).png", displayed: true, inDownloads: true }).text,
    "Saved cat (1).png to Downloads",
  );
  assert.match(
    downloadSavedNotice("cat.png", { path: "/x/cat (2).png", displayed: true, contentValidation: "mismatch" }, false).text,
    /^Saved cat \(2\)\.png, but/,
  );
  assert.equal(savedFileName({ path: "/", displayed: true }, "listed.bin"), "listed.bin");
});

test("a notice names Downloads only when native asserts the file is there", async () => {
  // Picker re-review LOW 1b and its re-check: a one-click save whose record had a choice, or a
  // journal at a place chosen earlier, did not go to Downloads although the caller's (possibly
  // stale) list said so. Native's assertion is required, so a missing flag names no folder.
  const elsewhere = await saveGroupFile(mockInvoker({ path: "/Chosen/cat.png", displayed: true, inDownloads: false }).invoke, 1, "ab12", "cat.png");
  assert.equal("inDownloads" in elsewhere, false);
  for (const result of [elsewhere, { ...elsewhere, contentValidation: "matched" as const }, { ...elsewhere, displayed: false }]) {
    assert.doesNotMatch(downloadSavedNotice("cat.png", result, true).text, /Downloads/);
  }
  const there = await saveGroupFile(mockInvoker({ path: "/Downloads/cat.png", displayed: true, inDownloads: true }).invoke, 1, "ab12", "cat.png");
  assert.equal(there.inDownloads, true);
  assert.equal(downloadSavedNotice("cat.png", there, true).text, "Saved cat.png to Downloads");
  // The caller's own "not a first one-click save" still wins over native's assertion.
  assert.doesNotMatch(downloadSavedNotice("cat.png", there, false).text, /Downloads/);
});
