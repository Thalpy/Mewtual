// The one door out of the Studio surface for App routes that change the main pane directly
// (`view = …`, the inbox, DM home) instead of going through switchView.
//
// Studio's onDestroy cannot veto a transition: it runs after the route flag has changed, so a
// drawing the bounded draft queue refused to stage is destroyed with the editor. Every such route
// therefore asks this gate first and changes its flags only when the gate releases. switchView,
// switchServer and switchTo use the same route object; studio-exit-routes.test.ts audits
// App.svelte so a new direct route cannot skip it.
//
// A route is one user action, owned from its first step to its last. An action that awaits
// anything (a wiki read, a server's load) must carry its route across the await and ask it again
// right before it changes the screen: a newer action (returning to the Studio, another click, a
// lock) retires the route, and the Studio may hold work created while it waited. Asking with a
// FRESH claim at that point would be wrong, because claiming makes the obsolete action current again.
//
// The claim includes the Studio session's navigation lease, which a change of the Studio's channel
// or server also ends. An action that moves the channel while the Studio is mounted therefore
// retires itself; such an action changes the surface first (the Studio unmounts) and the channel
// after, as applyLoc, goMention and goTickerMessage do.

/** What the gate needs from App. Every member is read at call time, never cached. */
export type StudioExitHost = {
  /** The main pane's view is the Studio (a superset of "the editor is mounted"). */
  onStudio: () => boolean;
  /** The Studio session holds edits, staging an in-progress stroke first. A staging refusal
   * must report true: the awaited Save then refuses with the reason instead of leaving. */
  hasWork: () => boolean;
  /** The window-close sequence owns the Studio Save; no route may race it. */
  closing: () => boolean;
  /** Take the navigation claim (App's route counter and the session lease). The returned check
   * is still true only if no newer route, lock or reload has claimed since. */
  claim: () => () => boolean;
  /** The awaited Save of every retained edit. Resolves false on refusal; the host says why. */
  save: () => Promise<boolean>;
  /** Leaving the Studio SURFACE only (`leave`), when the host can keep the work on this device:
   * stage the drawing, send what is queued, and seal a continuity snapshot holding every retained
   * edit. Resolves true once that snapshot is written, whatever the native Saves did; false on a
   * refusal (a stroke that cannot be staged, a snapshot write that failed), and the host says why.
   * Absent: `leave` waits for definite receipts through `save`, as `scope` always does. */
  preserve?: () => Promise<boolean>;
  /** With `preserve`: the retained work as it is now is already in a sealed snapshot, so leaving
   * the surface needs nothing more. Must stage the drawing first (or report false if it cannot). */
  preserved?: () => boolean;
  /** A departure gave up after MAX_SAVE_ROUNDS because new work kept arriving; say so. */
  exhausted?: () => void;
};

/** One navigation action's ownership. None of these renews the claim. */
export type StudioRoute = {
  /** Still the newest action: no newer route, lock or reload has claimed since `begin()`. */
  current: () => boolean;
  /** Leaving the Studio surface: resolves true when this action may change the main pane now,
   * saving first if the Studio is on screen with work. Ask again after every await. */
  leave: () => Promise<boolean>;
  /** Changing the channel or server under the Studio: as `leave`, but any retained edit counts,
   * on screen or not, because the Studio session's scope changes with it. */
  scope: () => Promise<boolean>;
};

export type StudioExit = {
  /** Claim navigation for a new action. */
  begin: () => StudioRoute;
  /** `begin().leave()`, for an action whose only await is this one. */
  release: () => Promise<boolean>;
  /** For synchronous routes (key handlers, menu items, rail buttons): `go` runs in this turn when
   * nothing needs saving, otherwise only after the Save succeeds and the action is still current.
   * It receives the action's route to hand to any switch it starts. */
  leaveThen: (go: (route: StudioRoute) => void) => void;
  /** Whether leaving the Studio surface now still needs a departure step: exactly the condition
   * `leave` settles on. With `preserve`, retained work that is already sealed does NOT need one;
   * a caller that asked "is there work?" instead would be told yes forever after a successful
   * departure, because preserved work stays in the session by design (review of 13792739, P1:
   * switchView re-entered on `hasWork` after `leave()` succeeded and looped without yielding). */
  mustLeave: () => boolean;
};

/** Saves one departure may run while new work keeps arriving during them. Beyond this the action
 * stays where it is (a refusal, never a departure with unsaved work); asking again starts afresh. */
const MAX_SAVE_ROUNDS = 16;

export function createStudioExit(host: StudioExitHost): StudioExit {
  /** Leaving the surface needs nothing: not on the Studio, no work, or work already sealed. */
  const leaveFree = () => !host.onStudio() || !host.hasWork() || (!!host.preserve && !!host.preserved?.());
  function begin(): StudioRoute {
    const current = host.claim();
    async function settle(needsSave: () => boolean, step: () => Promise<boolean>): Promise<boolean> {
      for (let round = 0; ; round++) {
        if (!current()) return false;
        if (!needsSave()) return true;
        if (host.closing()) return false;
        if (round >= MAX_SAVE_ROUNDS) { host.exhausted?.(); return false; }
        if (!(await step())) return false;
        // Loop: a stroke drawn while Save awaited its receipt (or while the snapshot was written)
        // is new work and needs its own round.
      }
    }
    const preserve = host.preserve;
    return {
      current,
      // Leaving the surface keeps the session (and its workers) alive, so with `preserve` it only
      // has to make the work durable on this device; native receipts follow in the background.
      leave: () => preserve ? settle(() => !leaveFree(), preserve) : settle(() => host.onStudio() && host.hasWork(), host.save),
      // A channel or server change replaces the session's scope, so it still needs receipts.
      scope: () => settle(() => host.hasWork(), host.save),
    };
  }
  function leaveThen(go: (route: StudioRoute) => void): void {
    const route = begin();
    if (leaveFree()) { go(route); return; }
    void route.leave().then((released) => { if (released) go(route); });
  }
  return { begin, release: () => begin().leave(), leaveThen, mustLeave: () => !leaveFree() };
}
