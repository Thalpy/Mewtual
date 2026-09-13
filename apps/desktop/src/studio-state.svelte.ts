// Shared studio state for the two places that show it: the contextual sidebar (StudioNav) and
// the content surface (Studio). One connected session per app session, pointed at the channel
// on screen; `rev` is bumped after every change so derived views re-read the plain-data session.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { StudioSession } from "./studio-session.ts";
import type { Hex32, StudioIpc } from "./studio-native.ts";

const ipc: StudioIpc = {
  invoke,
  listen: (name, handler) => listen(name, handler),
};

export const studio = $state({
  selected: "" as Hex32 | "",
  rev: 0,
});

let session: StudioSession | null = null;
let detach: (() => void) | null = null;

/// The one session, created on first use. The identity is display context only (the backend
/// derives every author); updating it never resets the channel or its pending saves.
export function ensureStudio(me: string): StudioSession {
  if (session) {
    session.me = me;
    return session;
  }
  session = new StudioSession({ ipc, me });
  session.onChange(() => { studio.rev++; });
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
  studio.selected = "";
  studio.rev++;
}

export function detachStudio(): void {
  detach?.();
  detach = null;
}

export function bump(): void {
  studio.rev++;
}
