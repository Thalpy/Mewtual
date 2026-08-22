import test from "node:test";
import assert from "node:assert/strict";
import { decodeTextFile, lineCountLabel, textFileKind, TEXT_PREVIEW_MAX_BYTES } from "./textfile.ts";

const utf8 = (s: string) => new TextEncoder().encode(s);

test("markdown extensions get the rendered viewer", () => {
  for (const name of ["notes.md", "NOTES.MD", "guide.markdown", "log.mkd"]) {
    assert.equal(textFileKind(name, ""), "markdown", name);
  }
});

test("documents, config and source share the plain-text viewer", () => {
  for (const name of ["changes.txt", "server.log", "config.toml", "main.rs", "deploy.ps1", "fix.patch"]) {
    assert.equal(textFileKind(name, ""), "text", name);
  }
});

test("extension-less conventions are text, including dotfiles", () => {
  assert.equal(textFileKind("LICENSE", ""), "text");
  assert.equal(textFileKind("Makefile", ""), "text");
  assert.equal(textFileKind(".gitignore", ""), "text");
  assert.equal(textFileKind("README", ""), "text");
});

test("an unknown name falls back to the declared MIME", () => {
  assert.equal(textFileKind("attachment", "text/plain; charset=utf-8"), "text");
  assert.equal(textFileKind("attachment", "text/markdown"), "markdown");
  assert.equal(textFileKind("attachment", "application/json"), "text");
  assert.equal(textFileKind("attachment", "application/vnd.acme+xml"), "text");
  assert.equal(textFileKind("attachment", "application/octet-stream"), "");
  assert.equal(textFileKind("attachment", ""), "");
});

test("media never opens the text reader, so one file never shows two previews", () => {
  assert.equal(textFileKind("cat.png", "image/png"), "");
  assert.equal(textFileKind("clip", "video/mp4"), "");
  assert.equal(textFileKind("song", "audio/ogg"), "");
});

test("the extension beats a MIME the uploader stamped wrong", () => {
  assert.equal(textFileKind("notes.md", "application/octet-stream"), "markdown");
  assert.equal(textFileKind("notes.md", "image/png"), "markdown");
  // An unknown extension with a text MIME still opens the reader; if the bytes are not text
  // after all, `decodeTextFile` says so rather than the pane rendering rubbish.
  assert.equal(textFileKind("cat.png", "text/plain"), "text");
});

test("a name with no real extension is not read off its trailing dot or leading dot", () => {
  assert.equal(textFileKind("archive.", ""), "");
  assert.equal(textFileKind(".md", ""), "");
});

test("decoding strips a UTF-8 BOM and normalises CRLF", () => {
  const bytes = new Uint8Array([0xef, 0xbb, 0xbf, ...utf8("a\r\nb\rc\n")]);
  const out = decodeTextFile(bytes);
  assert.deepEqual(out, { ok: true, text: "a\nb\nc\n", lines: 4 });
});

test("UTF-16 is decoded from its byte-order mark", () => {
  const le = new Uint8Array([0xff, 0xfe, 0x68, 0x00, 0x69, 0x00]);
  assert.deepEqual(decodeTextFile(le), { ok: true, text: "hi", lines: 1 });
  const be = new Uint8Array([0xfe, 0xff, 0x00, 0x68, 0x00, 0x69]);
  assert.deepEqual(decodeTextFile(be), { ok: true, text: "hi", lines: 1 });
});

test("invalid UTF-8 and embedded NULs are reported as binary, not shown as mojibake", () => {
  assert.deepEqual(decodeTextFile(new Uint8Array([0xff, 0x00, 0x41])), { ok: false, reason: "binary" });
  assert.deepEqual(decodeTextFile(new Uint8Array([0x41, 0x00, 0x42])), { ok: false, reason: "binary" });
});

test("an empty file decodes to nothing rather than one phantom line", () => {
  assert.deepEqual(decodeTextFile(new Uint8Array()), { ok: true, text: "", lines: 0 });
});

test("the inline cap stays well under the backend's 256 MiB file limit", () => {
  assert.ok(TEXT_PREVIEW_MAX_BYTES > 0 && TEXT_PREVIEW_MAX_BYTES <= 8 * 1024 * 1024);
});

test("line counts are labelled with a singular for one line", () => {
  assert.equal(lineCountLabel(1), "1 line");
  assert.equal(lineCountLabel(0), "0 lines");
  assert.equal(lineCountLabel(42), "42 lines");
});
