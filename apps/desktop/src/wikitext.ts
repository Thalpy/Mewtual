// Phase 10g; the wikitext layer: a MediaWiki-subset converter plus the shared token grammar.
//
// This is the LOWER half of the rich-text renderer. `render.ts` (marked + DOMPurify) imports the
// grammar regexes and the token renderers from here, so the markdown path and the wikitext path
// emit byte-identical HTML for the app's own tokens; `[[Page|label]]`, `:emoji:`, `![alt](cid:…)`,
// `[label](file|status|event:ID)`, `@[Name]`. One definition is what stops the two paths drifting
// into two subtly different surfaces for the sanitizer to police.
//
// Everything here is PURE; no DOM, no DOMPurify, no marked; so it unit-tests under plain Node.
//
// SECURITY: this is a converter, not a sanitizer. It emits an HTML *string* that `render.ts` always
// passes through DOMPurify before it reaches the page. It nevertheless holds the same line itself:
// every piece of member-authored text is interpolated through `escText`/`escAttr`, the only tags it
// can emit are the sanitizer's allow-list, the only `href` it can emit is an http(s) URL, and media
// (`![alt](cid:…)`) renders as the same inert `<span>` placeholder the resolver fills in from the
// group's own content-addressed blobs. Sanitize is the second line of defence here, not the first.

// --- escaping -----------------------------------------------------------------------------------

import { TEXT_EFFECT_RE, parseTextEffect, stripTextEffects, textEffectHtml } from "./message-effects.ts";

/** Escape for use inside a double-quoted attribute value. */
export function escAttr(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** Escape for use as text content. */
export function escText(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// --- the shared token grammar -------------------------------------------------------------------
// `^`-anchored because marked's tokenizers are handed the remaining source; the wikitext scanner
// below derives sticky copies (see `sticky`) so it can match at an offset without slicing.

/** `![alt](cid:HEX)`; a fileshare embed. */
export const EMBED_RE = /^!\[([^\]]*)\]\(cid:([0-9a-fA-F]{1,64})\)/;

/** `[label](file|status|event:ID)`; an in-app reference chip. */
export const REF_LINK_RE = /^\[([^\]\n]{1,160})\]\((file|status|event):([0-9a-zA-Z_-]{1,64})\)/;

/**
 * `[[Page]]` / `[[Page|label]]`; a wiki link. The page never contains `|` so the split is
 * unambiguous; the label is optional and falls back to the page name.
 */
export const WIKI_LINK_RE = /^\[\[([^\]|\n]{1,120})(?:\|([^\]\n]{1,120}))?\]\]/;

/** `:name:`; a custom emoji. */
export const EMOJI_RE = /^:([a-z0-9_+\-]{1,40}):/i;

/** `@[Name]`; a member mention. */
export const MENTION_RE = /^@\[([^\]\n]{1,40})\]/;

// --- the shared token renderers -------------------------------------------------------------------
// Called by BOTH `render.ts`'s marked extensions and the wikitext scanner below, so the two paths
// cannot produce different markup for the same token.

/** `[[Page|label]]` → the link the app navigates on click (it reads `data-wikilink`). */
export function wikiLinkHtml(page: string, label?: string): string {
  const text = (label ?? "").trim() || page;
  return `<a class="wikilink" data-wikilink="${escAttr(page)}">${escText(text)}</a>`;
}

/** `:name:` → a placeholder showing the literal `:name:` until the resolver swaps in the image. */
export function emojiHtml(name: string): string {
  return `<span class="emoji" data-emoji="${escAttr(name)}">:${escText(name)}:</span>`;
}

/** `![alt](cid:HEX)` → an inert placeholder the media resolver fills from the verified blob. */
export function embedHtml(alt: string, cid: string): string {
  return `<span class="embed" data-embed-cid="${escAttr(cid.toLowerCase())}" data-alt="${escAttr(alt)}"></span>`;
}

