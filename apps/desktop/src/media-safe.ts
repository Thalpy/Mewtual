// AVIF is deliberately absent, and it is the one entry whose absence is about the decoder rather
// than about active content. Every image type below is decoded in Rust natively and re-encoded
// before it is served, so the platform never parses what a peer wrote; AVIF has no pure-Rust
// decoder available, so it is not served inline at all and stays a file to open deliberately.
//
// This list must stay identical to the native `safe_media_mime` allowlist. A type listed here but
// not there gets an <img> built for a URL that will never return a body, which renders as a broken
// image instead of falling through to the download chip.
const SAFE_MEDIA_MIMES = new Set([
  "image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp",
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
