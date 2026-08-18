// Unit tests for the MediaWiki-subset wikitext converter.
//
// Run with `npm test` (Node's built-in runner + type stripping; no extra dependencies).
//
// Two things are worth pinning here. The first is the SYNTAX: wikitext is a format members type by
// hand, so every construct the editor's help panel advertises needs a test that says what it turns
// into. The second, and the reason this file byte-compares rather than pattern-matches, is the
// SANITIZER CONTRACT: `render.ts` hands this converter's output to DOMPurify with a fixed allow-list
// and then the app resolves `data-` attributes into media. Emitting a tag outside that list, or an
// attribute value that wasn't escaped, is the failure that turns member-authored text into markup;
// so the placeholder shapes are compared exactly, and the escaping cases are compared exactly.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  escAttr,
  escText,
  inlineToHtml,
  parseRedirect,
  plainSummary,
  stripMagicWords,
  tocDirective,
  wikitextToHtml,
  WIKI_LINK_RE,
} from "./wikitext.ts";

// --- headings -----------------------------------------------------------------------------------

test("== heading == is h2: `=` alone is the page title's level, which the body never uses", () => {
  assert.equal(wikitextToHtml("== Intro =="), "<h2>Intro</h2>");
  assert.equal(wikitextToHtml("= Intro ="), "<h2>Intro</h2>");
});

test("heading depth follows the `=` count", () => {
  assert.equal(wikitextToHtml("=== Sub ==="), "<h3>Sub</h3>");
  assert.equal(wikitextToHtml("==== Deep ===="), "<h4>Deep</h4>");
});

test("headings past h4 clamp: the sanitizer allows no h5/h6", () => {
  assert.equal(wikitextToHtml("===== Deeper ====="), "<h4>Deeper</h4>");
  assert.equal(wikitextToHtml("====== Deepest ======"), "<h4>Deepest</h4>");
});

test("the trailing `=`s are optional and the text is trimmed", () => {
  assert.equal(wikitextToHtml("==Intro"), "<h2>Intro</h2>");
  assert.equal(wikitextToHtml("==   Intro   =="), "<h2>Intro</h2>");
});

test("an empty heading is not a heading", () => {
  assert.equal(wikitextToHtml("===="), "<p>====</p>");
});

test("a heading may contain inline markup", () => {
  assert.equal(wikitextToHtml("== The ''good'' bit =="), "<h2>The <em>good</em> bit</h2>");
});

// --- bold / italic ------------------------------------------------------------------------------

test("apostrophe runs give italic, bold and the five-quote bold-italic", () => {
  assert.equal(wikitextToHtml("''i''"), "<p><em>i</em></p>");
  assert.equal(wikitextToHtml("'''b'''"), "<p><strong>b</strong></p>");
  assert.equal(wikitextToHtml("'''''bi'''''"), "<p><strong><em>bi</em></strong></p>");
});

test("italic nests inside bold", () => {
  assert.equal(wikitextToHtml("'''a ''b'' c'''"), "<p><strong>a <em>b</em> c</strong></p>");
});

test("a lone apostrophe is just an apostrophe", () => {
  assert.equal(wikitextToHtml("don't"), "<p>don't</p>");
});

test("an unclosed quote run stays literal", () => {
  assert.equal(wikitextToHtml("'''oops"), "<p>'''oops</p>");
});

// --- lists --------------------------------------------------------------------------------------

test("* and # build ul and ol", () => {
  assert.equal(wikitextToHtml("* a\n* b"), "<ul><li>a</li><li>b</li></ul>");
  assert.equal(wikitextToHtml("# a\n# b"), "<ol><li>a</li><li>b</li></ol>");
});

test("repetition nests, and the sub-list lands inside its parent item", () => {
  assert.equal(
    wikitextToHtml("* a\n** b\n* c"),
    "<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>",
  );
});

test("mixed markers nest a numbered list inside a bulleted one", () => {
  assert.equal(
    wikitextToHtml("* a\n*# one\n*# two"),
    "<ul><li>a<ol><li>one</li><li>two</li></ol></li></ul>",
  );
});

