import assert from "node:assert/strict";
import test from "node:test";
import {
  CACHE_LIMIT_CHOICES, cacheBytes, cacheFooter, cachePauseText, cachePercent, cacheShown, cacheTone,
  checkedCacheStatus, leaveLoss, leaveReportText, limitLabel, lowerLimitWarning, pinnedSummary,
  type CacheStatus,
} from "./cache-status.ts";

const GIB = 1024 ** 3;

function status(over: Partial<CacheStatus> = {}): CacheStatus {
  return {
    usage_bytes: 3 * GIB, limit_bytes: 5 * GIB, unlimited: false, pinned_bytes: 0,
    leaving_soon_files: 0, leaving_soon_bytes: 0, nearly_full: false, over_limit: false,
    held_by_pins: false, paused: null, ...over,
  };
}

test("malformed native statuses read as no status, never as a guess", () => {
  assert.deepEqual(checkedCacheStatus(status()), status());
  assert.equal(checkedCacheStatus(null), null);
  assert.equal(checkedCacheStatus({ ...status(), usage_bytes: -1 }), null);
  assert.equal(checkedCacheStatus({ ...status(), usage_bytes: 1.5 }), null);
  assert.equal(checkedCacheStatus({ ...status(), paused: "made-up" }), null);
  assert.equal(checkedCacheStatus({ ...status(), limit_bytes: "5" }), null);
  assert.deepEqual(checkedCacheStatus(status({ limit_bytes: null, unlimited: true })),
    status({ limit_bytes: null, unlimited: true }));
});

test("sizes use binary units up to TiB", () => {
  assert.equal(cacheBytes(0), "0 B");
  assert.equal(cacheBytes(512), "512 B");
  assert.equal(cacheBytes(5 * GIB), "5 GiB");
  assert.equal(cacheBytes(2.84 * GIB), "2.8 GiB");
  assert.equal(cacheBytes(1024 * GIB), "1 TiB");
});

test("the footer shows the percentage, the tone and the chip the planner reports", () => {
  const calm = cacheFooter(status());
  assert.equal(calm.value, "60%");
  assert.equal(calm.bar, 60);
  assert.equal(calm.tone, "ok");
  assert.equal(calm.chip, null);
  assert.match(calm.title, /^3 GiB of 5 GiB used on this device\. The oldest files leave first\./);

  const amber = cacheFooter(status({ usage_bytes: 4.7 * GIB, nearly_full: true, leaving_soon_files: 4 }));
  assert.equal(amber.tone, "warn");
  assert.equal(amber.chip, "4 leaving soon");

  const over = cacheFooter(status({ usage_bytes: 6 * GIB, over_limit: true, nearly_full: true }));
  assert.equal(over.tone, "over");
  assert.equal(over.bar, 100, "the bar is clamped");
  assert.equal(over.value, "120%");
  assert.equal(over.chip, "over limit");

  const pinned = cacheFooter(status({ usage_bytes: 6 * GIB, over_limit: true, held_by_pins: true, leaving_soon_files: 0 }));
  assert.equal(pinned.chip, "held by pinned files", "no count that nothing will act on");

  const paused = cacheFooter(status({ paused: "studio-unknown" }));
  assert.equal(paused.tone, "paused");
  assert.equal(paused.chip, "cleanup paused");

  const unlimited = cacheFooter(status({ limit_bytes: null, unlimited: true, usage_bytes: 3.2 * GIB }));
  assert.equal(unlimited.value, "3.2 GiB");
  assert.equal(unlimited.bar, null);

  const measuring = cacheFooter(status({ paused: "not-ready", limit_bytes: null }));
  assert.equal(measuring.bar, null);
  assert.equal(measuring.chip, null, "not knowing the limit yet is not an alarm");
});

test("tone and percentage follow the native flags, not a local recomputation", () => {
  assert.equal(cachePercent(status({ limit_bytes: null })), null);
  assert.equal(cacheTone(status({ nearly_full: true })), "warn");
  assert.equal(cacheTone(status({ paused: "not-ready" })), "ok");
  assert.equal(cacheTone(status({ paused: "no-cache" })), "ok");
  for (const pause of ["scan-incomplete", "setting-unreadable", "index-unreadable", "pins-unknown", "studio-unknown", "not-ready", "no-cache"] as const) {
    assert.ok(cachePauseText(pause).length > 10);
  }
});

