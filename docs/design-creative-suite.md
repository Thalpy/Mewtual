# Design: the creative suite (draw, doodle, flipnote, emoji sound, play, knock)

Status: proposal, revision 8, for review. Nothing here has shipped. Revision 7 removed the
close-record circularity, sourced the consented avatar from local attested state, and ordered
rounds by rank then id. Revision 8 addresses the seventh review: the registry is discovered by
walking winning closes forward from a fixed root, with full-projection seeds and lineage, and
no existence probing (2.9); a frozen epoch is rebuilt atomically from the retained closures and
excluded history is removed rather than hidden (2.9); closes and seeds are quarantined with
per-author processing budgets and never enter the durable log until retained (2.9); live and
queued rounds are kept apart from dead ones so re-announces are valid (2.14); equivocations
collapse to one candidate per identity and election and dead-set growth is budgeted per identity
(2.14); the consented avatar record is installed in two durable phases (2.12); and the medium
findings on intent ids and caps, cursor binding, and seed size are folded in.

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
  media bytes handed to a platform decoder. Two exceptions are named and bounded: GB cam (2.17)
  and the boss avatar, which reuses the existing avatar image path (2.12).

**Non-goals (v1).**

- Real audio samples on emoji. Sounds are synth patches.
- Anti-cheat for games. Scores are host-reported and labelled so.
- Preventing the single lowest-ranked member of a call from disrupting a game round.
  Disruption is made attributable, visible and muteable, not impossible (2.14).
- Persisting Pictochat drawings automatically. Pinning is the bridge into chat.
- Drawings or games visible to members not in the call.
- Direct-person ringing. Rooms only.
- Durable scoreboards of any kind.
- Cryptographic proof that a pinned drawing's pixels are unmodified (2.13).
- Retention enforcement. Recorded inputs only (2.10).
- Authority stronger than honest-client for studio content. Every document in this suite may be
  written by any member; the platform work in 2.9 guarantees convergence and bounded resources,
  not that a member cannot write bad content.

## 2. Settled decisions

Each of these is a fork. Change deliberately.

### 2.0 Conventions for hashes and canonical bytes

Every hash in this document is `H(domain, part1, part2, ...)` = SHA-256 over the concatenation
of, for each part in order, a 4-byte big-endian length followed by the part's bytes. `domain` is
an ASCII string. String parts are UTF-8 with no normalization unless stated. Integers are
8-byte big-endian. Identities are the 32 raw device-id bytes. Change hashes are their 32 raw
bytes. "Canonical JSON" means the serialization the jam patch validator already uses: object
keys sorted by UTF-16 code unit, no whitespace, integers only, strings escaped per RFC 8259 in
the shortest form. Every derivation ships a golden vector in the slice that introduces it.

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
  canonicalized.
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

A stamp is an ordinary fileshare entry through the existing upload path (staging, promotion,
manifest, index entry, author attestation, normal discovery, expiry and dedup). `publish_pix` is
only for blobs referenced from creative documents. The picker renders a `.pix` file through the
`pix` decoder rather than the image resolver.

### 2.3 New realtime traffic gets its own data channels, same admission model

The jam `inst` channel (id 7) has a strict admission model: byte cap before parse, a per-peer
token bucket charged before `JSON.parse`, an auto-mute after sustained exhaustion, and an opaque
per-channel capability every callback must present. Drawing and games reuse that model on their
own negotiated channels, `draw` (id 8, ordered) and `play` (id 9, ordered), with their own
buckets. The admission code moves into `rt-admission.ts` (C0b), no behaviour change for jam.

Every frame type in this suite has an exact key set; unknown or missing keys reject; every
string field has a fixed length or a stated maximum; every frame fits 200 bytes after the
1024-byte pre-parse cap. Each new channel also gets semantic limits on top of the byte bucket
(2.7, 2.14). Every per-peer key in this suite is the **full device identity**, canonical
64-character lowercase hex exactly as `MemberView.identity` already exposes it (2.18).

### 2.4 An emoji's sound is an `emoji-sound:v1` envelope in its own folder

A custom emoji lives in `emoji/<code>.<ext>`; its sound in `emoji-sound/<code>.json`:

```
{ "v": 1, "img": "<cid of the emoji image>", "patch": { ...jam-patch:v1... },
  "lo": 60, "hi": 84, "len": 120 }
```

`patch` is validated by the unchanged `jam-patch:v1` validator; `lo <= hi`, both 0..127; `len`
20..1000 ms; no velocity; unknown fields reject. `img` must equal the cid the picker currently
resolves for `<code>`; ties resolve to the smallest file cid. Authority: honoured only when the
sidecar listing's **verified** `author_identity` equals the image listing's, or belongs to an
owner or admin per the roles doc. Policy layer, not enforcement.

### 2.5 Clicking an emoji in chat is local; broadcasting is a separate, identity-dependent slice

A click plays a note-on locally at a random MIDI note in `[lo, hi]` and a note-off after `len`.
Nothing is sent. Per-viewer "emoji sounds" toggle; local clicks throttle to 8 per second (C2a).

In-call broadcast (C2b) is a per-session sender opt-in sending an ordinary `t:"n"` note on
`inst`, spending the musical bucket, and depends on C0d. No admin switch. `p` is honoured only
after a validated announce (one per 2 s, burst 3, 4-entry receiver cache); the sender mirrors
that cache and otherwise sends the legacy wave `w` only, and the opt-in text says so.

### 2.6 A drawing counts as speaking, but never over speech

