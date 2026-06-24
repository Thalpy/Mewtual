// Phase 10b — the shared rich-text renderer.
//
// Messages, statuses and wiki pages come from other (untrusted) group members, so rendering
// is markdown via `marked` followed by a strict `DOMPurify` sanitize. Three custom inline
// tokens layer on top:
//   - `[[Page]]`     → a wiki link the app navigates on click (resolved in 10d)
//   - `:name:`       → a custom emoji (resolved to an image in 10f; shows `:name:` until then)
//   - `![alt](cid:HEX)` → a fileshare embed (image/video/audio, resolved in 10c)
//
// SECURITY: the sanitizer does NOT allow <img>/<video>/<audio>/<script>/raw HTML. Custom
// emoji and embeds render as inert <span> placeholders; the resolver (resolveMedia, wired in
// 10c/10f) replaces those placeholders with media elements it builds in code from the group's
// own content-addressed blobs — so untrusted text can never inject a live tag or remote URL.

import { marked, type TokenizerAndRendererExtension } from "marked";
import DOMPurify from "dompurify";

function escAttr(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
function escText(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// `[[Page Name]]` → a clickable wiki link (navigation handled by the app via data-wikilink).
const wikiLink: TokenizerAndRendererExtension = {
  name: "wikilink",
  level: "inline",
  start(src) {
    const i = src.indexOf("[[");
    return i < 0 ? undefined : i;
  },
  tokenizer(src) {
    const m = /^\[\[([^\]\n]{1,120})\]\]/.exec(src);
    if (m) return { type: "wikilink", raw: m[0], text: m[1].trim() };
    return undefined;
  },
  renderer(token) {
    return `<a class="wikilink" data-wikilink="${escAttr(token.text)}">${escText(token.text)}</a>`;
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
    const m = /^:([a-z0-9_+\-]{1,40}):/i.exec(src);
    if (m) return { type: "emoji", raw: m[0], text: m[1] };
    return undefined;
  },
  renderer(token) {
    return `<span class="emoji" data-emoji="${escAttr(token.text)}">:${escText(token.text)}:</span>`;
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
    const m = /^@\[([^\]\n]{1,40})\]/.exec(src);
    if (m) return { type: "mention", raw: m[0], text: m[1].trim() };
    return undefined;
  },
  renderer(token) {
    const me = selfMentionName && token.text === selfMentionName ? " mention-me" : "";
    return `<span class="mention${me}" data-mention="${escAttr(token.text)}">@${escText(token.text)}</span>`;
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
    const m = /^!\[([^\]]*)\]\(cid:([0-9a-fA-F]{1,64})\)/.exec(src);
    if (m) return { type: "embed", raw: m[0], alt: m[1], cid: m[2].toLowerCase() };
    return undefined;
  },
  renderer(token) {
    return `<span class="embed" data-embed-cid="${escAttr(token.cid)}" data-alt="${escAttr(token.alt)}"></span>`;
  },
};

let configured = false;
function configure() {
  if (configured) return;
  marked.use({ extensions: [wikiLink, emoji, mention, embed], breaks: true, gfm: true });
  configured = true;
}

// No media/script/raw-HTML tags: emoji + embeds are <span> placeholders the resolver fills.
const SANITIZE = {
  ALLOWED_TAGS: [
    "a", "b", "strong", "i", "em", "u", "s", "del", "code", "pre", "span", "br", "p",
    "ul", "ol", "li", "blockquote", "hr", "h1", "h2", "h3", "h4",
    "table", "thead", "tbody", "tr", "th", "td",
  ],
  ALLOWED_ATTR: [
    "class", "href", "title", "data-wikilink", "data-emoji", "data-embed-cid", "data-alt", "data-mention",
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

/** Render a full wiki page: block markdown + the custom tokens, sanitized. */
export function renderWiki(text: string): string {
  configure();
  selfMentionName = ""; // no "mention-me" self-highlight outside chat/status
  return DOMPurify.sanitize(marked.parse(text ?? "") as string, SANITIZE) as string;
}
