# Design: the creative suite (draw, doodle, flipnote, emoji sound, play room, knock)

Status: proposal, for review. Nothing here has shipped. Builds on the jam layer
(`docs/INTERFACES.md` section 12, `apps/desktop/src/jam-*.ts`), the livery palette
(`design-livery.md`), the sealed blob store and fileshare, and the call-signal push.

The brief: a set of Game Boy Camera, Pictochat, Flipnote and Mario Paint flavoured features that
give people who are not talking a first-class way to take part in a call, that people build
together rather than alone, and that reuse the server's own identity (its palette, its custom
emoji, its members' avatars) instead of stock assets.

The plan is ordered so that every phase is a small addition to the one before it. Three shared
foundations carry the whole suite; each feature is a thin layer on those.

## 1. Goals / non-goals

**Goals.**

- A mute or muted member in a call can draw, and the drawing is noticed the way speech is.
- A flipnote can be made by several people at once, with sound, inside the app.
- A custom emoji can carry a sound. Clicking it in chat plays a note. The same sounds are the
  instruments of a Mario Paint style composer.
- A message or an announcement can be answered with a drawing.
- A voice room can host a small game whose pieces are the server's emoji and, with consent, its
  members' faces.
- Ringing someone can carry a short musical knock of the caller's choosing, under the callee's
  control.
- Everything new on the wire is events or tiny indexed rasters, never media bytes handed to a
  platform decoder. This is the jam layer's rule and it holds here.

**Non-goals (v1).**

- Real audio samples on emoji. Sounds are synth patches. Samples are a possible v2 kind.
- Any anti-cheat for games. Scores are honest-client and presented as such.
- Persisting Pictochat drawings automatically. Pinning is the bridge into chat.
- Drawings or games visible to members who are not in the call.
- GIF/APNG on the wire. Export to those formats is a local save only.

## 2. Settled decisions

Each of these is a fork. Change deliberately.

### 2.1 One pixel format, decoded by our own code

All drawings, doodles, stamps and flipnote frames are `pix:v1`: a fixed-size indexed raster.

- Header: `v` (literal 1), `w`, `h` (each 1..256), `pal`: 4..16 entries, each a palette
  **role** (`bg`, `fg`, `accent`, `muted`, and the four fixed tones `t0..t3`) or a literal
  `#rrggbb` fallback. Pixels: one byte per pixel, run-length encoded, index into `pal`.
- Caps: 64 KiB serialized. A frame that decodes to more than `w*h` pixels or fewer is rejected.
  Unknown fields reject.
- Rendering is a code path we own: decode into an `ImageData`, paint to a canvas. Bytes never
  reach `<img>`. This is the take player's rule applied to pictures, and it is also why doodles
  can render inline without the click-only image policy: there is no decoder surface and no
  address disclosure.
- Palette roles resolve against the viewer's livery at render time, so a doodle drawn under
  Nightshade recolours under a different preset. A literal fallback is written alongside each
  role so export outside the app is faithful.

Alternative considered: PNG through the existing media resolver. Rejected because it puts
untrusted bytes into the platform decoder, carries metadata, and cannot recolour.

### 2.2 One canvas component, several modes

