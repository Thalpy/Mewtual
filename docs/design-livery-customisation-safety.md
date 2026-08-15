# Livery / profile customisation — how far is safe? (security design)

Status: **assessment + proposal.** Answers "can we expose HTML/CSS for server livery and
user profiles, MySpace/Oshi style?" Short answer: **no raw HTML or raw CSS, for either** —
but the expressive *feel* is reachable through a widened allow-list + catalog assets + a
CID-based custom cursor. This doc records why, so the line isn't re-litigated later.

## The threat frame

The desktop client is a **Tauri WebView with the `invoke` bridge in document scope**, and
`tauri.conf.json` currently sets **`"csp": null`** (no Content-Security-Policy backstop).
Anything that executes as markup in that document can call every Tauri command the client
can: read/enumerate messages, mint invites, delete files, publish livery, walk the
fileshare, read anything the vault unlocked. So peer-authored markup is not a theming
question — it is **remote code execution inside a vault-unlocked client**. This is exactly
why `render.ts` runs a strict `marked` + DOMPurify allowlist and injects media from
content-addressed blobs *in code* — untrusted bytes never become live markup.

Two proposed mitigations do **not** work, and it's important to say why:

- **"Admin/owner only."** From a joiner's seat the admin *is* the adversary — anyone can
  found a server and invite you. Gating on role gates nothing for the victim.
- **"Set once at creation, immutable."** Immutability freezes the *payload*, not its
  *privileges*. An injected script you then can't patch is worse, not better.

### Even CSS-without-HTML is unsafe in chrome

- **Overlay phishing.** `position: fixed` + high `z-index` lets peer CSS paint a fake
  passphrase/unlock prompt over the real UI. With `csp:null` and a shared document, this
  is a credential-capture surface, not a cosmetic one.
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
2. **Custom cursor (livery + profile) — the fun one, done safely.** A cursor is an *image
   from the encrypted fileshare* (a CID), never a URL:
   - decoded client-side, **re-encoded** (strip metadata), dimension-capped (≤ 64×64) and
     byte-capped, applied as `cursor: url(data:image/png;base64,…) x y, auto`.
   - **always keep a real fallback** (`, auto`) and enforce a **minimum opaque area** so a
     1px/transparent cursor can't hide the pointer (a griefing vector, not RCE).
   - profile cursors apply only while hovering that user's surfaces (their card), not
     globally, to bound nuisance.
3. **No raw HTML/CSS anywhere.** If arbitrary layout is ever truly wanted, the *only* safe
   substrate is a **`<iframe sandbox>` with neither `allow-scripts` nor bridge access**,
   rendering to a fixed rectangle, CSP locked down. That's a large, separate project with
   its own review — explicitly out of scope here, and **profile HTML is strictly worse than
   server HTML** (attacker-to-attacker at DM range, no admin framing), so it goes last if
   ever.

## Prerequisite hardening (do regardless)

Set a real **CSP** in `tauri.conf.json` (`default-src 'self'`, `img-src 'self' data:`,
`connect-src 'self' ipc:`, no `unsafe-inline` for scripts). Even with today's sanitizer this
is defence-in-depth; before *any* customisation widening it's mandatory. Tracked as its own
slice.

## Verdict

| Ask | Verdict |
|---|---|
| Server livery raw HTML | ❌ RCE in a vault-unlocked client; admin-only / immutable don't mitigate |
| Server livery raw CSS | ❌ overlay-phishing + exfil in shared chrome |
| Custom mouse cursor (livery) | ✅ **yes**, as a CID image, re-encoded + size-floored + fallback |
| Widened token/catalog theming | ✅ **yes**, the MySpace feel without the engine |
| User profile raw HTML/CSS | ❌ worse than server (attacker-to-attacker, no framing) |
| Profile cursor / catalog theming | ✅ same rules as livery, scoped to the profile surface |
