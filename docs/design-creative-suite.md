# Design: the creative suite (draw, doodle, flipnote, emoji sound, play, knock)

Status: proposal, revision 5, for review. Nothing here has shipped. Revision 4 closed the
identity-transport, fetch-bounding, ring-size and export-capacity findings. Revision 5 addresses
the fourth review: avatar consent is bound to the pictured pixels (2.12), round admission is one
transition table with a partition path (2.14), presence session state moves off the signal path
onto the capability-bound data channels (2.12), the history prerequisite is an enforceable
strategy rather than paging alone and blocks every new document type (2.9, section 7), game
liveness and terminal state are specified (2.14), and the medium findings on the no-score export
header, the emoji slice split, the identity encoding, export over-cap and enumerator traversal,
attachment expiry, and stamp publication are folded in.

Builds on the jam layer (`docs/INTERFACES.md` section 12, `apps/desktop/src/jam-*.ts`), the
livery palette (`design-livery.md`), the sealed blob store with its staging area, the fileshare
and its `FileExpiry` semantics, the backend `Clock` seam, and the authenticated call-signal push
(`KIND_CALL_SIGNAL`).

The brief: Game Boy Camera, Pictochat, Flipnote and Mario Paint flavoured features that give
people who are not talking a first-class way to take part in a call, that people build together
rather than alone, and that reuse the server's own identity (its palette, its custom emoji, its
members' profile pictures with consent) instead of stock assets. "Avatar" throughout means the
picture a member has chosen for their server profile, never a camera image of them.

## 1. Goals / non-goals

**Goals.**

- A member who is not speaking in a call can draw, and the drawing is noticed the way speech is.
- A flipnote can be made by several people at once, with sound, inside the app.
- A custom emoji can carry a sound. Clicking it in chat plays a note locally. The same sounds
  are the instruments of a Mario Paint style composer.
- A message or an announcement can be answered with a drawing.
- A voice room can host a small game whose pieces are the server's emoji and, with live consent,
  its members' avatars as bosses.
- Ringing a room can carry a short musical knock chosen by the caller, under the callee's
  control.
- Everything new on the wire is events or tiny indexed rasters decoded by code we own, never
  media bytes handed to a platform decoder. The one exception is GB cam, which is a filter on the
  existing video track and is labelled as such.

**Non-goals (v1).**

- Real audio samples on emoji. Sounds are synth patches.
- Anti-cheat for games. Scores are host-reported and labelled so.
- Preventing a member from disrupting a game round. Disruption is made attributable, visible and
  mutable, not impossible (2.14).
- Persisting Pictochat drawings automatically. Pinning is the bridge into chat.
- Drawings or games visible to members not in the call.
- Direct-person ringing. Rooms only.
- Durable scoreboards of any kind.
- Cryptographic proof that a pinned drawing's pixels are unmodified (2.13).
- Retention enforcement. Recorded inputs only (2.10).

## 2. Settled decisions

Each of these is a fork. Change deliberately.

### 2.1 `pix:v1` is a byte-exact binary format decoded by our own code

All drawings, doodles, stamps and flipnote frames are `pix:v1`.

```
magic     4 bytes  "PIX1"
w1        u8       W = w1 + 1   (1..256)
h1        u8       H = h1 + 1   (1..256)
n1        u8       N = n1 + 1   (4..16; n1 is 3..15)
palette   N entries x 4 bytes: role u8, r u8, g u8, b u8
runs      pairs of (len1 u8, idx u8) until exactly W*H pixels are produced
```

- Roles: 0 literal, 1 `bg`, 2 `fg`, 3 `accent`, 4 `muted`, 5..8 the fixed tones `t0..t3`.
  Any other role byte rejects. Every entry carries an RGB fallback. Duplicate entries reject.
- A run is `len1 + 1` pixels (1..256) of palette index `idx`; `idx >= N` rejects. Runs are
  maximal: a run followed by a run of the same index rejects.
- Decode order: parse the fixed header; reject if `W*H > 65536`; allocate `W*H` bytes once;
  consume runs with bounds checks; reject on overshoot, undershoot, or trailing bytes.
- Cap: 64 KiB measured on the encoded byte length, checked before any decode.
- Canonicality, stated narrowly: the encoder emits maximal runs, so the same pixel array under
  the same palette always encodes to the same bytes. Palette order and unused entries are not
  canonicalized. Content addresses identify encodings, not pictures.
- Rendering: decode into an `ImageData`, paint to a canvas. Bytes never reach `<img>`.
- Recolouring: authored fallback colours by default; "adapt drawings to my theme" is a
  per-viewer option.

C0a ships this with golden vectors and rejection tests for every rule above.

### 2.2 One canvas component, several modes, two publication paths

`apps/desktop/src/pix-canvas.ts` (controller) plus a thin Svelte shell: a palette-locked pixel
editor with pen, eraser, fill, line, stamp, text stamp, mirror, and onion skin.

| Mode | Size | Output | Published through |
|---|---|---|---|
| draw (Pictochat) | 192x144 | live strokes over the call `draw` channel; ephemeral | nothing |
| doodle reply | 128x96 | one pix blob attached to a message or announcement reply | `publish_pix` (2.10) |
| stamp editor | 32x32 | one pix file in the `emoji/` fileshare folder | the existing `add_file` path |
| flipnote frame | 192x144 | one pix blob per frame in a flipnote doc | `publish_pix` (2.10) |

