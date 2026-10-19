// Shared studio state for the two places that show it: the contextual sidebar (StudioNav) and
// the content surface (Studio). One connected session per app session, pointed at the channel
// on screen; `rev` is bumped after every change so derived views re-read the plain-data session.
//
// The bump is deferred to a microtask and coalesced. A session call made from inside an effect
// (open, watch, want) therefore never makes that effect depend on the revision it changes, and
// a burst of session changes costs one re-read.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { StudioSession } from "./studio-session.ts";
import type { Hex32, StudioIpc } from "./studio-native.ts";
import type { SaveTally } from "./studio-save-words.ts";
import { decodeStudioDraft, encodeStudioDraft, type StudioDraft } from "./studio-draft.ts";

const tauriIpc: StudioIpc = {
  invoke,
  listen: (name, handler) => listen(name, handler),
};
let ipcOverride: StudioIpc | null = null;
let testAutosaveMs = 0;

export const studio = $state({
  selected: "" as Hex32 | "",
  rev: 0,
});

let session: StudioSession | null = null;
let detach: (() => void) | null = null;
let bumpQueued = false;
let restoredDraft: StudioDraft | null = null;
/** The scope a switch was deferred for, as `server:channel`, until it is made or abandoned. */
let deferredScope: string | null = null;

function scheduleBump(): void {
  if (bumpQueued) return;
  bumpQueued = true;
  queueMicrotask(() => {
    bumpQueued = false;
    studio.rev++;
  });
}

/// The one session, created on first use. The identity is display context only (the backend
/// derives every author); updating it never resets the channel or its pending saves.
export function ensureStudio(me: string): StudioSession {
  if (session) {
    session.me = me;
    return session;
  }
  session = new StudioSession({ ipc: ipcOverride ?? tauriIpc, me, autosaveMs: ipcOverride ? testAutosaveMs : 5000 });
  session.onChange(scheduleBump);
  if (restoredDraft) { session.restoreDraft(restoredDraft); restoredDraft = null; }
  void session.attach().then((d) => { detach = d; });
  return session;
}

function sameScope(server: number | null, channel: string): boolean {
  const next = server === null || !channel ? null : { server, channel };
  const cur = session?.scope ?? null;
  return (cur === null && next === null) || (cur !== null && next !== null && cur.server === next.server && cur.channel === next.channel);
}

/// Point the session at the channel on screen. A change closes the open document; the same scope
/// is a no-op. While the session still holds edits of the channel it is on (they were handed to the
/// background Save when the member left, and have not all landed), it stays on that channel: they
/// are sent now, and the switch waits for the last of them. The surfaces show that wait
/// (`studioScopeWaiting`) and ask again whenever the session changes, so the switch happens on its
/// own. Returns whether the session is now on the requested channel.
export function setStudioScope(server: number | null, channel: string): boolean {
  if (!session) return false;
  if (sameScope(server, channel)) { deferredScope = null; return true; }
  const key = `${server}:${channel}`;
  // While the window closes, the close owns the session: a switch now would throw inside the
  // surfaces' effects. Wait like a draining queue does; a cancelled close re-asks.
  if (session.closing) return false;
  if (session.retainsWork) {
    deferredScope = key;
    try { session.sendBeforeLeaving(); } catch { /* nothing is mounted to stage; the queue is already sent */ }
    return false;
  }
  // A switch this module deferred is arriving now: it must not retire App routes the member began
  // while it waited (see StudioSession.leaseGeneration).
  const deferred = deferredScope === key;
  deferredScope = null;
  session.setScope(server === null || !channel ? null : { server, channel }, { deferred });
  studio.selected = "";
  return true;
}

/// The Studio is waiting to show this channel until the previous channel's edits have saved (or
/// until a window close in progress lets go of the session).
export function studioScopeWaiting(server: number | null, channel: string): boolean {
  return !!session && !sameScope(server, channel) && (session.retainsWork || session.closing);
}

/// Lock or session end: forget everything held in the webview, including unsaved pixels.
export function resetStudio(): void {
  session?.reset();
  restoredDraft = null; deferredScope = null;
  studio.selected = "";
  studio.rev++;
}

/** Window close waits for the mounted drawing and every definite Save receipt. */
export async function saveStudio(): Promise<void> { await session?.saveBeforeLeaving(); }
/** Leaving the Studio surface: stage the mounted drawing and hand every edit to the background
 * Save, without waiting for any receipt. Throws, and the caller must stay, only when the drawing
 * could not be staged. */
export function sendStudio(): void { session?.sendBeforeLeaving(); }
export function claimStudioNavigation(): () => boolean { return session?.claimNavigation() ?? (() => true); }
export function beginStudioClose(): void { session?.beginClose(); }
export function endStudioClose(): void { session?.endClose(); }
/** Whether leaving must hand anything over first (an unstaged stroke, or an edit still waiting for
 * its deadline). Edits already sending, retrying or failed never keep anyone in the Studio. */
export function studioHasPending(): boolean {
  try { session?.stageDrawing(); return !!session?.hasUnsentWork(); }
  catch { return true; } // the hand-off reports the staging refusal without starting navigation
}
/** Outstanding Studio saves, for the background-save toast; null before the Studio first loads.
 * `generation` changes on a lock or scope reset: the queue was then snapshotted or dropped, not
 * saved, so a watcher must stop rather than read the empty queue as success. */
export function studioSaveTally(): { tally: SaveTally; generation: number } | null {
  return session ? { tally: session.saveTally(), generation: session.generation } : null;
}
/** Call `fn` after every session change; returns the unsubscribe. A no-op before the first load. */
export function onStudioChange(fn: () => void): () => void { return session?.onChange(fn) ?? (() => {}); }
export function studioDraftSnapshot(): string | null {
  session?.stageDrawing();
  return session?.draftSnapshot() ?? (restoredDraft ? encodeStudioDraft(restoredDraft) : null);
}
export function restoreStudioDraft(json: string | null): void {
  if (!json) return;
  const draft = decodeStudioDraft(json);
  if (session) session.restoreDraft(draft); else restoredDraft = draft;
}

export function detachStudio(): void {
  detach?.();
  detach = null;
}

/// Test seam: the bridge the next session is built on. Disposes the current session so the
/// surfaces build a fresh one against it.
export function useStudioIpc(ipc: StudioIpc | null, autosaveMs = 0): void {
  disposeStudio();
  ipcOverride = ipc;
  testAutosaveMs = autosaveMs;
}

export function disposeStudio(): void {
  detachStudio();
  session?.reset();
  session = null; deferredScope = null;
  studio.selected = "";
  studio.rev++;
}

export function bump(): void {
  studio.rev++;
}
