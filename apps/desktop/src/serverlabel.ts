/**
 * What a server is called in the rail, and how a list of rail entries is settled onto it.
 *
 * A server used to have no name of its own. The rail label was purely local and defaulted to the
 * display name of whoever was looking, so every group appeared to be named after its reader. The
 * group's own name now rides the livery document beside the icon; this module is the read side of
 * that, and the one place the precedence is written down.
 *
 * Extracted from App.svelte because the precedence has a failure that is invisible by inspection:
 * a published name can become known BEFORE the rail entry it belongs to exists (`join_server`
 * runs the livery catch-up before it returns), and a settle pass over the existing entries then
 * has nothing to apply it to. Kept pure so that ordering can be tested rather than reasoned about.
 */

/** The three sources of a rail label, in the order they outrank each other. */
export type LabelSources = {
  /** This member's own override for this server, "" if they have not renamed it. */
  local: string;
  /** The group's published name, "" if nobody has published one. */
  published: string;
  /** Whatever label the rail entry was created with (the join box, or a fallback). */
  created: string;
};

/** A rail entry, as much of one as labelling needs. */
export type LabelledEntry = { id: number; name: string; isDm: boolean };

/**
 * Resolve one label: this member's own override, then the group's published name, then the label
 * the entry was created with.
 *
 * A DM is never published and never overridden: its label is the friend, and the caller passes it
 * as `created`.
 */
export function resolveServerLabel(sources: LabelSources): string {
  return sources.local.trim() || sources.published.trim() || sources.created;
}

/**
 * Settle every entry's own `name` onto the label that should be showing, returning the entries
 * that moved. The rail, the crumbs, the cross-server inbox, the orbit view and the command
 * palette all read `entry.name`, so the resolution happens once here rather than at each of them.
 *
 * Mutates in place (the caller re-assigns the array to publish the change) and reports whether
 * anything changed, so a settle pass that finds nothing to do costs no re-render.
 */
export function settleServerLabels(
  entries: LabelledEntry[],
  sourcesFor: (entry: LabelledEntry) => LabelSources,
): boolean {
  let changed = false;
  for (const entry of entries) {
    if (entry.isDm) continue; // a DM's label is the friend, and is never published
    const label = resolveServerLabel(sourcesFor(entry));
    if (label && label !== entry.name) {
      entry.name = label;
      changed = true;
    }
  }
  return changed;
}
