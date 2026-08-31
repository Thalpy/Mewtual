# Design: Diagnostics / Debug Console

Status: design spec, not yet implemented. Reference mockup: `apps/desktop/design/debug-console.html`
(open it in a browser; it is self-contained and uses the real tokens).

## 1. Purpose

The app currently fails silently. Real incidents that the UI could not explain:

- Voice signalling transport dropped; calls died while the peer still showed "online".
- A node sat isolated for an hour dialling two IPv6 addresses an IPv4-only machine can
  never reach. No dial failure was surfaced anywhere.
- Frontend `console.warn` and backend `tracing` events only exist in the on-disk log file.

The debug console is a full-screen, in-app, live view of everything the log file knows,
segmented so a user (or a helper reading their screenshot) can find the failing layer in
seconds: is it the network, the voice stack, the backend, or the frontend?

Design intent in one line: an operator terminal you can screenshot at a stranger.

## 2. Entry points and shell

- Opened from Settings > Connection > Diagnostics via a new "Open debug console" button,
  and by a keybind (proposed: `Ctrl+Shift+D`, subject to the keybind table).
- It is a full-screen takeover exactly like the settings overlay: fixed, `inset:
  var(--titlebar-h) 0 0 0`, above the app, below the titlebar and toasts. `Esc` closes it
  (topmost-layer-first convention). Reuse the `stx-esc` / `stx-esc-ring` close control.
- Root class `dbg`. Like `.stx`, chrome is `user-select: none`; machine text (ids,
  addresses, log lines, table cells) opts back in via `.readout` semantics: everything
  inside `.dbg-feed-scroll`, `.dbg-table`, `.dbg-kv .v`, and `.fp` is selectable.

## 3. Layout

Four bands, top to bottom:

```
+------------------------------------------------------------------+
| dbg-head   crumb + title | severity roll-up | copy report | ESC  |
+----------+-------------------------------------------------------+
| dbg-rail | dbg-content  (one section visible at a time, scrolls) |
| Overview |                                                       |
| Network  |                                                       |
| Voice    |                                                       |
| Backend  |                                                       |
| Frontend |                                                       |
| Storage  |                                                       |
+----------+-------------------------------------------------------+
| dbg-foot   privacy sentence ................... redact toggle    |
+------------------------------------------------------------------+
```

### Why a left rail, not tabs

- Six sections each need a live badge (error / warning counts per section). A rail keeps
  all six badges visible at every width; a top tab strip would fight the header's
  severity roll-up for the same horizontal space and wraps at 1000px.
- It matches the settings takeover (`stx-nav` / `stx-item`), so the console reads as the
  same app and the implementation reuses existing conventions (active state is the same
  inset accent bar).