A stamp is an ordinary fileshare entry: the stamp editor calls the existing upload path, which
stages and promotes chunks, writes the manifest, index entry and author attestation, and gives
the file normal fileshare discovery, expiry and dedup. `publish_pix` is only for blobs referenced
from creative documents, and stamps never use it. The picker learns to render a `.pix` file
through the `pix` decoder rather than the image resolver.

### 2.3 New realtime traffic gets its own data channels, same admission model

The jam `inst` channel (id 7) has a strict admission model: byte cap before parse, a per-peer
token bucket charged before `JSON.parse`, an auto-mute after sustained exhaustion, and an opaque
per-channel capability every callback must present. Drawing and games reuse that model on their
own negotiated channels, `draw` (id 8, ordered) and `play` (id 9, ordered), with their own
buckets. The admission code moves out of `jam-engine.ts` into `rt-admission.ts` (C0b), no
behaviour change for jam.

Each new channel also gets semantic limits on top of the byte bucket (2.7, 2.14). Every per-peer
key in this suite is the **full device identity**, canonical 64-character lowercase hex exactly
as `MemberView.identity` already exposes it (2.18), never the eight-character display
fingerprint.

### 2.4 An emoji's sound is an `emoji-sound:v1` envelope in its own folder

A custom emoji lives in the `emoji/` fileshare folder as `emoji/<code>.<ext>`. Its sound lives
in a sibling folder `emoji-sound/<code>.json`. Explicit envelope, all integers, unknown fields
reject:

```
{ "v": 1, "img": "<cid of the emoji image>", "patch": { ...jam-patch:v1... },
  "lo": 60, "hi": 84, "len": 120 }
```

- `patch` is validated by the unchanged `jam-patch:v1` validator. `lo <= hi`, both 0..127.
  `len` 20..1000 ms. No velocity.
- Binding: `img` must equal the cid the picker currently resolves for `<code>`; ties resolve to
  the smallest file cid.
- Authority: honoured only when the sidecar listing's **verified** `author_identity` equals the
  image listing's verified `author_identity`, or belongs to an owner or admin per the roles doc.
  Policy layer, not enforcement.

### 2.5 Clicking an emoji in chat is local; broadcasting is a separate, identity-dependent slice

A click plays a note-on locally at a random MIDI note in `[lo, hi]` and a note-off after `len`.
Nothing is sent. A per-viewer "emoji sounds" toggle silences the feature; local clicks throttle
to 8 per second. This is C2a and depends on nothing call-side.

In-call broadcast (C2b) is a per-session sender opt-in that sends an ordinary `t:"n"` note on
`inst`, spending the musical bucket, and depends on the identity bridge (C0d) like every other
call-side slice. No admin switch. Patch policy, stated honestly: `p` is honoured only after a
validated announce (one per 2 s, burst 3, 4-entry receiver cache); the sender mirrors that cache
and otherwise sends the legacy wave `w` only, and the opt-in text says so.

### 2.6 A drawing counts as speaking, but never over speech

1. While a peer is drawing (segments within the last 1.5 s) they are marked active in the same
   `speaking` map the voice stage uses, with a distinct `drawing` flag, and fill the
   active-speaker slot only when nobody is audibly speaking.
2. A finished drawing plays a short pen blip for everyone else via `playBlip`, or the author's
   knock if the ring that started this call carried one (2.15). Blips are budgeted per 2.7.
3. Finished drawings stack newest-first in a strip, fade after 90 s into a bounded history, with
   an unread count when another surface was focused.
4. Anyone in the call can pin a drawing into the text channel as a doodle message (2.13).

**Non-verbal mode is presentational.** It is a self-set flag in the member's presence frame on
the `draw` channel (2.12), shown as a badge. It never raises any budget. "Open the Draw tab when
a non-verbal member starts drawing" is a receiver-side preference, off by default, once per peer
per 60 s, subject to the per-peer hide. The local preference lives in vault-sealed per-device,
per-server settings.

### 2.7 The draw protocol is a versioned state machine

Every frame carries the sender session `sn` (the per-call-join nonce jam uses) and a per-sender
uint32 `q` shared across all draw frames for one `sn`. A receiver keeps, per authenticated draw
channel capability, the highest `q` seen and rejects anything at or below it, and rejects
`q == 0xffff_ffff`; a sender approaching it reopens under a fresh `sn`.

```
{t:"b", v:1, sn, q, d}                       begin drawing d
{t:"k", sn, q, d, s, c, w, p:[...]}          segment: stroke s, palette idx c 0..15, width w
                                              1..8, up to 24 integer pairs in the 192x144 grid
{t:"e", sn, q, d, s}                         stroke end
{t:"f", sn, q, d, h}                         finish; h = 64 hex SHA-256 over the sender's
                                              pix:v1 encoding of its own raster
{t:"x", sn, q, d}                            discard before finish, retract after
{t:"r", sn, q, rq}                           replay request
{t:"y", sn, q, rq, d, h, n, i, s, c, w, p}   replay response segment i of n
{t:"c", sn, q, obj, fr, on, ask}             flipnote frame claim (2.10)
{t:"pv", sn, q, nv}                          presence: non-verbal flag (2.12)
```

