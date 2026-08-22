/**
 * The sanitizer boundary.
 *
 * `render.ts` is where text written by other group members becomes markup in the app, so it is the
 * one module where a mistake is an XSS rather than a cosmetic bug. Its own header states the
 * contract: no `<img>`/`<video>`/`<audio>`/`<script>`/raw HTML reaches the page, custom emoji and
 * embeds are inert `<span>` placeholders, and the resolver later replaces those placeholders with
 * elements it builds in code from the group's own content-addressed blobs.
 *
 * That contract went untested until now, because asserting it needs a DOM and the test runner had
 * none: `wikitext.test.ts` and `infobox.test.ts` both stop at the edge with a comment deferring to
 * "the DOMPurify allow-list". `scripts/test-dom.mjs` supplies the DOM; these tests cross the edge.
 *
 * Written against the rendered output rather than the allow-list arrays, so that a change to how
 * the allow-list is expressed cannot quietly pass while the guarantee changes underneath.
 */
import test from "node:test";
import assert from "node:assert/strict";

import { renderMessage, renderTextDocument, renderWiki } from "./render.ts";

/** Every renderer, so a hole cannot be opened in one surface and missed in the others. */
const RENDERERS: Array<[string, (s: string) => string]> = [
  ["renderMessage", (s) => renderMessage(s)],
  ["renderWiki/markdown", (s) => renderWiki(s)],
  ["renderWiki/wikitext", (s) => renderWiki(s, "wiki")],
  ["renderTextDocument", (s) => renderTextDocument(s)],
];

/** Markup that must never survive, whichever surface it arrives through. */
const HOSTILE = [
  "<script>alert(1)</script>",
  "<img src=x onerror=alert(1)>",
  "<video src=x onerror=alert(1)></video>",
  "<audio src=x onerror=alert(1)></audio>",
  "<iframe src='javascript:alert(1)'></iframe>",
  "<object data=x></object>",
  "<embed src=x>",
  "<svg><script>alert(1)</script></svg>",
  "<math><mtext><script>alert(1)</script></mtext></math>",
  "<form action=x><button>go</button></form>",
  "<style>body{display:none}</style>",
  "<link rel=stylesheet href=x>",
  "<base href='http://evil/'>",
  "<a href=\"javascript:alert(1)\">x</a>",
  "<div onclick=alert(1)>x</div>",
];

/**
 * Handlers placed on tags the allow-list *keeps*, which is the only way this surface can actually
 * fail. DOMPurify does not hard-block `on*`: an attribute added to `ALLOWED_ATTR` is honoured,
 * handler or not. A handler on a `<div>` proves nothing, because the div is dropped as a tag
 * before its attributes are ever considered.
 */
const HANDLER_EVENTS = ["onclick", "onmouseover", "onload", "onerror", "onfocus", "onanimationend"];
const ALLOWED_HOSTS = ["p", "span", "a", "code", "li", "td"];
for (const event of HANDLER_EVENTS) {
  for (const host of ALLOWED_HOSTS) {
    HOSTILE.push(`<${host} ${event}=alert(1)>x</${host}>`);
  }
}

/**
 * Parse rendered output the way the app will.
 *
 * Deliberately inspecting the tree rather than pattern-matching the string: the wikitext converter
 * *escapes* raw HTML instead of dropping it, so `&lt;img onerror=…&gt;` appears in safe output as
 * literal text. A regex over the string calls that a handler; the browser correctly calls it words.
 * Only the parsed form distinguishes markup from text, which is the distinction that matters.
 */
function parsed(html: string): Element {
  const host = document.createElement("div");
  host.innerHTML = html;
  return host;
}

const ACTIVE_TAGS =
  "script,img,video,audio,iframe,object,embed,svg,math,style,link,base,form,input,button,textarea";

test("no renderer lets an active element or handler through", () => {
  for (const [name, render] of RENDERERS) {
    for (const dirty of HOSTILE) {
      const tree = parsed(render(dirty));
      const active = tree.querySelector(ACTIVE_TAGS);
      assert.equal(
        active,
        null,
        `${name} produced a live <${active?.tagName.toLowerCase()}> for ${dirty}`,
      );
      for (const el of tree.querySelectorAll("*")) {
        for (const attr of el.getAttributeNames()) {
          assert.ok(
            !attr.toLowerCase().startsWith("on"),
            `${name} kept ${attr} on <${el.tagName.toLowerCase()}> for ${dirty}`,
          );
        }
        for (const url of ["href", "src", "action", "formaction", "xlink:href"]) {
          const value = (el.getAttribute(url) ?? "").replace(/\s/g, "").toLowerCase();
          assert.ok(
            !value.startsWith("javascript:") && !value.startsWith("vbscript:"),
            `${name} kept an executable ${url} for ${dirty}`,
          );
        }
      }
      // Escaped-to-text is a fine outcome; silently executing is not. Nothing above proves the
      // hostile input was actually *seen*, so check the renderer did not simply return nothing
      // useful for the one input that has visible text to keep.
      if (dirty.includes("<div onclick")) {
        assert.match(tree.textContent ?? "", /x/, `${name} lost the safe text of ${dirty}`);
      }
    }
  }
});

test("a link keeps only a real web scheme", () => {
  // The allow-list keeps `href`, so the scheme check is the whole defence here.
  assert.match(renderMessage("[x](https://example.com/a)"), /href="https:\/\/example\.com\/a"/);
  for (const scheme of [
    "javascript:alert(1)",
    "data:text/html,<script>alert(1)</script>",
    "vbscript:msgbox(1)",
  ]) {
    const out = renderMessage(`[x](${scheme})`);
    assert.ok(!out.includes("href="), `kept an href for ${scheme}: ${out}`);
    assert.ok(out.includes("x"), "the label survives as text");
  }
});