- The rail is fixed at 168px (narrower than settings' 256px: labels are single words).
  At 1000px window width that leaves roughly 800px of content, which is enough because
  every wide thing (tables, feeds) scrolls horizontally inside its own wrapper. The page
  itself never scrolls horizontally.

### Header (`dbg-head`)

- Left: `stx-crumb` reading `DIAGNOSTICS // <SECTION>` plus an `h1` "Debug console".
- Middle: the severity roll-up (`dbg-sev`): two chips, session error count and session
  warning count, coloured by job (`--danger`, `--warn`). When a count is zero the chip
  renders in the quiet style (`dbg-sev-chip quiet`) so "all clear" is visibly boring.
  Counts are cumulative for this app session, across all sections, whether or not the
  console was open when they happened (see capture model, section 4.1).
- Right: "Copy report" (`ghost small`), then the ESC ring.
- Clicking a severity chip is optional behaviour, not required for v1. If implemented it
  jumps to the section with the newest matching event.

### Rail (`dbg-rail`)

- `dbg-rail-item` per section: label plus an optional `dbg-rail-count` badge. Badge shows
  the section's own error count (danger) or, if zero errors but warnings exist, the
  warning count (warn). No badge when clean.
- Active item mirrors `stx-item.active`: accent-tinted background, inset 2px accent bar.
- Sections keep their state (filters, pause, scroll position) when you switch away and
  back within one console opening.

### Footer (`dbg-foot`)

- One privacy sentence, always visible, `muted small`:
  "Everything here can include your IP addresses, peer and device identifiers, and
  timing. It never includes message text, file contents, names or key material."
  **Superseded.** That sentence was not true of the implementation: the report writes server
  names, and the `tracing` compatibility bridge carries arbitrary prose. It now describes what a
  report may contain and asks the reader to check. See `docs/design-diagnostics-suite.md` and
  finding P3-002 in `docs/reviews/Mewtual_PFixes_Part3_Adversarial_Review.md`.
  This is the same contract as the Diagnostics settings page wording; every copy action
  in the console is covered by this strip, so individual buttons do not repeat it. The
  "Copy report" flow additionally appends the sentence to the copied bundle itself.
- Right side: the redact toggle (`toggle` + `dbg-redact`), labelled "Redact for
  screenshots". See 4.5.

## 4. Cross-cutting behaviour

### 4.1 Capture model (drives the roll-up)

Capture starts in Safe mode and can be changed or switched off at runtime.

- Backend: a `tracing` layer keeps one bounded in-memory ring and increments session
  counters from app start for admitted events. Every accepted event is stamped with its
  capture mode and mode epoch, and every displayed/copied event line includes that stamp.
  Safe capture destroys literal addresses, arbitrary prose, runtime field names, and
  non-allowlisted target text before ring insertion, so later selecting Full cannot reveal
  historical Safe values. A mixed-history report labels the current setting separately from all
  capture epochs present. Per-section levels survive mode changes; restoring a mode's recommended
  levels is a separate action. Turning capture Off stops new admission without claiming to erase
  existing bounded history. When the console opens, the frontend pages the existing ring and then
  polls its live tail; a mode change queues a follow-up poll if one is already in flight.
- Frontend: `console.error/warn/info`, `window.onerror` and `unhandledrejection` are
  wrapped once at boot into a ring in JS. Wrapping must preserve the original console
  behaviour (call through) so the on-disk log keeps working.
- Network/voice/storage state is snapshot-on-open plus event-driven updates.

This is what makes the header roll-up honest: the user opens the console after the
failure and still sees "12 errors this session".

### 4.2 Ring buffers, never silent truncation

Every feed is a ring buffer. Proposed sizes: backend 2000 entries, frontend 500, dial
attempts 500, signalling feed 300. Each ring tracks `seen` (total this session) as well
as `kept`. When `seen > kept`, the feed renders a pinned first line (`dbg-drop-note`):

> Ring full: oldest entries dropped. Showing the last 2,000 of 3,417 this session. The
> debug log file may hold some of these, but it is not a complete record of them.

The second sentence is generated by `retentionStatus` from the sink's own reported state,
not written as a constant. It once read "The debug log file keeps everything", which was
false in six separate ways: file logging may be switched off, its initialisation may have
failed, its filter is deliberately narrower than the ring's, its queue drops under
pressure, it stops at a session quota, and an event the capture settings excluded reached
neither store. Somebody reading that stops screenshotting and goes looking for a file that
may not exist. There is deliberately no wording for "the file has all of it", because no
state of this system supports that claim.

Where the sink reports something specific the note says that specific thing: logging off
names itself, a failed sink quotes its own error, and a sink that was never read says so
rather than reassuring. Measured caveats (dropped, truncated, quota reached, narrower
filter, capture-excluded) are appended as a list.

The note is part of the scroller, pinned at top, styled `muted small` with a soft top
border, and it also appears in copied section output so a pasted bug report is honest
about what it is missing. The Overview rail carries a "Debug log file" card showing the
sink's state directly, so the note's "check the file" has somewhere to point.

### 4.3 Pause / resume and follow

- Every live feed has a `dbg-feed-bar` with, in order: filter input, feed-specific
  filters (level chips, target box), pause button, copy button.
- Pause (`ghost small`, `aria-pressed`) freezes rendering only. Capture continues into
  the ring. The button label becomes `Resume (+N)` where N is entries arrived while
  paused. Resuming appends them and jumps to the bottom.
- Feeds auto-follow (stick to bottom) while the user is at the bottom; scrolling up
  breaks follow, a small "Jump to latest" affordance restores it. (The mockup shows the
  bar and pause states; auto-follow is an implementation behaviour.)

### 4.4 Filters

- Text filter: case-insensitive substring over the rendered line text. Filtering hides
  non-matching rows; the feed bar shows `n of m shown` so a filter can never look like
  an empty feed.
- Backend level filter: five toggle chips, ERROR / WARN / INFO / DEBUG / TRACE.
  Defaults: ERROR, WARN, INFO on; DEBUG, TRACE off. These filter display only; capture
  level is a backend concern and out of scope here.
- Backend target filter: a text input matching the `tracing` target prefix
  (`catcoms_net`, `catcoms_sync`, `catcoms_app`, ...). Substring match is fine.

### 4.5 Redact

- The footer toggle masks personally identifying values everywhere in the console:
  IP addresses (v4 and v6, including inside multiaddrs), peer ids, device fingerprints.
- Masking must preserve correlation: each distinct value gets a stable per-session alias
  rendered in the same style, e.g. `[ip 1]`, `[peer 3]`, `[addr 2]`. A screenshot must
  still show "it keeps dialling the same two addresses".
- Implementation rule: redaction swaps the actual rendered text (a `redact(value)`
  helper at render time), never a CSS overlay, so text selection and every copy action
  copy exactly what is on screen. The parser recognizes complete base58 multihash PeerIds
  plus IPv4 and compressed/zoned/bracketed/IPv4-mapped IPv6, including values embedded in
  direct and relay multiaddrs. Copy while masked uses the same masking setting for event
  lines and tables; server names in route tables become per-session server aliases.
- A local bundle does not claim publication safety. Its header states address/identifier
  masking separately and explicitly says user content and legacy prose may be present.
- Markup hook: every redactable value is wrapped in `<span class="dbg-pii" data-kind="ip|peer|addr|fp">`.

### 4.6 Copy

Three granularities, all `ghost small` or the tiny per-row `dbg-copy` button:

- Row: hover a table row or feed line, a `dbg-copy` button appears at the row end.
  Copies that row as one plain-text line.
- Section: in each card header (`dbg-card-actions`) and each feed bar. Copies the card
  or feed as labelled plain text, including the drop note when present.
- Everything: the header's "Copy report" copies a local bundle: app version, timestamp,
  display-masking state and explicit privacy claims, then every section in order under a
  `== SECTION ==` heading. It ends with the review-before-sharing sentence. "Prepare issue"
  is a different native path: an allowlist renderer excludes tables, targets, wall-clock time,
  addresses, user prose, runtime field names and bridged tracing before publication validation.

Copy buttons flash "Copied" for ~1.5s (same convention as the invite/copy buttons in
settings).

### 4.7 Time

All timestamps are local wall-clock `HH:MM:SS.mmm`, mono, `--faint`. Feeds are newest at
the bottom (log order). "Ago" phrasing ("42s ago") is allowed in tables (last sync, next
re-dial) where a countdown or age reads better than a wall time; hover title gives the
absolute time.

## 5. Sections

### 5.1 Overview

A health board: the answer to "is anything wrong" in one screen, no scrolling at normal
heights.

- Server table (`dbg-table`): one row per joined server.
  Columns: server name, peers connected (`n / roster`), roster size, last sync (age),
  eclipse detector (chip: `OK` in ok style, `CAUTION` in warn style), re-dial (chip
  `PENDING` in transfer style, or `-`).
  Row click navigates to Network with that server's peers filtered (v1 optional; the
  mockup marks the affordance).