/** `[label](file|status|event:ID)` → a reference chip; the app resolves the target from the data attr. */
export function refLinkHtml(kind: string, ref: string, text: string): string {
  const file = kind === "file";
  const attr = file ? "data-file-cid" : kind === "status" ? "data-status-id" : "data-event-id";
  const id = file ? ref.toLowerCase() : ref;
  const icon = file ? "📄" : kind === "status" ? "◈" : "⧗";
  return `<a class="reflink ${escAttr(kind)}-ref" ${attr}="${escAttr(id)}"><span class="reflink-ico" aria-hidden="true">${icon}</span>${escText(text)}</a>`;
}

/** `@[Name]` → a mention chip; `me` (the local display name) adds the self-highlight in chat only. */
export function mentionHtml(name: string, me = ""): string {
  const self = me && name === me ? " mention-me" : "";
  return `<span class="mention${self}" data-mention="${escAttr(name)}">@${escText(name)}</span>`;
}

/** `[https://example.com label]` → an external link. Callers must have matched an http(s) URL. */
export function extLinkHtml(url: string, label?: string): string {
  return `<a href="${escAttr(url)}">${escText((label ?? "").trim() || url)}</a>`;
}

// --- magic words & directives ---------------------------------------------------------------------

const REDIRECT_RE = /^\s*#redirect\s*\[\[[ \t]*([^\]|\n]{1,120}?)[ \t]*(?:\|[^\]\n]{0,120})?\]\]/i;
// Trailing horizontal space goes with the word, so a line led by one doesn't become an indented
// `<pre>` block once it's gone.
const MAGIC_RE = /__(?:NOTOC|TOC)__[ \t]*/g;

/**
 * The target of a `#REDIRECT [[Target]]` page, or null. Same syntax in both page formats; the
 * marker must lead the page (only whitespace may precede it). A piped label is ignored.
 */
export function parseRedirect(text: string): string | null {
  const m = REDIRECT_RE.exec(text ?? "");
  const target = m ? m[1].trim() : "";
  return target || null;
}

/** `__NOTOC__` / `__TOC__`; whether the page forces or suppresses its contents box. NOTOC wins. */
export function tocDirective(text: string): "notoc" | "force" | null {
  const t = text ?? "";
  if (t.includes("__NOTOC__")) return "notoc";
  if (t.includes("__TOC__")) return "force";
  return null;
}

/** Remove the TOC magic words: they are directives, not content, in both page formats. */
export function stripMagicWords(text: string): string {
  return (text ?? "").replace(MAGIC_RE, "");
}

/**
 * A one-line plain-text summary of a page/post body: the text a reader would see, with the markup
 * of *both* formats taken out (they share this one path because a link card never knows which
 * format the body it is previewing was written in).
 *
 * Deliberately a stripper and not a parser: a card preview only ever needs the prose, so an
 * unrecognized construct degrades to "the marks show through" rather than to broken output. Image
 * embeds drop out entirely; a link keeps its label, which IS the prose the author wrote.
 */