Rules: `d` strictly increasing per session; one open drawing per sender; a second `b` discards
the first; frames for a drawing not open reject; `f` with no segments or twice rejects; `x`
after `f` retracts. Finished drawings are immutable receiver-side snapshots; `h` mismatch marks
"may differ from the author's view". Replay: one outstanding `rq` per peer, 10 s timeout,
unsolicited responses reject, buffers keyed `(peer, rq)`, duplicate `i`, `i >= n`, `n > 2000`,
or a changing `d` or `h` aborts, over 200 KiB or timed out discards, a complete replay inserts
as replayed with no drawing state, blip or "new" treatment, and a sender answers at most one
replay per requester per 10 s for its most recent finished drawing. Reconnect: a new `sn`
resets `q` and `d`, closes any open drawing, permits one replay request.

Semantic limits per sender per receiver, separate from the byte bucket and unaffected by `nv`:
open drawings 1; begins 12 per minute; finishes 6 per minute then one blip per minute; segments
2000 per drawing; replays 1 per 10 s per requester and 200 KiB per response; strip 12; history
60 drawings or 4 MiB. "Hide this peer's drawings" suppresses rendering, attention, blips,
storage, history, replay and auto-open.

### 2.8 Scores use a discriminated cell schema and export deterministically to a valid take

The composer is a grid: rows are chromatic pitches C4..C6 (25 rows, MIDI 60..84), columns are
steps at sixteenth-note resolution. A scale is an editor view.

`jam-score:v1` root, in a `StudioObject` document (2.9):

```
{ v:1, kind:"score", id, channel, title, bpm:40..240, bpb:1..8, steps:1..256,
  patches: { <patch id>: <jam-patch:v1 descriptor> },        at most 64 entries
  cells:   { "<step>:<row>": {k:"n", p:<patch id>} | {k:"d", pad:0..9} } }
```

Caps: 64 patches and 4096 filled cells (the review measured 473,903 bytes for the maximal export
and the validator accepted it). Over-cap merges keep cells in ascending `(step, row)` order up
to 4096 and the 64 smallest patch ids, flagging the rest.

Export. Let `T = 15000`. Header: `group` = the stable group id the jam recorder uses; `call` =
lowercase hex SHA-256 over the canonical JSON array `["catcoms-score-export:v1", <score id>]`;
`met` = `{bpm, bpb}`; `parts` = `[exporter identity]`; `lanes` = `[{src:0, sn}]` with `sn` =
the first 16 hex of SHA-256 over `["catcoms-score-lane:v1", <score id>]`, last character `1` if
all zeros; `patches` in ascending id order. Timing: `on = floor(s * T / bpm)`,
`off = floor((s + 1) * T / bpm) - 1`, integer by construction, `off > on` for every valid bpm.
Events sorted by `(ms, off_before_on, row, kind)` then `q` assigned from 0. Canonical JSON.

Invariant, tested: **every accepted score exports to a take that `validateJamTake` accepts**,
and the exporter runs the validator before sharing.

### 2.9 Studio data lives in its own document types, under a history-bounded platform

Channel documents are observed through `ChannelChange`, so studio data gets its own document
types (next free discriminants after `Moderation = 14`):

| DocType | Id | One per | Holds |
|---|---|---|---|
| `StudioIndex = 15` | channel id | channel | `{ v:1, objects: { <object id>: {kind, title, created_by, ts, expiry, deleted?, doc} } }` |
| `StudioObject = 16` | object document id | one rotation epoch of a score or flipnote | the `jam-score:v1` or `flipnote:v1` root |
| `PostReplies = 17` | announcement post id | post | doodle replies (2.11) |

Every `StudioObject` root carries `kind`, `id` (the object id, stable across rotations),
`channel`, and `epoch`, validated against the document id and the index; a mismatch ignores
the object. Events: `AppEvent::StudioUpdated { channel, object: Option<id> }` and
`AppEvent::PostRepliesUpdated { post }`.

**Materialization caps bound the visible document, not its history.** The replication layer
retains every accepted signed operation, so a member overwriting one cell forever keeps the
visible map at one entry while growing disk, memory, sync and verification work on every peer
without limit. This is true of every document type today, including messages, and the studio
would make it cheaper. Paging catch-up divides that work; it does not bound it. So the
prerequisite in section 7, **P1, is an enforceable history-resource strategy**, and every new
remotely writable document type in this suite (studio index, studio objects, post replies)
depends on it. Its shape, decided here so the slices do not improvise:

- **Catch-up cursor is Automerge heads, never a numeric index.** Logs are stored in receiver
  arrival order and provider failover would skip or duplicate; the request carries the
  requester's known heads and the server returns changes not dominated by them, in a contiguous
  prefix that fits the response ceiling, with a safe restart when a provider changes.
