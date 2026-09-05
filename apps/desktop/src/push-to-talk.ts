/**
 * Push to talk: the microphone transmits only while a chosen key is held.
 *
 * ## Why this is in-app rather than system-wide
 *
 * A true global push-to-talk needs a low-level keyboard hook, because the point is to *observe*
 * a key that another window is using without taking it away. Tauri's global-shortcut plugin is
 * the wrong primitive: a registered global shortcut is exclusive, so binding one to push to talk
 * would stop that key working in the game you are playing while you use it. A keyboard hook in a
 * privacy-focused app is its own decision with its own review, so it is deliberately not made
 * here by accident.
 *
 * What this gives you is the thing the request actually asked for, which was a microphone that
 * is not live the whole call, while Mewtual has focus. The settings surface says so plainly
 * rather than letting it look broken when the key does nothing in another window.
 */

/** How the microphone is gated while in a call. */
export type MicMode = "open" | "ptt";

/** A stored push-to-talk configuration. */
export type PushToTalkSettings = {
  mode: MicMode;
  /** A `KeyboardEvent.code`, or empty when no key has been bound yet. */
  key: string;
};

export const DEFAULT_PUSH_TO_TALK: PushToTalkSettings = { mode: "open", key: "" };

/** Read a persisted setting without letting a junk value change how the microphone behaves. */
export function parsePushToTalk(raw: unknown): PushToTalkSettings {
  const parsed = raw && typeof raw === "object" ? raw as Partial<PushToTalkSettings> : {};
  const key = typeof parsed.key === "string" && parsed.key.length <= 32 ? parsed.key : "";
  // A push-to-talk mode with no key bound would mute the microphone for the whole call with no
  // way to open it, which reads as a broken app rather than as a setting. It needs both halves.
  const mode = parsed.mode === "ptt" && key ? "ptt" : "open";
  return { mode, key };
}

/**
 * Whether the microphone should be transmitting.
 *
 * The explicit mute always wins: pressing the key while muted does not transmit, because the
 * mute button is a statement about the whole call and push to talk is only about when an
 * unmuted microphone is live.
 */
export function micTransmitting(
  settings: PushToTalkSettings,
  muted: boolean,
  keyHeld: boolean,
): boolean {
  if (muted) return false;
  if (settings.mode !== "ptt" || !settings.key) return true;
  return keyHeld;
}

/** Keys that may not be bound: they are how you leave a dialog or reach the rest of the app. */
const RESERVED_KEYS = new Set(["Escape", "Tab", "Enter", "NumpadEnter", "MetaLeft", "MetaRight"]);

/** Whether a captured `KeyboardEvent.code` may be used as the push-to-talk key. */
export function bindableKey(code: string): boolean {
  return !!code && code.length <= 32 && !RESERVED_KEYS.has(code);
}

/** The parts of a key event and its target these rules read. */
export type KeyContext = {
  code: string;
  /** Whether the event happened inside something the user types into. */
  typing: boolean;
  /** Whether a call is in progress. */
  inCall: boolean;
};

/**
 * Whether a key event should move the push-to-talk gate.
 *
 * Typing is the case that matters. Binding a letter and then writing a message would otherwise
 * open the microphone on every occurrence of it, so a key that lands in a text field is a
 * keystroke rather than a talk button, whatever it is bound to.
 */
export function pushToTalkEvent(settings: PushToTalkSettings, context: KeyContext): boolean {
  return settings.mode === "ptt"
    && !!settings.key
    && context.inCall
    && !context.typing
    && context.code === settings.key;
}

/** A readable name for a `KeyboardEvent.code`, for the button that shows the binding. */
export function keyLabel(code: string): string {
  if (!code) return "not set";
  return code
    .replace(/^Key/, "")
    .replace(/^Digit/, "")
    .replace(/^Numpad/, "Numpad ")
    .replace(/^Arrow/, "Arrow ")
    .replace(/(Left|Right)$/, " $1")
    .trim();
}
