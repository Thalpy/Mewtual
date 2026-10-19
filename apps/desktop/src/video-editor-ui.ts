/**
 * Words and shortcuts shared by the pre-upload video editor and the Keybinds settings page.
 *
 * Kept apart from the component so both read one list: the settings page cannot describe a key
 * the editor does not handle, or miss one it does (`video-editor.test.ts` checks the list against
 * the editor's real key handling).
 */
import type { VideoEditorContext } from "./video-export.ts";

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
  { keys: "Delete", action: "Remove or restore the piece" },
  { keys: "I / O", action: "Trim the start / end to the playhead" },
  { keys: "Ctrl+Z / Ctrl+Shift+Z", action: "Undo / redo (Ctrl+Y also redoes)" },
  { keys: "M", action: "Mute the preview" },
  { keys: "Ctrl+Enter", action: "Post now (embed, or upload, in the wiki and Files)" },
  { keys: "Esc", action: "Leave blade mode, cancel an export, or discard the clip (asks first if edited)" },
];
