import { test } from "node:test";
import assert from "node:assert/strict";

import { isPasteableMedia, pastedMedia, pastedName } from "./clipboard-media.ts";

test("a paste is only ours to handle when it carries media", () => {
  assert.ok(isPasteableMedia("image/png"));
  assert.ok(isPasteableMedia("IMAGE/PNG"), "the declared type is not case-normalised for us");
  assert.ok(isPasteableMedia("video/mp4"));
  assert.ok(isPasteableMedia("audio/mpeg"));
  // The case that must NOT be swallowed: copying a region of a web page puts an HTML flavour on
  // the clipboard beside the image, and copying a document puts a file there with no image at
  // all. Treating either as an upload turns an ordinary text paste into a surprise file share.
  assert.equal(isPasteableMedia("text/html"), false);
  assert.equal(isPasteableMedia("text/plain"), false);
  assert.equal(isPasteableMedia("application/pdf"), false);
  assert.equal(isPasteableMedia(""), false);
});

test("only the media in a paste is taken, in clipboard order", () => {
  const files = [
    { name: "notes.txt", type: "text/plain" },
    { name: "shot.png", type: "image/png" },
    { name: "clip.webm", type: "video/webm" },
  ];
  assert.deepEqual(
    pastedMedia(files).map((f) => f.name),
    ["shot.png", "clip.webm"],
  );
  // A paste with nothing of ours in it must come back empty, because that is what tells the
  // composer to let the browser insert the text instead of preventing the default.
  assert.deepEqual(pastedMedia([{ name: "notes.txt", type: "text/plain" }]), []);
  assert.deepEqual(pastedMedia(null), []);
  assert.deepEqual(pastedMedia(undefined), []);
});

test("a screenshot gets a name of its own and a real file keeps the one it has", () => {
  const at = Date.UTC(2026, 8, 3, 14, 15, 30, 123);
  // Every clipboard bitmap arrives as the same synthesized "image.png". Two of them in one
  // conversation would otherwise land on the same path in the member's embed folder.
  assert.equal(pastedName({ name: "image.png", type: "image/png" }, at), "pasted-20260903-141530.png");
  assert.equal(pastedName({ name: "image.jpeg", type: "image/jpeg" }, at), "pasted-20260903-141530.jpg");
  assert.equal(pastedName({ name: "", type: "image/webp" }, at), "pasted-20260903-141530.webp");
  // An unknown type still gets a usable name rather than one with no extension at all.
  assert.equal(pastedName({ name: "blob", type: "image/heic" }, at), "pasted-20260903-141530.png");
  // A file pasted from a file manager has a real name, and it is the one the sender expects to
  // see in the fileshare.
  assert.equal(pastedName({ name: "holiday.png", type: "image/png" }, at), "holiday.png");
  assert.equal(pastedName({ name: "  spaced.gif  ", type: "image/gif" }, at), "spaced.gif");
  // Two pastes a second apart are two different files.
  assert.notEqual(
    pastedName({ name: "image.png", type: "image/png" }, at),
    pastedName({ name: "image.png", type: "image/png" }, at + 1000),
  );
});
