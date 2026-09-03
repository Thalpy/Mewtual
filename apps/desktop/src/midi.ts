// Web MIDI plumbing: message parsing, pedal-aware note routing, device rows, and the setup
// diagnosis Settings shows. Pure (no DOM, no Svelte, no audio) so `midi.test.ts` can pin all of
// it; the only browser-shaped things here are structural types describing what a `MIDIInput`
// looks like, so a test can hand in a plain object.
//
// Why this is a module rather than a few lines inside App.svelte: a controller that "sometimes
// works" is almost never a music problem, it is a plumbing problem. The port carrying notes is
// the second of the three the device published; the permission was never granted; the keyboard
// arrived after the app did; another program is holding the port. The app can only be honest
// about which of those it is if the diagnosis is a value that can be tested, rather than prose
// baked into markup.

import { noteName } from "./melody.ts";

// --- Message parsing ---------------------------------------------------------------------------

/** Sustain pedal: the only continuous controller the app acts on. */
export const CC_SUSTAIN = 64;
/** Both "stop everything" controllers are treated the same: lift every note we believe is on. */
export const CC_ALL_SOUND_OFF = 120;
export const CC_ALL_NOTES_OFF = 123;

/**
 * General MIDI drum notes for jam-kit:v1's fixed pad order:
 * kick, snare, rim, clap, closed/open hat, low/high tom, ride, crash.
 *
 * Controllers are not required to transmit on GM channel 10. The active Pads surface is the
 * receiver-side routing decision, so the note number alone selects a pad on any routed channel.
 */
export const MIDI_JAM_PAD_NOTES = [36, 38, 37, 39, 42, 46, 45, 50, 51, 49] as const;

/** Return the jam-kit:v1 pad for a General MIDI drum note, or null for an unmapped key. */
export function midiJamPad(note: number): number | null {
  if (!Number.isInteger(note)) return null;
  const pad = MIDI_JAM_PAD_NOTES.indexOf(note as (typeof MIDI_JAM_PAD_NOTES)[number]);
  return pad === -1 ? null : pad;
}

export type MidiInstrumentAction =
  | Readonly<{ kind: "note"; note: number; on: boolean }>
  | Readonly<{ kind: "pad"; pad: number }>;

/**
 * Translate one routed MIDI note edge for the currently visible instrument surface.
 * Pads are one-shots: only a mapped note-on creates an action; releases are bookkeeping only.
 */
export function midiInstrumentAction(
  note: number,
  on: boolean,
  mode: "keys" | "pads",
): MidiInstrumentAction | null {
  if (mode === "keys") return { kind: "note", note, on };
  const pad = midiJamPad(note);
  return on && pad !== null ? { kind: "pad", pad } : null;
}

/** One decoded packet. `realtime` and `other` exist so the monitor can name what it saw. */
export type MidiMessage =
  | { kind: "noteon"; channel: number; note: number; velocity: number }
  | { kind: "noteoff"; channel: number; note: number }
  | { kind: "sustain"; channel: number; down: boolean }
  | { kind: "panic"; channel: number }
  | { kind: "realtime"; status: number }
  | { kind: "other"; channel: number; status: number };

/**
 * Decode one raw MIDI packet, or null when there is nothing usable in it (empty, a truncated
 * channel message, or a data byte with no status in front of it).
 *
 * The edge cases that actually bite, handled here on purpose:
 *
 *   * A note-on with velocity 0 IS a note-off. Controllers genuinely disagree about which they
 *     send on release, and a keyboard whose releases are all velocity-0 note-ons would hang
 *     every note it played if this were read literally.
 *   * System realtime bytes (0xF8 clock, 0xFE active sensing, 0xFF reset) can arrive between
 *     anything and carry no channel. They are named rather than dropped, because "the cable is
 *     alive but no notes are coming" is a diagnosis worth being able to show.
 *   * Running status (a data-only packet reusing the previous status byte) is deliberately not
 *     reconstructed. Web MIDI delivers whole messages, so a packet whose first byte has the high
 *     bit clear is junk from a misbehaving driver, and guessing at it would invent notes.
 */
