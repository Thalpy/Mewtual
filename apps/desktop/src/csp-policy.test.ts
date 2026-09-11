/**
 * The webview's Content Security Policy is a security boundary, so it is pinned like one.
 *
 * SEC-EXFIL-001. A compromised renderer does not obey the app's own UI rules: it does not have to
 * call `mayAutoLoadRemoteUrl`, and it does not have to use the download helpers. What it cannot
 * do is make a request the CSP forbids, because that check happens in the engine rather than in
 * our JavaScript. That makes this policy one of the few controls that still holds after arbitrary
 * script execution in the renderer, which is exactly the situation a media decoder exploit
 * produces. Directives here are load-bearing, not hygiene.
 *
 * These tests parse the policy rather than searching it for substrings. A substring check passes
 * on a policy whose directives have been reordered, renamed or merged, and reports success when
 * the directive it was looking for is absent entirely, which is the failure that matters.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";

const configPath = fileURLToPath(new URL("../src-tauri/tauri.conf.json", import.meta.url));
const config = JSON.parse(readFileSync(configPath, "utf8"));

/** Every source expression in one directive, or `undefined` if the policy does not set it. */
function directive(policy: string, name: string): string[] | undefined {
  for (const clause of policy.split(";")) {
    const parts = clause.trim().split(/\s+/).filter(Boolean);
    if (parts.length > 0 && parts[0].toLowerCase() === name) return parts.slice(1);
  }
  return undefined;
}

/** The directive that actually governs `name`, following the `default-src` fallback. */
function effective(policy: string, name: string): string[] {
  const own = directive(policy, name);
  if (own !== undefined) return own;
  const fallback = directive(policy, "default-src");
  assert.ok(
    fallback !== undefined,
    `${name} is unset and there is no default-src to fall back to, so the policy allows anything`,
  );
  return fallback;
}

const production: string = config.app.security.csp;
const development: string = config.app.security.devCsp;

/** Source expressions that let a directive reach a host of the page's choosing. */
function remoteSources(sources: string[]): string[] {
  return sources.filter((source) => {
    const value = source.toLowerCase();
    if (value === "*") return true;
    // Bare scheme sources (`https:`, `http:`) permit every host on that scheme.
    if (value === "http:" || value === "https:" || value === "ws:" || value === "wss:") return true;
    // A host source with a wildcard label, such as `https://*.example.com`.
    if (value.includes("://*")) return true;
    return false;
  });
}

/** Hosts the app resolves through the loopback interface rather than the network. */
function isLocal(source: string): boolean {
  const value = source.toLowerCase();
  return value.includes("localhost") || value.includes("127.0.0.1");
}

test("both policies parse, and neither leaves a fetch directive to an absent default", () => {
  // A malformed policy is not a weak policy, it is an absent one: engines drop what they cannot
  // parse. Confirm the shape before asserting anything about the contents.
  for (const [label, policy] of [["csp", production], ["devCsp", development]] as const) {
    assert.equal(typeof policy, "string", `${label} must be a string`);
    assert.ok(directive(policy, "default-src") !== undefined, `${label} must set default-src`);
    for (const name of ["script-src", "img-src", "media-src", "connect-src", "frame-src", "object-src"]) {
      assert.ok(effective(policy, name).length > 0, `${label} leaves ${name} empty`);
    }
  }
});

test("SEC-EXFIL-001: production img-src cannot reach a host the page chooses", () => {
  // An image request is a one-way channel that needs no reply, so `connect-src` being tight buys
  // nothing while `img-src` admits `https:`. A compromised renderer exfiltrates by setting
  // `new Image().src` to an attacker host with the stolen bytes in the query string. Nothing in
  // the app has to cooperate, and nothing in the app can observe it.
  const img = effective(production, "img-src");
  assert.deepEqual(
    remoteSources(img),
    [],
    `img-src must not admit an arbitrary remote host, found: ${remoteSources(img).join(" ")}`,
  );
  for (const source of img) {
    assert.ok(
      !source.includes("://") || isLocal(source),
      `img-src source ${source} names a remote origin`,
    );
  }
});

test("the media scheme survives the img-src tightening on Windows", () => {
  // The other direction, and the reason this test exists next to the one above. Shared images are
  // served over the custom `catcoms-media:` scheme, which Windows rewrites to
  // `http://catcoms-media.localhost/...`. That host was previously covered only by the blanket
  // `http:` source. Removing `http:` without naming the host explicitly silently breaks every
  // inline image in the app on the one platform we currently ship, and no test that only checks
  // for the absence of remote sources would notice.
  const img = effective(production, "img-src");
  assert.ok(img.includes("catcoms-media:"), "img-src must admit the custom media scheme");
  assert.ok(
    img.some((source) => source.toLowerCase().includes("catcoms-media.localhost")),
    "img-src must admit the Windows rewrite host for the media scheme",
  );
  // Media playback goes through the same scheme and must keep both spellings for the same reason.
  const media = effective(production, "media-src");
  assert.ok(media.includes("catcoms-media:"), "media-src must admit the custom media scheme");
  assert.ok(
    media.some((source) => source.toLowerCase().includes("catcoms-media.localhost")),
    "media-src must admit the Windows rewrite host for the media scheme",
  );
});

