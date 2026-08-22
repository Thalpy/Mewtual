// Local notification-sound policy. These preferences never enter a server document: a member's
// choice of whether (and how) their own device rings must not become peer-visible metadata.

export const NOTIFICATION_SOUND_KINDS = ["message", "mention", "news"] as const;
export type NotificationSoundKind = (typeof NOTIFICATION_SOUND_KINDS)[number];
export type SoundOverride = "inherit" | "on" | "off";
// "crunch" is the second built-in: the paper-scrunch synth, selectable anywhere the
// default chime is. Built-ins stay asset-free; only "custom" carries a stored file.
export type BuiltInTone = "default" | "crunch";
export type ToneOverride = "inherit" | "default" | "crunch" | "custom";

export const MAX_CUSTOM_TONE_BYTES = 384 * 1024;
export const MAX_CUSTOM_TONE_SECONDS = 8;

export type StoredTone = {
  name: string;
  mime: string;
  dataUrl: string;
};

export type GlobalSoundPrefs = Record<NotificationSoundKind, {
  enabled: boolean;
  tone: BuiltInTone | "custom";
  custom: StoredTone | null;
}>;

export type ServerSoundPrefs = Record<NotificationSoundKind, {
  enabled: SoundOverride;
  tone: ToneOverride;
  custom: StoredTone | null;
}>;

export type ResolvedSound = {
  enabled: boolean;
  custom: StoredTone | null;
  builtIn: BuiltInTone;
  source: "built-in" | "global custom" | "server custom";
};

const MIME_BY_EXTENSION: Record<string, string> = {
  mp3: "audio/mpeg",
  wav: "audio/wav",
  ogg: "audio/ogg",
  oga: "audio/ogg",
  webm: "audio/webm",
  m4a: "audio/mp4",
  mp4: "audio/mp4",
  aac: "audio/aac",
  flac: "audio/flac",
};
const SUPPORTED_MIMES = new Set([
  "audio/mpeg", "audio/mp3", "audio/wav", "audio/x-wav", "audio/ogg", "audio/webm",
  "audio/mp4", "audio/x-m4a", "audio/aac", "audio/flac",
]);

const globalEntry = (): GlobalSoundPrefs[NotificationSoundKind] => ({
  enabled: true,
  tone: "default",
  custom: null,
});

const serverEntry = (): ServerSoundPrefs[NotificationSoundKind] => ({
  enabled: "inherit",
  tone: "inherit",
  custom: null,
});

export function defaultGlobalSoundPrefs(): GlobalSoundPrefs {
  return { message: globalEntry(), mention: globalEntry(), news: globalEntry() };
}

export function defaultServerSoundPrefs(): ServerSoundPrefs {
  return { message: serverEntry(), mention: serverEntry(), news: serverEntry() };
}

/** Resolve a browser MIME, falling back to the extension because Windows often supplies "". */
export function customToneMime(fileType: string, fileName: string): string | null {
  const declared = fileType.trim().toLowerCase();
  if (SUPPORTED_MIMES.has(declared)) return declared;
  const ext = fileName.toLowerCase().match(/\.([a-z0-9]+)$/)?.[1] ?? "";
  return MIME_BY_EXTENSION[ext] ?? null;
}

/** Return a user-facing rejection reason, or null when an imported tone is safe to retain. */
export function customToneError(mime: string | null, size: number, duration: number): string | null {
  if (!mime || !SUPPORTED_MIMES.has(mime.toLowerCase())) {
    return "Choose an MP3, WAV, OGG, WebM, M4A, AAC, or FLAC audio file.";
  }
  if (size <= 0) return "That audio file is empty.";
  if (size > MAX_CUSTOM_TONE_BYTES) {
    return `Keep notification tones under ${Math.round(MAX_CUSTOM_TONE_BYTES / 1024)} KiB.`;
  }
  if (!Number.isFinite(duration) || duration <= 0) return "That audio file could not be decoded.";
  if (duration > MAX_CUSTOM_TONE_SECONDS) {
    return `Keep notification tones at ${MAX_CUSTOM_TONE_SECONDS} seconds or shorter.`;
  }
  return null;
}