- **Deterministic rotation with checkpoints.** A document has a history budget: 20,000 changes
  or 4 MiB of signed ops, whichever first, counted identically by every peer over the set it
  holds. When a writer observes the budget exceeded, it creates the successor document whose id
  is SHA-256 over `["catcoms-doc-rotate:v1", <object id>, epoch + 1]`, writes the materialized
  state as the successor's first change, and points the index entry's `doc` at it. The id is
  deterministic, so two concurrent rotators create the same document and their checkpoints
  merge under the ordinary conflict rules. A document past its budget accepts no further
  changes at materialization: changes beyond the budget in canonical order (topological, ties by
  change hash) are ignored by every peer alike. A fresh peer fetches only the current epoch;
  older epochs are droppable once the successor is held.
- **Per-author quota within an epoch.** Within one epoch, at most 2,000 changes per author
  count at materialization, in the same canonical order; later ones are ignored by every peer
  alike. This bounds what one member can force into the epoch that everybody keeps.
- **Ingest rate limit per author, defence in depth.** The sync layer today has no per-author op
  rate limit. P1 adds one for these document types (sustained 10 changes per second, burst 50,
  per author per document); excess is dropped at ingest and reconciles through catch-up. This
  is not a convergence rule and the design does not rely on it for correctness.

Studio-side behaviour on top: the editor coalesces writes (a cell is written only when its value
changes; a burst commits at most 4 times per second), and a document that reaches its budget
is shown as "rotating" while the successor is created.

Tests, owned by P1: thousands of tiny overwrites from a remote peer, the epoch closing at the
budget identically on two peers, a fresh peer restoring the latest value from the successor
only, quota overflow by one author ignored identically everywhere, heads-based catch-up across
a provider change with no skip or duplicate.

Authorization is honest-client. Conflicts: where this document says "last writer wins", the
exact rule is Automerge's concurrent-put resolution (`get` returns the value with the greatest
operation id; every concurrent value stays readable through `get_all`), and the UI surfaces the
others as "another version by X".

### 2.10 Blob publication, bounded fetch, references, exports, and expiry

**Publication order is stage, promote, publish**, matching the existing upload path, through one
primitive `publish_pix(bytes) -> (cid, len)`. A crash before promotion leaves a staged blob the
startup sweep clears; a crash after promotion, or a publication failure, leaves an orphan.

**Fetch is bounded by the declared size before storage.** `request_blob_bounded(cid, max_len)`
in the sync layer rejects a signed response body above `max_len` before decoding or storing and
never retries that cid at another peer. Creative fetches pass the record's declared `bytes`,
capped at 64 KiB for `pix` and 9 MiB for `pixa`. Tests: declared 64 KiB + 1 and a served body at
the 16 MiB ceiling, both rejected without a store.

**References are one enumerator.** `creative_pinned_cids()` walks every studio object's frames
and exports in every epoch still held, every chat doodle attachment, and every announcement
reply, returning the cids referenced by anything not deleted and not past its recorded expiry.
It reads **every concurrent value** of a conflicted `cid` field through `get_all`, not only the
displayed winner, so a losing concurrent frame is pinned until its conflict is resolved. The
retention pass must consult it.

**Exports have a durable record.** Under each flipnote root:

```
exports: { <export id>: { cid, bytes, author, ts, expiry, deleted? } }
```

At most 8: a merge that exceeds 8 materializes by keeping the 8 smallest export ids and treating
the rest as absent for display while the enumerator still pins them until they are deleted.
Replacement is a new record plus `deleted` on the old.

**Expiry is recorded now, enforced later**, on studio objects, exports, announcement replies and
**chat doodle attachments** (the attachment carries `expiry` explicitly), using `FileExpiry`
three-state semantics from the backend `Clock`. Metadata until enforcement exists.

**`flipnote:v1` root**, in a `StudioObject` document:

```
{ v:1, kind:"flipnote", id, channel, epoch, title, fps:1..24, w, h,
  frames: <CRDT list of frame ids>,
  frame: { <frame id>: { cid, bytes, author, ts } },
  score?: <score object id>,
  sfx: [ { fr:<frame id>, p:<patch id>, n:0..127 } ],          at most 4096
  patches: { <patch id>: descriptor },
  exports: { ... } }
```

Patch union rule: the linked score's patches plus the sfx patches must not exceed 64;
materialization keeps score patches first then sfx patches, each ascending by id, flags sfx
whose patch fell outside, and export refuses while any is flagged. The promise is 8 MiB of
frames summed over declared `bytes` in list order; 999 is the list cap; over-cap frames grey out
and editing is refused while any exist.

**Claims are ephemeral**, on the `draw` channel: `{t:"c", sn, q, obj, fr, on:1}`, `on:0`,
`ask:1`; live 90 s from the receiver's last receipt, re-sent every 30 s; sticky notification
with countdown and Pass; advisory, always overridable; no claims outside a call.

### 2.11 Announcement doodles are a per-post reply document

```
{ v:1, post, replies: { <reply id>: { author, cid, bytes, ts, expiry, deleted? } } }
```

Caps: 64 replies per post, 4 per author, 64 KiB each. Chat doodles are a message attachment
`{cid, bytes, expiry}` with `reply_to` set, one per message. Both publish through
`publish_pix`, fetch bounded, and are enumerated. `PostReplies` depends on P1 like every other
new document type.

### 2.12 Consent rides the capability-bound data channel and names the pixels it covers

