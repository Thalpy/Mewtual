# Gate 6: sound and export

Started 2026-10-13 on `gate6-sound-export`, from the Gate 5 merge
`7cbe8cbe131e1be8e672d04341c31a1d2b52a98d` in `Create-suite-2`.
The owner has accepted the Gate 5 merge. Older status notes which call it a candidate
describe their earlier checkpoints, not a reason to restart Gate 5.

## Delivery order

Gate 6 extends the existing Studio paths. It does not create another document store,
blob transport or audio engine. The first reviewable slice supplies pure export formats;
later slices connect the remaining persistent document families and the real UI.

1. **Export foundations (this slice).** Deterministic score-to-`jam-take:v1` conversion,
   exact no-score derivations, the score-first/SFX-second patch union, and bounded
   `pixa:v1` encoding/decoding. These are leaf TypeScript modules with no native calls,
   publication, playback, timers or UI activation.
2. **Typed sound state.** Score operations and causal validation, score/SFX/patch/export
   projections, exact seed/recovery preflight, and conservative reference enumeration.
   Extend the existing `StudioTarget` and storage/intent ownership rather than bypassing P1.
3. **Application/native integration.** Durable score creation/edit/reopen, validated `.pixa`
   publication followed by its separately durable export record, and bounded readback.
   Reuse blob staging/promotion, the existing scoped native custody and retry model.
4. **Playback and UI connection.** Reuse the jukebox's take validator, bounded load/prepare
   coordinator, engine and playback lifecycle. Add frame-SFX timing and connect the existing
   Sound/Music/export surfaces. Deafen, leave and membership revocation retain their current
   authority. GIF is a local visual export; its exact save/encoding contract needs to be
   recorded before that slice (the product design currently names it without a byte contract).
5. **Combined acceptance.** Full suites, real export readback/restart/recovery scenarios,
   adversarial review and a complete UI hook inventory. This first slice cannot close Gate 6.

The isolated `gate6-typed-score` continuation starts step 2 with the missing static ScoreOp
grammar, mirrored frontend types and shared vectors (`GATE6-SCORE-CONTRACT.md`). It does not
yet add a Score target, causal projection, seed/recovery codec or durable writer. PR #44 retains
the export checkpoint and the external nonce-validation correction separately.

## Existing code to reuse

| Concern | Existing owner |
|---|---|
| PIX encoding, validation and palette semantics | `pix.ts` / app `creative::validate_pix` |
| Synth recipe normalization and identity | `jam-patch.ts::validateJamPatch`, `jamPatchId` |
| Take validation and import byte cap | `jam-recorder.ts::validateJamTake`, `parseJamTakeJson` |
| Take preparation, cancellation, cache, seek and Deafen | `jam-playback.ts`, `jam-engine.ts`, App jukebox consumer |
| Canonical Studio JSON | `studio-contract.ts::canonicalJson` |
| Persistent media | Existing sealed blob publication/fetch and fileshare paths |
| Studio durability, retries, settlement and recovery | `studio-session.ts`, `StudioTarget`, `StudioEpoch`, `ServerStore` |
| GIF decoding/encoding primitives | Existing desktop `image` dependency and `media_decode.rs` fixtures |

The shared deck already accepts validated `jam-take:v1` data. A score export deliberately
produces that same type; it needs no second synthesizer or streaming protocol. Creating a
take does not enqueue it in the jukebox or claim that its exporter authored every source edit.

## First-slice contract and review boundary

`studio-export.ts` accepts a resolved, bounded score export input, not an Automerge change
or an authenticated native view. Its patch keys must equal the existing recipe hashes.
The caller must resolve conflict/overflow/readiness and current authority before a later
publication step. Exact schemas, safe integers and descriptor validation reject malformed
inputs; all mutable input is validated/copied before an asynchronous hash can yield. The one
exception to JSON-safe numeric inputs is epoch: the leaf API requires `bigint` in the u64 range,
rejecting numeric epochs rather than rounding them.

