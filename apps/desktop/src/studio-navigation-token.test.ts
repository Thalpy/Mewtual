// Structural audit for the eb411f1 review's P2: a Studio document navigation (sidebar open, new
// flipnote, the Music tab's open/create) must claim the app's navigation counter the moment it is
// decided, before its departure Save can wait, so an App route (Back, a channel, a server) that was
// already waiting on its own Save finds itself superseded and does nothing. App.svelte cannot be
// mounted under node:test, so this pins the wiring: every `session.navigate(` in the two surfaces is
// preceded by the `onnavigate` call, App passes that prop to both surfaces, and the prop bumps the
// same counter the routes compare against after their awaits.
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const read = (name: string) => readFileSync(fileURLToPath(new URL(`./${name}`, import.meta.url)), "utf8");

test("every Studio document navigation announces itself before its departure Save", () => {
  for (const name of ["StudioNav.svelte", "Studio.svelte"]) {
    const source = read(name);
    const sites = [...source.matchAll(/session\.navigate\(/g)].map((m) => m.index!);
    assert.ok(sites.length >= 2, `${name} navigates somewhere`);
    for (const at of sites) {
      // The announcement is in the same handler, textually before the navigate call and after the
      // previous statement boundary of that handler (an earlier announcement would be a stale one).
      const handlerStart = Math.max(source.lastIndexOf("async function ", at), source.lastIndexOf("=> {", at), source.lastIndexOf("={(", at));
      const before = source.slice(handlerStart, at);
      assert.ok(/onnavigate\?\.\(\);/.test(before), `${name}: session.navigate at offset ${at} is not preceded by onnavigate?.() in its handler`);
    }
  }
});

test("App wires the announcement to the counter its routes compare against", () => {
  const app = read("App.svelte");
  assert.match(app, /function noteStudioNavigation\(\) \{ studioNavigation\+\+; \}/);
  assert.match(app, /<StudioNav [^\n]*onnavigate=\{noteStudioNavigation\}/);
  assert.match(app, /<StudioSurface [^\n]*onnavigate=\{noteStudioNavigation\}/);
  // The channel switch's final answer is the counter comparison: a Studio navigation during its
  // read makes the switch report itself superseded, so its callers do not force the pane.
  const switchTo = app.slice(app.indexOf("async function switchTo("), app.indexOf("\n  }\n", app.indexOf("async function switchTo(")));
  assert.match(switchTo, /return studioRequest === studioNavigation;/);
});
