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

/// Point the session at the channel on screen. A change closes the open document and drops
/// the previous channel's pending work; the same scope is a no-op.
export function setStudioScope(server: number | null, channel: string): void {
  if (!session) return;
  const next = server === null || !channel ? null : { server, channel };
  const cur = session.scope;
  const same = (cur === null && next === null) || (cur !== null && next !== null && cur.server === next.server && cur.channel === next.channel);
  if (same) return;
  session.setScope(next);
  studio.selected = "";
}

/// Lock or session end: forget everything held in the webview, including unsaved pixels.
export function resetStudio(): void {
  session?.reset();
  restoredDraft = null;
  studio.selected = "";
  studio.rev++;
}

/** Ordinary navigation/close waits for the currently mounted drawing and definite Save receipts. */
export async function saveStudio(): Promise<void> { await session?.saveBeforeLeaving(); }
export function claimStudioNavigation(): () => boolean { return session?.claimNavigation() ?? (() => true); }
export function beginStudioClose(): void { session?.beginClose(); }
export function endStudioClose(): void { session?.endClose(); }
export function studioHasPending(): boolean {
  try { session?.stageDrawing(); return !!session?.saves.length; }
  catch { return true; } // the awaited Save path reports the refusal without starting navigation
}
/// The server the Studio holds unsaved work for, and whether that work is a restored draft still
/// waiting for its channel (see StudioSession.workServer). Stages the drawing first, like
/// studioHasPending; a stroke that could not be staged counts as the scope's work.
export function studioWork(): { server: number; restored: boolean } | null {
  if (!session) return restoredDraft ? { server: restoredDraft.scope.server, restored: true } : null;
  let unstaged = false;
  try { session.stageDrawing(); } catch { unstaged = true; }
  return session.workServer(unstaged);
}
/// A server's native session ended; see StudioSession.retireServer. Also covers a restored draft
/// still waiting here for the session to be created.
export function retireStudioServer(server: number, keep: boolean): "none" | "kept" | "partial" | "dropped" {
  let outcome: "none" | "kept" | "partial" | "dropped" = "none";
  if (!keep && restoredDraft?.scope.server === server) { restoredDraft = null; outcome = "dropped"; }
  if (!session) return outcome;
  const scoped = session.scope?.server === server;
  const retired = session.retireServer(server, keep);
  if (scoped) { studio.selected = ""; studio.rev++; } // the retired scope's document is gone from the surface
  return retired === "none" ? outcome : retired;
}
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
  session = null;
  studio.selected = "";
  studio.rev++;
}

export function bump(): void {
  studio.rev++;
}