- "This device" card (`dbg-kv` pairs):
  public IPv4 (pii) with an AutoNAT chip (`CONFIRMED` ok / `UNCONFIRMED` warn),
  observed public IPv6 candidates (explicitly not an outbound-route test),
  active router mappings: one line per mapping, protocol chip (UPNP / PCP / NAT-PMP),
  internal to external port, expiry age.
- A short "attention" list: the newest 3 errors/warnings from any feed, each prefixed
  with its section name, each clickable to jump there. This is the "why is the badge
  red" shortcut. Empty state: "Nothing needs attention."
- Empty state (no servers): `dbg-empty` block, "No servers joined yet. Reachability
  below still describes this device."

### 5.2 Network

The big one. Three cards in order:

- Member-route table (`dbg-table` in a `dbg-table-wrap`):
  Columns: member, claimed transport peer (pii), typed state (`DIRECT PATH`, `RELAY PATH`,
  `PATH LIVE`, `NO RECORD`, `NO ROUTE`, `DIAL COOLDOWN`, `DIAL ELIGIBLE`, or forward-compatible
  `UNKNOWN`), route count, signed-record sequence, policy-approved dial batches submitted, and
  scheduler cooldown. A non-zero batch count is not danger-styled and is not called a failure,
  because the current transport seam does not return per-address outcomes.
  Row expansion (`dbg-row-toggle` on the row, `dbg-row-detail` row beneath) lists current coarse
  paths, time-bounded historical success, typed safe actions, and every signed candidate address.
  IPv6-only candidates are a clue, never a claim that this host lacks an outbound IPv6 route.
  A failed poll retains the previous rows only as an explicitly labelled last snapshot, suppresses
  live findings, and omits those rows from copied current-evidence output.