export function plainSummary(text: string, max = 180): string {
  let t = stripTextEffects(stripMagicWords(text ?? ""));
  t = t.replace(/<!--[\s\S]*?-->/g, " "); // html comments
  t = t.replace(/```[\s\S]*?(?:```|$)/g, " "); // fenced code, closed or running to the end
  t = t.replace(/^\s*#redirect\s*\[\[[^\]\n]*\]\]/i, " "); // a redirect marker is not prose
  t = t.replace(/!\[[^\]\n]*\]\([^)\n]*\)/g, " "); // an embed: the picture carries the meaning
  t = t.replace(/\[([^\]\n]*)\]\([^)\n]*\)/g, "$1"); // [label](target) keeps its label
  t = t.replace(/\[\[[ \t]*([^\]|\n]*?)[ \t]*(?:\|([^\]\n]*))?\]\]/g, (_m, page, label) =>
    (label ?? "").trim() || page,
  );
  t = t.replace(/\{\{[\s\S]*?\}\}/g, " "); // templates, including a multi-line {{Infobox …}} card
  t = t.replace(/^[ \t]*\|.*$/gm, " "); // table rows and their markup
  t = t.replace(/^[ \t]*[-=_]{3,}[ \t]*$/gm, " "); // horizontal rules / setext underlines
  t = t.replace(/^[ \t]*=+[ \t]*([^=\n]+?)[ \t]*=+[ \t]*$/gm, "$1"); // wikitext headings
  t = t.replace(/^[ \t]*#{1,6}[ \t]+/gm, ""); // markdown headings
  t = t.replace(/^[ \t]*(?:[*#:;>-]+|\d+[.)])[ \t]+/gm, ""); // bullets, indents, quotes
  t = t.replace(/(\*\*|__|~~|'{2,5}|`)/g, ""); // emphasis + inline code marks
  t = t.replace(/\s+/g, " ").trim();
  return t.length > max ? t.slice(0, max).trimEnd() + "\u2026" : t;
}

// --- the inline pass --------------------------------------------------------------------------------

/** A sticky (offset-matching) copy of an `^`-anchored grammar regex. */
function sticky(re: RegExp): RegExp {
  return new RegExp(re.source.replace(/^\^/, ""), re.flags.replace(/[gy]/g, "") + "y");
}

const S_NOWIKI = /<nowiki>([\s\S]*?)<\/nowiki>/y;
const S_TEXT_EFFECT = sticky(TEXT_EFFECT_RE);
const S_EMBED = sticky(EMBED_RE);
const S_WIKI = sticky(WIKI_LINK_RE);
const S_REF = sticky(REF_LINK_RE);
const S_EMOJI = sticky(EMOJI_RE);
const S_MENTION = sticky(MENTION_RE);
// Only http/https ever reaches an href. Anything else falls through and is escaped as plain text.
const S_EXT = /\[(https?:\/\/[^\s\]<>"]{1,500})(?:[ \t]+([^\]\n]{1,200}))?\]/iy;
const S_BOLD_IT = /'''''([^\n]+?)'''''/y;
const S_BOLD = /'''([^\n]+?)'''/y;
const S_ITALIC = /''([^\n]+?)''/y;

function at(re: RegExp, src: string, i: number): RegExpExecArray | null {
  re.lastIndex = i;
  return re.exec(src);
}

/**
 * Wikitext inline markup → HTML. Plain runs are buffered and escaped wholesale; every recognised
 * token is emitted through one of the shared renderers above. Anything unrecognised (`{{template}}`,
 * a stray bracket, raw HTML) stays literal escaped text; the converter has no passthrough.
 */
export function inlineToHtml(src: string): string {
  let out = "";
  let plain = "";
  let i = 0;
  const flush = () => {
    if (plain) {
      out += escText(plain);
      plain = "";
    }
  };
  while (i < src.length) {
    const c = src[i];
    let m: RegExpExecArray | null = null;
    if (c === "<") {
      // `<nowiki>…</nowiki>`: the contents are literal, with no wiki parsing inside.
      m = at(S_NOWIKI, src, i);
      if (m) {
        flush();
        out += escText(m[1]);
        i += m[0].length;
        continue;
      }
    } else if (c === "!") {
      m = at(S_EMBED, src, i);
      if (m) {
        flush();
        out += embedHtml(m[1], m[2]);
        i += m[0].length;
        continue;
      }
    } else if (c === "[") {
      m = at(S_TEXT_EFFECT, src, i);
      if (m) {
        const effect = parseTextEffect(m[0]);
        if (effect) {
          flush();
          out += textEffectHtml(effect.id, effect.text);
          i += effect.raw.length;
          continue;
        }
      }
      m = at(S_WIKI, src, i);
      if (m && m[1].trim()) {
        flush();
        out += wikiLinkHtml(m[1].trim(), m[2]);
        i += m[0].length;
        continue;
      }
      m = at(S_REF, src, i);
      if (m) {
        flush();
        out += refLinkHtml(m[2], m[3], m[1].trim());
        i += m[0].length;
        continue;
      }
      m = at(S_EXT, src, i);
      if (m) {
        flush();
        out += extLinkHtml(m[1], m[2]);
        i += m[0].length;
        continue;
      }
    } else if (c === "@") {
      m = at(S_MENTION, src, i);
      if (m) {
        flush();
        out += mentionHtml(m[1].trim());
        i += m[0].length;
        continue;
      }
    } else if (c === ":") {
      m = at(S_EMOJI, src, i);
      if (m) {
        flush();
        out += emojiHtml(m[1]);
        i += m[0].length;
        continue;
      }
    } else if (c === "'") {
      m = at(S_BOLD_IT, src, i);
      if (m) {
        flush();
        out += `<strong><em>${inlineToHtml(m[1])}</em></strong>`;
        i += m[0].length;
        continue;
      }
      m = at(S_BOLD, src, i);
      if (m) {
        flush();
        out += `<strong>${inlineToHtml(m[1])}</strong>`;
        i += m[0].length;
        continue;
      }
      m = at(S_ITALIC, src, i);
      if (m) {
        flush();
        out += `<em>${inlineToHtml(m[1])}</em>`;
        i += m[0].length;
        continue;
      }
    }
    plain += c;
    i++;
  }
  flush();
  return out;
}

/** Inline-render a run of lines, joined the way the markdown path's `breaks: true` joins them. */
function inlineLines(s: string): string {
  return s
    .split("\n")
    .map((l) => inlineToHtml(l.trim()))
    .join("<br>");
}

// --- the block pass ---------------------------------------------------------------------------------

const RE_BLANK = /^[ \t]*$/;
const RE_HR = /^-{4,}[ \t]*$/;
const RE_HEADING = /^(=+)[ \t]*(.*?)[ \t]*=*[ \t]*$/;
const RE_LIST = /^([*#]+)[ \t]?(.*)$/;
const RE_INDENT = /^(?:;|:+)[ \t]?(.*)$/;
const RE_PRE = /^ /;
const RE_TABLE = /^[ \t]*\{\|/;

/**
 * A leading `:` is MediaWiki indentation; except when it opens one of the app's own `:emoji:`
 * tokens, which is far more likely at the start of a line here than a one-level indent is.
 */
function isIndent(line: string): boolean {
  return line[0] === ":" && !EMOJI_RE.test(line);
}

/** Does this line open a block, i.e. end the paragraph being accumulated? */
function startsBlock(line: string): boolean {
  return (
    RE_BLANK.test(line) ||
    RE_HR.test(line) ||
    RE_TABLE.test(line) ||
    RE_PRE.test(line) ||
    /^[=*#;]/.test(line) ||
    isIndent(line)
  );
}

function listTag(marker: string): "ul" | "ol" {
  return marker === "#" ? "ol" : "ul";
}

/**
 * `*`/`#` runs, nested by marker repetition (`**`, `##`, `*#`). Levels open lazily and close on the
 * longest common prefix with the previous item, so a sub-list always lands inside its parent `<li>`.
 */
function renderList(items: { marker: string; text: string }[]): string {
  let html = "";
  const open: string[] = [];
  for (const item of items) {
    const mk = item.marker;
    let common = 0;
    while (common < open.length && common < mk.length && open[common] === mk[common]) common++;
    while (open.length > common) html += `</li></${listTag(open.pop() as string)}>`;
    if (open.length === mk.length) {
      html += "</li><li>";
    } else {
      for (let level = open.length; level < mk.length; level++) {
        open.push(mk[level]);
        html += `<${listTag(mk[level])}>`;
        // A deeper list with no item of its own at this level still needs an <li> to live in.
        if (level < mk.length - 1) html += "<li>";
      }
      html += "<li>";
    }
    html += inlineToHtml(item.text.trim());
  }
  while (open.length) html += `</li></${listTag(open.pop() as string)}>`;
  return html;
}

/**
 * Find the `:` that separates `; term : definition`, skipping any inside `[[…]]`, `[…]` or `{{…}}`
 * so a piped wiki link or a URL in the term doesn't split it.
 */
function splitTerm(s: string): [string, string | null] {
  let depth = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === "[" || c === "{") depth++;
    else if (c === "]" || c === "}") depth = Math.max(0, depth - 1);
    else if (c === ":" && depth === 0) return [s.slice(0, i), s.slice(i + 1)];
  }
  return [s, null];
}

/** A `;`-led run: `; term`, `: definition`, or `; term : definition` on one line. */
function renderDefList(lines: string[]): string {
  let html = "<dl>";
  for (const line of lines) {
    const m = RE_INDENT.exec(line);
    const body = m ? m[1] : line;
    if (line[0] === ";") {
      const [term, def] = splitTerm(body);
      html += `<dt>${inlineToHtml(term.trim())}</dt>`;
      if (def !== null) html += `<dd>${inlineToHtml(def.trim())}</dd>`;
    } else {
      html += `<dd>${inlineToHtml(body.trim())}</dd>`;
    }
  }
  return html + "</dl>";
}

/** A bare `:` indent run → nested blockquotes, one level per leading colon. */
function renderIndent(lines: string[]): string {
  let html = "";
  let depth = 0;
  let buf: string[] = [];
  const flush = () => {
    if (buf.length) {
      html += `<p>${buf.join("<br>")}</p>`;
      buf = [];
    }
  };
  for (const line of lines) {
    const m = /^(:+)[ \t]?(.*)$/.exec(line);
    const want = m ? m[1].length : 1;
    if (want !== depth) {
      flush();
      while (depth < want) {
        html += "<blockquote>";
        depth++;
      }
      while (depth > want) {
        html += "</blockquote>";
        depth--;
      }
    }
    buf.push(inlineToHtml((m ? m[2] : line).trim()));
  }
  flush();
  while (depth > 0) {
    html += "</blockquote>";
    depth--;
  }
  return html;
}

/**
 * Drop a cell's attribute string (`| style="…" | content`). Only a prefix that looks like
 * attributes; has an `=`, no brackets; is dropped, so `| [[Page|label]]` keeps its whole content.
 */
function cellBody(s: string): string {
  const p = s.indexOf("|");
  if (p >= 0 && /^[^[\]{}<>]*=[^[\]{}<>]*$/.test(s.slice(0, p))) return s.slice(p + 1);
  return s;
}

type Cell = { header: boolean; text: string };
type Row = { cells: Cell[] };

function renderRow(r: Row): string {
  const cells = r.cells
    .map((c) => {
      const tag = c.header ? "th" : "td";
      return `<${tag}>${inlineLines(c.text.trim())}</${tag}>`;
    })
    .join("");
  return `<tr>${cells}</tr>`;
}

/**
 * `{|` … `|}`, with `|-` row separators, `!` header cells, `|` data cells, `||`/`!!` inline cell
 * separators and an optional `|+` caption. Attribute strings on the table, rows and cells are
 * dropped (they are presentation the app's own stylesheet owns). Nested tables are not supported.
 */
function parseTable(lines: string[], start: number): { html: string; next: number } {
  let i = start + 1; // the `{|` line carries only attributes
  let caption = "";
  const rows: Row[] = [];
  // The row currently accepting cells, opened lazily by the first cell after a `|-` (so a stray
  // separator never yields an empty row). Assigned inline rather than through a helper: an
  // assignment inside a closure is invisible to the type checker's flow analysis.
  let current: Row | null = null;

  for (; i < lines.length; i++) {
    const line = lines[i];
    if (/^[ \t]*\|\}/.test(line)) {
      i++;
      break;
    }
    if (/^[ \t]*\|\+/.test(line)) {
      caption = cellBody(line.replace(/^[ \t]*\|\+/, "")).trim();
      continue;
    }
    if (/^[ \t]*\|-/.test(line)) {
      current = null; // the next cell line opens the next row
      continue;
    }
    const head = /^[ \t]*!(.*)$/.exec(line);
    const data = head ? null : /^[ \t]*\|(.*)$/.exec(line);
    if (head || data) {
      if (!current) {
        current = { cells: [] };
        rows.push(current);
      }
      const header = head !== null;
      const parts = (head ?? data)![1].split(header ? "!!" : "||");
      for (const t of parts) current.cells.push({ header, text: cellBody(t) });
      continue;
    }
    // A continuation line of the cell above.
    if (current && current.cells.length) {
      current.cells[current.cells.length - 1].text += "\n" + line;
    }
  }

  const parts = ["<table>"];
  if (caption) parts.push(`<caption>${inlineToHtml(caption)}</caption>`);
  let body = rows;
  if (rows.length && rows[0].cells.length && rows[0].cells.every((c) => c.header)) {
    parts.push(`<thead>${renderRow(rows[0])}</thead>`);
    body = rows.slice(1);
  }
  body = body.filter((r) => r.cells.length > 0);
  if (body.length) parts.push(`<tbody>${body.map((r) => renderRow(r)).join("")}</tbody>`);
  parts.push("</table>");
  return { html: parts.join(""), next: i };
}

/**
 * A MediaWiki-subset wikitext page → an HTML string, ready for `render.ts`'s sanitize step.
 *
 * Supported: `==` headings (clamped to h2–h4), `*`/`#` lists with nesting, `;`/`:` definition lists,
 * `:` indent blockquotes, `----` rules, `{|` tables, space-indented `<pre>`, `<nowiki>`, paragraphs
 * with `<br>` line breaks, `'''bold'''`/`''italic''`, `[[Page|label]]`, `[https://… label]`, and the
 * app's own tokens. Not supported (and rendered as literal text): templates, `~~~~`, HTML passthrough.
 */
export function wikitextToHtml(src: string): string {
  const lines = stripMagicWords(String(src ?? "").replace(/\r\n?/g, "\n")).split("\n");
  const out: string[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (RE_BLANK.test(line)) {
      i++;
      continue;
    }

    if (RE_TABLE.test(line)) {
      const t = parseTable(lines, i);
      out.push(t.html);
      i = t.next;
      continue;
    }

    if (RE_HR.test(line)) {
      out.push("<hr>");
      i++;
      continue;
    }

    const head = RE_HEADING.exec(line);
    if (head && head[2].trim()) {
      // `=` is the page title's level in MediaWiki, so the body starts at h2; 5+ clamps to h4
      // because the sanitizer allows no h5/h6.
      const level = Math.min(4, Math.max(2, head[1].length));
      out.push(`<h${level}>${inlineToHtml(head[2].trim())}</h${level}>`);
      i++;
      continue;
    }

    if (RE_PRE.test(line)) {
      const buf: string[] = [];
      while (i < lines.length && RE_PRE.test(lines[i])) {
        buf.push(lines[i].slice(1));
        i++;
      }
      out.push(`<pre>${escText(buf.join("\n"))}</pre>`);
      continue;
    }

    if (/^[*#]/.test(line)) {
      const items: { marker: string; text: string }[] = [];
      while (i < lines.length) {
        const m = RE_LIST.exec(lines[i]);
        if (!m) break;
        items.push({ marker: m[1], text: m[2] });
        i++;
      }
      out.push(renderList(items));
      continue;
    }

    if (line[0] === ";") {
      const buf: string[] = [];
      while (i < lines.length && (lines[i][0] === ";" || isIndent(lines[i]))) {
        buf.push(lines[i]);
        i++;
      }
      out.push(renderDefList(buf));
      continue;
    }

    if (isIndent(line)) {
      const buf: string[] = [];
      while (i < lines.length && isIndent(lines[i])) {
        buf.push(lines[i]);
        i++;
      }
      out.push(renderIndent(buf));
      continue;
    }

    // A paragraph: the first line always joins it (which guarantees progress even for a line that
    // looks block-ish but produced no block, e.g. a bare `====`), then up to the next block.
    const buf = [inlineToHtml(line)];
    i++;
    while (i < lines.length && !startsBlock(lines[i])) {
      buf.push(inlineToHtml(lines[i]));
      i++;
    }
    out.push(`<p>${buf.join("<br>")}</p>`);
  }

  return out.join("");
}