The initial exporter follows creative design 2.0 and 2.8: framed SHA-256, raw 16-byte object ids,
u64 big-endian epoch, raw full exporter identity, ascending patch ids, integer note timing,
off-before-on ties, and one contiguous `q` sequence. Note-on's required legacy `w` follows
`PATCH_OSC_WAVES[descriptor.o[0].w]`; `p` still selects the complete descriptor in the existing
engine. The no-score take follows 2.16. The
existing take validator runs on every result. Exported `parts` names the exporting identity;
it is content attribution, never live call or membership authority.

Patch union keeps ascending score ids first, then ascending distinct SFX ids. A reused id
must name the same verified recipe. More than 64 distinct recipes refuses export. Score
event patch indices must retain their meaning when SFX patches are appended.

`pixa.ts` uses the exact binary layout in creative design 2.16. Decode checks the whole
9-MiB ceiling before taking one owned input snapshot, including for shared memory or Node Buffer.
Before cloning or decoding individual frames it verifies FPS/count, each frame's length and the summed
8-MiB frame ceiling, the 512-KiB take ceiling and 32-KiB/4096-entry SFX ceiling. A first pass
checks complete framing, indices and trailing bytes before any frame is decoded. PIX frames
must validate and be 192 by 144. The take is strict UTF-8, canonical Studio JSON and is passed
through the existing take validator. Duplicate keys/noncanonical JSON cannot acquire a second
byte representation. SFX frame/patch indices and MIDI notes must be in range.

The decoder returns owned encoded frames, not 999 expanded rasters. Each frame is validated
one at a time through the PIX decoder, bounding transient raster work to one frame. SFX ids
are not stored in `.pixa`; the builder orders them before encoding, and the decoder preserves
wire order. A decoded package carries no receipt, signature, CID or durable-save guarantee.

### Required first-slice tests

- Exact framed-hash and no-score `.pixa` vectors, including epoch beyond JS-safe integer range.
- Deterministic score event/patch order, all tempo edges, drum/note cells and validator reuse.
- Authored grid lengths, adjacent holds, same-row overlap refusal, invalid/end-overflow lengths,
  and duration snapshots. Authored SFX durations, exact nonempty bytes and old-draft refusal.
- Maximal 4096-cell/64-patch score remains a valid take within its existing byte cap.
- 64 score patches plus one disjoint SFX recipe refuses; 63 plus one round-trips.
- Truncation at every boundary, malicious lengths, count/byte ceilings, malformed UTF-8,
  noncanonical/duplicate JSON, bad PIX/dimensions, invalid SFX indices/notes and trailing bytes.
- Caller mutation during asynchronous hashing cannot change the validated export.
- Buffer/subclass/shared-memory inputs cannot substitute aliased or post-validation frame bytes.
- Encoder/decoder output ownership and imports from the existing take/PIX paths.

No native command, Studio writer, playback callback, registration or production control is
enabled by this slice. It changes no existing wire or persistence format. Subsequent slices
must extend the fail-closed sound/export admission checks together with storage/reference
coverage before making those controls callable.

## Early design review

The independent read-only review found no BLOCKER/HIGH. Its MEDIUM contract omission was the
otherwise unspecified note-on fallback `w`; the first-oscillator rule above closes that design
gap and requires all-wave/multi-oscillator tests. Epoch is explicitly a bounded `bigint`.
The reviewer independently checked a maximal 4096-note-cell/64-maximal-patch take at 473,840
bytes, below the existing 524,288-byte limit; a maximal-input regression now pins that bound.
The local GIF contract remains later-slice work, not a claim made by the export codec.

## Authored duration requirement (owner clarification, 2026-10-14)

The owner clarified that note length belongs to the music/sound creator. Do not introduce a
global 120-ms or one-animation-frame hold rule. Existing jam takes already retain note-on/off
times and must keep those durations. An emoji sound already has a validated `len` field.