test("the limit picker offers 1 GiB to 1 TiB and No limit", () => {
  assert.deepEqual(CACHE_LIMIT_CHOICES.map((c) => c.label),
    ["1 GiB", "2 GiB", "5 GiB", "10 GiB", "20 GiB", "50 GiB", "100 GiB", "1 TiB", "No limit"]);
  assert.equal(CACHE_LIMIT_CHOICES[0].bytes, GIB);
  assert.equal(CACHE_LIMIT_CHOICES.at(-1)?.bytes, null);
  assert.equal(limitLabel(5 * GIB), "5 GiB");
  assert.equal(limitLabel(null), "No limit");
  assert.equal(limitLabel(6 * GIB), "6 GiB", "a migrated custom value still reads sensibly");
});

test("lowering the limit below what is cached warns first; raising it does not", () => {
  assert.equal(lowerLimitWarning(status(), 10 * GIB), null);
  assert.equal(lowerLimitWarning(status(), null), null);
  const warning = lowerLimitWarning(status(), 2 * GIB);
  assert.match(warning ?? "", /About 1 GiB of the oldest files will be removed/);
  assert.match(pinnedSummary(status({ pinned_bytes: GIB })), /^1 GiB is pinned/);
});

test("the leave confirmation names Keep copies and unfinished downloads", () => {
  const files = [{ cid: "aa", name: "song.mp3" }, { cid: "bb", name: "clip.mp4" }];
  const loss = leaveLoss(
    {
      files: [{ cid: "aa" }],
      transfers: [{ cid: "cc11223344", phase: "paused" }, { cid: "bb", phase: "deleting" }],
    },
    [{ cid: "bb", status: "paused" }, { cid: "aa", status: "saved" }],
    files,
  );
  assert.deepEqual(loss.keep, ["file cc112233", "song.mp3"]);
  assert.deepEqual(loss.downloads, ["clip.mp4"], "finished saves are not lost; removals are already going");
  assert.equal(loss.unknown, false);
});

test("an unreadable inventory is reported as unknown, never as nothing to lose", () => {
  assert.equal(leaveLoss(null, [], []).unknown, true);
  assert.equal(leaveLoss({ files: [], transfers: [] }, null, []).unknown, true);
  assert.equal(leaveLoss({ files: [], transfers: [], error: "Kept storage needs attention" }, [], []).unknown, true);
  assert.deepEqual(leaveLoss({ files: [], transfers: [], error: null }, [], []), { keep: [], downloads: [], unknown: false });
});

test("the footer hides for a server with no size-limited cache", () => {
  assert.equal(cacheShown(null), false);
  assert.equal(cacheShown(status({ paused: "no-cache" })), false);
  assert.equal(cacheShown(status({ paused: "not-ready" })), true);
  assert.equal(checkedCacheStatus(status({ paused: "no-cache" }))?.paused, "no-cache");
});

test("the leave toast claims only bytes actually removed", () => {
  assert.equal(leaveReportText("Lantern Room", null), "Left Lantern Room.");
  assert.equal(
    leaveReportText("Lantern Room", { freed_bytes: 1.2 * GIB, kept_bytes: 0, failed: 0, cleaned: true }),
    "Left Lantern Room. Freed 1.2 GiB on this device.",
  );
  assert.equal(
    leaveReportText("Lantern Room", { freed_bytes: 0, kept_bytes: 0, failed: 0, cleaned: false }),
    "Left Lantern Room. Its cached files stay on this device for now.",
    "a skipped cleanup never reads as a clean one",
  );
  const partial = leaveReportText("Lantern Room", { freed_bytes: 0, kept_bytes: 2048, failed: 3, cleaned: true });
  assert.match(partial, /Kept 2 KiB that Studio still uses\./);
  assert.match(partial, /tries again after the next unlock/);
  assert.doesNotMatch(partial, /Freed/);
});

test("no separators or em dashes in any of this text", () => {
  const texts = [
    cacheFooter(status()).title, cacheFooter(status({ held_by_pins: true })).chip ?? "",
    lowerLimitWarning(status(), GIB) ?? "", pinnedSummary(status()),
    leaveReportText("x", { freed_bytes: 1, kept_bytes: 1, failed: 1, cleaned: true }),
    leaveReportText("x", { freed_bytes: 0, kept_bytes: 0, failed: 0, cleaned: false }),
    ...(["scan-incomplete", "studio-unknown"] as const).map(cachePauseText),
  ];
  for (const text of texts) assert.doesNotMatch(text, /[·—]/);
});