- Dial attempt feed (`dbg-feed`): every dial with timestamp, peer (short, pii), address
  (pii), outcome (`OK` ok / `FAILED` danger) and the error reason verbatim
  (e.g. `network unreachable (no IPv6 route on this host)`). Bounded, pausable,
  filterable, per 4.2 to 4.4.
- Router / port mapping card: current mappings (same rows as Overview but with lease
  detail and renewal time), plus a small event list (mapping obtained / renewed /
  lost) using the feed line style without pause controls (low volume).

### 5.3 Voice

Meaningful only during or after a call; the empty state must say so plainly.

- Empty state (`dbg-empty`): "No call this session. This section fills during a call
  and keeps the last call until the app closes."
- Per-peer card (`dbg-voice-peer`, one per call participant):
  `dbg-kv` rows for `connectionState`, `iceConnectionState`, `signalingState`, each
  value a chip coloured by job (connected/completed ok, checking/disconnected warn,
  failed danger, closed faint);
  selected candidate pair: local and remote candidate (pii), each with its type chip
  (HOST / SRFLX / RELAY);
  media path: `DIRECT` ok chip or `TURN RELAY` warn chip (relayed is worth noticing,
  not an error);
  ICE candidate counts by type: `host n · srflx n · relay n`, mono.
- STUN/TURN server errors: a short list; each line server URL, error, time. Empty:
  "No STUN/TURN errors."
- Signalling feed (`dbg-feed`): direction tag `TX` / `RX` (mono, TX in `--transfer`,
  RX in `--text-2`), message kind (offer / answer / ice / hangup), peer (short, pii),
  delivery outcome chip (`DELIVERED` ok / `SENT` faint / `FAILED` danger with reason).
  The transport-drop incident renders here as a run of `FAILED (signalling transport
  closed)` lines: precisely what was invisible before.

### 5.4 Backend

One full-height feed card.

- Feed bar: text filter, five level chips, target filter input, pause, copy.
- Line format, mono, `--fs-msg`: `HH:MM:SS.mmm  LEVEL  target  message`.
  LEVEL is a fixed-width tag: ERROR `--danger`, WARN `--warn`, INFO `--text-2`,
  DEBUG `--muted`, TRACE `--faint`. The line text itself stays `--text-2` for
  ERROR/WARN/INFO and dims with the level below that; only the tag carries the loud
  colour so a wall of errors stays readable.
- Structured `tracing` fields render appended as `key=value` pairs, mono, `--muted`.
- Empty state: "No backend events captured yet." (should be rare: startup logs exist).

### 5.5 Frontend

Same feed pattern as Backend, different sourcing.

- Sources, shown as the line's tag: `error`, `warn`, `info` (console), `uncaught`
  (window.onerror), `unhandled` (promise rejection). Level colours: uncaught/unhandled
  and console.error use `--danger`, warn `--warn`, info `--text-2`.
- Lines with a stack render a `stack` expander (`dbg-line-stack`): collapsed by
  default, click to reveal the stack in a `--bg-0` block, mono, pre-wrap. The expander
  is part of the line so copy-row includes the stack.
- Feed bar: text filter, level chips (ERROR / WARN / INFO, defaults all on), pause,
  copy.
- Empty state: "Nothing captured. Frontend console output and uncaught errors appear
  here as they happen."

### 5.6 Storage

Small, table-first, no feeds.

- Per-server table: server, documents (count), blob store (size), pending uploads,
  pending downloads. Pending cells use `--transfer` when non-zero (bytes want to move),
  `--muted` zeroes.
- Totals row (top border, `--text`).
- A `muted small` line naming the data directory (pii-adjacent but local-only; not
  masked) and the blob store's on-disk unit.
