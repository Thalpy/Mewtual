// Is the answer we are holding actually this group's answer, and what may act on it before it is?
//
// Switching groups empties every pane synchronously, so between the switch and its first reads
// landing there is a window where a lot of state reads as empty. Empty is not "no", it is "not read
// yet", and every predicate below is one where an adversarial review found the difference mattered:
// each had a permissive or destructive reading of unread state. They are named and pinned here
// rather than left as bare comparisons at their call sites, because the bare comparison is exactly
// what made them easy to get wrong.

/// A captured navigation scope: which move it belongs to, and which group it addressed.
export type ViewScope = {
  /// Bumped by every move that changes which group the panes show.
  generation: number;
  /// The group the read was issued for; null when there is no active group.
  server: number | null;
};

/// Whether a scope captured before an await is still the one on screen.
///
/// Both halves matter, and for different reasons. The generation catches a move away, including a
/// move to somewhere with no active group at all. The server id catches a read issued for a
/// different group at the SAME generation, which is what every event-driven refresh is: they write
/// group-scoped state without any navigation having happened.
export function scopeCurrent(captured: ViewScope, live: ViewScope): boolean {
  return captured.generation === live.generation && captured.server === live.server;
}

/// Whether sensitive group-scoped data may land in the webview after an asynchronous native call.
///
/// Navigation scope alone is insufficient for the lock path: native work may finish just before
/// lock while its Promise continuation remains queued until afterward. Lock clears the visible
/// state and caches synchronously, so a late continuation must also observe that the UI remains
/// unlocked before it may repopulate either one.
export function unlockedScopeCurrent(captured: ViewScope, live: ViewScope, locked: boolean): boolean {
  return !locked && scopeCurrent(captured, live);
}

/// Whether a long create/join command may apply its result to the current frontend session.
///
/// Native generation checks stop stale work from committing after lock. This separate webview
/// check covers the opposite ordering: native commit completed first, but its resolved Promise
/// callback was still queued when the synchronous lock handler cleared the UI.
export function sessionContinuationCurrent(
  capturedGeneration: number,
  liveGeneration: number,
  locked: boolean,
): boolean {
  return !locked && capturedGeneration === liveGeneration;
}

/// The value `wikiReviewDays` carries before this server's policy has been read.
///
/// Deliberately NOT zero: zero is a real policy meaning "edits publish immediately", so clearing to
/// it on a switch would grant every member the structural controls on a review-gated server until
/// its real policy arrived.
export const WIKI_REVIEW_UNKNOWN = -1;

/// Whether this member may rename or delete pages, and may create one eagerly rather than having
/// their first save queue as a proposal.
///
/// Moderators always may. Everyone else may only when the policy has been read AND says edits
/// publish directly, so an unread policy denies rather than grants.
export function mayEditWikiStructure(reviewDays: number, canModerate: boolean): boolean {
  return canModerate || reviewDays === 0;
}

/// Whether the moderation surface is actually on screen, as opposed to merely being the selected
/// tab.
///
/// The distinction is not pedantry: the inbox and the orbit view replace the whole sidebar and main
/// pane without touching the selected tab, so a moderator who opens the inbox with Moderation
/// selected would otherwise keep triggering the full every-channel corpus sweep, once per incoming
/// message, for a surface nobody is looking at.
export function moderationSurfaceOpen(view: string, inboxView: boolean, spaceOpen: boolean): boolean {
  return view === "moderation" && !inboxView && !spaceOpen;
}

/// Whether the livery editor's draft may be published to `server`.
///
/// An empty livery is byte-for-byte the payload that REMOVES a server's branding for every member,
/// and a switch empties `livery` synchronously while the settings wrench stays reachable. So a
/// draft may be published only when it was seeded from a livery actually read, for the very server
/// it is about to be sent to.
export function mayPublishLivery(
  liveryLoaded: boolean,
  draftSeededFor: number | null,
  server: number | null,
): boolean {
  return liveryLoaded && server !== null && draftSeededFor === server;
}
