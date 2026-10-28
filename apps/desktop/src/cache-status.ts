// Cache housekeeping, frontend side: the footer segment, the Storage settings page and the leave
// confirmation. Everything here is pure so it can be tested without the app; the numbers come
// from the native planner (`get_cache_status` / the `cache-status` event) and are shown as given,
// never recomputed, so the footer always says exactly what the planner would do.

/** What native sends. Byte counts are sealed on-disk sizes on this device. */
export type CacheStatus = {
  usage_bytes: number;
  /** `null` for No limit, or while the limit is not known yet (see `paused`). */
  limit_bytes: number | null;
  unlimited: boolean;
  pinned_bytes: number;
  leaving_soon_files: number;
  leaving_soon_bytes: number;
  nearly_full: boolean;
  over_limit: boolean;
  held_by_pins: boolean;
  /** Why nothing is being cleaned up right now, or `null`. */
  paused: CachePause | null;
};

export type CachePause =
  | "no-cache"
  | "not-ready"
  | "scan-incomplete"
  | "setting-unreadable"
  | "index-unreadable"
  | "pins-unknown"
  | "studio-unknown";

const PAUSES: readonly CachePause[] = [
  "no-cache", "not-ready", "scan-incomplete", "setting-unreadable", "index-unreadable",
  "pins-unknown", "studio-unknown",
];

/** Whether the footer should show a cache segment at all: not for a server with no size-limited
 * cache (the in-memory fallback before the vault store attaches), whose usage is unknown. */
export function cacheShown(status: CacheStatus | null): status is CacheStatus {
  return !!status && status.paused !== "no-cache";
}

const count = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;

/** Validate a native status; anything malformed reads as "no status" rather than a guess. */
export function checkedCacheStatus(raw: unknown): CacheStatus | null {
  if (!raw || typeof raw !== "object") return null;
  const s = raw as Record<string, unknown>;
  const limit = s.limit_bytes;
  const paused = s.paused;
  if (
    !count(s.usage_bytes) || !(limit === null || count(limit)) || typeof s.unlimited !== "boolean"
    || !count(s.pinned_bytes) || !count(s.leaving_soon_files) || !count(s.leaving_soon_bytes)
    || typeof s.nearly_full !== "boolean" || typeof s.over_limit !== "boolean"
    || typeof s.held_by_pins !== "boolean"
    || !(paused === null || (typeof paused === "string" && PAUSES.includes(paused as CachePause)))
  ) return null;
  return {
    usage_bytes: s.usage_bytes as number,
    limit_bytes: limit as number | null,
    unlimited: s.unlimited as boolean,
    pinned_bytes: s.pinned_bytes as number,
    leaving_soon_files: s.leaving_soon_files as number,
    leaving_soon_bytes: s.leaving_soon_bytes as number,
    nearly_full: s.nearly_full as boolean,
    over_limit: s.over_limit as boolean,
    held_by_pins: s.held_by_pins as boolean,
    paused: paused as CachePause | null,
  };
}

/** Sizes in binary units up to TiB, matching the rest of the app's byte formatting. */
export function cacheBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** unit;
  const text = value >= 10 || unit === 0 || Number.isInteger(value) ? value.toFixed(0) : value.toFixed(1);
  return `${text} ${units[unit]}`;
}

/** Whole percent of the limit used, or `null` with no limit to compare against. */
export function cachePercent(status: CacheStatus): number | null {
  if (status.limit_bytes === null || status.limit_bytes <= 0) return null;
  return Math.floor((status.usage_bytes * 100) / status.limit_bytes);
}

export type CacheTone = "ok" | "warn" | "over" | "paused";

export function cacheTone(status: CacheStatus): CacheTone {
  if (status.paused && status.paused !== "not-ready" && status.paused !== "no-cache") return "paused";
  if (status.over_limit) return "over";
  if (status.nearly_full) return "warn";
  return "ok";
}

/** Short reasons for a paused cleanup, for the settings page. */
export function cachePauseText(pause: CachePause): string {
  switch (pause) {
    case "no-cache": return "This server has no disk cache on this device.";
    case "not-ready": return "Measuring this device's cache.";
    case "scan-incomplete": return "Some cached files could not be measured, so nothing is removed for now.";
    case "setting-unreadable": return "The saved limit could not be read. Choose a limit again to resume cleanup.";
    case "index-unreadable": return "The shared file list could not be read, so nothing is removed for now.";
    case "pins-unknown": return "Kept files or downloads could not be checked, so nothing is removed for now.";
    case "studio-unknown": return "Studio work is still being checked, so nothing is removed for now.";
  }
}

export type CacheFooter = {
  /** Value text beside the bar: a percentage, or the size when there is no limit. */
  value: string;
  /** Bar width, clamped to 0 to 100, or `null` for no bar. */
  bar: number | null;
  tone: CacheTone;
  /** The alert chip, if any. */
  chip: string | null;
  /** Hover text explaining what the bar means. */
  title: string;
};

const KEEPS = "Kept files, files you shared from this device, Studio work and files in use stay.";