test("changing the marker at the same depth starts a new list", () => {
  assert.equal(wikitextToHtml("* a\n# b"), "<ul><li>a</li></ul><ol><li>b</li></ol>");
});

test("a list that opens at depth still produces valid nesting", () => {
  assert.equal(wikitextToHtml("** deep"), "<ul><li><ul><li>deep</li></ul></li></ul>");
});

test("a blank line separates two lists", () => {
  assert.equal(wikitextToHtml("* one\n\n* two"), "<ul><li>one</li></ul><ul><li>two</li></ul>");
});

// --- definition lists and indentation -------------------------------------------------------------

test("; term : definition builds a dl", () => {
  assert.equal(wikitextToHtml("; term : def"), "<dl><dt>term</dt><dd>def</dd></dl>");
});

test("a ; run absorbs the : lines that follow it", () => {
  assert.equal(
    wikitextToHtml("; term\n: one\n: two"),
    "<dl><dt>term</dt><dd>one</dd><dd>two</dd></dl>",
  );
});

test("the term/definition split ignores a `:` inside a piped link", () => {
  assert.equal(
    wikitextToHtml("; [[A:B|see]] : def"),
    '<dl><dt><a class="wikilink" data-wikilink="A:B">see</a></dt><dd>def</dd></dl>',
  );
});

test("a bare : run is a blockquote, nested one level per colon", () => {
  assert.equal(wikitextToHtml(": quoted"), "<blockquote><p>quoted</p></blockquote>");
  assert.equal(
    wikitextToHtml(":: deep\n: shallow"),
    "<blockquote><blockquote><p>deep</p></blockquote><p>shallow</p></blockquote>",
  );
});

test("a line opening with an :emoji: is text, not an indent", () => {
  assert.equal(
    wikitextToHtml(":tada: shipped"),
    '<p><span class="emoji" data-emoji="tada">:tada:</span> shipped</p>',
  );
});

// --- rules, paragraphs, pre -----------------------------------------------------------------------

test("four or more dashes alone is a horizontal rule", () => {
  assert.equal(wikitextToHtml("----"), "<hr>");
  assert.equal(wikitextToHtml("--------"), "<hr>");
  assert.equal(wikitextToHtml("---"), "<p>---</p>");
});

test("blank lines separate paragraphs and single newlines are line breaks", () => {
  assert.equal(wikitextToHtml("one\ntwo\n\nthree"), "<p>one<br>two</p><p>three</p>");
});

test("a space-led run is a pre block with literal, escaped content", () => {
  assert.equal(wikitextToHtml(" a = 1\n b = <2>"), "<pre>a = 1\nb = &lt;2&gt;</pre>");
});

test("pre content is not wiki-parsed", () => {
  assert.equal(wikitextToHtml(" [[Page]] '''b'''"), "<pre>[[Page]] '''b'''</pre>");
});

// --- nowiki ---------------------------------------------------------------------------------------

test("nowiki renders its contents literally", () => {
  assert.equal(
    wikitextToHtml("see <nowiki>[[Page]] '''b''' :x:</nowiki> ok"),
    "<p>see [[Page]] '''b''' :x: ok</p>",
  );
});

test("nowiki contents are still escaped", () => {
  assert.equal(wikitextToHtml("<nowiki><b>hi</b></nowiki>"), "<p>&lt;b&gt;hi&lt;/b&gt;</p>");
});

test("an unclosed nowiki tag is escaped text", () => {
  assert.equal(wikitextToHtml("<nowiki>oops"), "<p>&lt;nowiki&gt;oops</p>");
});

// --- tables -----------------------------------------------------------------------------------------

test("a table renders header, body, caption and inline cell separators", () => {
  const src = ['{| class="wikitable"', "|+ Roster", "! Name !! Role", "|-", "| Nina || Ops", "|}"].join("\n");
  assert.equal(
    wikitextToHtml(src),
    "<table><caption>Roster</caption>" +
      "<thead><tr><th>Name</th><th>Role</th></tr></thead>" +
      "<tbody><tr><td>Nina</td><td>Ops</td></tr></tbody></table>",
  );
});

