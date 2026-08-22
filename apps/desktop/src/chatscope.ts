// Which conversation the chat pane is currently holding, and what to do when that changes
// underneath it.
//
// Two separate bugs share one root here: something that names a (server, channel) pair gets
// compared, or reassigned, without the channel-scoped state belonging to it being reloaded. Both
// are far easier to reason about (and to test) as pure functions over ids than as guards buried in
// async callbacks, so the decisions live here and App.svelte only wires them up.

/// The key naming one conversation. Message windows, read marks and drafts are all keyed by it, so
/// it gets exactly one definition rather than a template literal repeated at every call site.
export function chatScopeKey(server: number, channel: string): string {
  return `${server}:${channel}`;
}

/// Whether the rows the chat pane currently holds are this conversation's rows.
///
/// `loaded` is the scope stamped by the last read that completed, so this asks a question about
/// what is in memory rather than about what is on screen. The chat refresh is coalesced: several
/// channel notifications share ONE promise that resolves when the whole drain empties, which may
/// be a pass that loaded a *different* conversation. A notification handler waiting on that
/// promise must therefore ask this before reading the shared message array, instead of assuming
/// the pass it waited on was its own.
///
/// An empty `loaded` means nothing has been read yet, which is never a match.
export function scopeHoldsConversation(loaded: string, server: number, channel: string): boolean {
  return loaded !== "" && loaded === chatScopeKey(server, channel);
}

/// The channel that should be active after the shared directory has been re-read, and whether that
/// moved it.
///
/// A channel can leave the list without anyone deleting one, and nothing in the app can delete one:
/// the backend drops any catalog entry whose name does not hash to its id (the defence against a
/// malformed or hostile directory write), and a server whose channel-index document has not synced
/// yet lists only the legacy `general`. Either way the channel someone is reading can vanish from
/// under them, and `changed` is the caller's signal that the messages, topic and delivery ticks on
/// screen no longer belong to the channel now named as active.
export function reconcileActiveChannel(
  channels: readonly { id: string }[],
  active: string,
): { active: string; changed: boolean } {
  // An empty list is a failed or racing read, never an instruction to move: the caller keeps what
  // it had rather than being walked off a channel that is probably still there.
  if (!channels.length) return { active, changed: false };
  if (active && channels.some((channel) => channel.id === active)) return { active, changed: false };
  const first = channels[0].id;
  return { active: first, changed: first !== active };
}