The older Studio design omitted duration from both note cells and per-frame SFX. This first
codec slice now preserves optional score `len` (sixteenth steps, absent means the original
one-step draft), rejects same-row overlapping holds rather than shortening them, and requires
frame-SFX `durationMs` in 1..600000 ms. Its eight-byte SFX entry appends u32 LE duration, making
the component cap 32 KiB. The unreleased design is amended before first exposure; strict
framing rejects the old four-byte SFX draft, and the zero-SFX golden stays byte-identical.
The independent design review found no blocker and verified the maximal 8,949,673-byte package
still fits 9 MiB. Nonempty golden bytes and authored-length regressions pin the amendment.

Persistent Gate 6 sound operations/projections still need these fields. Later stop/loop/seek,
overlap display and release-tail tests must use the existing engine. Its 30-second watchdog
also limits archival notes today; long exported holds preserve content but are not a claim of
audible playback for that whole duration. Integrate a bounded archival-note path without
weakening the live watchdog. This remains Gate 6 implementation work, not an unanswered owner
choice. Existing jam note-on/off data, take schema and audio engine are unchanged in this slice.

## Checkpoint verification and implementation review (2026-10-14)

- Focused `pixa.test.ts` and `studio-export.test.ts`: 26/26 pass.
- Complete root workspace: `cargo test --all --all-features` passes, including integrations and
  doc tests. Existing opt-in ignored fixtures remain ignored.
- Separate native workspace: `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`
  passes (337 library and 5 command-ACL tests, plus binary/doc targets).
- Complete frontend: `npm --prefix apps/desktop test` passes 1418/1418. `check` has zero
  errors/warnings; production `build` passes with Vite's large-chunk advisory.
- `git diff --check` passes. Startup and chat/friend flow gates do not apply to these unconnected
  leaf helpers. No Rust code, dependency or build configuration changes; Rust lint/ambient/deny
  checks were not rerun for this TypeScript-only slice.

The independent read-only implementation review identified and closed Buffer aliasing,
validation/serialization scalar rereads and growable-buffer allocation beyond an admitted size.
Owned fixed-length snapshots, one-time scalar capture and dedicated regressions now pin those
boundaries. The authored-duration amendment received both design and actual-diff implementation
re-review; six focused duration/snapshot checks also passed independently. No remaining
BLOCKER/HIGH/MEDIUM/LOW finding was reported for this checkpoint. The primary ran the full suites;
the reviewer did not duplicate them. This is not acceptance of the unimplemented Gate 6 slices.

## External review correction: nonce types (2026-10-14)

The external review of `575f01de` found that the inherited shared `validNonce` guard used
`RegExp.test` without a runtime string check. A JSON number or single-element array could pass
through take parsing and canonical `.pixa` admission as a non-string lane nonce. The exporter
itself generated strings; this was an imported-input schema defect, not established live authority
escalation. The shared guard now accepts `unknown`, requires a primitive string and then applies
the same lowercase-hex rule. Legal takes retain their exact schema and bytes.

Three new regressions failed before the correction and pass afterward. They cover shared JSON
and direct take validation, recorder rejection before coercion/lane allocation, and independent
`.pixa` framing through decoder/encoder rejection. All 44 recorder/export focused tests pass.
Independent read-only correction review closes the MEDIUM finding with no remaining finding;
the prior export ownership/bounds review remains intact. On correction `863fe0fe`, complete
root `cargo test --all --all-features` and separate native suites pass (337 native library and
5 command-ACL tests, plus binary/doc targets). Complete frontend passes 1421/1421; check has zero
errors/warnings, production build passes with its large-chunk advisory, and `git diff --check`
passes. No Rust source/dependency or setup/UI/flow change was made by this correction, so Rust
lint/ambient/deny and startup/flow gates were not repeated for this TypeScript-only fix.
Exact-head hosted checks remain queued, not confirmed green. These results supersede the earlier
checkpoint counts only for the correction; they do not close Gate 6.
