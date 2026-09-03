/**
 * Pasting media into a composer.
 *
 * Dropping a file onto the composer has always worked; pasting one never did, because nothing
 * listened for the event. A screenshot is the common case and the worst one to be missing: the
 * webview hands it over as a synthesized file rather than as text, so with no paste handler the
 * keystroke did nothing at all and looked like a broken app rather than a missing feature.
 *
 * The rules live here, structurally typed, so they are decidable without a browser: what matters
 * about a pasted file is only its declared type and the name the webview invented for it.
 */

/** A pasted file as these rules need to see it. */
export type PastedItem = { name: string; type: string };

/**
 * Names a webview synthesizes for clipboard bitmaps, which carry no name of their own.
 *
 * Every paste of a screenshot produces the same one, so two pastes into the same conversation
 * would land on the same path in the member's embed folder. Renaming only these leaves a file
 * pasted from a file manager, which does have a real name, alone.
 */
const SYNTHETIC_NAMES = new Set(["image.png", "image.jpeg", "image.jpg", "image", "blob"]);

/** The extension to give a synthesized name, by declared type. */
const IMAGE_EXTENSIONS: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/avif": "avif",
  "image/bmp": "bmp",
};

/**
 * Whether a paste carrying this item should be handled as an embed rather than as text.
 *
 * Deliberately narrow. Copying a region of a web page puts BOTH an HTML flavour and an image
 * flavour on the clipboard in some browsers, and treating every file-bearing paste as an upload
 * would turn an ordinary text paste into a surprise file share. Media is what the composer's
 * attach button already accepts, so it is what paste accepts too.
 */
export function isPasteableMedia(type: string): boolean {
  const mime = type.toLowerCase();
  return mime.startsWith("image/") || mime.startsWith("video/") || mime.startsWith("audio/");
}

/**
 * The name to share a pasted file under.
 *
 * A real name is kept as it is. A synthesized one becomes a timestamped name, in UTC so that the
 * same paste produces the same name everywhere and the ordering in a folder listing matches the
 * order things were pasted in.
 */
export function pastedName(item: PastedItem, at: number): string {
  const given = item.name.trim();
  if (given && !SYNTHETIC_NAMES.has(given.toLowerCase())) return given;
  const extension = IMAGE_EXTENSIONS[item.type.toLowerCase()] ?? "png";
  const iso = new Date(at).toISOString(); // 2026-09-03T14:15:30.123Z
  const stamp = `${iso.slice(0, 10).replace(/-/g, "")}-${iso.slice(11, 19).replace(/:/g, "")}`;
  return `pasted-${stamp}.${extension}`;
}

/**
 * The media a paste carries, in clipboard order. Empty means the paste is not ours to handle and
 * the composer must let the browser insert whatever text was on the clipboard.
 */
export function pastedMedia<T extends PastedItem>(files: ArrayLike<T> | null | undefined): T[] {
  const out: T[] = [];
  for (let i = 0; i < (files?.length ?? 0); i += 1) {
    const file = files?.[i];
    if (file && isPasteableMedia(file.type)) out.push(file);
  }
  return out;
}
