// Plain words for Studio saves that have not landed yet.
//
// A save record's `error` is the exact text the native bridge or the session produced. It stays on
// the record unchanged, because it is diagnostic evidence (the lock snapshot carries it, and the
// banner keeps it in a tooltip), but it is written for whoever debugs the bridge: native command
// names, retry semantics, epochs. Shown as-is it reads as noise, and noise trains people to ignore
// the one banner that matters. This module is the only place that decides what a person reads about
// a save, so the wording stays short and consistent across the banner, the sidebar and the toast.
//
// Nothing here changes what a save does. The session decides whether a record is retried on its own
// (`StudioSession.autoRetryAt`) or needs the person (retry, save as a new edit, discard); these
// helpers only describe that state.

/** Why a save has not landed, as one short sentence. Unknown texts fall back to a generic line,
 *  which for an `uncertain` save (it may have landed) must not claim it failed; the exact text is
 *  still on the record for the tooltip and diagnostics. */
export function saveProblemText(error: string, uncertain = false): string {
  const known = knownProblem(error);
  if (known) return known;
  return uncertain ? "The app could not confirm this save." : "The save did not go through.";
}

function knownProblem(error: string): string | null {
  // An exhausted pre-dispatch refusal after an earlier uncertain attempt carries this suffix. The
  // person-facing reason is the refusal itself; the uncertainty is already the record's status.
  const text = error.replace(/; an earlier attempt may already have committed.*$/s, "");
  if (/Studio storage busy|Studio actor busy|too many inline downloads|local draft Save busy/.test(text)) return "The app was busy.";
  if (/^outcome=uncertain/.test(text) || /may still complete/.test(text)) return "The app could not confirm this save.";
  // Tauri's own refusal of a command no capability grants ("<cmd> not allowed. Command not found").
  // It is raised before the command runs; it means this build is missing a permission.
  if (/not allowed\. Command not found|not allowed by ACL/i.test(text)) return "This build of the app blocked the save.";
  if (/Shared history changed|edit belongs to a retired epoch/.test(text)) return "The document changed before this edit was saved.";
  if (/object already exists|already occurs in the index/.test(text)) return "A document with this id already exists.";
  if (/Studio requires current membership|local device is not a current member/.test(text)) return "This device is no longer a member of the server.";
  if (/unknown Studio channel/.test(text)) return "This channel no longer exists.";
  if (/could not read this document|still reading this document/.test(text)) return "The document had not loaded yet.";
  if (/has not read this channel's index|could not read the channel index/.test(text)) return "The Studio list had not loaded yet.";
  if (/server for these saved edits changed/.test(text)) return "The server changed, so these edits were not sent.";
  if (/locked or changed UI session|Studio changed|Studio session changed/.test(text)) return "The app was locked during the save.";
  if (/server changed/.test(text)) return "The server reloaded during the save.";
  if (/cancelled/.test(text)) return "The save was interrupted.";
  if (/batched Apply item \d+ of \d+ was refused|was refused/.test(text)) return "The app refused this edit.";
  if (/local draft/.test(text)) return "The local draft could not be saved.";
  return null;
}

/** "Trying again in 4 s" for a scheduled automatic resend, "Trying again now" once it is due. */
export function retryText(at: number, now: number): string {
  const seconds = Math.ceil((at - now) / 1000);
  return seconds > 0 ? `Trying again in ${seconds} s` : "Trying again now";
}

/** The banner's first line: what happened and to which edit. `uncertain` is a save that may have
 *  landed without the app hearing back ("not confirmed"); otherwise it is known not to have
 *  ("not saved"). Saying which is the one distinction worth the words: only a known-unsaved edit
 *  can be saved again as a new edit. `retrying` adds "yet" while the session will resend it on its
 *  own; `others` counts the other edits waiting behind or beside it, whose own state may differ. */
export function saveHeadline(label: string, opts: { retrying: boolean; uncertain: boolean; others: number }): string {
  const state = `${opts.uncertain ? "Not confirmed" : "Not saved"}${opts.retrying ? " yet" : ""}`;
  return `${state}: ${label}${opts.others > 0 ? `, ${opts.others} more waiting` : ""}`;
}

/** How many Studio saves are outstanding, from the session's own records. */
export type SaveTally = {
  /** Every record not landed yet: queued, in flight, retrying or stuck. */
  left: number;
  /** Failed records the session will resend on its own. */
  retrying: number;
  /** Failed records only the person can move (retry, save as a new edit, discard). */
  stuck: number;
  /** Records the person has discarded on purpose since the session began (a running count). */
  discarded: number;
};

/** What the background-save toast says for a tally, with its bar from 0 to 1. The bar carries the
 *  count, so the words only say what is happening. `total` is the most records outstanding since
 *  the toast began, so the bar never runs backwards when an edit made meanwhile joins the queue.
 *  `done` means the toast has nothing left to report; `dismiss` means it should go without a word.
 *  `discardedSince` counts edits the person discarded since the toast began: a queue emptied that
 *  way was not saved, so the toast then ends silently rather than saying "saved". A stuck edit may
 *  have landed without the app hearing back, so its words claim neither "saved" nor "failed". */
export function saveToast(tally: SaveTally, total: number, discardedSince = 0): { text: string; kind: "info" | "ok" | "warn"; progress: number; done: boolean; dismiss: boolean } {
  const all = Math.max(total, tally.left, 1);
  const progress = (all - tally.left) / all;
  if (tally.left === 0) {
    return discardedSince > 0
      ? { text: "", kind: "info", progress: 1, done: true, dismiss: true }
      : { text: "Studio edits saved", kind: "ok", progress: 1, done: true, dismiss: false };
  }
  if (tally.stuck > 0) {
    const text = tally.stuck === 1 ? "A Studio edit needs you. Open the Studio to retry or discard it." : `${tally.stuck} Studio edits need you. Open the Studio to retry or discard them.`;
    return { text, kind: "warn", progress, done: true, dismiss: false };
  }
  if (tally.retrying > 0) return { text: "Studio edits not saved yet. Trying again.", kind: "info", progress, done: false, dismiss: false };
  return { text: "Saving Studio edits", kind: "info", progress, done: false, dismiss: false };
}