test("an embed is an inert placeholder, never a media element", () => {
  // The load-bearing half of the contract: untrusted text can name a file, but only the resolver
  // (in code, from the group's own blobs) may turn that name into something that fetches.
  const out = renderMessage("![a cat](cid:AABBCC)");
  assert.match(out, /^<span class="embed" data-embed-cid="aabbcc" data-alt="a cat"><\/span>$/);
  assert.ok(!out.includes("src"), "nothing that fetches");

  // A remote markdown image is also held inert: the URL survives as data for the resolver to
  // validate, not as an attribute the browser will act on.
  const remote = renderMessage("![cat](https://example.com/c.gif)");
  assert.match(remote, /data-remote-url="https:\/\/example\.com\/c\.gif"/);
  assert.ok(!remote.includes("<img"), "still not an image element");
});

test("app-only reference schemes never become an href", () => {
  // `file:`/`status:`/`event:` are resolved from data attributes, so a reference can only ever
  // address this group's own content; reaching an <a href> would make them real URLs.
  for (const [markup, attr] of [
    ["[doc](file:AABB)", "data-file-cid"],
    ["[post](status:12)", "data-status-id"],
    ["[party](event:9)", "data-event-id"],
  ]) {
    const out = renderMessage(markup);
    assert.ok(out.includes(attr), `${markup} should carry ${attr}: ${out}`);
    assert.ok(!out.includes("href"), `${markup} must not become a link: ${out}`);
  }
});

test("placeholders the app resolves survive with their hooks intact", () => {
  // These are the attributes the resolvers key off. If the allow-list dropped one, the feature
  // would fail silently rather than loudly, so they are pinned here.
  assert.match(renderMessage(":wave:"), /class="emoji" data-emoji="wave"/);
  assert.match(renderMessage("@[Ada]"), /class="mention" data-mention="Ada"/);
  assert.match(renderMessage("@[Ada]", "Ada"), /mention-me/, "a mention of yourself is marked");
  assert.doesNotMatch(renderMessage("@[Ada]", "Bob"), /mention-me/);
  assert.match(renderMessage("||secret||"), /class="spoiler" data-spoiler=""/);
  // A wiki link carries the page name, never the label: navigation must not follow display text.
  assert.match(renderMessage("[[Real Page|shown]]"), /data-wikilink="Real Page"/);
});

test("a spoiler's body cannot smuggle markup", () => {
  // The spoiler renderer escapes its own text rather than nesting formatting, so this is checking
  // that escaping, not the sanitizer's.
  const out = renderMessage("||<script>alert(1)</script>||");
  assert.ok(!out.includes("<script"), out);
  assert.match(out, /class="spoiler"/);
});

test("wikitext pages go through the identical boundary", () => {
  // Two authoring formats, one sanitizer. The risk is a converter growing a surface the markdown
  // path never had, so the wikitext path is checked for the structures it is *allowed* to emit too.
  const table = renderWiki('{| class="wikitable"\n|-\n! h\n| c\n|}', "wiki");
  assert.match(table, /<table/, "tables are allowed structure");
  assert.ok(!table.includes("onclick"), table);
  assert.match(renderWiki("== Heading ==", "wiki"), /<h2/);
  // `colspan` is allowed (the infobox needs it); other table attributes are not.
  const attrs = renderWiki('{|\n|-\n| colspan="2" style="color:red" | c\n|}', "wiki");
  assert.ok(!attrs.includes("style"), `style must not survive: ${attrs}`);
});

test("an uploaded markdown file gets the same allow-list and none of the app's tokens", () => {
  // A shared `.md` is a document someone uploaded, not a page of this wiki, so it renders with a
  // marked instance carrying none of the app's extensions.
  assert.match(renderTextDocument("# Title\n\nsome **text**"), /<h1[^>]*>Title<\/h1>/);
  assert.match(renderTextDocument("[[Page]]"), /\[\[Page\]\]/, "wiki syntax stays literal");
  // Markdown images are dropped rather than kept literal: the extensions that would turn
  // `cid:` into a placeholder are not registered, so it parses as a plain image and the
  // allow-list removes it. Correct for safety (an uploaded file cannot fetch anything) and
  // pinned here because render.ts's comment reads as though the text survived.
  assert.equal(renderTextDocument("![a](cid:AABB)").trim(), "<p></p>");
  assert.equal(renderTextDocument("![a](https://example.com/c.gif)").trim(), "<p></p>");
});

test("ordinary formatting is not collateral damage", () => {
  // A sanitizer that strips everything passes every test above. This is the other half.
  assert.match(renderMessage("**bold** and *italic* and `code`"), /<strong>bold<\/strong>/);
  assert.match(renderMessage("**bold** and *italic* and `code`"), /<em>italic<\/em>/);
  assert.match(renderMessage("**bold** and *italic* and `code`"), /<code>code<\/code>/);
  assert.match(renderWiki("- one\n- two"), /<li>one<\/li>/);
  assert.match(renderWiki("> quoted"), /<blockquote>/);
});

test("empty and absent input render to nothing rather than throwing", () => {
  for (const [name, render] of RENDERERS) {
    for (const input of ["", undefined as unknown as string, null as unknown as string]) {
      assert.doesNotThrow(() => render(input), `${name} threw on ${String(input)}`);
      assert.equal(render(input).trim(), "", `${name} invented output for ${String(input)}`);
    }
  }
});
