/**
 * Words and shortcuts shared by the pre-upload video editor and the Keybinds settings page.
 *
 * Kept apart from the component so both read one list: the settings page cannot describe a key
 * the editor does not handle, or miss one it does (`video-editor.test.ts` checks the list against
 * the editor's real key handling).
 */
import type { VideoEditorContext } from "./video-export.ts";
import type { AudioItem, Overlay, Placement, VideoLayer } from "./clip-doc.ts";

export type EditorWords = {
  /** The editor's one action button, lower case: "post now", "embed", "upload". */
  readonly action: string;
  /** Its verb, capitalised for tooltips: "Post", "Embed", "Upload". */
  readonly Verb: string;
  /** What happened to a file that went through: "posted", "embedded", "shared". */
  readonly done: string;
  /**
   * Whether the action also sends: in the chat and announcement composers it attaches the video
   * and sends the message there and then. The wiki and Files have nothing to send.
   */
  readonly posts: boolean;
};

/** How the editor names its one action for each place a video can be added. */
export function editorWords(context: VideoEditorContext): EditorWords {
  switch (context) {
    case "chat":
    case "status":
      return { action: "post now", Verb: "Post", done: "posted", posts: true };
    case "wiki":
      return { action: "embed", Verb: "Embed", done: "embedded", posts: false };
    case "files":
      return { action: "upload", Verb: "Upload", done: "shared", posts: false };
  }
}

export type EditorShortcut = { readonly keys: string; readonly action: string };

/**
 * Every shortcut the editor offers, plus the Shift gestures that skip it. Shown on the Keybinds
 * page. These are fixed, like the app's other navigation shortcuts listed there.
 */
export const VIDEO_EDITOR_SHORTCUTS: readonly EditorShortcut[] = [
  { keys: "Shift + drop / Shift+click 📎", action: "Add a video as is, without opening the editor" },
  { keys: "Space", action: "Play / pause the preview" },
  { keys: "← / →", action: "Step 0.1 s (with Shift: 1 s)" },
  { keys: "Home / End", action: "Jump to the trim start / end" },
  { keys: "B", action: "Blade: click the timeline to cut" },
  { keys: "S", action: "Split at the playhead" },
  { keys: "Delete", action: "Delete the selected text, sticker, sound or layer; otherwise remove or restore the piece" },
  { keys: "I / O", action: "Trim the start / end to the playhead" },
  { keys: "R", action: "Record motion: play, and drag the selected text or sticker to animate it" },
  { keys: "L", action: "Show or hide the library" },
  { keys: "Ctrl+Z / Ctrl+Shift+Z", action: "Undo / redo (Ctrl+Y also redoes)" },
  { keys: "M", action: "Mute the preview" },
  { keys: "Ctrl+Enter", action: "Post now (embed, or upload, in the wiki and Files)" },
  { keys: "Esc", action: "Stop recording, leave blade mode, deselect, cancel an export, or discard the clip (asks first if edited)" },
];

// --- Clip maker panels ----------------------------------------------------------------------------

/** The library's tabs. */
export type LibraryTab = "text" | "stickers" | "music" | "sounds" | "video";
/** What a file picked from a library tab is meant to become. */
export type FileIntent = "image" | "music" | "sound" | "video";
export const FILE_INTENTS: readonly FileIntent[] = ["image", "music", "sound", "video"];
/** Reading the channel's song list: not asked yet, reading, read, or failed (until asked again). */
export type SongsStatus = "idle" | "loading" | "done" | "error";
/** A Studio song's state in the library while it is rendered for the clip. */
export type SongState = { readonly state: "rendering" | "ready" | "error"; readonly message?: string; readonly seconds?: number };

/** What the timeline and stage have selected. A piece is a stretch of the clip between cuts. */
export type ClipSelection =
  | { readonly kind: "piece"; readonly index: number }
  | { readonly kind: "overlay"; readonly id: string }
  | { readonly kind: "audio"; readonly id: string }
  | { readonly kind: "layer"; readonly id: string }
  | null;

/** Text colours offered for plain text. Effects that colour their own letters ignore them. */
export const TEXT_COLOURS: readonly { readonly hex: string; readonly name: string }[] = [
  { hex: "#ffffff", name: "White" }, { hex: "#ffe66b", name: "Yellow" }, { hex: "#ef6f63", name: "Red" },
  { hex: "#5ec9f0", name: "Blue" }, { hex: "#6bf08a", name: "Green" }, { hex: "#f07ad8", name: "Pink" },
];

/** What the inspector row shows controls for. */
export type InspectorTarget =
  | { readonly kind: "overlay"; readonly overlay: Overlay; readonly placement: Placement; readonly name: string }
  | { readonly kind: "audio"; readonly item: AudioItem; readonly name: string; readonly origin: string }
  | { readonly kind: "layer"; readonly layer: VideoLayer; readonly label: string; readonly name: string; readonly first: boolean; readonly last: boolean }
  | { readonly kind: "base"; readonly layer: VideoLayer; readonly name: string; readonly hasAudio: boolean };

/** The word a new text overlay starts with; selected at once so typing replaces it. */
export const NEW_TEXT = "Hello!";
