// Phase 10b; the shared rich-text renderer.
//
// Messages, statuses and wiki pages come from other (untrusted) group members, so rendering
// is markdown via `marked` followed by a strict `DOMPurify` sanitize. Custom inline tokens
// layer on top:
//   - `[[Page]]` / `[[Page|label]]` → a wiki link the app navigates on click (resolved in 10d)
//   - `:name:`       → a custom emoji (resolved to an image in 10f; shows `:name:` until then)
//   - `![alt](cid:HEX)` → a fileshare embed (image/video/audio, resolved in 10c)
//   - `[label](file:HEX)` / `[label](status:ID)` → an in-app reference the composer's "+" picker
//     inserts: a fileshare file (opens its info pane) or one of this server's status posts.
//
// A wiki page can also be authored in MediaWiki wikitext instead of markdown (`format === "wiki"`).
// That path swaps `marked` for `wikitext.ts`'s converter and keeps the identical sanitize step;
// see `wikitext.ts`, which also owns the token grammar and the token renderers BOTH paths use, so
// the two can't drift into two different surfaces for the sanitizer to police.
//
// SECURITY: the sanitizer does NOT allow <img>/<video>/<audio>/<script>/raw HTML. Custom
// emoji and embeds render as inert <span> placeholders; the resolver (resolveMedia, wired in
// 10c/10f) replaces those placeholders with media elements it builds in code from the group's
// own content-addressed blobs; so untrusted text can never inject a live tag or remote URL.

import { marked, type TokenizerAndRendererExtension } from "marked";
import DOMPurify from "dompurify";

import {
  EMBED_RE,
  EMOJI_RE,
  MENTION_RE,
  REF_LINK_RE,
  WIKI_LINK_RE,
  embedHtml,
  emojiHtml,
  escText,
  mentionHtml,
  refLinkHtml,
  stripMagicWords,
  wikiLinkHtml,
  wikitextToHtml,
} from "./wikitext.ts";

// The token grammar lives in `wikitext.ts` (both renderers need it) but is re-exported here: this
// is the module `refs.ts`'s round-trip tests pin the composer's markers against.
export { EMBED_RE, REF_LINK_RE, WIKI_LINK_RE };
export { parseRedirect, tocDirective } from "./wikitext.ts";

// `[[Page Name]]` / `[[Page Name|label]]` → a clickable wiki link (navigation handled by the app
// via data-wikilink, which always carries the page name, never the label).
const wikiLink: TokenizerAndRendererExtension = {
  name: "wikilink",
  level: "inline",
  start(src) {
    const i = src.indexOf("[[");
    return i < 0 ? undefined : i;
  },
  tokenizer(src) {
    const m = WIKI_LINK_RE.exec(src);
    if (!m) return undefined;
    const page = m[1].trim();
    if (!page) return undefined; // `[[ |label]]` has no target: leave it as literal text
    return { type: "wikilink", raw: m[0], page, text: (m[2] ?? "").trim() || page };
  },
  renderer(token) {
    return wikiLinkHtml(token.page, token.text);
  },
};

// `:name:` → a custom-emoji placeholder. Shows the literal `:name:` until the resolver swaps
// in the image (10f), so an unknown emoji just reads as text.
const emoji: TokenizerAndRendererExtension = {
  name: "emoji",
  level: "inline",
  start(src) {
    const i = src.indexOf(":");
    return i < 0 ? undefined : i;
  },
  tokenizer(src) {
    const m = EMOJI_RE.exec(src);
    if (m) return { type: "emoji", raw: m[0], text: m[1] };
    return undefined;
  },
  renderer(token) {
    return emojiHtml(token.text);
  },
};

// `@[Name]` → a member mention. The bracket form is self-contained (handles names with spaces and
// needs no member list in the renderer); the composer's @-autocomplete inserts it. Renders as a
// highlighted `@Name` chip, with an extra `mention-me` class when it names the local member.
let selfMentionName = "";
const mention: TokenizerAndRendererExtension = {
  name: "mention",
  level: "inline",
  start(src) {
    const i = src.indexOf("@[");
    return i < 0 ? undefined : i;
  },
  tokenizer(src) {
    const m = MENTION_RE.exec(src);
    if (m) return { type: "mention", raw: m[0], text: m[1].trim() };
    return undefined;
  },
  renderer(token) {
    return mentionHtml(token.text, selfMentionName);
  },
};