test("connect-src was not broadened to compensate for the img-src tightening", () => {
  // The obvious wrong fix for a blocked remote image is to fetch it through script instead. This
  // pins the channel shut so that change has to be argued for rather than slipped in.
  const connect = effective(production, "connect-src");
  assert.deepEqual(
    remoteSources(connect),
    [],
    `connect-src must not admit an arbitrary remote host, found: ${remoteSources(connect).join(" ")}`,
  );
  for (const source of connect) {
    assert.ok(
      !source.includes("://") || isLocal(source) || source.toLowerCase() === "ipc:",
      `connect-src source ${source} names a remote origin`,
    );
  }
});

test("script execution stays confined to the bundle in production", () => {
  // Inline script and eval are what turn an HTML injection into script execution. The renderer
  // holds the account, so the bar for running code in it is that we shipped the code.
  const script = effective(production, "script-src");
  assert.deepEqual(script, ["'self'"], "production script-src must be exactly 'self'");
  for (const forbidden of ["'unsafe-inline'", "'unsafe-eval'", "'wasm-unsafe-eval'"]) {
    assert.ok(!script.includes(forbidden), `script-src must not include ${forbidden}`);
  }
});

test("plugin, base and form targets stay closed in both policies", () => {
  for (const [label, policy] of [["csp", production], ["devCsp", development]] as const) {
    assert.deepEqual(effective(policy, "object-src"), ["'none'"], `${label} object-src must be 'none'`);
    const base = directive(policy, "base-uri");
    assert.ok(base !== undefined, `${label} must set base-uri`);
    // A rewritten base URI silently redirects every relative URL on the page.
    assert.ok(
      base.length === 1 && (base[0] === "'self'" || base[0] === "'none'"),
      `${label} base-uri must be 'self' or 'none', found: ${base.join(" ")}`,
    );
    const form = directive(policy, "form-action");
    assert.ok(form !== undefined, `${label} must set form-action`);
    assert.deepEqual(remoteSources(form), [], `${label} form-action must not post to a remote host`);
  }
});

/**
 * Media providers framed for chat embeds. Each entry is a third party that learns this device's
 * address and what it is looking at, and runs its own script inside our window, so the list is
 * pinned exactly rather than described by a pattern: adding a provider should cost one deliberate
 * edit here and the review that goes with it.
 *
 * On Windows there is a second reason to keep this short. wry injects Tauri's initialization
 * scripts into every subframe regardless of the main-frame-only flag, so each origin below
 * receives the IPC internals object. Remote origins are refused natively before any command runs,
 * so this is not currently an execution path, but the length of this list is the width of that
 * exposure.
 */
const REVIEWED_EMBED_HOSTS = [
  "https://embed.bsky.app",
  "https://embed.music.apple.com",
  "https://open.spotify.com",
  "https://player.mixcloud.com",
  "https://player.vimeo.com",
  "https://w.soundcloud.com",
  "https://www.youtube-nocookie.com",
];

test("the framed embed hosts are an exact reviewed list, not a scheme", () => {
  // A bare `https:` here would hand a compromised renderer a frame to any origin it likes, which
  // is an exfiltration channel wearing a different hat.
  //
  // These frames are ordinary product features. They are not a security boundary, and nothing that
  // needs isolation may be built inside one.
  const frame = effective(production, "frame-src");
  assert.deepEqual(
    [...frame].sort(),
    REVIEWED_EMBED_HOSTS,
    "frame-src changed: add or remove the host in REVIEWED_EMBED_HOSTS in the same commit, so the list stays a decision rather than a leftover",
  );
  // Whatever the list holds, every entry must be one exact https origin.
  for (const host of frame) {
    assert.ok(host.startsWith("https://"), `framed origin ${host} is not https`);
    assert.ok(!host.includes("*"), `framed origin ${host} uses a wildcard`);
    assert.equal(host.split("/").length, 3, `framed origin ${host} must be a bare origin`);
  }
});

test("the development policy relaxes only what Vite needs, and only toward localhost", () => {
  // A dev policy is still shipped in the repository and still shapes what anyone testing a change
  // sees. If it is loose, every manual check happens under weaker rules than production and the
  // difference is invisible until release.
  const img = effective(development, "img-src");
  assert.deepEqual(remoteSources(img), [], "devCsp img-src must not admit an arbitrary remote host");
  const connect = effective(development, "connect-src");
  assert.deepEqual(remoteSources(connect), [], "devCsp connect-src must not admit an arbitrary remote host");
  for (const source of connect) {
    assert.ok(
      !source.includes("://") || isLocal(source) || source.toLowerCase() === "ipc:",
      `devCsp connect-src source ${source} is neither loopback nor the IPC scheme`,
    );
  }
  assert.deepEqual(effective(development, "script-src"), ["'self'"], "devCsp script-src must be exactly 'self'");
});
