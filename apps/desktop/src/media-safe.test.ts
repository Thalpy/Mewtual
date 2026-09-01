import assert from "node:assert/strict";
import test from "node:test";
import { safeMediaMime } from "./media-safe.ts";

test("only the native inert-media allowlist is eligible for an inline decoder", () => {
  assert.equal(safeMediaMime("VIDEO/MP4; codecs=avc1"), "video/mp4");
  assert.equal(safeMediaMime("image/png"), "image/png");
  assert.equal(safeMediaMime("image/svg+xml"), "");
  assert.equal(safeMediaMime("text/html"), "");
  assert.equal(safeMediaMime("application/vnd.apple.mpegurl"), "");
});
