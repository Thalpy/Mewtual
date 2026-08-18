// The wiki infobox: the summary card a page floats at its top right (Wikipedia's `{{Infobox …}}`).
//
// Authored as one MediaWiki-style template block, pipe-separated `key = value` fields:
//
//   {{Infobox
//   | title   = Whiskers
//   | image   = ![a tabby cat](cid:9f86d0…)
//   | caption = Photographed at the cafe
//   | Species = Cat
//   | Owner   = [[Alice]]
//   | Details =
//   | Age     = 4
//   }}
//
// `title`, `image` and `caption` are the card's own chrome; every other field is a label/value
// row **in the order the author wrote them**, and a field with an EMPTY value is a section band
// (`Details` above). The block is format-agnostic on purpose: a page may be markdown or wikitext,
// so `render.ts` extracts it before either converter runs and passes back that page's own inline
// renderer for the values. One syntax, one card, both formats.
//
// Two deliberate limits:
//   * Only the FIRST block on a page becomes the card (a page has one infobox); a second stays
//     literal text, so a stray `{{` can never swallow the article.
//   * An image must be written as the app's `![alt](cid:HEX)` embed marker, never a bare content
//     address. The marker is what the backend's never-decay scan looks for, so an infobox picture
//     is pinned like any other embed; a bare address would quietly expire out of circulation.
//
// PURE (no DOM, no marked): unit-tested under plain Node, like `wikitext.ts`. It emits an HTML
// *string* that `render.ts` always sanitizes; the only tags it can produce are the sanitizer's
// table allow-list, and every author-supplied character reaches the output through the caller's
// inline renderer, which escapes exactly as it does for the body.

/** One label/value row. An empty `value` marks a section band spanning the card. */
export type InfoboxField = { label: string; value: string };

/** A parsed infobox: the card's chrome plus its rows, in author order. */
export type Infobox = {
  /** The card's heading; empty if the author declared none. */
  title: string;
  /** The `![alt](cid:HEX)` embed marker for the picture, or empty. */
  image: string;
  /** The line under the picture; empty if none. */
  caption: string;
  /** The label/value rows. */
  fields: InfoboxField[];
};

/** What a page splits into: its card (or none) and the body with the block removed. */
export type ParsedInfobox = { box: Infobox | null; rest: string };

/// A card every member replicates is a shared surface, so it is bounded like the other shared
/// values (topics, livery, badges): a hostile page cannot render a mile-long sidebar.
const MAX_FIELDS = 60;
const MAX_LABEL_CHARS = 80;
const MAX_VALUE_CHARS = 600;
const MAX_TITLE_CHARS = 120;