function validStoredTone(value: unknown): StoredTone | null {
  if (!value || typeof value !== "object") return null;
  const tone = value as Partial<StoredTone>;
  if (typeof tone.name !== "string" || !tone.name.trim() || tone.name.length > 96) return null;
  if (typeof tone.mime !== "string" || !SUPPORTED_MIMES.has(tone.mime.toLowerCase())) return null;
  if (typeof tone.dataUrl !== "string") return null;
  // The encoded-size ceiling keeps malformed localStorage from becoming an unbounded DOM URL.
  const maxEncoded = Math.ceil(MAX_CUSTOM_TONE_BYTES * 4 / 3) + 160;
  if (tone.dataUrl.length > maxEncoded) return null;
  const prefix = `data:${tone.mime};base64,`;
  if (!tone.dataUrl.startsWith(prefix) || !/^[A-Za-z0-9+/]*={0,2}$/.test(tone.dataUrl.slice(prefix.length))) return null;
  return { name: tone.name.trim(), mime: tone.mime.toLowerCase(), dataUrl: tone.dataUrl };
}

export function parseGlobalSoundPrefs(raw: string | null): GlobalSoundPrefs {
  const out = defaultGlobalSoundPrefs();
  if (!raw) return out;
  try {
    const value = JSON.parse(raw) as Record<string, unknown>;
    for (const kind of NOTIFICATION_SOUND_KINDS) {
      const entry = value?.[kind];
      if (!entry || typeof entry !== "object") continue;
      const candidate = entry as Record<string, unknown>;
      if (typeof candidate.enabled === "boolean") out[kind].enabled = candidate.enabled;
      const custom = validStoredTone(candidate.custom);
      if (custom) out[kind].custom = custom;
      if (candidate.tone === "custom" && custom) {
        out[kind].tone = "custom";
      } else if (candidate.tone === "crunch") {
        out[kind].tone = "crunch";
      }
    }
  } catch {
    // Corrupt local settings degrade to defaults; notification preferences are never fatal.
  }
  return out;
}

export function parseServerSoundPrefs(raw: string | null): ServerSoundPrefs {
  const out = defaultServerSoundPrefs();
  if (!raw) return out;
  try {
    const value = JSON.parse(raw) as Record<string, unknown>;
    for (const kind of NOTIFICATION_SOUND_KINDS) {
      const entry = value?.[kind];
      if (!entry || typeof entry !== "object") continue;
      const candidate = entry as Record<string, unknown>;
      if (candidate.enabled === "inherit" || candidate.enabled === "on" || candidate.enabled === "off") {
        out[kind].enabled = candidate.enabled;
      }
      const custom = validStoredTone(candidate.custom);
      if (candidate.tone === "default" || candidate.tone === "crunch" || candidate.tone === "inherit") out[kind].tone = candidate.tone;
      else if (candidate.tone === "custom" && custom) out[kind].tone = "custom";
      if (custom) out[kind].custom = custom;
    }
  } catch {
    // Corrupt local settings degrade to inheritance, preserving the global policy.
  }
  return out;
}

/** Apply master → server enable override → server/global tone precedence in one tested place. */
export function resolveNotificationSound(
  masterEnabled: boolean,
  global: GlobalSoundPrefs,
  server: ServerSoundPrefs | null,
  kind: NotificationSoundKind,
): ResolvedSound {
  const local = server?.[kind];
  const categoryEnabled = local?.enabled === "on"
    ? true
    : local?.enabled === "off"
      ? false
      : global[kind].enabled;
  if (!masterEnabled || !categoryEnabled) return { enabled: false, custom: null, builtIn: "default", source: "built-in" };

  if (local?.tone === "custom" && local.custom) {
    return { enabled: true, custom: local.custom, builtIn: "default", source: "server custom" };
  }
  if (local?.tone === "default" || local?.tone === "crunch") {
    return { enabled: true, custom: null, builtIn: local.tone, source: "built-in" };
  }
  if (global[kind].tone === "custom" && global[kind].custom) {
    return { enabled: true, custom: global[kind].custom, builtIn: "default", source: "global custom" };
  }
  return { enabled: true, custom: null, builtIn: global[kind].tone === "crunch" ? "crunch" : "default", source: "built-in" };
}
