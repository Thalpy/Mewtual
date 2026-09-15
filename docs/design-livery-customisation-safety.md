# Livery / profile customisation; how far is safe? (security design)

Status: **implemented (the safe 90%); assessment retained.** Answers "can we expose
HTML/CSS for server livery and user profiles, MySpace/Oshi style?" Short answer: **no raw
HTML or raw CSS, for either**; but the expressive *feel* is reachable through a widened
allow-list + catalog assets + a CID-based custom cursor. This doc records why, so the line
isn't re-litigated later.

What shipped: the CSP backstop (below), the widened token vocabulary
(`App.svelte`, the `LIVERY_RADIUS` / `LIVERY_FONTS` / `LIVERY_PATTERNS` catalogs read by
`sanitizeLivery`: `--radius` enum, bundled font catalog, background-pattern catalog),
and the custom cursor with its dimension cap, opaque-area minimum and mandatory `, auto`
fallback (`App.svelte`, `validateCursor`), written through the `set_server_cursor` command
(`apps/desktop/src-tauri/src/lib.rs`). Still open: the optional **contrast floor** and
the publisher-side **debounce** (see `design-livery.md`).

## The threat frame

The desktop client is a **Tauri WebView with the `invoke` bridge in document scope**. When
this was written `tauri.conf.json` set **`"csp": null`** (no Content-Security-Policy
backstop); it now ships a real policy (see "Prerequisite hardening" below), but the frame
below is why that policy is a *second* wall and not the argument on its own.
Anything that executes as markup in that document can call every Tauri command the client
can: read/enumerate messages, mint invites, delete files, publish livery, walk the
fileshare, read anything the vault unlocked. So peer-authored markup is not a theming
question; it is **remote code execution inside a vault-unlocked client**. This is exactly
why `render.ts` runs a strict `marked` + DOMPurify allowlist and injects media from
content-addressed blobs *in code*; untrusted bytes never become live markup.

Two proposed mitigations do **not** work, and it's important to say why:

- **"Admin/owner only."** From a joiner's seat the admin *is* the adversary; anyone can
  found a server and invite you. Gating on role gates nothing for the victim.
- **"Set once at creation, immutable."** Immutability freezes the *payload*, not its
  *privileges*. An injected script you then can't patch is worse, not better.

### Even CSS-without-HTML is unsafe in chrome

- **Overlay phishing.** `position: fixed` + high `z-index` lets peer CSS paint a fake
  passphrase/unlock prompt over the real UI. In a shared document this is a
  credential-capture surface, not a cosmetic one, and CSP does not stop it: the style
  allowance below (`'unsafe-inline'` for style attributes) is exactly what such CSS uses.
- **CSS exfiltration.** Attribute-selector + value-triggered background requests can leak
  DOM contents; today's blob-only media path removes the `url()` fetch vector, and raw CSS
  would hand it back.
- **Layout DoS / clickjacking.** Unbounded CSS can cover, move, or hide real controls.

## What we ship instead (the safe 90%)

Everything below is **data, not code**: bounded scalars validated on read, no string that
becomes markup or a network fetch. This is the same shape as the existing livery
(`design-livery.md`), just a richer vocabulary.

1. **Expanded token vocabulary.** Beyond the current colour tokens: a larger colour set,
   `--radius` from an enum {sharp, soft, round}, a **font choice from a bundled catalog**
   (an id → one of N faces we ship; never a family string, never a URL), a **background
   pattern** id from a fixed catalog (CSS gradients/SVG patterns we author). Each value is
   allow-listed; anything unknown is dropped. Contrast floor optional (design-livery.md).
2. **Custom cursor (livery + profile); the fun one, done safely.** A cursor is *image
   bytes carried inline in the livery doc* (like the server icon; a fileshare CID would be
   equivalent), never a URL:
   - decoded client-side, **re-encoded** (strip metadata), dimension-capped (≤ 64×64) and
     byte-capped, applied as `cursor: url(data:image/png;base64,…) x y, auto`.
   - **always keep a real fallback** (`, auto`) and enforce a **minimum opaque area** so a
     1px/transparent cursor can't hide the pointer (a griefing vector, not RCE).
   - profile cursors apply only while hovering that user's surfaces (their card), not
     globally, to bound nuisance.
3. **No raw HTML/CSS anywhere.** If arbitrary layout is ever truly wanted, the *only* safe
   substrate is a **`<iframe sandbox>` with neither `allow-scripts` nor bridge access**,
   rendering to a fixed rectangle, CSP locked down. That's a large, separate project with
   its own review; explicitly out of scope here, and **profile HTML is strictly worse than
   server HTML** (attacker-to-attacker at DM range, no admin framing), so it goes last if
   ever.

## Prerequisite hardening; ✅ done

`tauri.conf.json` now ships a real **CSP** (plus a `devCsp` that differs only by allowing
Vite's HMR socket, `ws://localhost:1420 http://localhost:1420`, in `connect-src`). The
live policy, the `app.security.csp` key of `apps/desktop/src-tauri/tauri.conf.json`:

- `default-src 'self'`, `script-src 'self'` (Tauri auto-nonces its own bootstrap),
  `object-src 'none'`, `base-uri 'self'`, `form-action 'self'`, `font-src 'self'`.
- `img-src` / `media-src`: `'self' data: blob: catcoms-media: http://catcoms-media.localhost`.
  `data:`/`blob:` are the content-addressed embed pipeline; the two `catcoms-media` entries
  are the local custom protocol that streams decrypted attachment bytes (and its Windows
  `http://…localhost` spelling), so large media need not be inlined.
- `connect-src 'self' ipc: http://ipc.localhost`: the IPC scheme only, no outbound origin.
- `frame-src`: **not** `'none'`. Seven allow-listed embed hosts are permitted so link embeds can
  render their players: `https://open.spotify.com`, `https://www.youtube-nocookie.com`,
  `https://w.soundcloud.com`, `https://player.vimeo.com`, `https://player.mixcloud.com`,
  `https://embed.music.apple.com`, `https://embed.bsky.app`. That is a deliberate, named
  exception: those origins are sandboxed cross-origin frames with no bridge access, but any
  widening of this list is a security change, not a styling one, and it is a privacy change as
  well as a script-execution one. The device-wide **Chat & Media → load these cards without
  asking** preference is a single switch over this whole list rather than a per-host consent, so
  adding a host silently extends an answer the member already gave, and the member-facing copy
  (`USER_GUIDE.md`, `CHANGELOG.md`) has to be updated in the same change.
- Style attributes keep `'unsafe-inline'` (profile colours/bubbles are inline styles; style
  attrs cannot execute script, but see the overlay-phishing note above).

With this, a sanitizer slip degrades to "markup appeared" rather than "peer code ran with
bridge access"; the second wall the doc above assumes.

## Verdict

| Ask | Verdict |
|---|---|
| Server livery raw HTML | ❌ RCE in a vault-unlocked client; admin-only / immutable don't mitigate |
| Server livery raw CSS | ❌ overlay-phishing + exfil in shared chrome |
| Custom mouse cursor (livery) | ✅ **yes**, as a CID image, re-encoded + size-floored + fallback |
| Widened token/catalog theming | ✅ **yes**, the MySpace feel without the engine |
| User profile raw HTML/CSS | ❌ worse than server (attacker-to-attacker, no framing) |
| Profile cursor / catalog theming | ✅ same rules as livery, scoped to the profile surface |
