import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { safeMediaMime } from "./media-safe.ts";

test("only the native inert-media allowlist is eligible for an inline decoder", () => {
  assert.equal(safeMediaMime("VIDEO/MP4; codecs=avc1"), "video/mp4");
  assert.equal(safeMediaMime("image/png"), "image/png");
  assert.equal(safeMediaMime("image/svg+xml"), "");
  assert.equal(safeMediaMime("text/html"), "");
  assert.equal(safeMediaMime("application/vnd.apple.mpegurl"), "");
});

test("the frontend and native media allowlists are the same list", () => {
  // Two copies of a security allowlist in two languages is a drift hazard, and the drift is
  // invisible in both directions. A type the frontend admits but the native scheme does not gets
  // an <img> pointed at a URL that will never return a body, which renders as a broken image
  // instead of falling through to the download chip. A type the native side admits but the
  // frontend does not is a file that silently never loads inline at all.
  //
  // This is the guard that would have caught dropping AVIF natively without dropping it here.
  const bridge = readFileSync(
    fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url)),
    "utf8",
  );
  const start = bridge.indexOf("fn safe_media_mime(");
  assert.ok(start > 0, "safe_media_mime must remain findable in the bridge");
  const allowlist = bridge.slice(start, bridge.indexOf("\n}", start));
  const matches = allowlist.indexOf("matches!(");
  assert.ok(matches > 0, "safe_media_mime must keep its literal matches! allowlist");
  const arm = allowlist.slice(matches, allowlist.indexOf(");", matches));
  // Only string literals on the match arms, so the comments above them cannot be mistaken for
  // entries. AVIF is named in that prose precisely because it is NOT in the list.
  const native = [...arm.matchAll(/"([a-z]+\/[a-z0-9.+-]+)"/g)].map((m) => m[1]);
  assert.ok(
    native.length > 15,
    `extraction found only ${native.length} native media types: the allowlist is being under-read, and an under-read list agrees with anything`,
  );

  const source = readFileSync(fileURLToPath(new URL("./media-safe.ts", import.meta.url)), "utf8");
  const set = source.slice(source.indexOf("new Set(["), source.indexOf("]);"));
  const frontend = [...set.matchAll(/"([a-z]+\/[a-z0-9.+-]+)"/g)].map((m) => m[1]);

  assert.deepEqual(
    [...frontend].sort(),
    [...native].sort(),
    "media-safe.ts and safe_media_mime in lib.rs disagree about what may load inline",
  );
  // And the shipped helper really does answer from that list, rather than the list being decorative.
  for (const mime of frontend) assert.equal(safeMediaMime(mime), mime);
  assert.ok(!frontend.includes("image/avif"), "AVIF has no pure-Rust decoder and must not load inline");
});