`apps/desktop/src/pix-canvas.svelte` (or a `.ts` controller with a thin Svelte shell): a
palette-locked pixel editor with pen, eraser, fill, line, stamp (any custom emoji or pix blob),
text stamp, mirror, and onion skin (previous frame, next frame, other authors' live strokes).
Modes differ only in canvas size, persistence and where the output goes:

| Mode | Size | Output |
|---|---|---|
| draw (Pictochat) | 192x144 | live strokes over the call data channel; ephemeral |
| doodle reply | 128x96 | one pix blob attached to a message or announcement |
| stamp editor | 32x32 | one pix blob saved as a custom emoji |
| flipnote frame | 192x144 | one pix blob per frame in a flipnote doc |

The tokens-first UI refactor applies: the editor's chrome uses the operator-terminal tokens, and
the four fixed tones `t0..t3` are derived from the active preset so the editor looks like the
server.

### 2.3 New realtime traffic gets its own data channels, same admission model

The jam layer's `inst` channel (id 7) has a strict admission model: byte cap before parse, a
per-peer token bucket charged before `JSON.parse`, an auto-mute after sustained exhaustion, and
an opaque per-channel capability that every callback must present. Drawing and games reuse that
model verbatim but on their own negotiated channels: `draw` (id 8, ordered) and `play` (id 9,
ordered). They do not share the `inst` bucket, so a drawing storm cannot silence an instrument
and a note storm cannot freeze a game.

The admission code moves out of `jam-engine.ts` into a shared `rt-admission.ts` (bucket, size
caps, capability seam, exhaustion auto-mute) so the three channels cite one implementation.
This is a refactor with no behaviour change for jam and is the first thing the draw phase does.

### 2.4 An emoji's sound is a patch reference, not a sample

A custom emoji lives in the `emoji/` fileshare folder as `emoji/<name>.<ext>`. Its sound is a
sidecar file `emoji/<name>.patch.json` holding one `jam-patch:v1` descriptor, validated by the
same validator as wire patches. Optional extra fields, all integers, unknown reject: `lo`,
`hi` (MIDI 0..127, default 60 and 84, the C4..C6 range), `vel` (0..100, default 80), `len`
(hold in ms, 20..1000, default 120).

Why a sidecar in the folder rather than a new doc: the emoji folder already carries the
permission rules for who may manage emoji, sync, expiry and GC, and the picker already enumerates
it. Why a patch rather than a sample: a patch is a few hundred bytes, is already strictly
validated, is pitched (so the composer can place it on a staff), and renders through the
receiver-owned bus and limiter. There is no new audio-decoding surface.

### 2.5 Clicking an emoji in chat is local

A click plays a note-on locally at a uniformly random MIDI note in `[lo, hi]` and a note-off
after `len`. Nothing is sent. This is what keeps emoji sounds from becoming a spam channel.
Inside a call there is an opt-in "broadcast my emoji clicks" preference that sends the same
event as a jam note on the `inst` channel, spending the sender's musical bucket like any other
note. Receivers treat it as an ordinary note; there is no special frame.

A per-viewer "emoji sounds" toggle (default on, next to notification sounds) silences the
whole feature. Local clicks are throttled to 8 per second so a held mouse button is not a
siren.

### 2.6 A drawing counts as speaking

The point of drawing in a call is to include the people who are not talking. It fails if the
drawing sits in a side panel nobody looks at. So a drawing rides the attention paths that
speech already has, in this order of importance:

1. While a peer is drawing (strokes arriving within the last 1.5 s) they are marked active in
   the same `speaking` map the voice stage uses, with a distinct `drawing` flag so the ring
   renders in the draw colour rather than the voice colour and the active-speaker layout gives
   them the slot.
2. When a drawing is finished a short pen blip plays for everyone else (the same
   `playBlip` path used for arrival), or the author's knock if they have one.
3. Finished drawings stack newest-first in a strip inside the call view. A drawing fades out of
   the strip after 90 s into a scrollable call history, and the strip shows an unread count if
   the viewer had another surface focused.
4. Anyone in the call can pin a drawing into the text channel as a doodle message. The pinner
   rasterizes the strokes locally, uploads the pix blob, and the message is attributed
   "drawn by A, pinned by B" (recorder-attested, the take player's phrasing).

### 2.7 Composer output is a take; the editable score is a separate doc

The composer (Mario Paint) is a grid: rows are pitches C4..C6, columns are steps, a cell holds
an emoji (its patch) or a drum pad. Playback and sharing use the existing `jam-take:v1` player
unchanged: exporting a score writes a take whose `patches` array holds the emoji patch
descriptors and whose single lane is the exporter's fingerprint under a fresh `sn`.

The editable form is `jam-score:v1`, a CRDT map in the channel document keyed
`score/<id>/cell/<step>/<row>` with the emoji's patch id as the value, plus a header
(`bpm`, `bpb`, `steps` 1..256, `title`). Concurrent edits merge cell by cell, which is what
makes it co-op. Patch descriptors are embedded in the score by id when first placed, so a score
still plays after the emoji is deleted from the folder.

### 2.8 A flipnote is a channel-doc object; frames are blobs

`flipnote/<id>` in the channel document: header (`title`, `fps` 1..24, `w`, `h`, `created_by`),
an ordered CRDT list of frame ids, per frame `{cid, author, ts, claim?: {fp, until}}`, an optional
`score` (a `jam-score` id) and a list of `sfx` events `{frame, patch_id, n}`.

Concurrent edits to different frames merge. Concurrent edits to the same frame are last-writer-
wins on `cid`. To make that rare rather than merely resolved, a frame carries a soft claim with
a 2 min expiry; the editor shows who holds the claim and refuses to open a claimed frame unless
the user overrides. This is coordination, not a lock.

Caps: 999 frames (the original's limit and a good one), 64 KiB per frame, 8 MiB per flipnote.
Frames are ordinary sealed blobs with the usual expiry and fetch rules. Deleting a flipnote
drops the doc entry; blobs follow through GC.

Export: a `pixa:v1` blob (the frame sequence and the flattened take) shared as a message and
played by our own player, and an optional local GIF/APNG save built on a canvas for sharing
outside the app.

### 2.9 Games are host-simulated over inputs

A game round has one host, chosen the way the metronome anchor is chosen: the first valid `on:1`
edge, not supersedable while active. The host runs the simulation and broadcasts compact state;
players send inputs only. Host leaving ends the round. There is no lockstep and no rollback;
the games are small and the goal is fun, not fairness. Inputs and state frames are capped at
200 bytes and spend the `play` bucket.

Two launch games: paddle ball (two to four paddles around a court) and fly swatter (the Mario
Paint one, flies are the server's emoji, the boss is a member's avatar). The boss face requires
that member's profile flag `play_face` (default off). Swatting an emoji with a sound plays its
note through the local synth.

Scores live at `scores/<game>` in the channel document, top 10, self-reported by the host,
displayed with "reported by" attribution.

### 2.10 A knock is a bounded take carried in the ring signal

Profile gains `knock: JamKnock | null`: `{v:1, p:<jam-patch:v1>, e:[{ms, n, on}]}` with
`ms <= 3000`, at most 32 events, validated by the patch validator plus these caps. A new
`type:"ring"` call signal (authenticated push, the same path as `hello`) carries the caller's
knock, an optional per-ring override, and an optional message of up to 140 characters.

The callee is sovereign: the knock renders through its own synth and limiter, is dropped under
Deafen or a "knocks off" preference or configured quiet hours, is replaced by the default ring
blip if invalid, and one ring per caller per 30 s is honoured. The ring dialog on the caller's
side (knock, message) is the first "invite settings" surface; later invite options attach to it.

### 2.11 GB cam is a sender-side filter on the normal video track

Low-bandwidth camera mode captures at 160x144, converts to four grey tones with an ordered
dither, and sends that through the existing video path via a canvas capture stream with the
sender's encoder bitrate capped. Receivers map the four greys onto their own livery tones, so the
picture is in the viewer's palette, not the sender's. Greys on the wire, colour at the edge; the
same shape as notes on the wire, timbre at the receiver.

A data-channel frame format was considered and rejected for v1: at 8 fps it costs more than a
capped video track and adds a new surface for nothing.

## 3. Data model summary

| Thing | Where | Format | Cap |
|---|---|---|---|
| doodle, stamp, frame | sealed blob | `pix:v1` | 64 KiB |
| emoji sound | `emoji/<name>.patch.json` | `jam-patch:v1` + `lo/hi/vel/len` | 4 KiB |
| score | channel doc `score/<id>` | `jam-score:v1` | 256 steps x 25 rows |
| flipnote | channel doc `flipnote/<id>` + blobs | header + frame list | 999 frames, 8 MiB |
| flipnote export | sealed blob | `pixa:v1` | 8 MiB |
| game scores | channel doc `scores/<game>` | top 10 | 10 entries |
| knock | profile doc | `JamKnock` | 3 s, 32 events |
| play face consent | profile doc `play_face` | bool | |
| doodle reply (chat) | message attachment | pix cid + `reply_to` | 1 per message |
| doodle reply (announcement) | status doc `doodles/<post>/<id>` | `{author, cid, ts}` | 64 per post, 4 per author |

## 4. Wire summary

All on the per-peer negotiated WebRTC data channels, JSON text, admission per 2.3.

**`draw` (id 8).** `{t:"k",v:1,sn,q,s,c,w,p:[x0,y0,x1,y1,...]}` stroke segment: `s` stroke id
(uint32 per sender), `c` palette index, `w` width 1..8, up to 24 coordinate pairs in the 192x144
grid. `{t:"e",sn,q,s}` stroke end. `{t:"f",sn,q,d}` drawing finished (`d` drawing id).
`{t:"x",sn,q,d}` author cleared drawing `d`. `{t:"r",sn,q}` request replay: the peer resends
its most recent finished drawing as strokes, at most one replay per requester per 10 s, at most
2000 segments. `q` is the per-sender sequence shared across all draw frames for one `sn`.

**`play` (id 9).** `{t:"g",v:1,sn,q,on:1,game,rev,seed}` host start edge, `{t:"g",on:0}` stop.
`{t:"i",sn,q,k:<input bits>}` player input. `{t:"s",sn,q,st:<compact state>}` host state at up
to 20 per second. Games define `st` under a per-game version string; unknown games ignore.

**Call signal.** `{type:"ring", callId, knock?, msg?}` on the existing authenticated push.

**`inst` (id 7).** Unchanged. Broadcast emoji clicks are ordinary `t:"n"` notes with `p` set to
the emoji's patch id after the usual announce.

## 5. UI

- **Call view.** A Draw tab beside the existing stage. The strip of finished drawings sits under
  the peer tiles. The draw ring colour is a new semantic token. Pin is a button on each drawing.
- **Chat.** Emoji with sound show a small note glyph in the picker and on hover. A "reply with
  a doodle" action next to reply. Doodles render inline at 128x96 with the author's frame.
- **Announcements.** A doodle wall under each post: a grid of replies, newest last, plus an
  "add yours" tile.
- **Studio.** A new surface per channel (like Wiki) listing flipnotes and scores. Opening one
  shows the timeline, the canvas, the onion skin controls, and the score. Live collaborators
  appear on the timeline with their claimed frame highlighted.
- **Play.** A Play tab in the call view. Room members pick a game; the host is whoever presses
  start. Scores show beside the court.
- **Settings.** Emoji sounds toggle; broadcast clicks in calls toggle; knocks off and quiet
  hours; play face consent (also on the Profile page); GB cam toggle in the camera menu.
- **Profile.** Knock editor: pick a patch, tap up to 32 notes on a 3 s strip, preview.

## 6. Threat notes

- **Untrusted bytes.** `pix:v1` is decoded by bounds-checked code we own; caps are enforced
  before allocation; a mismatched pixel count rejects. Patches on emoji and knocks reuse the
  jam validator with no relaxation. Scores and flipnote headers are strict-decoded like patches.
- **Amplification and floods.** Draw and play channels get their own buckets, byte caps before
  parse, and the exhaustion auto-mute. Replay requests are rate-limited per requester and byte-
  capped per reply. Emoji clicks never leave the client unless the broadcast opt-in is on, and
  then they spend the musical bucket.
- **Harassment.** Per-peer "hide drawings" in the call, in addition to mute. Pinned doodles are
  ordinary messages and fall under existing moderation. Announcement doodle walls are capped per
  author and removable by the poster and by moderators.
- **Consent.** A member's face appears in a game only with `play_face` on. Drawing presence is
  visible to the whole call, like recording state. A knock plays only within the callee's caps
  and preferences, and the callee can mute knocks per caller.
- **Privacy.** Strokes reveal nothing beyond what voice presence already does. `pix:v1` carries
  no metadata. Recorder-attested attribution on pinned drawings is labelled as such, matching the
  jam take rule. Palette roles leak the viewer's preset choice to nobody: resolution is local.
- **Clocks.** Games and flipnote playback use the jam clock for scheduling only. Never for
  authorization, expiry or persistence.

## 7. Phases

Each phase is one PR-sized unit with its own tests, and each is useful on its own.

| Phase | Delivers | Depends on |
|---|---|---|
| C0 | `pix:v1` codec with strict decode + roundtrip and rejection tests; `rt-admission.ts` extracted from the jam engine with jam tests unchanged | jam layer as landed |
| C1 | pixel canvas component; stamp editor mode saving custom emoji as pix | C0 |
| C2 | emoji sound sidecar; validation; picker glyph; local click playback; settings toggle; in-call broadcast opt-in | C0 |
| C3 | doodle reply mode on messages; inline render; announcement doodle wall | C1 |
| C4 | `draw` channel; Pictochat tab in calls; drawing-as-speaking; pen blip; strip and history; pin to chat; replay on join | C0, C1 |
| C5 | composer: `jam-score` doc, grid editor, take export, playback via the take player | C2 |
| C6 | flipnote studio: doc, frame claims, timeline, onion skin, soundtrack and sfx, `pixa` export, local GIF save | C1, C4, C5 |
| C7 | knock: profile field and editor, `ring` signal, ring dialog, callee preferences | C2 |
| C8 | play room: `play` channel, host election, paddle ball, fly swatter, scores, play face consent | C0, C2 |
| C9 | GB cam: sender filter, bitrate cap, receiver palette mapping | none (independent) |

C2 and C3 can run in parallel after C1. C7 and C9 are independent of the studio work and can be
picked up whenever a session wants a small self-contained piece. C6 is the largest and should
not start until C4 and C5 have had a review pass, because it inherits both of their formats.

Prerequisites already noted in the jam section still apply to anything that renders sound:
Deafen must gate rendering, and roster revocation must tear down the removed member's call
connections.

## 8. Open questions for review

1. Palette roles in `pix:v1`: should a doodle recolour with the viewer's livery (the proposal),
   or keep the colours it was drawn with and only recolour on explicit request?
2. Drawing-as-speaking: does a drawer take the active-speaker slot from someone who is actually
   talking, or only fill it when nobody is? Proposal: only when nobody is talking, otherwise
   the ring and the strip carry it.
3. Emoji click broadcast in calls: opt-in per session (proposal) or a per-server admin switch?
4. Composer rows: a chromatic C4..C6 (25 rows, proposal) or a selectable scale that hides
   wrong notes, which is friendlier but needs a scale field in the score header?
5. Flipnote frame claims: 2 min expiry, or held until the editor closes with a heartbeat?
6. Should the announcement doodle wall be a status-doc structure (proposal) or should
   announcements gain general replies first, with doodles as one reply kind?
7. Knock in the DM ring as well as the room ring, or rooms only in v1?
8. Play room as a tab inside any voice room (proposal) versus a dedicated room type. A dedicated
   type would let a server keep a permanent arcade with persistent scores independent of calls.