test("cells on their own lines build one row per |-", () => {
  const src = ["{|", "| a", "| b", "|-", "| c", "|}"].join("\n");
  assert.equal(
    wikitextToHtml(src),
    "<table><tbody><tr><td>a</td><td>b</td></tr><tr><td>c</td></tr></tbody></table>",
  );
});

test("cell attribute strings are dropped, keeping the cell", () => {
  const src = ["{|", '| style="color:red" | red', "|}"].join("\n");
  assert.equal(wikitextToHtml(src), "<table><tbody><tr><td>red</td></tr></tbody></table>");
});

test("a pipe inside a wiki link is not mistaken for an attribute separator", () => {
  const src = ["{|", "| [[Page|label]]", "|}"].join("\n");
  assert.equal(
    wikitextToHtml(src),
    '<table><tbody><tr><td><a class="wikilink" data-wikilink="Page">label</a></td></tr></tbody></table>',
  );
});

test("a table with no header row has no thead", () => {
  assert.equal(wikitextToHtml("{|\n| a\n|}"), "<table><tbody><tr><td>a</td></tr></tbody></table>");
});

test("an unterminated table still closes its tags and consumes the rest", () => {
  assert.equal(wikitextToHtml("{|\n| a"), "<table><tbody><tr><td>a</td></tr></tbody></table>");
});

// --- wiki links ---------------------------------------------------------------------------------------

test("[[Page]] links with the page as its own label", () => {
  assert.equal(
    wikitextToHtml("[[Getting Started]]"),
    '<p><a class="wikilink" data-wikilink="Getting Started">Getting Started</a></p>',
  );
});

test("[[Page|label]] keeps the page in data-wikilink and shows the label", () => {
  assert.equal(
    wikitextToHtml("[[Getting Started|start here]]"),
    '<p><a class="wikilink" data-wikilink="Getting Started">start here</a></p>',
  );
});

test("a wiki link with no target stays literal text", () => {
  assert.equal(wikitextToHtml("[[|x]]"), "<p>[[|x]]</p>");
});

test("a wiki link's page name is escaped into the attribute", () => {
  // escAttr quotes `"` (it would close the attribute); escText leaves it, being harmless in text.
  assert.equal(
    wikitextToHtml('[[a"b<c]]'),
    '<p><a class="wikilink" data-wikilink="a&quot;b&lt;c">a"b&lt;c</a></p>',
  );
});

// --- external links ------------------------------------------------------------------------------------

test("[https://url label] links with the label", () => {
  assert.equal(
    wikitextToHtml("[https://example.com/x docs]"),
    '<p><a href="https://example.com/x">docs</a></p>',
  );
});

test("[https://url] links with the url as its own label", () => {
  assert.equal(
    wikitextToHtml("[https://example.com]"),
    '<p><a href="https://example.com">https://example.com</a></p>',
  );
});

test("http is accepted too", () => {
  assert.equal(wikitextToHtml("[http://x.test a]"), '<p><a href="http://x.test">a</a></p>');
});

test("a javascript: URL never reaches an href; it renders as text", () => {
  const html = wikitextToHtml("[javascript:alert(1) click]");
  assert.equal(html, "<p>[javascript:alert(1) click]</p>");
  assert.ok(!html.includes("href"));
});

test("data: and file: URLs likewise render as text", () => {
  for (const src of ["[data:text/html,<script>x</script> go]", "[file:///etc/passwd go]"]) {
    const html = wikitextToHtml(src);
    assert.ok(!html.includes("href"), `${src} must not produce an href`);
    assert.ok(!html.includes("<script"), `${src} must not produce a live tag`);
  }
});

test("an external link's URL is attribute-escaped", () => {
  assert.equal(
    wikitextToHtml("[https://x.test/?a=1&b=2 q]"),
    '<p><a href="https://x.test/?a=1&amp;b=2">q</a></p>',
  );
});

// --- the app's own tokens ---------------------------------------------------------------------------
// Byte-compared: `render.ts` resolves these placeholders by reading the data- attributes, so the
// shape is an interface, not a detail. Both renderers call the same helpers in wikitext.ts.

test(":name: renders the emoji placeholder", () => {
  assert.equal(
    wikitextToHtml("hi :tada: there"),
    '<p>hi <span class="emoji" data-emoji="tada">:tada:</span> there</p>',
  );
});