export function parseMidiMessage(data: ArrayLike<number> | null | undefined): MidiMessage | null {
  if (!data || data.length < 1) return null;
  const first = data[0];
  if (first < 0x80) return null; // data byte with no status: not ours to interpret
  if (first >= 0xf8) return { kind: "realtime", status: first };
  if (first >= 0xf0) return { kind: "other", channel: 0, status: first }; // system common, sysex
  const status = first & 0xf0;
  const channel = first & 0x0f;
  if (status === 0x80 || status === 0x90) {
    if (data.length < 3) return null;
    const note = data[1] & 0x7f;
    const velocity = data[2] & 0x7f;
    if (status === 0x90 && velocity > 0) return { kind: "noteon", channel, note, velocity };
    return { kind: "noteoff", channel, note };
  }
  if (status === 0xb0) {
    if (data.length < 3) return null;
    const cc = data[1] & 0x7f;
    const value = data[2] & 0x7f;
    // 64 is the MIDI-wide half-way point: below it the pedal is up, at or above it is down.
    if (cc === CC_SUSTAIN) return { kind: "sustain", channel, down: value >= 64 };
    if (cc === CC_ALL_SOUND_OFF || cc === CC_ALL_NOTES_OFF) return { kind: "panic", channel };
  }
  return { kind: "other", channel, status: first };
}

/** Channels are 0-based on the wire and 1-based everywhere a human reads them. */
export const midiChannelLabel = (channel: number): string => `ch${channel + 1}`;

/** One line for the Settings monitor: what the controller just sent, spelled out. */
export function describeMidiMessage(msg: MidiMessage): string {
  switch (msg.kind) {
    case "noteon":
      return `${noteName(msg.note)} on · vel ${msg.velocity} · ${midiChannelLabel(msg.channel)}`;
    case "noteoff":
      return `${noteName(msg.note)} off · ${midiChannelLabel(msg.channel)}`;
    case "sustain":
      return `sustain ${msg.down ? "down" : "up"} · ${midiChannelLabel(msg.channel)}`;
    case "panic":
      return `all notes off · ${midiChannelLabel(msg.channel)}`;
    case "realtime":
      return `realtime 0x${msg.status.toString(16).toUpperCase()}`;
    default:
      return `status 0x${msg.status.toString(16).toUpperCase()}`;
  }
}

/**
 * Clock and active sensing arrive several times a second on some gear, and letting them into the
 * monitor would push the note you just played off the top of it. They still count as traffic,
 * which is why the caller records them separately rather than this discarding them outright.
 */
export const isMonitorWorthy = (msg: MidiMessage): boolean => msg.kind !== "realtime";

/** Newest first, oldest dropped: the monitor is a glance, not a log. */
export const MIDI_MONITOR_LINES = 8;
/**
 * `routed` records whether this line's port was allowed to play. The monitor deliberately shows
 * filtered-out ports too: watching the OTHER port light up is how someone works out which of a
 * controller's three ports actually carries the keys.
 */
export type MidiMonitorLine = { seq: number; port: string; text: string; routed: boolean };
export const pushMonitorLine = (
  lines: MidiMonitorLine[],
  line: MidiMonitorLine,
): MidiMonitorLine[] => [line, ...lines].slice(0, MIDI_MONITOR_LINES);

// --- Note routing ------------------------------------------------------------------------------

/** What the app should do to a sounding note. */
export type NoteAction = { note: number; on: boolean };

/**
 * What the router remembers between messages.
 *
 * `held` is what is physically down on the keyboard. `sustained` is what has been released but is
 * still sounding because the pedal is down. A note is never in both: releasing a key moves it
 * across, re-striking it moves it back.
 */
