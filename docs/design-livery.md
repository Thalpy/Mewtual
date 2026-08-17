# Server livery (owner-published UI scheme); design

Status: **implemented (L1–L3 ✅, 2026-08-15).** `DocType::Livery = 10` mirrors the Profile
doc end to end (lazy open, doc sync + snapshot catch-up, generic persistence); writes are
owner/admin-gated in `Server::set_livery` (same policy layer as roles; the attributable-but-
not-rejected residual applies, as scoped below); the client validates on read, applies with
the precedence below, and ships the Server-settings Livery section + the per-server
Appearance follow-toggle. `tokens` overrides are plumbed but the publisher UI writes an
empty map in v1 (preset + accent only). The shared **server icon** (`icon` key,
`set_server_icon` invoke) has its backend half in place; no publisher UI yet. Not yet done:
rail-monogram tint (optional), the contrast floor and debounce mitigations (noted below,
revisit if abused).

## Goals / non-goals

**Goals.** A server owner/admin can publish a **livery**; a preset id + accent (and,
later, a bounded set of colour-token overrides); that every member's client applies while
that server is active. Members can **opt out per server** (use their own theme). Joiners
inherit the livery with normal group sync; no invite-blob changes. A hostile admin can at
worst *recolor* the app, never restyle layout, load remote resources, or inject CSS.

**Non-goals (first cut).** Custom fonts; background images/wallpapers (needs fileshare
integration and a separate safety pass); arbitrary CSS; per-channel liveries; animated
themes. Density and terminal-chrome stay personal; a server never changes how much fits on
your screen, only what colour it is.

## Data model

New `DocType::Livery` (next free discriminant in `catcoms-wire/src/context.rs` after
`Profile = 9`), one shared CRDT doc per server, same signed-op machinery as `Profile` /
`MemberRoles`. Schema (one map, versioned):

```
{
  v: 1,
  preset: "aurum" | "nightshade" | "verdant" | "garnet" | "slate" | "",  // "" = default
  accent: "#rrggbb" | "",                    // optional accent override
  tokens: { "<allow-listed token>": "#rrggbb", ... },  // v1: may be empty/absent
  icon: "<base64 image bytes>" | ""          // shared server icon; absent = "" = none
}
```

- **`icon`** is an *additive* key (still `v: 1`; an older doc simply lacks it and reads as
  `""`). It carries the image **inline**; unlike a member avatar, which gossips a content
  address; so it is capped at `MAX_SERVER_ICON_BYTES` (64 KiB decoded, the avatar budget)
  and rejected if it is not valid base64. It has its **own** command (`set_server_icon`,
  same owner/admin gate): `set_livery` is a read-modify-write of preset/accent/tokens that
  carries the stored icon through untouched, so republishing colours never resends the image
  and removing the livery never clears it. `""` clears the icon.

- **Write policy**: owner/admin only, enforced at the same policy layer as `MemberRoles`
  (same caveat as roles: cryptographic enforcement is the existing named follow-up; the op
  log is signed, so authorship is attributable either way).
- **Read validation (client, mandatory)**: unknown `preset` → ignore field; `accent` must
  match `^#[0-9a-f]{6}$` (case-insensitive) → else ignore; `tokens` keys must be in the
  client's allow-list (colour tokens only: `--bg-0/--panel/--bg-elev/--border/--border-soft/
  --text/--text-2/--muted/--faint/--accent/--accent-hi`), values hex-only → else drop the
  entry. Never sizes, fonts, URLs, or anything that reaches layout. Malformed docs degrade
  to "no livery", never to an error.

## Client behaviour

Precedence, most specific wins:

1. **User per-server override**; "use my own theme here" (localStorage,
   `catcoms.appearance.override.<server-id>`), which re-applies the user's global scheme.
2. **Server livery**; the validated doc, applied while that server is active.
3. **User global appearance**; existing `catcoms.appearance`.
4. Built-in default (Nightshade).

Application reuses the existing `$effect` that stamps `data-preset` / `--accent` on
`<html>`: switching servers (or to DM home / inbox, which never have liveries) swaps the
scheme live. Semantic tokens (`--ok/--warn/--danger`) are **not** livery-controllable;
green/gold/red keep their jobs under any admin.

## UI

- **Server settings → Livery** (owner/admin only): the same preset tiles + accent swatches
  as the personal Appearance section, plus *Publish* / *Remove livery*. A muted note states
  what members see and that they can override it.
- **Settings → Appearance**: when the active server publishes a livery, a line appears;
  "This server sets a livery (Aurum). ☑ Follow it here"; the checkbox is the per-server
  override, default on (follow).
- Rail affordance (later, optional): servers with a livery could tint their monogram ring.

## Sync / joining

Nothing new: the livery doc rides the existing doc-sync + snapshot catch-up path like
Profile/Wiki/Status. A joiner sees the livery as soon as docs sync; no invite changes
(contrast with the TURN-in-invite mechanism, which had to ride the invite because it is
needed *before* joining).

## Threat notes

- Values are untrusted input from the group; the allow-list + hex validation above is the
  entire attack surface. No URL-shaped values exist in v1, so no fetch/exfil vector.
- Contrast abuse (admin sets text ≈ background): bound by validation? No; v1 accepts any
  hex. Mitigation: the per-server override is one click, and the Appearance section always
  renders in the *user's* scheme so the escape hatch stays legible. If this proves annoying,
  add a client-side contrast floor (reject token sets where text/bg contrast < 3:1).
- Flicker/rapid-change spam: doc updates are rate-limited by the same op-log path as other
  docs; additionally the client may debounce livery application (e.g. 2s).

## Phases

- **L1 (backend)**: `DocType::Livery`, get/set invokes (`set_livery`, gated owner/admin;
  livery included in the server-state payload the UI already loads), change event to the UI.
- **L2 (client apply)**: validation + precedence + live swap on server switch; per-server
  override persistence. Frontend-only once L1 lands.
- **L3 (UI)**: Server-settings Livery section; Appearance follow-toggle; (optional) rail tint.

L1 touches wire/context + the app actor + the Tauri bridge; small, review-friendly. L2/L3
are the same shape as the already-shipped Appearance work.
