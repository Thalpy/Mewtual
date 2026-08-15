// Reference markers for the composer's "+" insert picker.
//
// The picker turns a piece of this server's own content — a fileshare file, one of your status
// posts, a wiki page — into the marker text that goes in the message. `render.ts` tokenizes those
// markers back into chips/embeds, so the two files agree on exactly one thing: the marker grammar.
// `refs.test.ts` pins that agreement by checking every builder here against the renderer's own
// regexes, which is the seam that silently breaks when either side is edited.
//
// Pure string handling only (no Svelte, no DOM), so it is directly unit-testable.

/**
 * A `[label](…)` / `![alt](…)` label. `[`, `]` and newlines would terminate the marker early and
 * break the tokenizer, so they're replaced rather than escaped; whitespace collapses and the result
 * is bounded to the tokenizer's own label limit.
 */
export function refLabel(s: string, n = 60): string {
  return s
    .replace(/[[\]\n]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, n);
}

/**
 * A fileshare file. Media embeds inline (the resolver builds the element from the verified blob);
 * anything else can only be a link chip, which opens the file-info pane.
 */
export function fileMarker(name: string, cid: string, asEmbed: boolean): string {
  const label = refLabel(name) || "file";
  return asEmbed ? `![${label}](cid:${cid})` : `[${label}](file:${cid})`;
}

/** One of your status posts, labelled with a snippet of the post itself. */
export function statusMarker(text: string, id: string): string {
  return `[${refLabel(text, 48) || "status"}](status:${id})`;
}

/** A wiki page — the long-standing `[[Page]]` form the wiki already uses everywhere. */
export function wikiMarker(page: string): string {
  return `[[${refLabel(page, 120) || "page"}]]`;
}

/**
 * Splice `insert` into `draft` over the selection `[start, end)`, space-separated from the text on
 * either side so consecutive picks don't run together — but only where a space isn't already there,
 * otherwise inserting mid-message leaves a double space. Returns the new draft and where the caret
 * should land (just after the insertion).
 */
export function insertInto(
  draft: string,
  start: number,
  end: number,
  insert: string,
): { text: string; caret: number } {
  const before = draft.slice(0, start);
  const after = draft.slice(end);
  const lead = before && !/\s$/.test(before) ? " " : "";
  const trail = /^\s/.test(after) ? "" : " ";
  const chunk = lead + insert + trail;
  return { text: before + chunk + after, caret: before.length + chunk.length };
}
