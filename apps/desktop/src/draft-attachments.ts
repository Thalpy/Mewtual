// Attachments that live INSIDE the composer draft.
//
// "Post to chat" from the Studio and the "+" insert picker both put an embed marker
// (`![label](cid:HEX)`, built by `fileMarker` in refs.ts) into the draft text. The text stays the
// one source of truth: the durable send path, its retry identity and the renderer all read the
// draft as written, and nothing here adds a second place where an attachment could exist. What
// the composer gains is a chip per marker so a member sees what is attached and can take it off
// without hunting for the markdown; removing the chip simply edits the text.
//
// Pure string handling only (no Svelte, no DOM), so it is directly unit-testable. The regex must
// agree with the renderer's `EMBED_RE` and with `fileMarker`; `draft-attachments.test.ts` pins both.

export type DraftAttachment = Readonly<{ label: string; cid: string; start: number; end: number }>;

// Same label grammar as the renderer: no `[`, `]` or newline inside the label, and a CID of up to
// 64 hex characters (the renderer accepts shorter ones; the fileshare only ever mints 64).
const MARKER_RE = /!\[([^\]\n]*)\]\(cid:([0-9a-fA-F]{1,64})\)/g;

/** Every embed marker in the draft, in document order, with its exact character span. */
export function draftAttachments(draft: string): DraftAttachment[] {
  const out: DraftAttachment[] = [];
  MARKER_RE.lastIndex = 0;
  for (let m = MARKER_RE.exec(draft); m; m = MARKER_RE.exec(draft)) {
    out.push({ label: m[1], cid: m[2].toLowerCase(), start: m.index, end: m.index + m[0].length });
  }
  return out;
}

/**
 * The draft with one marker taken out. The span is re-checked against the current text first: a
 * chip rendered from an older draft must not cut a different piece of a draft the member has since
 * edited, so a stale span leaves the text untouched. One adjacent space (trailing, else leading)
 * goes with the marker, which is the space `insertInto` added around it.
 */
export function removeDraftAttachment(draft: string, attachment: DraftAttachment): string {
  const { start, end } = attachment;
  if (start < 0 || end > draft.length || start >= end) return draft;
  const span = draft.slice(start, end);
  MARKER_RE.lastIndex = 0;
  const m = MARKER_RE.exec(span);
  if (!m || m.index !== 0 || m[0].length !== span.length || m[2].toLowerCase() !== attachment.cid) return draft;
  let from = start, to = end;
  if (draft[to] === " ") to += 1;
  else if (from > 0 && draft[from - 1] === " ") from -= 1;
  return draft.slice(0, from) + draft.slice(to);
}
