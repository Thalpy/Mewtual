/**
 * "Add as is": holding Shift through the gesture that starts an upload skips the video editor.
 *
 * The moment that counts is the gesture, not the moment the files arrive:
 *
 * - Drop: the `drop` event carries `shiftKey` itself; nothing to remember (see the callers).
 * - Picker: the file dialog is modal, and by the time `change` fires the person has long let go
 *   of Shift, so Shift is read from the click on the picker's label and remembered until the pick
 *   arrives. Only a click aimed at the label itself counts: the label forwards its own click to
 *   the hidden input, and that one bubbles back up through the label as a second click event.
 *
 * Paste has no Shift form. A `paste` event carries no modifier state, and the obvious chord,
 * Ctrl+Shift+V, is "paste as plain text" in the webview, which delivers text only: a screenshot
 * pasted that way never arrives as a file. A pasted video still skips editing in one click, since
 * the editor's main button sends an untouched clip as it is.
 *
 * A reading is taken once: a remembered Shift applies to the next pick only, so a cancelled
 * Shift+pick can never make a later ordinary pick skip the editor.
 */

type PickClick = { shiftKey: boolean; target: unknown; currentTarget: unknown };
type Listening = {
  addEventListener(type: string, listener: (event: Event) => void): void;
  removeEventListener(type: string, listener: (event: Event) => void): void;
};

export class ShiftIntent {
  #pick = false;

  /**
   * A click on a picker's label. Every such click overwrites the reading, so a cancelled Shift+pick
   * is forgotten by the next ordinary click. The label's forwarded click (target: the input) is
   * ignored: it is the same gesture arriving a second time. A click on anything else inside the
   * label (its text, an icon) is the person's own click and counts.
   */
  notePickClick(e: PickClick): void {
    if ((e.target as { tagName?: string } | null)?.tagName === "INPUT") return;
    this.#pick = e.shiftKey;
  }

  /** Whether the pick now arriving was started with Shift. Reading it clears it. */
  takePick(): boolean {
    const shift = this.#pick;
    this.#pick = false;
    return shift;
  }
}

/**
 * Svelte action for a picker's `<label>`: every click on it goes to `intent.notePickClick`.
 * An action rather than an `onclick` attribute because a click listener on a label is flagged by
 * the accessibility lint, and this one adds no behaviour a keyboard user would miss (the picker
 * opens exactly as before; Shift only changes what happens to the files picked).
 *
 * The label must not be text-selectable (`user-select: none`): a Shift+click on selectable text
 * extends the selection, and the browser then does not forward the click to the input at all.
 */
export function shiftPick(node: Listening, intent: ShiftIntent) {
  let current = intent;
  const onClick = (event: Event) => current.notePickClick(event as unknown as PickClick);
  node.addEventListener("click", onClick);
  return {
    update(next: ShiftIntent) { current = next; },
    destroy() { node.removeEventListener("click", onClick); },
  };
}
