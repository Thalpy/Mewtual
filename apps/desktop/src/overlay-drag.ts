/**
 * Dragging a floating panel around the window, without letting it leave.
 *
 * The voice stage is docked top or bottom and centred horizontally, which is fine until it sits
 * on top of the thing you are reading. Dragging it is the request; keeping it reachable is the
 * part that has to be designed, because a panel dragged off the edge cannot be dragged back and
 * a window resized smaller can strand one that was perfectly placed a moment earlier. Both are
 * the same bug as the off-screen collapse button, so both are answered here rather than left to
 * whoever notices first.
 */

export type Point = { x: number; y: number };
export type Size = { width: number; height: number };

/**
 * How much of the panel must remain on screen along each axis.
 *
 * Enough to grab: the drag handle is the panel's header, so what has to stay reachable is a
 * piece of that header wide enough to press.
 */
export const MIN_VISIBLE_PX = 64;

/**
 * The area a panel may occupy: the window, minus the app's own fixed title bar at the top.
 * A panel is never allowed under the title bar, because the header would be unclickable there.
 */
export type DragBounds = Size & { insetTop: number };

/**
 * Move `position` back inside `bounds`, keeping at least [`MIN_VISIBLE_PX`] of the panel
 * reachable on every edge.
 *
 * Horizontally a panel may hang off either side, so long as a graspable strip remains. Vertically
 * it may hang off the bottom but never off the top: the header is the handle, and a header above
 * the title bar is a panel that can never be moved again.
 */
export function clampToBounds(position: Point, panel: Size, bounds: DragBounds): Point {
  const visible = Math.min(MIN_VISIBLE_PX, panel.width);
  const minX = visible - panel.width;
  const maxX = Math.max(minX, bounds.width - visible);
  const maxY = Math.max(bounds.insetTop, bounds.height - Math.min(MIN_VISIBLE_PX, panel.height));
  return {
    x: Math.min(Math.max(position.x, minX), maxX),
    y: Math.min(Math.max(position.y, bounds.insetTop), maxY),
  };
}

/**
 * Where a panel lands, given where it started and how far the pointer has travelled.
 *
 * Expressed as a delta from the press rather than "put the panel under the cursor", so the panel
 * does not jump to centre itself on the pointer the instant a drag begins.
 */
export function dragTo(
  origin: Point,
  pointerStart: Point,
  pointer: Point,
  panel: Size,
  bounds: DragBounds,
): Point {
  return clampToBounds(
    { x: origin.x + (pointer.x - pointerStart.x), y: origin.y + (pointer.y - pointerStart.y) },
    panel,
    bounds,
  );
}

/**
 * Whether a pointer press on the drag handle should begin a drag.
 *
 * A header full of buttons is also the handle, so a press that landed on a control is that
 * control's press and never a drag. Only the primary button drags.
 */
export function startsDrag(button: number, onInteractiveControl: boolean): boolean {
  return button === 0 && !onInteractiveControl;
}

/** Parse a stored position, rejecting anything that is not a pair of finite numbers. */
export function parsePosition(raw: unknown): Point | null {
  const parsed = raw && typeof raw === "object" ? raw as Partial<Point> : null;
  if (!parsed || typeof parsed.x !== "number" || typeof parsed.y !== "number") return null;
  if (!Number.isFinite(parsed.x) || !Number.isFinite(parsed.y)) return null;
  return { x: parsed.x, y: parsed.y };
}