1. While a peer is drawing (segments within the last 1.5 s) they are marked active in the same
   `speaking` map the voice stage uses, with a distinct `drawing` flag, and fill the
   active-speaker slot only when nobody is audibly speaking.
2. A finished drawing plays a short pen blip for everyone else via `playBlip`, or the author's
   knock if the ring that started this call carried one (2.15). Blips are budgeted per 2.7.
3. Finished drawings stack newest-first in a strip, fade after 90 s into a bounded history, with
   an unread count when another surface was focused.
4. Anyone in the call can pin a drawing into the text channel as a doodle message (2.13).

**Non-verbal mode is presentational.** A self-set flag in the member's presence frame on the
`draw` channel (2.12), shown as a badge. It never raises any budget. "Open the Draw tab when a
non-verbal member starts drawing" is a receiver-side preference, off by default, once per peer
per 60 s, subject to the per-peer hide. The local preference lives in vault-sealed per-device,
per-server settings.

### 2.7 The draw protocol is a versioned state machine

Every frame carries the sender session `sn` (the per-call-join nonce jam uses, 16 lowercase
hex) and a per-sender uint32 `q` shared across all draw frames for one `sn`. A receiver keeps,
per authenticated draw channel capability, the highest `q` seen and rejects anything at or
below it, and rejects `q == 0xffff_ffff`; a sender approaching it reopens under a fresh `sn`.

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
{t:"c", sn, q, obj, fr, on, ask}             flipnote frame claim (2.10); obj and fr 32 hex
{t:"pv", sn, q, nv}                          presence: non-verbal flag 0|1 (2.12)
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
{ v:1, kind:"score", id, channel, epoch, title, bpm:40..240, bpb:1..8, steps:1..256,
  patches: { <patch id>: <jam-patch:v1 descriptor> },        at most 64 entries
  cells:   { "<step>:<row>": {k:"n", p:<patch id>} | {k:"d", pad:0..9} } }