/** `{{Infobox` (optionally `{{Infobox person`), at the start of a line. */
const INFOBOX_OPEN = /^[ \t]*\{\{[ \t]*infobox\b/im;

/**
 * The offset just past the `}}` closing the template that opens at `from`, or -1 if it never
 * closes. Nested `{{…}}` are counted, so a value holding a template doesn't end the block early.
 */
function closeOf(src: string, from: number): number {
  let depth = 0;
  for (let i = from; i < src.length - 1; i++) {
    if (src[i] === "{" && src[i + 1] === "{") {
      depth++;
      i++;
    } else if (src[i] === "}" && src[i + 1] === "}") {
      depth--;
      i++;
      if (depth === 0) return i + 1;
    }
  }
  return -1;
}

/**
 * Split a template's interior on the `|` field separators, ignoring any `|` nested inside
 * `[[…]]`, `[…]`, `{{…}}` or `{…}`; so `[[Page|label]]` and a wikitext table in a value stay
 * whole. The first segment is the template name (`Infobox person`), never a field.
 */
function splitFields(body: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = "";
  for (let i = 0; i < body.length; i++) {
    const c = body[i];
    if (c === "[" || c === "{") depth++;
    else if (c === "]" || c === "}") depth = Math.max(0, depth - 1);
    if (c === "|" && depth === 0) {
      out.push(cur);
      cur = "";
      continue;
    }
    cur += c;
  }
  out.push(cur);
  return out;
}

/** Split `key = value` at the first `=` outside brackets, so `[[A|b=c]]` in a key can't split it. */
function splitField(seg: string): [string, string] {
  let depth = 0;
  for (let i = 0; i < seg.length; i++) {
    const c = seg[i];
    if (c === "[" || c === "{") depth++;
    else if (c === "]" || c === "}") depth = Math.max(0, depth - 1);
    else if (c === "=" && depth === 0) return [seg.slice(0, i), seg.slice(i + 1)];
  }
  return [seg, ""];
}

/** Collapse a field's interior whitespace to single spaces per line, and trim the whole. */
function tidy(s: string, max: number): string {
  const t = s
    .split("\n")
    .map((l) => l.trim())
    .join("\n")
    .replace(/\n{2,}/g, "\n")
    .trim();
  return t.length > max ? t.slice(0, max).trimEnd() : t;
}

/**
 * Pull the page's infobox out of `text`. Returns the parsed card (or `null` when the page has
 * none, or opens one it never closes) and the body with the block removed.
 */
export function extractInfobox(text: string): ParsedInfobox {
  const src = text ?? "";
  const open = INFOBOX_OPEN.exec(src);
  if (!open) return { box: null, rest: src };
  const start = src.indexOf("{{", open.index);
  const end = closeOf(src, start);
  // Unterminated: leave the text exactly as written, so a half-typed block stays visible to
  // the author instead of silently eating the rest of the page.
  if (end < 0) return { box: null, rest: src };

  const segments = splitFields(src.slice(start + 2, end - 2));
  const box: Infobox = { title: "", image: "", caption: "", fields: [] };
  for (const seg of segments.slice(1)) {
    if (box.fields.length >= MAX_FIELDS) break;
    const [rawKey, rawValue] = splitField(seg);
    const key = tidy(rawKey, MAX_LABEL_CHARS);
    if (!key) continue;
    const value = tidy(rawValue, MAX_VALUE_CHARS);
    switch (key.toLowerCase()) {
      case "title":
        box.title = tidy(value, MAX_TITLE_CHARS);
        break;
      case "image":
        // Only the embed marker: a bare content address would not be seen by the never-decay
        // scan, so the picture could expire out of circulation while the card still asked for it.
        box.image = /^!\[[^\]\n]*\]\(cid:[0-9a-fA-F]{1,64}\)$/.test(value) ? value : "";
        break;
      case "caption":
        box.caption = tidy(value, MAX_TITLE_CHARS);
        break;
      default:
        box.fields.push({ label: key, value });
    }
  }
  // A block with nothing in it is not a card; keep the page as the author wrote it.
  if (!box.title && !box.image && !box.caption && box.fields.length === 0) {
    return { box: null, rest: src };
  }
  // Drop the block's WHOLE lines (its indent, and the newline that ended it), so removing it
  // leaves no seam: the body reads as if the author had written the prose alone.
  const lineStart = open.index;
  const afterLine = src[end] === "\n" ? end + 1 : end;
  return { box, rest: src.slice(0, lineStart) + src.slice(afterLine) };
}

/**
 * Render a parsed card to HTML. `inline` is the page's own inline renderer (markdown or
 * wikitext), so a value's `[[links]]`, `:emoji:` and `'''bold'''` read exactly as they would in
 * the body; a multi-line value renders one line per row, joined like the body's soft breaks.
 *
 * The markup is a `<table>` on purpose: the sanitizer already allows table structure, so the
 * card needs no new tag or capability to exist.
 */
export function infoboxHtml(box: Infobox, inline: (s: string) => string): string {
  const lines = (s: string) =>
    s
      .split("\n")
      .map((l) => inline(l))
      .join("<br>");
  let html = '<table class="wiki-infobox">';
  if (box.title) html += `<caption>${inline(box.title)}</caption>`;
  html += "<tbody>";
  if (box.image) {
    html += `<tr class="ib-media"><td colspan="2">${inline(box.image)}`;
    if (box.caption) html += `<span class="ib-caption">${lines(box.caption)}</span>`;
    html += "</td></tr>";
  } else if (box.caption) {
    html += `<tr class="ib-media"><td colspan="2"><span class="ib-caption">${lines(box.caption)}</span></td></tr>`;
  }
  for (const f of box.fields) {
    if (!f.value) {
      html += `<tr class="ib-section"><th colspan="2">${inline(f.label)}</th></tr>`;
      continue;
    }
    html += `<tr><th>${inline(f.label)}</th><td>${lines(f.value)}</td></tr>`;
  }
  return html + "</tbody></table>";
}

/**
 * The skeleton the editor's toolbar button inserts. Written with the same field spacing the
 * examples use, so a page's blocks all line up however they were started.
 */
export function infoboxTemplate(pageName: string): string {
  return [
    "{{Infobox",
    `| title   = ${pageName}`,
    "| image   = ",
    "| caption = ",
    "| Label   = value",
    "}}",
    "",
  ].join("\n");
}
