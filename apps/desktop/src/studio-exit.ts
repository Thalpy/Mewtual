// The one door out of the Studio surface for App routes that change the main pane directly
// (`view = …`, the inbox, DM home) instead of going through switchView.
//
// Studio's onDestroy cannot veto a transition: it runs after the route flag has changed, so a
// drawing the bounded draft queue refused to stage is destroyed with the editor. Every such route
// therefore asks this gate first and changes its flags only when the gate releases. switchView,
// switchServer and switchTo run the same awaited Save inline; studio-exit-routes.test.ts audits
// App.svelte so a new direct route cannot skip all of them.

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
};

export type StudioExit = {
  /** Resolves true when the caller may leave the Studio now; false when Save refused, the window
   * is closing, or a newer navigation superseded this one. Use it inside async routes. */
  release: () => Promise<boolean>;
  /** For synchronous routes (key handlers, menu items, rail buttons): `go` runs in this turn when
   * nothing needs saving, otherwise only after the Save succeeds and this route is still current. */
  leaveThen: (go: () => void) => void;
};

export function createStudioExit(host: StudioExitHost): StudioExit {
  // The claim is taken on every call, as switchView takes it, so an older pending Studio Save can
  // never complete its own route on top of a newer one.
  async function release(): Promise<boolean> {
    const current = host.claim();
    if (!host.onStudio() || !host.hasWork()) return true;
    if (host.closing()) return false;
    if (!(await host.save()) || !current()) return false;
    // A stroke drawn while Save awaited its receipt is new work and needs its own Save first.
    return release();
  }
  function leaveThen(go: () => void): void {
    if (!host.onStudio() || !host.hasWork()) { host.claim(); go(); return; }
    void release().then((released) => { if (released) go(); });
  }
  return { release, leaveThen };
}