Presence pings travel on the reordered, replayable signal path, and a delayed old `hello` can
recover a peer and make old session state look current again. Nothing that authorizes the use
of a member's avatar may depend on that path. So consent and non-verbal state do not use
presence pings at all:

- **Transport.** Avatar consent is a frame on the `play` channel, `{t:"pf", sn, q, on:0|1, cid}`;
  non-verbal is `{t:"pv", sn, q, nv}` on the `draw` channel. Both are ordered by the channel's
  `q`, bound to the channel's capability, and impossible to replay from an earlier session
  because an earlier session's channel no longer exists. A `hello`, delayed or replayed, can at
  most trigger the existing renegotiation; it cannot rebind anything this suite reads.
- **Freshness.** The sender restates each frame every 5 s; a receiver reads `on` or `nv` as false
  unless restated within 15 s, and as false the moment the channel closes.
- **The pixels are named.** `cid` is the content address of the avatar the member consents to
  show, at most 64 KiB (the existing avatar cap). A receiver renders as a boss **only** a blob
  whose address equals the `cid` from that member's own authenticated consent frame, fetched
  through `request_blob_bounded`, and never an avatar resolved through the profile document or
  any fingerprint-keyed map. Profiles and avatar lookup remain keyed by the short fingerprint
  today and are written by convention; that is why the avatar is bound here and not there. A collision fixture (two identities sharing four fingerprint bytes) is exercised through
  actual boss rendering: A's avatar cannot appear under M's consent, because M's frame names
  M's cid and the receiver renders nothing else.
- **Receivers enforce independently**: a conforming client stops rendering the avatar the moment
  its own consent view is false or stale, whatever the host's state frames say. The host also
  must not start a round with an avatar it does not hold consent for.
- What this governs: conforming clients. A member who already fetched an avatar can reuse it.

Tests: consent from A cannot set B's; old-session channel closed then a new session opened and
the old frames replayed are rejected by capability; a replayed `hello` after a new session does
not restore consent; consent lapses after 15 s and on channel close; a receiver hides an avatar
the host still sends; a boss cid that resolves to a blob over 64 KiB is rejected; the collision
fixture renders only M's named blob.

### 2.13 Attribution says what the evidence supports

A pinned drawing renders as "B says this was drawn by A", with the draw session `sn`, drawing
id `d`, and the author's `h` recorded for corroboration. Scores read "reported by <host>".

### 2.14 Games: committed rounds, one transition table, liveness, terminal state

**Round commitment.** A start frame is
`{t:"g", v:1, sn, q, on:1, g, r, desc}` where `desc` is the canonical start descriptor
`{game:"paddle"|"swat", players:2..4, seed:<uint32>, opts:{...per game, bounded}}` and
`r` = lowercase hex SHA-256 over `["catcoms-play-round:v1", callId, g, senderIdentity,
canonical(desc)]`. The receiver recomputes `r` from the authenticated sender and the received
`desc`; a mismatch rejects. Because the descriptor is inside the hash, a re-announce with a
changed game or options is a different round, not an update. Because there is no sender nonce,
each identity has exactly one `r` per `(g, desc)`, so one candidate per identity per generation
holds by construction and `r` cannot be ground.

**Receiver state.** `live = {g, r, host, desc, lastSeen}` or none; `G` = `live.g` if live, else
the generation of the last round that ended; `next` = at most 4 queued proposals for `G + 1`,
each expiring 30 s after its last re-announce; tombstones = `r` values of ended rounds, kept
10 minutes.

**Transition table** for an admitted proposal with generation `gp` and round `rp`:

| Condition | Action |
|---|---|
| `rp` tombstoned | reject |
| `gp < G` and not live | adopt `rp` as live (we are behind a room that is still playing) |
| `gp < G` and live | foreign collision: see below |
| `gp == G` and not live | reject (that generation ended here) |
| `gp == G` and live, same `desc` lineage | same-generation candidate: smaller `r` wins, at any time |
| `gp == G + 1` and live | queue in `next`; refresh its expiry on re-announce |
| `gp == G + 1` and not live | adopt as the election for `G + 1`; competes with other `G + 1` candidates by smaller `r` |
| `gp > G + 1` and not live | adopt (we are behind) and set `G = gp` |
| `gp > G + 1` and live | foreign collision: see below |

**Foreign collision** (a live round meets a proposal from a different generation): a single
frame never ends a game. The foreign proposal becomes a *candidate* and is promoted to a live
foreign round only after a second re-announce at least 2 s after the first. Two live rounds then
resolve by the smaller `r`, compared as hex; the losing host sends stop for its own round and
every peer sets `G` to the winner's generation. This is how a healed partition converges
whichever side advanced further, including one side advancing two or more generations. A
member can force a collision deliberately; with ungrindable `r` they win it only if their
identity's `r` happens to be smaller, the UI names them ("X started a competing round"), the
per-peer play mute ignores their proposals entirely, and moderation applies. That residual is
the stated non-goal.

