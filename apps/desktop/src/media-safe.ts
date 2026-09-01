const SAFE_MEDIA_MIMES = new Set([
  "image/png", "image/jpeg", "image/gif", "image/webp", "image/avif", "image/bmp",
  "image/tiff", "image/x-icon",
  "audio/mpeg", "audio/ogg", "audio/wav", "audio/x-wav", "audio/flac", "audio/mp4",
  "audio/aac", "audio/webm",
  "video/mp4", "video/webm", "video/ogg", "video/quicktime", "video/x-msvideo",
]);

/** Keep frontend embed eligibility identical to the native protocol's inert-media allowlist. */
export function safeMediaMime(declared: string): string {
  const base = String(declared || "").trim().toLowerCase().split(";", 1)[0].trim();
  return SAFE_MEDIA_MIMES.has(base) ? base : "";
}
