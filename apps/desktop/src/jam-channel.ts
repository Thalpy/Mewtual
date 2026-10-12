// The jam instrument channel's capability seam. The mechanism is shared with every realtime call
// channel and lives in rt-admission.ts (slice C0b); this module keeps the jam names its callers and
// tests already use.
import { RtSourceChannelRegistry, type RtSourceChannel } from "./rt-admission.ts";

/**
 * Opaque capability for one exact authenticated `inst` data-channel generation.
 *
 * Event callbacks close over this object. Identity, not a caller-supplied generation number,
 * prevents an already queued callback from an older/removed channel being mistaken for current.
 */
export type JamSourceChannel = RtSourceChannel<"jam">;

export class JamSourceChannelRegistry extends RtSourceChannelRegistry<"jam"> {
  constructor() {
    super("jam");
  }
}
