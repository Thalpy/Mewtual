// Unit tests for the wiki infobox (`{{Infobox …}}` -> the card floated at the top of a page).
//
// Run with `npm test`. Three things are pinned here. The PARSING: field order is the author's,
// `title`/`image`/`caption` are chrome rather than rows, and a `|` inside `[[Page|label]]` is not
// a field separator. The REFUSALS: an unterminated block, an empty block, and a bare content
// address as an image all leave the page exactly as written (that last one keeps the backend's
// never-decay scan honest, since it only recognises the `cid:` marker). And the MARKUP shape,
// byte-compared, because `render.ts` hands this string to DOMPurify with a fixed allow-list.

import { test } from "node:test";
import assert from "node:assert/strict";

import { extractInfobox, infoboxHtml, infoboxTemplate } from "./infobox.ts";

/** A stand-in inline renderer: the real ones escape, this marks its input so tests can see it. */
const ident = (s: string) => s;

test("fields parse in author order, with title/image/caption lifted out as chrome", () => {
  const { box, rest } = extractInfobox(
    ["{{Infobox", "| title = Whiskers", "| image = ![a cat](cid:deadbeef)", "| caption = At the cafe", "| Species = Cat", "| Age = 4", "}}", "The body."].join("\n"),
  );
  assert.ok(box);
  assert.equal(box.title, "Whiskers");
  assert.equal(box.image, "![a cat](cid:deadbeef)");
  assert.equal(box.caption, "At the cafe");
  assert.deepEqual(box.fields, [
    { label: "Species", value: "Cat" },
    { label: "Age", value: "4" },
  ]);
  assert.equal(rest, "The body.");
});

test("a template type after the name is accepted and ignored (paste from Wikipedia)", () => {
  const { box } = extractInfobox("{{Infobox person\n| Born = 1970\n}}");
  assert.deepEqual(box?.fields, [{ label: "Born", value: "1970" }]);
});

test("a pipe inside [[Page|label]] does not split a field", () => {
  const { box } = extractInfobox("{{Infobox\n| Owner = [[Alice|the owner]]\n}}");
  assert.deepEqual(box?.fields, [{ label: "Owner", value: "[[Alice|the owner]]" }]);
});

test("the first `=` splits key from value; later ones stay in the value", () => {
  const { box } = extractInfobox("{{Infobox\n| Formula = a = b = c\n}}");
  assert.deepEqual(box?.fields, [{ label: "Formula", value: "a = b = c" }]);
});

test("an empty value marks a section band", () => {
  const { box } = extractInfobox("{{Infobox\n| Details =\n| Age = 4\n}}");
  assert.deepEqual(box?.fields, [
    { label: "Details", value: "" },
    { label: "Age", value: "4" },
  ]);
});

test("a multi-line value keeps its lines", () => {
  const { box } = extractInfobox("{{Infobox\n| Notes = first\nsecond\n}}");
  assert.deepEqual(box?.fields, [{ label: "Notes", value: "first\nsecond" }]);
});

test("the block is lifted from wherever it sits, and only the first one is the card", () => {
  const { box, rest } = extractInfobox("Intro line\n{{Infobox\n| A = 1\n}}\nOutro\n{{Infobox\n| B = 2\n}}");
  assert.deepEqual(box?.fields, [{ label: "A", value: "1" }]);
  assert.equal(rest, "Intro line\nOutro\n{{Infobox\n| B = 2\n}}", "the second block stays literal text");
});

test("an unterminated block leaves the page untouched", () => {
  const src = "{{Infobox\n| title = half typed";
  assert.deepEqual(extractInfobox(src), { box: null, rest: src });
});

test("an empty block is not a card", () => {
  const src = "{{Infobox}}\nBody";
  assert.deepEqual(extractInfobox(src), { box: null, rest: src });
});

test("a bare content address is refused as an image (it would not be pinned)", () => {
  const { box } = extractInfobox("{{Infobox\n| image = deadbeef\n| A = 1\n}}");
  assert.equal(box?.image, "", "only the ![alt](cid:…) marker counts");
});

test("a page with no infobox is returned unchanged", () => {
  assert.deepEqual(extractInfobox("Just prose"), { box: null, rest: "Just prose" });
  assert.deepEqual(extractInfobox(""), { box: null, rest: "" });
});

test("fields, labels and values are bounded", () => {
  const many = ["{{Infobox", ...Array.from({ length: 80 }, (_, i) => `| F${i} = v`), "}}"].join("\n");
  assert.equal(extractInfobox(many).box?.fields.length, 60);
  const long = `{{Infobox\n| ${"L".repeat(200)} = ${"v".repeat(900)}\n}}`;
  const f = extractInfobox(long).box?.fields[0];
  assert.equal(f?.label.length, 80);
  assert.equal(f?.value.length, 600);
});

// --- markup shape (what the sanitizer sees) -------------------------------------------------

test("the card is a table: caption, media row, section band, label/value rows", () => {
  const box = {
    title: "Whiskers",
    image: "IMG",
    caption: "At the cafe",
    fields: [
      { label: "Details", value: "" },
      { label: "Age", value: "4" },
    ],
  };
  assert.equal(
    infoboxHtml(box, ident),
    '<table class="wiki-infobox">' +
      "<caption>Whiskers</caption><tbody>" +
      '<tr class="ib-media"><td colspan="2">IMG<span class="ib-caption">At the cafe</span></td></tr>' +
      '<tr class="ib-section"><th colspan="2">Details</th></tr>' +
      "<tr><th>Age</th><td>4</td></tr>" +
      "</tbody></table>",
  );
});

test("a card with no picture and no title renders just its rows", () => {
  assert.equal(
    infoboxHtml({ title: "", image: "", caption: "", fields: [{ label: "A", value: "1" }] }, ident),
    '<table class="wiki-infobox"><tbody><tr><th>A</th><td>1</td></tr></tbody></table>',
  );
});

test("a multi-line value becomes soft-broken rows", () => {
  assert.match(infoboxHtml({ title: "", image: "", caption: "", fields: [{ label: "N", value: "a\nb" }] }, ident), /<td>a<br>b<\/td>/);
});

test("every author-supplied string goes through the caller's inline renderer", () => {
  const seen: string[] = [];
  infoboxHtml(
    { title: "T", image: "I", caption: "C", fields: [{ label: "L", value: "V" }] },
    (s) => {
      seen.push(s);
      return s;
    },
  );
  assert.deepEqual(seen.sort(), ["C", "I", "L", "T", "V"], "nothing reaches the output unrendered");
});

test("the toolbar skeleton round-trips through the parser", () => {
  const { box } = extractInfobox(infoboxTemplate("Whiskers"));
  assert.equal(box?.title, "Whiskers");
  assert.deepEqual(box?.fields, [{ label: "Label", value: "value" }]);
});
