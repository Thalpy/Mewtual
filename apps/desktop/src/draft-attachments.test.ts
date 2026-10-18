import assert from "node:assert/strict";
import test from "node:test";
import { draftAttachments, removeDraftAttachment } from "./draft-attachments.ts";
import { fileMarker, insertInto } from "./refs.ts";
import { EMBED_RE } from "./wikitext.ts";

const CID = "e".repeat(64);
const CID2 = "f".repeat(64);

test("every fileMarker embed is found with its exact span, and the renderer agrees it is an embed", () => {
  const marker = fileMarker("Mewtual-flipnote.pixa", CID, true);
  const draft = insertInto("here is the loop", 16, 16, marker).text;
  const found = draftAttachments(draft);
  assert.equal(found.length, 1);
  assert.equal(found[0].cid, CID);
  assert.equal(found[0].label, "Mewtual-flipnote.pixa");
  assert.equal(draft.slice(found[0].start, found[0].end), marker);
  assert.ok(EMBED_RE.test(marker), "the chip grammar and the renderer grammar must name the same markers");
});

test("link markers are not attachments; two embeds come back in order with lowercase cids", () => {
  const draft = `${fileMarker("notes.pdf", CID, false)} ${fileMarker("a.pixa", CID.toUpperCase(), true)} and ${fileMarker("b.pixa", CID2, true)}`;
  const found = draftAttachments(draft);
  assert.deepEqual(found.map((a) => [a.label, a.cid]), [["a.pixa", CID], ["b.pixa", CID2]]);
  assert.ok(found[0].start < found[1].start);
});

test("removing a chip edits only its marker and one surrounding space", () => {
  const marker = fileMarker("a.pixa", CID, true);
  const middle = `before ${marker} after`;
  const [a] = draftAttachments(middle);
  assert.equal(removeDraftAttachment(middle, a), "before after");
  const tail = `before ${marker}`;
  assert.equal(removeDraftAttachment(tail, draftAttachments(tail)[0]), "before");
  const only = marker;
  assert.equal(removeDraftAttachment(only, draftAttachments(only)[0]), "");
});

test("a stale span never cuts text that is no longer that marker", () => {
  const marker = fileMarker("a.pixa", CID, true);
  const draft = `x ${marker}`;
  const [a] = draftAttachments(draft);
  const edited = `yy ${marker}`; // the member typed before the chip; the old span now covers other text
  assert.equal(removeDraftAttachment(edited, a), edited);
  assert.equal(removeDraftAttachment(draft, { ...a, cid: CID2 }), draft, "a cid mismatch is also refused");
  assert.equal(removeDraftAttachment(draft, { ...a, end: draft.length + 5 }), draft, "an out-of-range span is refused");
});
