/**
 * Data URLs for the small images that live in documents as base64 (avatars, banners, server and
 * livery icons).
 *
 * Building one is a string concatenation over the whole payload, and the same handful of images
 * are rebuilt on every render of every row that shows them: a roster of avatars re-rendered per
 * profile update cost megabytes of transient strings. The value is a pure function of the bytes,
 * so it is memoized by them. The cache is bounded and stops growing rather than evicting: an
 * entry can be referenced by a mounted `<img>`, and a bound that is reached is a signal to render
 * uncached, never to invalidate something on screen.
 */

/** Distinct images memoized at once. Comfortably above one server's roster. */
export const IMAGE_SRC_CACHE_MAX = 64;

/**
 * The MIME type of a stored image, sniffed from its base64 prefix (the first magic bytes survive
 * base64 alignment). Profiles store opaque bytes, so an animated GIF or WebP a member uploaded
 * plays back as itself instead of being branded a JPEG.
 */
export function imageMime(b64: string): string {
  if (b64.startsWith("R0lGOD")) return "image/gif";
  if (b64.startsWith("iVBOR")) return "image/png";
  if (b64.startsWith("UklGR")) return "image/webp";
  return "image/jpeg";
}

/** The data URL for stored image bytes, uncached. */
export function imageSrc(b64: string): string {
  return `data:${imageMime(b64)};base64,${b64}`;
}

/** A bounded memo of [`imageSrc`], keyed by the image bytes themselves. */
export class ImageSrcCache {
  readonly #entries = new Map<string, string>();
  readonly capacity: number;

  constructor(capacity = IMAGE_SRC_CACHE_MAX) {
    if (!Number.isInteger(capacity) || capacity < 1) throw new Error("cache capacity must be positive");
    this.capacity = capacity;
  }

  /** The data URL for `b64`, memoized while there is room. Empty in, empty out. */
  src(b64: string): string {
    if (!b64) return "";
    const hit = this.#entries.get(b64);
    if (hit !== undefined) return hit;
    const url = imageSrc(b64);
    // Past the bound the answer is still correct, just not remembered. Nothing already handed to
    // a mounted element is ever dropped or rewritten.
    if (this.#entries.size < this.capacity) this.#entries.set(b64, url);
    return url;
  }

  /** Drop every memo. Called at the lock boundary, where the images leave the screen anyway. */
  clear(): void {
    this.#entries.clear();
  }

  get size(): number {
    return this.#entries.size;
  }
}
