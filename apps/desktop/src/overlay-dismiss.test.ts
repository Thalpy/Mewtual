import { test } from "node:test";
import assert from "node:assert/strict";

import { backdropDismissed, dismissOnBackdrop } from "./overlay-dismiss.ts";

const backdrop = { name: "backdrop" };
const card = { name: "card" };

test("a drag that merely ended on the backdrop is not a dismissal", () => {
  // The reported bug: press inside the card to select text, release past its edge. The click
  // fires on the nearest common ancestor, which is the backdrop, and the overlay closed and
  // threw away what had been typed.
  assert.equal(backdropDismissed({ down: card, click: backdrop, backdrop }), false);
});

test("pressing and releasing the backdrop still dismisses", () => {
  assert.equal(backdropDismissed({ down: backdrop, click: backdrop, backdrop }), true);
});

test("a click that never touches the backdrop is not a dismissal", () => {
  assert.equal(backdropDismissed({ down: card, click: card, backdrop }), false);
  assert.equal(backdropDismissed({ down: backdrop, click: card, backdrop }), false);
  // A pointer already down when the overlay opened has no observed origin, so the release is
  // not treated as a deliberate backdrop click either.
  assert.equal(backdropDismissed({ down: null, click: backdrop, backdrop }), false);
  assert.equal(backdropDismissed({ down: undefined, click: backdrop, backdrop }), false);
});

test("the action dismisses only on a press and release of the backdrop itself", () => {
  const listeners = new Map<string, (event: Event) => void>();
  let dismissed = 0;
  const node = {
    addEventListener: (type: string, fn: (event: Event) => void) => void listeners.set(type, fn),
    removeEventListener: (type: string) => void listeners.delete(type),
  };
  const handle = dismissOnBackdrop(node, () => { dismissed += 1; });
  const down = (target: unknown) => listeners.get("pointerdown")!({ target } as unknown as Event);
  const click = (target: unknown) => listeners.get("click")!({ target } as unknown as Event);

  down(node);
  click(node);
  assert.equal(dismissed, 1, "a real backdrop click closes it");

  down(card);
  click(node);
  assert.equal(dismissed, 1, "a drag out of the card does not");

  // The origin is consumed by each click, so the drag above cannot leave anything behind that a
  // later stray click matches against.
  click(node);
  assert.equal(dismissed, 1, "a click with no press before it does nothing");

  handle.update(() => { dismissed += 10; });
  down(node);
  click(node);
  assert.equal(dismissed, 11, "the newest callback is the one that runs");

  handle.destroy();
  assert.equal(listeners.size, 0, "and it lets go of the node");
});
