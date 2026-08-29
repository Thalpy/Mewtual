import test from "node:test";
import assert from "node:assert/strict";
import {
  completedDownload,
  downloadSavedNotice,
  guideSavedNotice,
  saveGroupFile,
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
    downloadSavedNotice("cat.png", { path: "/Downloads/cat.png", displayed: true }),
    { text: "Saved cat.png to Downloads", kind: "ok", ms: 4_000 },
  );
  assert.deepEqual(
    downloadSavedNotice("cat.png", {
      path: "/Downloads/cat.png",
      displayed: false,
      warning: "file manager unavailable",
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
});