export type MidiRouterState = { sustain: boolean; held: Set<number>; sustained: Set<number> };

export const newMidiRouter = (): MidiRouterState => ({
  sustain: false,
  held: new Set<number>(),
  sustained: new Set<number>(),
});

/**
 * Fold one message into the router and return the note changes it implies.
 *
 * `sustainEnabled` decides whether the pedal may defer note-offs, and callers MUST pass `false`
 * while the melody unlock lock is the target. That is not a taste call: the lock's secret is
 * derived from which notes overlapped and how long each group was held, so a pedal that kept
 * notes down would silently encode a different secret from the same performance and seal a vault
 * that has no recovery path. The pedal is therefore a call-instrument feature only.
 *
 * The router is also what makes a duplicate note-on harmless: key repeat, a driver echoing its
 * own output, and two ports of one controller all deliver the same note twice.
 */
export function routeMidi(
  state: MidiRouterState,
  msg: MidiMessage,
  sustainEnabled: boolean,
): NoteAction[] {
  switch (msg.kind) {
    case "noteon": {
      if (state.sustained.delete(msg.note)) {
        // Re-struck while the pedal was holding it: re-articulate, because doing nothing would
        // make a repeated note under the pedal inaudible.
        state.held.add(msg.note);
        return [
          { note: msg.note, on: false },
          { note: msg.note, on: true },
        ];
      }
      if (state.held.has(msg.note)) return [];
      state.held.add(msg.note);
      return [{ note: msg.note, on: true }];
    }
    case "noteoff": {
      if (!state.held.delete(msg.note)) return []; // never started here: nothing to lift
      if (sustainEnabled && state.sustain) {
        state.sustained.add(msg.note);
        return [];
      }
      return [{ note: msg.note, on: false }];
    }
    case "sustain": {
      if (msg.down === state.sustain) return []; // pedals resend position; only edges matter
      state.sustain = msg.down;
      if (msg.down) return [];
      return drainSustained(state); // pedal up always empties, even if sustain was since disabled
    }
    case "panic":
      return releaseAllNotes(state);
    default:
      return [];
  }
}

const drainSustained = (state: MidiRouterState): NoteAction[] => {
  const lifted = [...state.sustained].sort((a, b) => a - b);
  state.sustained.clear();
  return lifted.map((note) => ({ note, on: false }));
};

/**
 * Lift everything the router believes is sounding and forget the pedal. Called when the device
 * that was playing goes away, when the surface receiving notes changes, and by the panic button
 * in Settings, because a stuck note outlives whatever caused it.
 */
export function releaseAllNotes(state: MidiRouterState): NoteAction[] {
  const notes = [...new Set([...state.held, ...state.sustained])].sort((a, b) => a - b);
  state.held.clear();
  state.sustained.clear();
  state.sustain = false;
  return notes.map((note) => ({ note, on: false }));
}

// --- Devices -----------------------------------------------------------------------------------

/** The parts of a `MIDIInput` this module needs, structurally, so tests need no browser. */
export type MidiPortLike = {
  id: string;
  name?: string | null;
  manufacturer?: string | null;
  /** "connected" | "disconnected": whether the hardware is present. */
  state?: string;
  /** "open" | "closed" | "pending": whether messages are actually flowing to us. */
  connection?: string;
};

export type MidiDeviceRow = {
  id: string;
  label: string;
  maker: string;
  /** The hardware is present. */
  connected: boolean;
  /** The port is open, so its messages reach the app. */
  listening: boolean;
  /** It passes the input filter, so its notes are played rather than dropped. */
  routed: boolean;
};

export const midiPortLabel = (port: MidiPortLike): string =>
  (port.name ?? "").trim() || "Unnamed MIDI input";