```

Caps: 64 patches and 4096 filled cells (the review measured 473,903 bytes for the maximal export
and the validator accepted it). Over-cap merges keep cells in ascending `(step, row)` order up
to 4096 and the 64 smallest patch ids, flagging the rest.

Export. Let `T = 15000`. Header: `group` = the stable group id the jam recorder uses; `call` =
hex of `H("catcoms-score-export:v1", score id)`; `met` = `{bpm, bpb}`; `parts` = `[exporter
identity]`; `lanes` = `[{src:0, sn}]` with `sn` = the first 16 hex of
`H("catcoms-score-lane:v1", score id)`, last character `1` if all zeros; `patches` in ascending
id order. Timing: `on = floor(s * T / bpm)`, `off = floor((s + 1) * T / bpm) - 1`, integer by
construction, `off > on` for every valid bpm. Events sorted by `(ms, off_before_on, row, kind)`
then `q` assigned from 0. Canonical JSON.

Invariant, tested: **every accepted score exports to a take that `validateJamTake` accepts**,
and the exporter runs the validator before sharing.

### 2.9 Studio data lives in its own document types, under a history-bounded platform

Channel documents are observed through `ChannelChange`, so studio data gets its own document
types (next free discriminants after `Moderation = 14`):

| DocType | Id | One per | Holds |
|---|---|---|---|
| `StudioIndex = 15` | per epoch | channel | `{ v:1, kind:"index", channel, epoch, objects: { <object id>: {kind, title, created_by, ts, expiry, deleted?} } }` |
| `StudioObject = 16` | per epoch | score or flipnote | the `jam-score:v1` or `flipnote:v1` root |
| `PostReplies = 17` | per epoch | announcement post | doodle replies (2.11) |
| `DocRegistry = 18` | per epoch, rooted at a fixed id | server | current epoch of every logical document above, plus lineage |

Every root carries `kind`, its logical key (`channel`, or `id` and `channel`, or `post`) and
`epoch`, validated against the registry entry it was reached from; a mismatch ignores the
document. Events: `AppEvent::StudioUpdated { channel, object: Option<id> }`,
`AppEvent::PostRepliesUpdated { post }`, `AppEvent::RegistryUpdated`.

**Materialization caps bound the visible document, not its history.** The replication layer
retains every accepted signed operation, so a member overwriting one cell forever keeps the
visible map at one entry while growing disk, memory, sync and verification work on every peer
without limit. So **P1 is a monotonic epoch-close protocol**, and every new remotely writable
document type in this suite depends on it. What P1 guarantees is bounded resources and
convergence among peers holding the same information; authority stays honest-client (section 1).
Its shape, decided here so the slices do not improvise:

**Logical documents and epochs.** A logical document is a chain of epoch documents, each an
ordinary replicated document with its own 128-bit id. Epoch 0's id is the first 16 bytes of
`H("catcoms-doc-epoch0:v1", doc type tag, logical key)` (for the registry, the logical key is
the server id). Every later epoch's id is derived from the close it descends from (below), so a
chain is **forward-walkable**: holding any epoch, a peer can compute its successor's id from that
epoch's winning close, and the current epoch is the deepest one reachable from epoch 0 through
valid winning closes. There is no existence probe anywhere in this protocol; an empty catch-up
response means only that one provider had nothing.

**Budget.** An epoch's budget is 20,000 changes and 4 MiB of signed ops. Both are hard maxima.

**Close.** Any member may write a close change into an epoch document: `close = { heads:
[<change hash>...] }`, at most 64 heads, nothing else. Its **successor id** is
`first 16 bytes of H("catcoms-doc-epoch:v1", doc type tag, logical key, epoch + 1, close
change hash)`, derived after the change is hashed. A close is **valid** only if its closure
(every change reachable from `heads`) satisfies all of: at least 10,000 changes or at least
2 MiB; at most 20,000 changes; at most 4 MiB.

**Quarantine before the log.** A close, and any change that arrives for an epoch the peer
already knows to be closing, is held in a bounded quarantine and never appended to the durable
op log until it is retained under the rules below. Quarantine bounds: 16 closes per epoch and
2 MiB of non-close changes per epoch, oldest evicted. Close processing is budgeted per author:
at most one close validation in flight per author per epoch, at most 4 validations per author
per epoch per hour, and a close naming heads the peer does not hold triggers at most 3 provider
fetch attempts before it is parked and re-examined only when a missing head arrives through
ordinary gossip. Invalid closes go into a negative cache by change hash for the epoch's life.
A cheap close naming nonexistent heads therefore costs the peer at most three bounded fetches
and one cache entry, and an author cannot churn candidates faster than the hourly budget.

**Equivocation.** If an author writes several valid closes in one epoch, the one with the
smallest change hash counts and the others are ignored, identically for peers holding the same
set. Grinding this chooses only which valid bounded closure that author freezes.

**Candidates and retention.** A peer retains at most 8 candidate closes per epoch, the 8 with
the smallest change hash among valid ones, and the dependency-closed union of exactly their
closures: at most 32 MiB plus 8 closes, regardless of member count.

**Freezing rebuilds the document.** Whenever the retained candidate set changes, the peer
**reconstructs** the epoch's `EncryptedDoc` from scratch out of the dependency-closed union of
the retained closures, then atomically replaces the live document, its log and its persisted
snapshot with the reconstruction. Changes already ingested before the first close that lie
outside that union are thereby removed, not hidden: they leave the Automerge state, the log,
snapshots and materialization alike, so a peer that ingested a stray branch before the close
and a peer restored solely from the winning closure hold identical epochs. Content is projected
from the **winner** only (below); the union is retained so a later candidate change does not
require refetching. After freezing, the ingest rule is: accept only changes inside a retained
closure, or closes (through quarantine).

**Winner.** Among retained candidates, the close with the smallest change hash wins. Peers with
partial information may follow a loser transiently and switch when they learn of a smaller-hash
close; every switch is a rebuild as above.

**Seed.** The successor's initial content is a **seed change**: an ordinary signed change,
transported and persisted like every other, whose content is the materialization of exactly the
winning `heads`, serialized canonically, applied under actor id
`H("catcoms-seed-actor:v1", successor id, writer identity)`, encoded size at most 2 MiB (below
the sealed-op ceiling, verified in P1 with the maximal score and flipnote seeds). The winning
close's author writes it; if none appears within 60 s of a peer observing the winner, any member
may. Seeds are **quarantined until selected**: the successor document is not opened for reading
or editing until one seed is selected, and selection is: among seeds the peer has verified
against its own recomputation from the closure, the one from the smallest writer identity; a
peer that cannot yet verify (it lacks the previous epoch's closure) selects the smallest-identity
seed provisionally, shows "seed not yet verified", and **disables editing** until it has fetched
that closure (bounded by one budget) and verified. If verification or a newly arrived seed
changes the selected writer, the successor is rebuilt from the selected seed and the peer's own
intents (below) are replayed onto it. Non-selected seeds stay quarantined and never enter the
successor's log, so later edits can never attach to a non-selected or unverified seed. A wrong
seed is attributable to its writer and grants nothing an ordinary bad edit would not.

**Post-close edits and durable intents.** A change to a closed epoch that is not in the winning
closure is late: dropped, absent from the frozen materialization. The writing client keeps
every local edit intent, vault-sealed, until it observes that intent's effect inside the winning
closure or inside a verified selected seed. Each intent has a stable id
`H("catcoms-intent:v1", logical key, writer identity, local nonce)` written into the change it
produces (a hidden `iid` on the value, or an entry under `applied/<iid>` for list insertions),
and replay is idempotent: an intent whose `iid` is already present in the target epoch is
confirmed, not re-applied, so a crash between replaying a frame insertion and recording its
confirmation cannot duplicate the frame. Caps: 64 KiB per intent, 10,000 intents and 4 MiB per
logical document, 64 MiB across the vault; beyond a cap the oldest are dropped with a visible
warning. No peer ever migrates another peer's late edits.

**Registry.** `DocRegistry` is a logical document under exactly the same protocol, rooted at the
fixed epoch-0 id above, so its current epoch is found by walking closes forward from that root.
Two properties make the walk cheap and lossless:

- Every registry seed carries the **complete pointer projection** of the closed epoch, so a
  pointer present in epoch `n` is present in every later seed until explicitly deleted; nothing
  is lost across rotations and no republication is needed.
- Every registry seed also carries `lineage`, the ordered list of winning close hashes from
  epoch 0 to its predecessor (32 bytes each, at most 4096 entries), so a peer that reaches any
  registry epoch can verify, hop by hop and lazily, that it descends from the root, and two
  peers that reach different candidates for "current" compare lineages: the longer valid
  lineage wins, and equal-length lineages that diverge resolve at the divergence point by the
  winner rule.

A joiner starts at the root and walks forward; each hop fetches one bounded epoch. A current
member's catch-up response may carry a **hint** (a registry epoch id and its lineage) that lets
the joiner start its walk near the top; the hint is verified by walking, not trusted, and a
forged far-future epoch is unreachable because it has no valid close chain from the root. In
practice `n` is tiny: a registry change is one pointer update per rotation of any logical
document, each rotation costs at least half a budget of real ops in that document, and the
registry itself rotates after roughly 10,000 such updates. Tests: a forged far-future registry
epoch is never adopted; providers with partial epoch sets; an empty newly created epoch; a
pointer surviving five rotations without any writer touching it; restoration after 500
rotations in a bounded number of requests.

**Catch-up is cursor-driven; heads only prune.** A request carries up to 64 of the requester's
heads and an optional opaque provider cursor. A response page is **topologically ordered and
dependency-complete** relative to the requester's heads plus earlier changes on the same page,
and returns a new cursor. The cursor is an HMAC under a provider-local secret over
`(provider identity, requester identity, doc type, doc id, frozen-log generation, position,
expiry)`, expiry 10 minutes; a provider whose log was rebuilt (any freeze) bumps the generation,
so an old position is never interpreted against new contents and the provider answers "restart".
Progress is guaranteed by the cursor: a page may include changes the requester already holds,
which apply idempotently, and a requester with more than 64 heads still advances. Tests: 65 and
20,000 concurrent heads, a reversed-arrival dependency case, provider failover mid-transfer with
no skipped or duplicated application, a provider rebuild forcing restart.

**Ingest rate limit per author, defence in depth**: sustained 10 changes per second, burst 50,
per author per epoch document, dropped at ingest; not relied on for correctness.

Studio-side behaviour on top: the editor coalesces writes (a cell is written only when its value
changes; a burst commits at most 4 times per second), and a document at or past its lower bound
is shown as "rotating" while a close is written.

Tests, owned by P1: excluded changes ingested before the winning close are absent after the
freeze, after snapshot and after restore, and the peer's epoch equals one restored solely from
the closure; concurrent closers with different partial histories converging on the smallest-hash
winner with a rebuild on switch; an equivocating author reduced to one close; a ninth candidate
not retained; an oversized closure rejected; a close naming unknown heads parked after three
fetches; the hourly per-author validation budget; a late change dropped identically
everywhere; an acknowledged edit outside the winning heads replayed from a retained intent and
not duplicated across a crash mid-replay; an offline writer replaying intents; a crash after the
close and before the registry pointer (any observer derives the successor and writes the
pointer); a wrong seed rejected by recomputation and the successor rebuilt onto the selected
seed with intents replayed; editing disabled while a seed is unverified; the maximal score and
flipnote seeds under 2 MiB; the registry tests above; golden vectors for every derivation.

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
capped at 64 KiB for `pix` and 9 MiB for `pixa`.

**References are one enumerator.** `creative_pinned_cids()` walks every studio object's frames
and exports in every epoch still held, every chat doodle attachment, and every announcement
reply, returning the cids referenced by anything not deleted and not past its recorded expiry,
reading **every concurrent value** of a conflicted `cid` field through `get_all`. The retention
pass must consult it.

**Exports have a durable record.** Under each flipnote root:

```
exports: { <export id>: { cid, bytes, author, ts, expiry, deleted? } }
```

At most 8 for display (a merge beyond 8 keeps the 8 smallest ids visible; the enumerator still
pins the rest until deleted). Replacement is a new record plus `deleted` on the old.

**Expiry is recorded now, enforced later**, on studio objects, exports, announcement replies and
chat doodle attachments (`{cid, bytes, expiry}`), using `FileExpiry` three-state semantics from
the backend `Clock`.

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
with countdown and Pass; advisory; no claims outside a call.

### 2.11 Announcement doodles are a per-post reply document

```
{ v:1, kind:"replies", post, epoch, replies: { <reply id>: { author, cid, bytes, ts, expiry, deleted? } } }
```

Caps: 64 replies per post, 4 per author, 64 KiB each. Chat doodles are a message attachment
`{cid, bytes, expiry}` with `reply_to` set, one per message. Both publish through
`publish_pix`, fetch bounded, and are enumerated. `PostReplies` depends on P1.

### 2.12 Avatar consent rides the capability-bound data channel and names locally attested pixels

Presence pings travel on the reordered, replayable signal path, and a delayed old `hello` can
recover a peer and make old session state look current again. Nothing that authorizes the use
of a member's avatar may depend on that path, and nothing may depend on the profile document
either, because profile entries are keyed by the four-byte fingerprint and written by
convention, so a colliding identity can overwrite a member's own entry.

- **Transport.** Avatar consent is `{t:"pf", sn, q, on, cid}` on the `play` channel: exact
  keys, `on` 0 or 1, `cid` exactly 64 lowercase hex, `sn` 16 lowercase hex, unknown or missing
  keys reject. Non-verbal is `{t:"pv", sn, q, nv}` on the `draw` channel, `nv` 0 or 1. Both are
  ordered by the channel's `q`, bound to the channel's capability, and impossible to replay from
  an earlier session because that session's channel no longer exists. A `hello`, delayed or
  replayed, can at most trigger the existing renegotiation; it rebinds nothing this suite reads.
- **Freshness.** The sender restates each frame every 5 s; a receiver reads `on` or `nv` as false
  unless restated within 15 s, and as false the moment the channel closes.
- **The pixels are named from locally attested state, installed in two durable phases.** The
  backend keeps a **vault-sealed record keyed by (server id, this device's full identity)**
  holding the cid of the avatar this device last published successfully. `my_avatar_cid()`
  reads only that record, never the profile document. Replacing or clearing an avatar is a
  two-phase operation: first the record is durably cleared (the vault write completes before
  anything else happens), then the profile is published, then the new cid is durably installed
  only after publication reports success. A crash or vault-write failure at any point leaves
  the record empty, so consent is off; the member sets the avatar again to restore it. An
  installation from before this record existed has no record and no consent until the member
  sets an avatar. The sender places the recorded cid in its consent frame; with no record the
  consent control is greyed with an explanation.
- **Receivers render only the named blob.** A receiver renders as a boss **only** a blob whose
  address equals the `cid` in that member's own authenticated consent frame, fetched through
  `request_blob_bounded` at the 64 KiB avatar cap, never an avatar resolved through the profile
  document or any fingerprint-keyed map.
- **Decoder exception, stated.** Avatars are opaque JPEG, PNG, GIF or WebP bytes that the app
  already sniffs by magic and renders through an image element. The boss path reuses exactly
  that existing, size-bounded platform-decoder surface: the fetched blob is sniffed the same way,
  an unrecognised magic is rejected before rendering, and the blob goes to no other decoder.
- **Async completion rechecks.** When the bounded fetch completes, the receiver renders only if
  the delivering channel capability is still current, consent for that identity is still fresh
  and `on`, and the fetched cid equals the cid in the latest consent frame; otherwise the result
  is discarded.
- **Receivers enforce independently**: a conforming client stops rendering the avatar the moment
  its own consent view is false or stale, whatever the host's state frames say. The host also
  must not start a round with an avatar it does not hold consent for.
- What this governs: conforming clients. A member who already fetched an avatar can reuse it.

Tests: a colliding identity overwriting the profile entry before the query does not change the
consented cid; the record survives restart; a crash after the clear and before publication, and
a crash after publication and before the install, both leave consent off; a vault-write failure
at either phase leaves consent off; clearing the avatar clears the record; consent from A cannot
set B's; old-session frames replayed after a new session are rejected by capability; a replayed
`hello` does not restore consent; consent lapses after 15 s and on channel close; a receiver
hides an avatar the host still sends; a cid over 64 KiB is rejected; an unrecognised image
magic is rejected; fetches completing after revocation or avatar change discard; the
four-byte-collision fixture renders only the named blob; malformed `pf` frames reject.

### 2.13 Attribution says what the evidence supports

A pinned drawing renders as "B says this was drawn by A", with the draw session `sn`, drawing
id `d`, and the author's `h` recorded for corroboration. Scores read "reported by <host>".

### 2.14 Games: identity rank, chained rounds, named players, liveness, terminal state

**Rank is fixed per identity per call and contains no sender choice.**
`k = H("catcoms-play-rank:v1", callId, senderIdentity)`, compared as bytes. The one member
whose `k` is smallest in a call wins every collision they choose to force; that is the residual
named in the non-goals: one specific, attributable, muteable person.

**Total order.** Candidates are ordered by `(k, r)`: `k` primary, round id `r` secondary.

**Rounds are chained, not numbered.** A start frame is
`{t:"g", v:1, sn, q, on:1, prev, r, desc}` with `r = H("catcoms-play-round:v1", callId,
senderIdentity, prev, canonical(desc))` as 64 hex, `prev` the `r` of the round this one follows
(64 zeros for a first round), and `desc` the committed descriptor. The receiver recomputes `r`;
a mismatch rejects. The simulation seed is derived, never chosen: the first 4 bytes of
`H("catcoms-play-seed:v1", r)`. A re-announce with a changed `desc` is a different round.

**Three sets, kept apart.**

- **Live and queued**: the live round's `r` and the `next` queue's `r` values. An exact
  re-announce of one of these from the same authenticated host refreshes it; that is how
  liveness and queue expiry work, and it is never a rejection.
- **Dead**: the `r` of every round that ended (stop, terminal, host silence, foreign loss) and
  of every candidate that lost an election. A proposal naming a dead `r` rejects for the rest of
  the call.
- **One candidate per (identity, prev)**: for a given election (`prev`), a receiver keeps at
  most one candidate per proposer identity, the smallest `(k, r)` it has seen from them; a
  different descriptor from the same identity for the same `prev` replaces it if smaller and is
  otherwise dropped, and neither outcome adds to the dead set. So one identity sending different
  descriptors to different peers still yields one winner everywhere, and an identity cannot fill
  anyone's dead set by equivocating.

Dead-set growth is budgeted per identity: each identity may contribute at most 64 dead entries
per call (its own ended rounds and lost candidacies); beyond that, its proposals are refused
while everyone else's continue. The global dead set is additionally capped at 4096 as a hard
bound; the per-identity budget means no single member can reach it.

**Committed descriptor**, canonical JSON, all integers except identities:

```
{ game: "paddle" | "swat",
  players: [ <64 hex identity>, ... ],      2..4, unique, ordered; slot i is players[i]
  opts: paddle -> { speed: 1..3, to: 5..21 }
        swat   -> { waves: 1..10, boss: 0|1 } }