// `||text||` → a spoiler: rendered blurred/blacked-out until clicked (the app toggles a `revealed`
// class via the data-spoiler hook). The content is escaped plain text (no nested formatting).
// Markdown-path only: in wikitext `||` is the table cell separator.
const spoiler: TokenizerAndRendererExtension = {
  name: "spoiler",
  level: "inline",
  start(src) {
    const i = src.indexOf("||");
    return i < 0 ? undefined : i;
  },
  tokenizer(src) {
    const m = /^\|\|([^\n]+?)\|\|/.exec(src);
    if (m) return { type: "spoiler", raw: m[0], text: m[1] };
    return undefined;
  },
  renderer(token) {
    return `<span class="spoiler" data-spoiler tabindex="0" role="button">${escText(token.text)}</span>`;
  },
};

// `![alt](cid:HEX)` → a fileshare embed placeholder (resolved to media in 10c). Matched ahead
// of marked's own image syntax; plain `![](http…)` falls through and is stripped by sanitize.
const embed: TokenizerAndRendererExtension = {
  name: "embed",
  level: "inline",
  start(src) {
    const i = src.indexOf("![");
    return i < 0 ? undefined : i;
  },
  tokenizer(src) {
    const m = EMBED_RE.exec(src);
    if (m) return { type: "embed", raw: m[0], alt: m[1], cid: m[2] };
    return undefined;
  },
  renderer(token) {
    return embedHtml(token.alt, token.cid);
  },
};

// `[label](file:HEX)` / `[label](status:ID)` → an in-app reference chip (inserted by the composer's
// "+" picker). Matched ahead of marked's own link syntax so these app-only schemes never reach an
// `<a href>`; the app resolves the target from the data- attribute instead, so a reference can
// only ever address this group's own content. `![alt](cid:…)` is unaffected: the embed extension
// starts at the `!` and consumes the whole thing, and `cid` isn't in this alternation anyway.
const refLink: TokenizerAndRendererExtension = {
  name: "reflink",
  level: "inline",
  start(src) {
    const i = src.indexOf("[");
    return i < 0 ? undefined : i;
  },
  tokenizer(src) {
    const m = REF_LINK_RE.exec(src);
    if (m) return { type: "reflink", raw: m[0], kind: m[2], ref: m[3], text: m[1].trim() };
    return undefined;
  },
  renderer(token) {
    return refLinkHtml(token.kind, token.ref, token.text);
  },
};

let configured = false;
function configure() {
  if (configured) return;
  marked.use({ extensions: [wikiLink, emoji, mention, spoiler, embed, refLink], breaks: true, gfm: true });
  configured = true;
}

// No media/script/raw-HTML tags: emoji + embeds are <span> placeholders the resolver fills.
// h5/h6 stay out (the wikitext converter clamps deep headings to h4); <dl>/<dt>/<dd> and <caption>
// are here for wikitext's definition lists and table captions; inert structure, no new capability.
const SANITIZE = {
  ALLOWED_TAGS: [
    "a", "b", "strong", "i", "em", "u", "s", "del", "code", "pre", "span", "br", "p",
    "ul", "ol", "li", "dl", "dt", "dd", "blockquote", "hr", "h1", "h2", "h3", "h4",
    "table", "caption", "thead", "tbody", "tr", "th", "td",
  ],
  ALLOWED_ATTR: [
    "class", "href", "title", "data-wikilink", "data-emoji", "data-embed-cid", "data-alt",
    "data-mention", "data-spoiler", "data-file-cid", "data-status-id", "data-event-id",
    "tabindex", "role", "aria-hidden",
  ],
};

/**
 * Render a chat/status message: inline markdown + the custom tokens, sanitized. Pass `me` (the
 * local member's display name) so a mention of yourself gets the extra `mention-me` highlight.
 */
export function renderMessage(text: string, me = ""): string {
  configure();
  selfMentionName = me;
  return DOMPurify.sanitize(marked.parseInline(text ?? "") as string, SANITIZE) as string;
}

/**
 * Render a full wiki page, sanitized. `format` is the page's stored authoring format: `"wiki"` for
 * MediaWiki wikitext, anything else (including omitted) for the markdown default.
 */
export function renderWiki(text: string, format?: string): string {
  configure();
  selfMentionName = ""; // no "mention-me" self-highlight outside chat/status
  const src = text ?? "";
  // __TOC__/__NOTOC__ are directives the page chrome reads (tocDirective), not content, in both.
  const html = format === "wiki" ? wikitextToHtml(src) : (marked.parse(stripMagicWords(src)) as string);
  return DOMPurify.sanitize(html, SANITIZE) as string;
}
