import test from "node:test";
import assert from "node:assert/strict";
import { pastedImageUrl, safeRemoteUrl } from "./remote-media.ts";

test("safeRemoteUrl accepts only bounded HTTP(S) URLs", () => {
  assert.equal(safeRemoteUrl("https://example.test/cat.png"), "https://example.test/cat.png");
  assert.equal(safeRemoteUrl("http://localhost:1420/cat.gif"), "http://localhost:1420/cat.gif");
  assert.equal(safeRemoteUrl("javascript:alert(1)"), "");
  assert.equal(safeRemoteUrl("data:image/png;base64,AAAA"), "");
  assert.equal(safeRemoteUrl("not a URL"), "");
  assert.equal(safeRemoteUrl(`https://example.test/${"x".repeat(4096)}`), "");
});

test("pastedImageUrl recognises direct image links", () => {
  assert.equal(
    pastedImageUrl("https://cdn.example.test/photo.JPEG?width=800"),
    "https://cdn.example.test/photo.JPEG?width=800",
  );
  assert.equal(pastedImageUrl("https://example.test/article"), "");
});

test("pastedImageUrl converts common Giphy share links", () => {
  assert.equal(
    pastedImageUrl("https://giphy.com/gifs/cat-happy-AbC123"),
    "https://media.giphy.com/media/AbC123/giphy.gif",
  );
  assert.equal(
    pastedImageUrl("https://www.giphy.com/gifs/AbC123"),
    "https://media.giphy.com/media/AbC123/giphy.gif",
  );
});
