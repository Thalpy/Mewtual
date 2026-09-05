/**
 * Dismissing an overlay by clicking its backdrop, without losing a drag that ended there.
 *
 * A `click` event fires on the nearest common ancestor of where the button went down and where
 * it came up. Selecting text inside an overlay's card and releasing past its edge therefore
 * delivers a click whose target is the BACKDROP, and every overlay in this app read that as
 * "the user clicked outside" and closed. Reported against the feedback window, where it also
 * threw away whatever had been typed, but it was never specific to that overlay: it happens
 * anywhere a pointer drag can leave the card, which is anywhere text can be selected.
 *
 * The fix is to require the gesture to have STARTED on the backdrop as well as ended there.
 */

/** The parts of an event these rules read, so they are decidable without a DOM. */
export type DismissTargets = {
  /** Where the pointer went down, or null if that was never observed. */
  down: unknown;
  /** The click event's target. */
  click: unknown;
  /** The backdrop element itself. */
  backdrop: unknown;
};

/**
 * Whether a click on an overlay should dismiss it.
 *
 * Both ends of the gesture must be the backdrop. A press that began on the card is a drag out
 * of it, never a dismissal, however far it travelled; and a press whose origin was never seen
 * (the pointer was already down when the overlay opened) is not treated as one either.
 */
export function backdropDismissed({ down, click, backdrop }: DismissTargets): boolean {
  return down !== null && down !== undefined && down === backdrop && click === backdrop;
}

type DismissNode = {
  addEventListener(type: string, listener: (event: Event) => void): void;
  removeEventListener(type: string, listener: (event: Event) => void): void;
};

/**
 * Svelte action: close an overlay when its backdrop is both pressed and released.
 *
 * Used in place of an inline `onclick` comparing `target` to `currentTarget`, which is the
 * comparison that cannot tell a backdrop click from a drag that ended on the backdrop.
 */
export function dismissOnBackdrop(node: DismissNode, onDismiss: () => void) {
  let dismiss = onDismiss;
  let down: unknown = null;
  const onPointerDown = (event: Event) => {
    down = event.target;
  };
  const onClick = (event: Event) => {
    const target = event.target;
    // Read and clear together: a click that is not a dismissal must not leave an origin behind
    // for the next one to match against.
    const origin = down;
    down = null;
    if (backdropDismissed({ down: origin, click: target, backdrop: node })) dismiss();
  };
  node.addEventListener("pointerdown", onPointerDown);
  node.addEventListener("click", onClick);
  return {
    update(next: () => void) {
      dismiss = next;
    },
    destroy() {
      node.removeEventListener("pointerdown", onPointerDown);
      node.removeEventListener("click", onClick);
    },
  };
}