**Liveness.** The host re-announces its start frame every 2 s. Proposals and re-announces share a
per-sender budget of one per 2 s, burst 3. A live round whose host has not re-announced for 6 s
(three missed intervals, receiver-observed) is presumed gone: the round ends, its `r` is
tombstoned, and the oldest unexpired `next` candidate whose proposer is still connected starts
after a 2 s election window; if none, the room is idle. Queued proposals are revalidated at start
(proposer connected, budget not exhausted, `r` still not tombstoned).

**Ordering.** `sn` and `q` follow the draw rules: per-sender uint32 shared across all play
frames for one `sn`, strictly increasing, rejected at the ceiling, reset only by a channel reopen.
Re-announces are new frames with fresh `q`. Every non-proposal frame names `g` and `r`. Inputs
`{t:"i", sn, q, g, r, k}` from players; state `{t:"s", sn, q, g, r, tick, st}` from the host
only, `tick` a uint32 strictly increasing per round; stop `{t:"g", sn, q, on:0, g, r}` from the
host only; terminal `{t:"z", sn, q, g, r, tick, res}` from the host only, at most once per `r`,
and any state or terminal frame for that `r` after it rejects. Only `z` may append a result.
State frames have their own bucket, 20 per second, burst 40.

**Per-game schemas**, strict, unknown fields reject, all integers:

- Paddle: `k` bitfield 0..3. `st` = `{b:[x,y], v:[dx,dy], p:[y...], s:[...]}` with `p` and `s`
  exactly `players` long, positions 0..1000, velocities -50..50, scores 0..99.
- Swatter: `k` = `{x, y, hit:0|1}` in 0..1000. `st` = `{f:[{i, e, x, y}], boss?:{who, x, y,
  hp}, s:[...]}` with at most 16 flies, `i` unique within the frame (0..65535), `e` an index
  0..255 into the call's emoji list, `hp` 0..100, `s` exactly `players` long, 0..999, and the
  boss avatar rendered only if the receiver's own consent view for `who` (2.12) is true.
- `res` = `{s:[...]}` exactly `players` long plus `winner` 0..3 or -1.

**Results** are per call: each `z` appends one entry to an in-memory call history capped at 20,
oldest evicted; nothing is written to any document.

Tests: reversed simultaneous delivery converging; a same-generation proposal delayed 30 s still
converging; a partition where one side advances two generations converging on recovery; a
next-generation proposal during a live round queued, expiring after 30 s, and revalidated at
start; a re-announce with a changed `desc` treated as a different round; a mismatched `r`
rejected; a tombstoned `r` rejected; a single foreign frame not ending a game; non-host state,
stop and terminal rejected; duplicate and out-of-order `q` and `tick`; duplicate fly ids and
oversized arrays rejected; a second terminal for one `r` rejected; host silence for 6 s;
the state bucket; the result history cap.

### 2.15 A knock is a bounded take carried inside each authenticated ring

There is no profile knock. The caller keeps a default knock in local vault-sealed settings and
supplies it inside every ring. `JamKnock`: `{v:1, p:<jam-patch:v1>, e:[{ms, n, on}]}` with
`ms <= 3000`, at most 32 events.

`ring:v1`: the core authenticates the sender, charges a per-device bucket (refill 8 per second,
burst 64) before queueing, bounds the queue at 256, and caps the whole control request at
64 KiB. The ring limit is post-parse: a `ring` payload is dropped if the **original decoded
UTF-8 byte length** exceeds 2 KiB, measured on the bytes the bridge delivered. Exact fields,
unknown reject: `{type:"ring", v:1, callId, rn, ttl, knock?, msg?}`; `rn` 16 hex; `ttl`
1..60000 ms from receipt; `msg` at most 140 characters after NFC, no control characters. Replay
suppression keyed by full identity: `(sender, rn)` for 5 minutes, one honoured ring per
`(sender, callId)` per 30 s. Playback is a standalone renderer on the notification
`AudioContext`, one voice, hard-stopped at 3 s, torn down on Deafen, "knocks off", quiet hours,
vault lock, roster removal, a preference change, or joining the room.

### 2.16 `pixa:v1` is byte-exact and holds every accepted flipnote

```
magic     4 bytes  "PIXA"
fps       u8       1..24
count     u16 LE   1..999
frames    count x ( len u32 LE, pix:v1 bytes )
take_len  u32 LE
take      canonical jam-take:v1 JSON; patch array = the materialized union (2.10)
sfx_n     u16 LE   0..4096
sfx       sfx_n x ( frame u16 LE, patch u8, note u8 )
```

**The no-score take is fully specified**: `group` = the stable group id; `call` = lowercase hex
SHA-256 over `["catcoms-flipnote-export:v1", <flipnote id>, epoch]`; `met` = `{bpm:120, bpb:4}`;
`parts` = `[exporter identity]`; `lanes` = `[{src:0, sn}]` with `sn` derived as in 2.8 from
`["catcoms-flipnote-lane:v1", <flipnote id>]`; `patches` = the union; `events` = `[]`. With a
score, the take is the score export from 2.8 with its patch array replaced by the union (score
patches keep their indices because they come first). Caps: frames 8 MiB, take 512 KiB, sfx
16 KiB, whole export 9 MiB. Decoding validates every length before allocating and re-runs
`validateJamTake`. Tests: a byte-exact no-score golden vector; a 64-patch score plus one
disjoint sfx patch refused at materialization; a 63-patch score plus one sfx patch round-trips.