export function cacheFooter(status: CacheStatus): CacheFooter {
  const tone = cacheTone(status);
  const percent = cachePercent(status);
  const used = cacheBytes(status.usage_bytes);
  if (status.paused === "not-ready" && !status.unlimited) {
    return { value: used, bar: null, tone: "ok", chip: null, title: `${used} cached on this device.` };
  }
  let chip: string | null = null;
  if (tone === "paused") chip = "cleanup paused";
  else if (status.held_by_pins) chip = "held by pinned files";
  else if (status.leaving_soon_files > 0) {
    chip = `${status.leaving_soon_files} leaving soon`;
  } else if (status.over_limit) chip = "over limit";
  if (status.unlimited || percent === null) {
    return { value: used, bar: null, tone, chip, title: `${used} cached on this device, with no limit.` };
  }
  const limit = cacheBytes(status.limit_bytes as number);
  return {
    value: `${percent}%`,
    bar: Math.max(0, Math.min(100, percent)),
    tone,
    chip,
    title: `${used} of ${limit} used on this device. The oldest files leave first. ${KEEPS}`,
  };
}

/** The limit picker's choices. `null` is No limit. */
export const CACHE_LIMIT_CHOICES: readonly { label: string; bytes: number | null }[] = [
  ...[1, 2, 5, 10, 20, 50, 100].map((gib): { label: string; bytes: number | null } => ({
    label: `${gib} GiB`,
    bytes: gib * 1024 ** 3,
  })),
  { label: "1 TiB", bytes: 1024 ** 4 },
  { label: "No limit", bytes: null },
];

/** A choice's label for the current limit, or a size label for a migrated custom value. */
export function limitLabel(limit: number | null): string {
  if (limit === null) return "No limit";
  return CACHE_LIMIT_CHOICES.find((c) => c.bytes === limit)?.label ?? cacheBytes(limit);
}

/** The confirmation shown before lowering the limit below what is cached, or `null`. */
export function lowerLimitWarning(status: CacheStatus, next: number | null): string | null {
  if (next === null || status.usage_bytes <= next) return null;
  return `This limit is below the ${cacheBytes(status.usage_bytes)} cached now. About ${cacheBytes(status.usage_bytes - next)} of the oldest files will be removed from this device straight away. Kept files, files you shared from this device, Studio work and files in use stay.`;
}

/** One line for the settings page summarising what is pinned. */
export function pinnedSummary(status: CacheStatus): string {
  return status.pinned_bytes > 0
    ? `${cacheBytes(status.pinned_bytes)} is pinned and stays whatever the limit. ${KEEPS}`
    : `Nothing is pinned right now. ${KEEPS}`;
}

/** What leaving deletes, for the confirmation: names of Keep copies and unfinished Downloads.
 * `unknown` is set when either inventory could not be read: the dialog must then say it could not
 * check, never that there is nothing to lose. */
export type LeaveLoss = { keep: string[]; downloads: string[]; unknown: boolean };

/** Build the confirmation lists. Names come from the server's file list where it still has the
 * file; otherwise a short address stands in, so nothing is silently left out. */
export function leaveLoss(
  kept: { files: { cid: string }[]; transfers: { cid: string; phase?: string }[]; error?: string | null } | null,
  downloads: { cid: string; status: string }[] | null,
  files: { cid: string; name: string }[],
): LeaveLoss {
  const unknown = !kept || !!kept.error || downloads === null;
  const nameOf = (cid: string) => files.find((f) => f.cid === cid)?.name ?? `file ${cid.slice(0, 8)}`;
  const keep = new Set<string>();
  for (const f of kept?.files ?? []) keep.add(nameOf(f.cid));
  for (const t of kept?.transfers ?? []) if (t.phase !== "deleting") keep.add(nameOf(t.cid));
  const pending = new Set<string>();
  for (const d of downloads ?? []) if (d.status !== "saved" && d.status !== "removing") pending.add(nameOf(d.cid));
  return { keep: [...keep].sort(), downloads: [...pending].sort(), unknown };
}

/** The toast after leaving. Only bytes actually removed are claimed. */
export function leaveReportText(
  name: string,
  report: { freed_bytes: number; kept_bytes: number; failed: number; cleaned: boolean } | null,
): string {
  if (!report) return `Left ${name}.`;
  // Nothing was deleted and nothing is left to resume (the group is in use again, another entry
  // shares it, or it had nothing on disk), so no "for now": these files are not removed later.
  if (!report.cleaned && report.failed === 0) {
    return `Left ${name}. Its files on this device stay, including any Keep copies and unfinished downloads.`;
  }
  const parts = [`Left ${name}.`];
  if (report.freed_bytes > 0) parts.push(`Freed ${cacheBytes(report.freed_bytes)} on this device.`);
  if (report.kept_bytes > 0) parts.push(`Kept ${cacheBytes(report.kept_bytes)} that Studio still uses.`);
  if (report.failed > 0) parts.push("Some files could not be removed yet; Mewtual tries again after the next unlock.");
  return parts.join(" ");
}
