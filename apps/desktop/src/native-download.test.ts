import test from "node:test";
import assert from "node:assert/strict";
import {
  completedDownload,
  downloadSavedNotice,
  guideSavedNotice,
  saveGroupDownload,
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

test("group downloads use the native save command with the original name and bytes", async () => {
  const response: SavedFileResult = {
    path: String.raw`C:\Users\cat\Downloads\notes (2).txt`,
    displayed: true,
  };
  const mock = mockInvoker(response);

  assert.deepEqual(await saveGroupDownload(mock.invoke, "notes.txt", "Y2F0"), response);
  assert.deepEqual(mock.calls, [{
    command: "save_download",
    args: { name: "notes.txt", dataBase64: "Y2F0" },
  }]);
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
});

test("malformed native responses cannot falsely mark a download done", async () => {
  const missingPath = mockInvoker({ displayed: true });
  await assert.rejects(
    saveGroupDownload(missingPath.invoke, "cat.png", "Y2F0"),
    /invalid save result/,
  );

  const badWarning = mockInvoker({ path: "/Downloads/cat.png", displayed: false, warning: 7 });
  await assert.rejects(
    saveGroupDownload(badWarning.invoke, "cat.png", "Y2F0"),
    /invalid save result/,
  );
});