/**
 * Is this port allowed to play? `preferred` is "" for "every connected input", which is the
 * default and the right answer for almost everyone; anything else is one saved port identity.
 *
 * Matched by id and then by name, because Chromium's port ids are stable for a device on one
 * install but not across reinstalls or a profile reset. A saved preference that quietly stops
 * matching is exactly the "it just stopped working one day" failure this panel exists to end,
 * and the name is the human's idea of the device anyway.
 */
export const isPortRouted = (port: MidiPortLike, preferred: string): boolean =>
  !preferred || port.id === preferred || midiPortLabel(port) === preferred;

/**
 * Rows for the Settings list: connected first, then by label, id as the tiebreak so the order
 * cannot flicker between scans. An absent `state` counts as connected, which is what a browser
 * that only reports live ports gives us.
 */
export function deviceRows(ports: Iterable<MidiPortLike>, preferred = ""): MidiDeviceRow[] {
  const rows = [...ports].map((port) => ({
    id: port.id,
    label: midiPortLabel(port),
    maker: (port.manufacturer ?? "").trim(),
    connected: port.state !== "disconnected",
    listening: port.state !== "disconnected" && port.connection === "open",
    routed: isPortRouted(port, preferred),
  }));
  rows.sort(
    (a, b) =>
      Number(b.connected) - Number(a.connected) ||
      a.label.localeCompare(b.label) ||
      a.id.localeCompare(b.id),
  );
  return rows;
}

/** Connected inputs whose notes the app will actually play. */
export const routedDevices = (rows: MidiDeviceRow[]): MidiDeviceRow[] =>
  rows.filter((row) => row.connected && row.routed);

// --- Diagnosis ---------------------------------------------------------------------------------

export type MidiPermission = "granted" | "denied" | "prompt" | "unknown";

/** Everything `midiStatus` is allowed to reason from. */
export type MidiEnvironment = {
  /** `navigator.requestMIDIAccess` exists in this webview. */
  supported: boolean;
  /** Access has been asked for at least once this session. */
  requested: boolean;
  /** A request is in flight. */
  busy: boolean;
  /** Message from a rejected request, "" when there was none. */
  failure: string;
  permission: MidiPermission;
  devices: MidiDeviceRow[];
};

export type MidiStatusCode =
  | "unsupported"
  | "scanning"
  | "off"
  | "denied"
  | "error"
  | "no-devices"
  | "not-routed"
  | "ready";

export type MidiStatus = {
  code: MidiStatusCode;
  /** How the row is tinted; idle means "nothing wrong, nothing running". */
  level: "ok" | "warn" | "bad" | "idle";
  title: string;
  detail: string;
};

/**
 * Turn the environment into one honest sentence.
 *
 * Ordered most-fundamental first, so the reason shown is the one that must be fixed before any of
 * the others can even be observed: no MIDI support at all, then a refused permission, then a
 * failed request, then no hardware, then hardware the routing filter is throwing away.
 *
 * Port `connection` deliberately does not feed the verdict. It passes through "closed" for a
 * moment after a port is wired, so treating it as authoritative would flash a scary "not
 * delivering" banner every time a device is plugged in. It stays a per-row detail, and the
 * "another program is holding the port" case is covered by the troubleshooting list instead.
 */