```

Slot `i` is owned by `players[i]`. An input is accepted only from the channel capability whose
identity is that slot's owner. A player whose channel closes leaves the slot absent; the host
may continue with the slot idle or end the round; the same identity re-claims its slot when its
channel reopens. The host need not be a player.

**Receiver state.** `live = {r, prev, host, desc, lastSeen}` or none; `L` = `live.r` if live,
else the `r` of the last round that ended here (or zeros); `next` = at most 4 queued proposals
with `prev == live.r`, each expiring 30 s after its last re-announce.

**Transition table** for a proposal `(rp, prevp, hostp)` whose `rp` is not dead and which
survives the one-candidate rule:

| Condition | Action |
|---|---|
| `rp` is live or queued and `hostp` is its host | refresh `lastSeen` or queue expiry |
| not live, `prevp == L` | adopt as the election; competes with other candidates having the same `prevp` by smallest `(k, r)`, at any time; losers go dead |
| not live, `prevp != L` | foreign: adopt (the only live game we know of) and set `L` accordingly |
| live, `prevp == live.prev` | same-election candidate: smallest `(k, r)` wins, at any time; loser goes dead |
| live, `prevp == live.r` | queue in `next` |
| live, otherwise | foreign collision: see below |

**Foreign collision** (a live round meets a live round of another lineage, the healed-partition
case): a single frame never ends a game. The foreign proposal becomes a candidate and is promoted
only after a second re-announce at least 2 s after the first. Two live rounds then resolve by the
smaller host `(k, r)`; the losing host sends stop for its own round, the loser goes dead, and
every peer adopts the winner. A member can force this deliberately and wins only if their `k` is
the smaller; the UI names them ("X started a competing round"), the per-peer play mute ignores
their proposals entirely, and moderation applies.

**Liveness.** The host re-announces its start frame every 2 s. Proposals and re-announces share a
per-sender budget of one per 2 s, burst 3. A live round whose host has not re-announced for 6 s
(receiver-observed) is presumed gone: the round goes dead, and the unexpired `next` candidate
with the smallest `(k, r)` whose proposer is still connected starts after a 2 s election window;
if none, the room is idle. Queued proposals are revalidated at start.

**Ordering.** `sn` and `q` follow the draw rules. Re-announces are new frames with fresh `q`.
Every non-proposal frame names `r`. Inputs `{t:"i", sn, q, r, k}` from slot owners; state
`{t:"s", sn, q, r, tick, st}` from the host only, `tick` strictly increasing per round; stop
`{t:"g", sn, q, on:0, r}` from the host only; terminal `{t:"z", sn, q, r, tick, res}` from the
host only, at most once per `r`, after which any state or terminal for that `r` rejects. Only
`z` may append a result. State frames have their own bucket, 20 per second, burst 40.

**Per-game schemas**, strict, unknown fields reject, all integers unless stated:

- Paddle: input `k` bitfield 0..3. `st` = `{b:[x,y], v:[dx,dy], p:[...], s:[...]}` with `p`
  and `s` exactly `players.length` long, positions 0..1000, velocities -50..50, scores 0..99.
- Swat: input `k` = `{x, y, hit:0|1}` in 0..1000. `st` = `{f:[{i, e, x, y}], boss?:{who, x,
  y, hp}, s:[...]}` with at most 16 flies, `i` unique within the frame (0..65535), `e` 0..255
  into the call's emoji list, all coordinates 0..1000, `hp` 0..100, `who` exactly 64 lowercase
  hex and rendered only if the receiver's own consent view for that identity (2.12) is true,
  `s` exactly `players.length` long, 0..999.
- `res` = `{s:[...]}` exactly `players.length` long plus `winner` = -1 or 0..players.length-1.

**Results** are per call: each `z` appends one entry to an in-memory history capped at 20.

Tests: a live host's 2 s re-announce and a queued candidate's refresh are accepted for the life
of the round; offline seed and option grinding has no effect on selection; one identity sending
two descriptors with reversed delivery on two peers converging on the same `(k, r)` winner with
no dead-set growth; 5,000 distinct descriptors from one higher-ranked identity refused after its
budget while others keep playing; concurrent starts with different descriptors resolving by
`(k, r)`; reversed queue arrival promoting the smallest `(k, r)`; an old round re-proposed after
a channel reopen rejected by the dead set; a partition where one side played three rounds
converging on recovery; a next-round proposal during a live round queued, expiring after 30 s,
revalidated at start; a changed `desc` treated as a different round; a mismatched `r` rejected;
a single foreign frame not ending a game; input from a non-slot identity rejected; a player
reconnecting to its slot; non-host state, stop and terminal rejected; duplicate and
out-of-order `q` and `tick`; duplicate fly ids, out-of-range coordinates and oversized arrays
rejected; `winner` out of range rejected; a second terminal for one `r` rejected; host silence
for 6 s; the state bucket; the result history cap.

### 2.15 A knock is a bounded take carried inside each authenticated ring

There is no profile knock. The caller keeps a default knock in local vault-sealed settings and
supplies it inside every ring. `JamKnock`: `{v:1, p:<jam-patch:v1>, e:[{ms, n, on}]}` with
`ms <= 3000`, at most 32 events.

`ring:v1`: the core authenticates the sender, charges a per-device bucket (refill 8 per second,
burst 64) before queueing, bounds the queue at 256, and caps the whole control request at
64 KiB. The ring limit is post-parse: a `ring` payload is dropped if the **original decoded
UTF-8 byte length** exceeds 2 KiB. Exact fields, unknown reject: `{type:"ring", v:1, callId,
rn, ttl, knock?, msg?}`; `rn` 16 hex; `ttl` 1..60000 ms from receipt; `msg` at most 140
characters after NFC, no control characters. Replay suppression keyed by full identity:
`(sender, rn)` for 5 minutes, one honoured ring per `(sender, callId)` per 30 s. Playback is a
standalone renderer on the notification `AudioContext`, one voice, hard-stopped at 3 s, torn
down on Deafen, "knocks off", quiet hours, vault lock, roster removal, a preference change, or
joining the room.

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

The no-score take: `group` = the stable group id; `call` = hex of
`H("catcoms-flipnote-export:v1", flipnote id, epoch)`; `met` = `{bpm:120, bpb:4}`; `parts` =
`[exporter identity]`; `lanes` = `[{src:0, sn}]` with `sn` derived from
`H("catcoms-flipnote-lane:v1", flipnote id)` as in 2.8; `patches` = the union; `events` = `[]`.
With a score, the take is the 2.8 export with its patch array replaced by the union. Caps:
frames 8 MiB, take 512 KiB, sfx 16 KiB, whole export 9 MiB. Decoding validates every length
before allocating and re-runs `validateJamTake`. Tests: a byte-exact no-score golden vector; a
64-patch score plus one disjoint sfx patch refused; a 63-patch score plus one sfx patch
round-trips.

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
  device id, exactly the encoding `MemberView.identity` already uses, alongside `from_fp`.
- The outbound `SendCallSignal` contract takes a full identity as its primary parameter. The
  legacy short-fingerprint route remains only for older call sites, refuses when more than one
  current member matches, and is removed once C0d re-keys the callers.
- Every authority, session, replay, bucket, claim, consent, host, rank and slot key in the UI
  uses `from`. Existing call state is re-keyed in the same change. `from_fp` remains for
  display.

This is C0d and a prerequisite for every call-side slice. Fixture: two injected device
identities sharing their first four bytes, exercised through consent, boss rendering, ring
replay keys, rank and host attribution.

## 3. Data model summary

| Thing | Where | Format | Cap |
|---|---|---|---|
| doodle, frame | sealed blob via `publish_pix`, fetched bounded | `pix:v1` | 64 KiB encoded |
| stamp | `emoji/` fileshare entry via `add_file` | `pix:v1` | 64 KiB encoded |
| emoji sound | `emoji-sound/<code>.json` | `emoji-sound:v1` | 4 KiB |
| registry | `DocRegistry` epoch chain from a fixed root | full pointer projection plus lineage | 4096 epochs |
| studio index | `StudioIndex` epoch per channel | objects map with expiry | 64 objects |
| score | `StudioObject` epoch | `jam-score:v1` | 64 patches, 4096 cells |
| flipnote | `StudioObject` epoch + blobs | `flipnote:v1` | 8 MiB frames, 999 frames, 4096 sfx, union 64 |
| flipnote export | sealed blob, recorded under `exports` | `pixa:v1` | 9 MiB, 8 visible records |
| announcement doodles | `PostReplies` epoch per post | replies map with expiry | 64 per post, 4 per author |
| chat doodle | message attachment `{cid, bytes, expiry}` | pix cid | 1 per message |
| knock | local vault-sealed settings, sent in each ring | `JamKnock` | 3 s, 32 events |
| consented avatar cid | vault-sealed local state keyed by (server, identity), two-phase install | cid | one per server |
| avatar consent | `play` channel `t:"pf"` | 0/1 + 64 hex cid | 15 s freshness |
| non-verbal | `draw` channel `t:"pv"` | 0/1 | 15 s freshness |
| frame claims | `draw` channel `t:"c"` | ephemeral | 90 s receiver-observed |
| round dead set | in-memory per call | round ids | 64 per identity, 4096 hard |
| round results | in-memory call history | per game | 20 per call |
| epoch history | per epoch document | signed ops | 20,000 changes and 4 MiB hard; close valid from 10,000 or 2 MiB |
| frozen epoch retention | per epoch document | union of candidate closures | 8 candidates, 32 MiB |
| quarantine | per epoch document | closes, changes, seeds | 16 closes, 2 MiB changes |
| seed | successor's first change | canonical materialization | 2 MiB |
| edit intents | vault-sealed local state | stable ids | 64 KiB each, 10,000 and 4 MiB per document, 64 MiB total |

## 4. Wire summary

- **`draw` (id 8).** The state machine in 2.7, claims, and the `pv` presence frame.
- **`play` (id 9).** The chained-round protocol, per-game schemas, terminal frame, and the `pf`
  consent frame in 2.12 and 2.14.
- **Call signal.** `ring:v1` per 2.15. `CallSignal` carries the full sender identity inbound
  and `SendCallSignal` takes one outbound (2.18). Presence pings are unchanged.
- **`inst` (id 7).** Unchanged.
- **Replication.** Close changes, seed changes, registry hints, and cursor-driven
  dependency-complete catch-up pages with bound cursors, all owned by P1.

## 5. UI

- **Call view.** Draw and Play tabs; strip under the peer tiles; draw ring colour token; pin on
  each drawing; non-verbal toggle and badge; round results and history; "X started a competing
  round" notice with a per-peer play mute; player slots shown by name.
- **Chat.** Note glyph on emoji with sound; "reply with a doodle"; inline doodles at 128x96.
- **Announcements.** A doodle wall under each post.
- **Studio.** Per-channel surface listing flipnotes, scores, exports, expiry and rotation state;
  timeline, canvas, onion skin, score; claims with countdown and Pass; conflicted frames as
  "another version by X"; "seed not yet verified" (read-only) and "wrong seed by X" states.
- **Settings.** Emoji sounds; broadcast clicks (with the legacy-wave caveat); knocks off, quiet
  hours, default knock editor; adapt drawings to my theme; "use my avatar as a boss" (greyed
  with an explanation when no locally attested avatar exists) and non-verbal preferences; open
  Draw when a non-verbal member draws; GB cam.

## 6. Threat notes

- **Identity.** Every per-peer key is the full device identity in one encoding (2.18).
- **Avatars.** The consented cid comes from vault-sealed local state installed in two durable
  phases around this device's own publication, never from the convention-owned profile
  document; a boss renders only from that cid, through the existing bounded avatar image path
  with magic sniffing, with async completion rechecked (2.12).
- **Untrusted bytes.** `pix:v1` and `pixa:v1` decode with caps enforced before allocation;
  fetches are bounded by declared size before storage; patches everywhere reuse the jam
  validator; document roots and every frame are strict-decoded.
- **History.** Epochs close on verifiable head sets with lower and upper bounds; closes, stray
  changes and seeds are quarantined with per-author processing budgets and never enter the log
  until retained; freezing rebuilds the document from retained closures and removes excluded
  history from state, log and snapshots; seeds are recomputable, selected before use, and
  editing waits for verification; late edits replay idempotently from durable intents; the
  registry is forward-walkable from a fixed root with lossless seeds and lineage (2.9).
- **Amplification and floods.** Own byte buckets, byte caps before parse, exhaustion auto-mute,
  semantic limits on begins, finishes, replays, proposals, re-announces, state frames and
  retained bytes. Rings are bounded post-parse on decoded bytes and rate-limited by the core.
- **Availability.** Round selection uses `(k, r)` with no sender input in `k`; a single frame
  never ends a game; live and queued rounds are refreshed, not rejected, by their host's
  re-announces; the dead set lasts the call with per-identity budgets; the lowest-ranked member
  of a call can disrupt rounds and is named, muteable and moderatable.
- **Harassment.** Per-peer hide covers attention, sound, storage, replay and auto-open.
  Non-verbal mode raises no budget.
- **Privacy.** Strokes are content and any recipient can retain them; disclosed once in the
  Draw tab. Local preferences and intents are vault-sealed. `pix:v1` carries no metadata.
- **Honest-client boundary.** Studio authorization, sidecar authority, attribution fields,
  reported scores, consent flags, claims, seed content and registry content are honest-client
  and labelled so.
- **Clocks.** Scheduling and receiver-observed freshness only. Expiry deadlines come from the
  backend `Clock`.

## 7. Phases and prerequisites

**Platform prerequisites this suite depends on but does not own.**

| Prereq | Delivers | Blocks |
|---|---|---|
| P1 | the epoch-close protocol of 2.9: heads-only closes with lower and upper closure bounds, successor ids derived from the close hash, quarantine with per-author processing budgets and negative cache, equivocation and candidate-retention rules, freeze-by-reconstruction with atomic replacement, seed changes with quarantine, verification, selection and rebuild, durable idempotent intents with stable ids and caps, the forward-walkable registry with full-projection seeds, lineage and hints, cursor-driven dependency-complete catch-up with bound cursors and restart, per-author ingest rate limit; its own adversarial review; golden vectors for every derivation | C3a, C5a, and everything after them |
| P2 | retention enforcement consulting `wiki_pinned_cids()` and `creative_pinned_cids()`; not blocking | none |

**Slices.** Each is one reviewable PR with its own tests.

| Slice | Delivers | Depends on |
|---|---|---|
| C0a | `pix:v1` codec, golden vectors, rejection tests | none |
| C0b | `rt-admission.ts` extracted; jam tests unchanged | none |
| C0c | `publish_pix`, `request_blob_bounded` with tests, `creative_pinned_cids()` skeleton reading `get_all`, the two-phase vault-sealed consented-avatar record around `set_profile` and `my_avatar_cid()` | none |
| C0d | full identity (lowercase hex) through `CallSignal` inbound and `SendCallSignal` outbound; call state re-keyed; four-byte-collision fixture | none |
| C1 | canvas component; stamp editor writing a `.pix` fileshare entry through `add_file`; picker renders `.pix` | C0a |
| C2a | `emoji-sound:v1` envelope, folder, binding and authority rules, picker glyph, local click, settings toggle | C0a |
| C2b | in-call broadcast opt-in with the announce policy | C2a, C0d |
| C3a | `PostReplies` under the registry, event, expiry, enumerator coverage; announcement doodle wall | C1, C0c, P1 |
| C3b | chat doodle attachment with expiry, inline render, enumerator coverage | C1, C0c |
| C4a | `draw` channel transport, state machine, `pv` frame, semantic limits; debug list only | C0b, C0d |
| C4b | Draw tab, strip, history, attention rules, blips, hide-peer, non-verbal badge and auto-open preference | C4a |
| C4c | pin to chat with 2.13 attribution; replay on join | C4b, C3b |
| C5a | `StudioIndex` and `StudioObject` under the registry, self-describing roots, typed events, bounds, materialization, expiry, editor coalescing, intent retention UI | P1, C0c |
| C5b | score editor and CRDT cell writes | C5a, C2a |
| C5c | deterministic take export and the export invariant suite | C5b |
| C6a | `flipnote:v1` root, frame records with `bytes`, patch union rule, over-cap materialization, export records, enumerator coverage | C5a |
| C6b | timeline, frame editor, conflict surfacing, ephemeral claims with Pass | C6a, C1, C4a |
| C6c | soundtrack and sfx playback | C6b, C5c |
| C6d | `pixa:v1` export with the no-score golden vector, local GIF save | C6c |
| C7 | knock: local default, editor, `ring:v1` with the decoded-byte check, ring dialog, callee preferences and teardown | C2a, C0d |
| C8a | `play` channel, `(k, r)` ordering, chained rounds with committed descriptors and named players, live/queued/dead sets with per-identity budgets, one candidate per identity and election, transition table, foreign-collision promotion, liveness, ordering, terminal frame, state bucket, per-peer play mute, strict `pf` consent frame with the locally attested cid and async rechecks | C0b, C0c, C0d |
| C8b | paddle schema and game | C8a |
| C8c | swat schema and game with emoji and consenting members' avatars rendered only from named cids through the existing avatar image path | C8a, C2a |
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