- Empty state: "No servers, nothing stored yet."

## 6. Class inventory

New classes are all `dbg-` prefixed. Existing classes reused as-is: `stx-crumb`,
`stx-esc`, `stx-esc-ring`, `chip` (+ existing `chip.ok`), `fp`, `muted`, `small`,
`ghost`, `toggle`, `field` (where a labelled input appears in feed bars).

New chip variants (add next to `.chip.ok` in app.css, same recipe):

```
.chip.warn     { color: var(--warn);     border-color: color-mix(in oklab, var(--warn) 40%, transparent); }
.chip.danger   { color: var(--danger);   border-color: color-mix(in oklab, var(--danger) 40%, transparent); }
.chip.transfer { color: var(--transfer); border-color: color-mix(in oklab, var(--transfer) 40%, transparent); }
.chip.faint    { color: var(--faint);    border-color: var(--border-soft); }
```

Shell:

| class | job |
| --- | --- |
| `dbg` | fixed full-screen root, `inset: var(--titlebar-h) 0 0 0`, column flex, `--panel`, `user-select: none` |
| `dbg-head` | header band, bottom border `--border-soft` |
| `dbg-title` | crumb + h1 stack, h1 at 1.05rem |
| `dbg-sev` | roll-up chip row, centred, `margin-inline: auto` |
| `dbg-sev-chip` | mono count chip; `.err` danger, `.warn` warn, `.quiet` faint when zero |
| `dbg-head-actions` | right-side button row |
| `dbg-body` | flex row: rail + content, `min-height: 0` |
| `dbg-rail` | 168px, `--panel`, right border, own scroll |
| `dbg-rail-item` | nav button; `.active` matches `stx-item.active` (accent tint + inset bar) |
| `dbg-rail-count` | right-aligned mono badge; `.err` danger text, `.warn` warn text |
| `dbg-content` | `--bg-0` floor, scrolls vertically, `padding: 14px 18px`, `min-width: 0` |
| `dbg-foot` | footer band, top border, privacy sentence left, redact toggle right |
| `dbg-redact` | the toggle's label styling (mono micro-label) |

Content blocks:

| class | job |
| --- | --- |
| `dbg-card` | bounded panel: `--panel`, 1px `--border-soft`, `--r-lg`, `--panel-pad`, stacked with 12px gap |
| `dbg-card-h` | mono micro-label header row (0.62rem, 0.14em tracking, `--faint`, ruled line after), holds `dbg-card-actions` |
| `dbg-card-actions` | right-aligned buttons in a card header |
| `dbg-grid` | two-column card grid, `minmax(0, 1fr)`; collapses to one column under 860px content width |
| `dbg-kv` | label/value grid, `grid-template-columns: max-content 1fr`; `.k` muted small label, `.v` value (mono where machine text), selectable |
| `dbg-empty` | empty-state block: centred `muted small`, dashed `--border-soft` border, generous padding |
| `dbg-note` | inline advisory line under a value, `muted small` |

Tables:

| class | job |
| --- | --- |
| `dbg-table-wrap` | `overflow-x: auto`, owns the horizontal scrollbar |
| `dbg-table` | dense table: mono cells at 0.72rem for machine text, `--ui` for name cells; header row mono micro-label style; row borders `--border-soft`; text selectable |
| `dbg-row-toggle` | expandable row affordance (chevron cell, `cursor: pointer`, hover `--bg-elev`) |
| `dbg-row-detail` | expansion row: full-width cell, `--bg-0` inset block listing addresses/outcomes |
| `num` | right-aligned numeric cell, `font-variant-numeric: tabular-nums` |

Feeds:

| class | job |
| --- | --- |
| `dbg-feed` | card variant holding a bar + scroller, `display: flex; flex-direction: column; min-height: 0` |
| `dbg-feed-bar` | controls row: filter input, chips, pause, copy; wraps at narrow widths |
| `dbg-feed-filter` | the text filter input, `--bg-elev`, mono, flex 1, `min-width: 140px` |
| `dbg-lvl-chips` | level toggle chip group; chips get `.on` (job colour) vs off (`--faint`, `--border-soft`) |
| `dbg-feed-count` | `n of m shown` readout in the bar, mono micro, `--faint` |
| `dbg-feed-scroll` | the scroller: `--bg-0`, `--r`, inner border, `overflow-y: auto`, fixed max-height per section, `role="log"`, selectable |
| `dbg-drop-note` | pinned ring-buffer notice at the scroller top, `muted small`, bottom border |
| `dbg-line` | one feed line, mono, `--fs-msg`, pre-wrap + `overflow-wrap: anywhere`; level classes `lvl-err`, `lvl-warn`, `lvl-info`, `lvl-debug`, `lvl-trace` colour the tag |
| `dbg-ts` | timestamp span, `--faint` |
| `dbg-tag` | fixed-width level/source tag span |
| `dbg-target` | tracing target span, `--muted` |
| `dbg-dir` | TX/RX span; `.tx` `--transfer`, `.rx` `--text-2` |
| `dbg-line-stack` | collapsed stack expander inside a line; open state reveals a `--bg-0` pre block |
| `dbg-copy` | tiny per-row/per-line copy button, hidden until row hover/focus-within |
| `dbg-pii` | wrapper for redactable values; carries `data-kind`; no visual change unredacted |

Voice:

| class | job |
| --- | --- |
| `dbg-voice-peer` | per-participant card inside Voice, uses `dbg-kv` internally |

## 7. Token rules (restating the contract)

- Never hardcode a colour; presets redefine the grounds and accent.
- Status is a job: `--ok` connected/healthy, `--warn` advisory (backing off, CAUTION,
  TURN relay, unconfirmed AutoNAT), `--danger` failure (unreachable, ERROR, failed
  dial/delivery), `--transfer` bytes or attempts in motion (dialling, pending re-dial,
  pending uploads, TX). Accent appears only in navigation chrome (active rail item,
  focus rings, toggle), never as status.
- `--mono` for machine text: ids, addresses, log lines, timestamps, counts, chips.
  `--ui` for labels, prose, member names.
- Grounds: console chrome on `--panel`, content floor and scrollers on `--bg-0`,
  inputs/hover on `--bg-elev`. Radii: `--r` for rows/controls/chips, `--r-lg` for cards.

## 8. Component sketch (Svelte 5 runes)

Capture ring, shared by frontend feed and the roll-up:

```js
// debug-console.svelte.js
export function makeRing(cap) {
  let items = $state([]);
  let seen = $state(0);
  return {
    get items() { return items; },
    get seen() { return seen; },
    get dropped() { return Math.max(0, seen - items.length); },
    push(entry) {
      seen += 1;
      items.push(entry);
      if (items.length > cap) items.splice(0, items.length - cap);
    },
  };
}

export const feRing = makeRing(500);
export const sevCounts = $state({ errors: 0, warnings: 0 });
```

Feed pause and filter, inside the console component:

```svelte
<script>
  let { ring } = $props();
  let paused = $state(false);
  let frozen = $state([]);
  let filter = $state("");
  const source = $derived(paused ? frozen : ring.items);
  const shown = $derived(source.filter((e) =>
    !filter || e.text.toLowerCase().includes(filter.toLowerCase())));
  const pending = $derived(paused ? ring.seen - frozenSeen : 0);
</script>

<button class="ghost small" aria-pressed={paused}
  onclick={() => { paused = !paused; if (paused) { frozen = [...ring.items]; frozenSeen = ring.seen; } }}>
  {paused ? `Resume (+${pending})` : "Pause"}
</button>
```

Backend side: a `tracing_subscriber` layer writes into a `Mutex<VecDeque>` ring plus two
`AtomicU64` counters; a Tauri command returns the snapshot and counters, and while a
front subscription is live the layer also emits a batched Tauri event (flush at most
every 250ms, arrays not single events) so a TRACE storm cannot melt the IPC.

## 9. Implementation notes

- Render batching: append feed lines through a 250ms (or rAF) batcher; never one DOM
  update per event.
- Feed scrollers use fixed max-heights (Backend/Frontend around `60vh` since they own
  their section; Network's dial feed around `30vh` because the peer table matters more).
- Keep the console mounted only while open; capture rings live outside the component.
- Copy must go through the same text serialiser the renderer uses, so copy, redact and
  display can never disagree.
- The section badge counts in the rail derive from the same rings; no second bookkeeping.
- Everything at 1000px was checked in the mockup: rail 168px, cards single-column below
  860px content width, tables scroll inside `dbg-table-wrap`.