### 2.17 GB cam is a filter on the video track and a visual effect, not anonymization

Capture at 160x144, four grey tones with an ordered dither, sent through the existing video
path with the sender's bitrate capped. The receiver applies nearest-luma quantization to four
levels after platform decoding and maps them onto its livery tones. Labelled as a look and a
bandwidth saver. Depends on the existing capture-session leases, per-edge sender caps, a
`captureStream` fallback, the receiver render-loop lifecycle, and the fail-closed sender.

### 2.18 The full device identity crosses the bridge, as lowercase hex, inbound and outbound

The core reduces an authenticated sender to `roles::fingerprint` (four bytes, eight hex
characters) before `CallSignal` reaches the actor, the bridge and the UI, and a member can
generate a device key sharing those four bytes with a victim. Decision:

- `CallSignal` carries `from: String`, the canonical **64-character lowercase hex** of the
  device id, exactly the encoding `MemberView.identity` already uses, alongside `from_fp`. The
  bridge forwards both. This closes the encoding question: no second representation.
- The outbound `SendCallSignal` contract takes a full identity as its primary parameter. The
  legacy short-fingerprint route remains only for older call sites, refuses when more than one
  current member matches, and is removed once C0d re-keys the callers.
- Every authority, session, replay, bucket, claim, consent, host and per-game key in the UI uses
  `from`. Existing call state (`callPeers`, `peerMeta`, `speaking`, `callProfiles` lookups made
  from call state) is re-keyed in the same change. `from_fp` remains for display.

This is C0d and a prerequisite for every call-side slice. Fixture: two injected device
identities sharing their first four bytes, exercised through consent, boss rendering (2.12),
ring replay keys, and host attribution.

## 3. Data model summary

| Thing | Where | Format | Cap |
|---|---|---|---|
| doodle, frame | sealed blob via `publish_pix`, fetched bounded | `pix:v1` | 64 KiB encoded |
| stamp | `emoji/` fileshare entry via `add_file` | `pix:v1` | 64 KiB encoded |
| emoji sound | `emoji-sound/<code>.json` | `emoji-sound:v1` | 4 KiB |
| studio index | `StudioIndex` doc per channel | objects map with expiry and current `doc` | 64 objects |
| score | `StudioObject` doc per epoch | `jam-score:v1` | 64 patches, 4096 cells |
| flipnote | `StudioObject` doc per epoch + blobs | `flipnote:v1` | 8 MiB frames, 999 frames, 4096 sfx, union 64 |
| flipnote export | sealed blob, recorded under `exports` | `pixa:v1` | 9 MiB, 8 records |
| announcement doodles | `PostReplies` doc per post | replies map with expiry | 64 per post, 4 per author |
| chat doodle | message attachment `{cid, bytes, expiry}` | pix cid | 1 per message |
| knock | local vault-sealed settings, sent in each ring | `JamKnock` | 3 s, 32 events |
| avatar consent | `play` channel `t:"pf"` with named avatar cid | 0/1 + cid | 15 s freshness |
| non-verbal | `draw` channel `t:"pv"` | 0/1 | 15 s freshness |
| frame claims | `draw` channel `t:"c"` | ephemeral | 90 s receiver-observed |
| round results | in-memory call history | per game | 20 per call |
| document history | per epoch | signed ops | 20,000 changes or 4 MiB; 2,000 per author |

## 4. Wire summary

- **`draw` (id 8).** The state machine in 2.7, claims, and the `pv` presence frame.
- **`play` (id 9).** The round protocol, per-game schemas, terminal frame, and the `pf` consent
  frame in 2.12 and 2.14.
- **Call signal.** `ring:v1` per 2.15. `CallSignal` carries the full sender identity inbound
  and `SendCallSignal` takes one outbound (2.18). Presence pings are **unchanged**: nothing in
  this suite reads authority, session or consent from them.
- **`inst` (id 7).** Unchanged.

## 5. UI

- **Call view.** Draw and Play tabs; strip under the peer tiles; draw ring colour token; pin on
  each drawing; non-verbal toggle and badge; round results and history; "X started a competing
  round" notice with a per-peer play mute.
- **Chat.** Note glyph on emoji with sound; "reply with a doodle"; inline doodles at 128x96.
- **Announcements.** A doodle wall under each post.
- **Studio.** Per-channel surface listing flipnotes, scores, exports, expiry and rotation state;
  timeline, canvas, onion skin, score; claims with countdown and Pass; conflicted frames as
  "another version by X".
- **Settings.** Emoji sounds; broadcast clicks (with the legacy-wave caveat); knocks off, quiet
  hours, default knock editor; adapt drawings to my theme; "use my avatar as a boss" and non-verbal
  preferences; open Draw when a non-verbal member draws; GB cam.

## 6. Threat notes

- **Identity.** Every per-peer key is the full device identity in one encoding (2.18).
- **Avatars.** A boss avatar renders only from the cid its owner named in a capability-bound, fresh
  consent frame, never from a fingerprint-keyed or convention-written record (2.12).
- **Untrusted bytes.** `pix:v1` and `pixa:v1` decode with caps enforced before allocation;
  fetches are bounded by declared size before storage; patches everywhere reuse the jam
  validator; document roots are strict-decoded and self-describing.
