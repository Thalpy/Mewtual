import { test } from "node:test";
import assert from "node:assert/strict";

import { IMAGE_SRC_CACHE_MAX, ImageSrcCache, imageMime, imageSrc } from "./image-src.ts";

test("a stored image keeps its own format", () => {
  assert.equal(imageMime("R0lGODlhAQ"), "image/gif");
  assert.equal(imageMime("iVBORw0KGgo"), "image/png");
  assert.equal(imageMime("UklGRiQ"), "image/webp");
  assert.equal(imageMime("/9j/4AAQ"), "image/jpeg");
  assert.equal(imageSrc("iVBORw0KGgo"), "data:image/png;base64,iVBORw0KGgo");
});

test("the same bytes render the same url once, and an empty avatar renders nothing", () => {
  const cache = new ImageSrcCache(2);
  assert.equal(cache.src(""), "");
  assert.equal(cache.size, 0, "nothing is memoized for an absent image");
  const first = cache.src("iVBORw0KGgo");
  assert.equal(cache.src("iVBORw0KGgo"), first);
  assert.equal(cache.size, 1);
  cache.clear();
  assert.equal(cache.size, 0);
});

test("past the bound the answer is still right, and no memo already handed out is rewritten", () => {
  const cache = new ImageSrcCache(2);
  const a = cache.src("R0lGOD-a");
  cache.src("R0lGOD-b");
  const overflow = cache.src("R0lGOD-c");
  assert.equal(overflow, imageSrc("R0lGOD-c"), "an uncached image still renders correctly");
  assert.equal(cache.size, 2, "the bound holds");
  assert.equal(cache.src("R0lGOD-a"), a, "the entries already in hand are untouched");
  assert.ok(IMAGE_SRC_CACHE_MAX >= 32, "the default bound covers a roster");
  assert.throws(() => new ImageSrcCache(0));
});