export function midiStatus(env: MidiEnvironment): MidiStatus {
  if (!env.supported) {
    return {
      code: "unsupported",
      level: "bad",
      title: "No MIDI support here",
      detail:
        "This build reaches controllers through the system WebView2 runtime, and this one offers no MIDI at all. Update WebView2 (or Windows) and restart Mewtual.",
    };
  }
  if (env.busy) {
    return {
      code: "scanning",
      level: "idle",
      title: "Looking for controllers",
      detail: "Asking the system what is plugged in.",
    };
  }
  if (!env.requested) {
    return {
      code: "off",
      level: "idle",
      title: "MIDI input is off",
      detail:
        "Nothing is listening yet. Turn it on to let a controller play the melody lock and the call instrument; Mewtual only reads notes and never sends anything back to your device.",
    };
  }
  if (env.permission === "denied") {
    return {
      code: "denied",
      level: "bad",
      title: "MIDI access was refused",
      detail:
        "The webview is blocking MIDI for Mewtual. Restart the app and allow the prompt when it appears: a dismissed prompt is remembered as a refusal.",
    };
  }
  if (env.failure) {
    return { code: "error", level: "bad", title: "MIDI could not start", detail: env.failure };
  }
  const connected = env.devices.filter((row) => row.connected);
  if (!connected.length) {
    return {
      code: "no-devices",
      level: "warn",
      title: "No controller found",
      detail:
        "Nothing is present that the system calls a MIDI input. Connect it, switch it on, then rescan: USB controllers are usually recognised a second or two after being plugged in.",
    };
  }
  const routed = routedDevices(env.devices);
  if (!routed.length) {
    return {
      code: "not-routed",
      level: "warn",
      title: "Every input is filtered out",
      detail:
        "Input routing points at a device that is not connected, so nothing that IS connected is allowed to play. Set routing back to every connected input.",
    };
  }
  return {
    code: "ready",
    level: "ok",
    title: routed.length === 1 ? "Listening" : `Listening to ${routed.length} inputs`,
    detail: `${routed.map((row) => row.label).join(", ")}. Play a key: it should appear in the monitor below.`,
  };
}

export type MidiHelpStep = { title: string; detail: string };

/** The happy path, in the order it has to happen. */
export const MIDI_SETUP_STEPS: MidiHelpStep[] = [
  {
    title: "Connect the controller first",
    detail:
      "USB straight into the machine rather than through an unpowered hub, and switched on. A Bluetooth controller has to be paired in Windows settings before Mewtual can see it at all: Mewtual only ever sees what the system already has.",
  },
  {
    title: "Turn MIDI input on and allow the prompt",
    detail:
      "The webview asks once. Dismissing that prompt counts as a refusal and is remembered, so if you missed it, restart Mewtual and accept it next time.",
  },
  {
    title: "Check the device list",
    detail:
      "Your keyboard should be listed and marked routed. Controllers routinely publish two or three ports and only one of them carries the keys, so if notes never arrive, pick the specific port under input routing.",
  },
  {
    title: "Play a key and watch the monitor",
    detail:
      "Every message appears with its note name, velocity and channel. If the monitor moves, the plumbing is finished and the rest is only about which surface is open.",
  },
  {
    title: "Play it somewhere",
    detail:
      "Notes sound on the melody unlock lock (lock screen, Melody tab) and in the call instrument drawer (voice stage, Instruments). Anywhere else they are still received and shown here, they just have nothing to play.",
  },
];

/** The failures worth naming, because each has a different fix. */
export const MIDI_FIXES: MidiHelpStep[] = [
  {
    title: "It was plugged in after Mewtual started",
    detail:
      "Hot-plugging is handled and the list updates by itself, but a device still enumerating when the app asked can be missed. Rescan is the whole fix and costs nothing.",
  },
  {
    title: "The list shows it, but the monitor stays empty",
    detail:
      "Something else has the port open exclusively. DAWs, vendor control panels and MIDI loopback tools all do this on Windows. Close that program, then rescan.",
  },
  {
    title: "Only some keys, or only one octave, does anything",
    detail:
      "That is the on-screen board showing two octaves, not the controller. Every note you send is played and named in the monitor even when it sits outside the drawn register.",
  },
  {
    title: "Notes hang on",
    detail:
      "Usually a note-off lost to a yanked cable, or a pedal still down. Release stuck notes lifts everything Mewtual believes is sounding and forgets the pedal.",
  },
  {
    title: "It worked yesterday and not today",
    detail:
      "If input routing is pinned to one port, a reinstalled driver can rename or renumber it. Every connected input is the setting that survives that.",
  },
];
