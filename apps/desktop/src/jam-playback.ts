import type { JamTakeEvent } from "./jam-contract.ts";

/**
 * Decide whether a recorded event may reach the synth while the call is deafened.
 *
 * Local take previews are deliberately independent of call Deafen. A room-synchronised jukebox
 * take is not: new notes and drum hits stay silent, but note-offs must still run so a note that
 * began before Deafen cannot resurrect when the receiver opens the master gate again.
 */
export function shouldDispatchTakeEvent(
  event: JamTakeEvent,
  deckPlayback: boolean,
  callDeafened: boolean,
): boolean {
  if (!deckPlayback || !callDeafened) return true;
  return !("d" in event) && event.on === 0;
}