test("![alt](cid:HEX) renders the inert embed placeholder with a lower-cased cid", () => {
  assert.equal(
    wikitextToHtml("![a cat](cid:DEADBEEF01)"),
    '<p><span class="embed" data-embed-cid="deadbeef01" data-alt="a cat"></span></p>',
  );
});

test("[label](file:HEX) renders the file reference chip", () => {
  assert.equal(
    wikitextToHtml("[notes.pdf](file:ABC123)"),
    '<p><a class="reflink file-ref" data-file-cid="abc123">' +
      '<span class="reflink-ico" aria-hidden="true">📄</span>notes.pdf</a></p>',
  );
});

test("[label](status:ID) and [label](event:ID) render their chips", () => {
  assert.equal(
    wikitextToHtml("[shipped](status:S1)"),
    '<p><a class="reflink status-ref" data-status-id="S1">' +
      '<span class="reflink-ico" aria-hidden="true">◈</span>shipped</a></p>',
  );
  assert.equal(
    wikitextToHtml("[games](event:E1)"),
    '<p><a class="reflink event-ref" data-event-id="E1">' +
      '<span class="reflink-ico" aria-hidden="true">⧗</span>games</a></p>',
  );
});

test("@[Name] renders a mention with no self-highlight on the wiki", () => {
  const html = wikitextToHtml("@[Nina Ray] wrote this");
  assert.equal(html, '<p><span class="mention" data-mention="Nina Ray">@Nina Ray</span> wrote this</p>');
  assert.ok(!html.includes("mention-me"));
});

test("an embed's alt text is attribute-escaped", () => {
  assert.equal(
    wikitextToHtml('![a "quote" & <tag>](cid:ab)'),
    '<p><span class="embed" data-embed-cid="ab" data-alt="a &quot;quote&quot; &amp; &lt;tag&gt;"></span></p>',
  );
});

// --- things deliberately not supported ------------------------------------------------------------------

// The converter has no template engine: a `{{template}}` stays literal here. The one exception
// lives a layer up, in `render.ts`, which lifts a page's `{{Infobox …}}` block out before either
// converter runs (see `infobox.ts`), so this pass never sees it.
test("{{templates}} render as literal text", () => {
  assert.equal(wikitextToHtml("{{Infobox|a=1}}"), "<p>{{Infobox|a=1}}</p>");
});

test("__TOC__ and __NOTOC__ are stripped from the output", () => {
  assert.equal(wikitextToHtml("__TOC__\n== A =="), "<h2>A</h2>");
  assert.equal(wikitextToHtml("__NOTOC__ hello"), "<p>hello</p>");
  assert.equal(wikitextToHtml("a __TOC__ b"), "<p>a b</p>");
});

test("stripMagicWords leaves other text alone", () => {
  assert.equal(stripMagicWords("keep __BOLD__ me"), "keep __BOLD__ me");
});

// --- escaping: the sanitizer's first line of defence ------------------------------------------------------

test("raw HTML in the source comes out escaped", () => {
  assert.equal(
    wikitextToHtml("<script>alert(1)</script>"),
    "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>",
  );
});

test("an img tag in the source cannot survive as a tag", () => {
  // The angle brackets are what matter: `onerror="…"` survives as inert text, never as an attribute,
  // because there is no tag left for it to sit on.
  assert.equal(
    wikitextToHtml('<img src=x onerror="alert(1)">'),
    '<p>&lt;img src=x onerror="alert(1)"&gt;</p>',
  );
});

test("ampersands are escaped once, in text and in attributes", () => {
  assert.equal(wikitextToHtml("a & b"), "<p>a &amp; b</p>");
  assert.equal(escText("&<>"), "&amp;&lt;&gt;");
  assert.equal(escAttr('&<>"'), "&amp;&lt;&gt;&quot;");
});

test("a heading cannot smuggle a tag", () => {
  assert.equal(wikitextToHtml("== <b>x</b> =="), "<h2>&lt;b&gt;x&lt;/b&gt;</h2>");
});