- **History.** Materialization caps do not bound op logs; P1 bounds them with heads-based
  catch-up, deterministic rotation with checkpoints, per-author epoch quotas, and an ingest rate
  limit, and blocks every new document type (2.9, section 7).
- **Amplification and floods.** Own byte buckets, byte caps before parse, exhaustion auto-mute,
  semantic limits on begins, finishes, replays, proposals, re-announces, state frames and
  retained bytes. Rings are bounded post-parse on decoded bytes and rate-limited by the core.
- **Availability.** A round can be disrupted by a deliberate collision; a single frame never
  ends a game, `r` is ungrindable and bound to the descriptor, the disruptor is named, the
  per-peer play mute ignores them, and moderation applies.
- **Harassment.** Per-peer hide covers attention, sound, storage, replay and auto-open.
  Non-verbal mode raises no budget.
- **Privacy.** Strokes are content and any recipient can retain them; disclosed once in the
  Draw tab. Local preferences are vault-sealed. `pix:v1` carries no metadata.
- **Honest-client boundary.** Studio authorization, sidecar authority, attribution fields,
  reported scores, consent flags and claims are honest-client and labelled so.
- **Clocks.** Scheduling and receiver-observed freshness only. Expiry deadlines come from the
  backend `Clock`.

## 7. Phases and prerequisites

**Platform prerequisites this suite depends on but does not own.**

| Prereq | Delivers | Blocks |
|---|---|---|
| P1 | history-bounded documents per 2.9: heads-based catch-up cursor with safe provider restart, deterministic rotation with checkpoints, per-author epoch quota, per-author ingest rate limit; its own adversarial review | C3a, C5a, and everything after them |
| P2 | retention enforcement consulting `wiki_pinned_cids()` and `creative_pinned_cids()`; not blocking, the reason expiry is recorded now | none |

**Slices.** Each is one reviewable PR with its own tests.

| Slice | Delivers | Depends on |
|---|---|---|
| C0a | `pix:v1` codec, golden vectors, rejection tests | none |
| C0b | `rt-admission.ts` extracted; jam tests unchanged | none |
| C0c | `publish_pix`, `request_blob_bounded` with tests, `creative_pinned_cids()` skeleton reading `get_all` | none |
| C0d | full identity (lowercase hex) through `CallSignal` inbound and `SendCallSignal` outbound; call state re-keyed; four-byte-collision fixture | none |
| C1 | canvas component; stamp editor writing a `.pix` fileshare entry through `add_file`; picker renders `.pix` | C0a |
| C2a | `emoji-sound:v1` envelope, folder, binding and authority rules, picker glyph, local click, settings toggle | C0a |
| C2b | in-call broadcast opt-in with the announce policy | C2a, C0d |
| C3a | `PostReplies` doc type, event, expiry, enumerator coverage; announcement doodle wall | C1, C0c, P1 |
| C3b | chat doodle attachment with expiry, inline render, enumerator coverage | C1, C0c |
| C4a | `draw` channel transport, state machine, `pv` frame, semantic limits; debug list only | C0b, C0d |
| C4b | Draw tab, strip, history, attention rules, blips, hide-peer, non-verbal badge and auto-open preference | C4a |
| C4c | pin to chat with 2.13 attribution; replay on join | C4b, C3b |
| C5a | `StudioIndex` and `StudioObject` doc types with epochs, self-describing roots, typed events, bounds, materialization, expiry, editor coalescing | P1, C0c |
| C5b | score editor and CRDT cell writes | C5a, C2a |
| C5c | deterministic take export and the export invariant suite | C5b |
| C6a | `flipnote:v1` root, frame records with `bytes`, patch union rule, over-cap materialization, export records with the over-cap rule, enumerator coverage | C5a |
| C6b | timeline, frame editor, conflict surfacing, ephemeral claims with Pass | C6a, C1, C4a |
| C6c | soundtrack and sfx playback | C6b, C5c |
| C6d | `pixa:v1` export with the no-score golden vector, local GIF save | C6c |
| C7 | knock: local default, editor, `ring:v1` with the decoded-byte check, ring dialog, callee preferences and teardown | C2a, C0d |
| C8a | `play` channel, round commitment, transition table, foreign-collision promotion, liveness, queue expiry, ordering, terminal frame, state bucket, per-peer play mute, `pf` consent frame with named cid | C0b, C0c, C0d |
| C8b | paddle schema and game | C8a |
| C8c | swatter schema and game with emoji and consenting members' avatars rendered only from named cids | C8a, C2a |
| C8d | per-call results in the Play tab and bounded history | C8a |
| C9 | GB cam | video path as landed |

C2a, C3b, C8a are independent after their prerequisites. C6 must not start until C4 and C5 have
had a review pass. Prerequisites noted in the jam section still apply to anything that renders
sound: Deafen must gate rendering, and roster revocation must tear down the removed member's
call connections.

## 8. Remaining open questions

Nothing is open at the design level. One implementation check is owned by its slice: C5a
confirms the exact `AppEvent` emission point and that the desktop bridge forwards the new events
with the same shape as `StatusUpdated`.