test("a table cell cannot smuggle a tag", () => {
  assert.equal(
    wikitextToHtml("{|\n| <b>x</b>\n|}"),
    "<table><tbody><tr><td>&lt;b&gt;x&lt;/b&gt;</td></tr></tbody></table>",
  );
});

// --- misc ---------------------------------------------------------------------------------------------------

test("empty and blank input render to nothing", () => {
  assert.equal(wikitextToHtml(""), "");
  assert.equal(wikitextToHtml("\n\n  \n"), "");
});

test("CRLF input is normalised", () => {
  assert.equal(wikitextToHtml("a\r\n\r\nb"), "<p>a</p><p>b</p>");
});

test("inlineToHtml is the inline pass on its own, with no block wrapper", () => {
  assert.equal(inlineToHtml("a ''b''"), "a <em>b</em>");
});

// --- directives ------------------------------------------------------------------------------------------------

test("parseRedirect reads the target of a leading #REDIRECT", () => {
  assert.equal(parseRedirect("#REDIRECT [[Home]]"), "Home");
  assert.equal(parseRedirect("#redirect[[Home]]"), "Home");
  assert.equal(parseRedirect("  \n#REDIRECT [[Getting Started]]\nignored"), "Getting Started");
});

test("parseRedirect ignores a piped label", () => {
  assert.equal(parseRedirect("#REDIRECT [[Home|the front page]]"), "Home");
});

test("parseRedirect returns null when the marker does not lead the page", () => {
  assert.equal(parseRedirect("intro\n#REDIRECT [[Home]]"), null);
  assert.equal(parseRedirect("# REDIRECT [[Home]]"), null); // a markdown h1, not a redirect
  assert.equal(parseRedirect("#REDIRECT Home"), null);
  assert.equal(parseRedirect(""), null);
});

test("tocDirective reports the magic words, with NOTOC winning", () => {
  assert.equal(tocDirective("__TOC__\ntext"), "force");
  assert.equal(tocDirective("__NOTOC__\ntext"), "notoc");
  assert.equal(tocDirective("__TOC__ and __NOTOC__"), "notoc");
  assert.equal(tocDirective("plain text"), null);
  assert.equal(tocDirective(""), null);
});

// --- the grammar the composer's markers are built against ------------------------------------------------------

test("WIKI_LINK_RE splits page from label and leaves the label optional", () => {
  const plain = WIKI_LINK_RE.exec("[[Page]]");
  assert.equal(plain?.[1], "Page");
  assert.equal(plain?.[2], undefined);
  const piped = WIKI_LINK_RE.exec("[[Page|label]]");
  assert.equal(piped?.[1], "Page");
  assert.equal(piped?.[2], "label");
});

test("WIKI_LINK_RE rejects a page name long enough to be a denial-of-service", () => {
  assert.equal(WIKI_LINK_RE.exec(`[[${"x".repeat(200)}]]`), null);
});

// --- link-card previews ---------------------------------------------------------------------------

test("plainSummary reads out the prose of either format, not its markup", () => {
  assert.equal(
    plainSummary("== Rules ==\n* '''No''' spoilers\n* Be [[Kind|kind]]"),
    "Rules No spoilers Be kind",
  );
  assert.equal(
    plainSummary("# Heading\n\nSome **bold** text with a [label](file:ab) link."),
    "Heading Some bold text with a label link.",
  );
});

test("plainSummary drops what a preview cannot show: embeds, code, templates, tables", () => {
  assert.equal(plainSummary("![a cat](cid:deadbeef) Look at this"), "Look at this");
  assert.equal(plainSummary("before\n```\ncode()\n```\nafter"), "before after");
  assert.equal(plainSummary("{{infobox}}Body"), "Body");
  assert.equal(plainSummary("| a | b |\nprose"), "prose");
  assert.equal(plainSummary("#REDIRECT [[Elsewhere]]"), "");
});

test("plainSummary bounds the preview and marks where it was cut", () => {
  const out = plainSummary("word ".repeat(80), 20);
  assert.equal(out, "word word word word…", "cut at the bound, with no trailing space before it");
  assert.ok(out.length <= 21, "never longer than the bound plus the ellipsis");
  assert.equal(plainSummary("short", 20), "short", "under the bound, nothing is added");
});
