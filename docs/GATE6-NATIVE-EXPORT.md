# Gate 6 native animation export

This isolated continuation follows frame-sound core `f4711f4cc63fd7af3e18c9769a04d9a6d4f75d70`.
It reuses the existing creative blob store/transport, PIX validator and native download/session
ownership. It introduces no synth, file transport, document store or replicated GIF format.

## Actual-byte validation and publication

`catcoms_app::creative::validate_pixa(bytes)` validates the whole9MiB-bounded input before
returning `ValidatedPixa`, which exposes borrowed encoded frame spans and FPS. Its lifetime
remains tied to that exact immutable input; no999-raster allocation occurs. Framing checks
precede JSON/PIX decoding:1..999frames,1..24FPS, each frame1..65536bytes, sum<=8MiB,
take1..524288bytes,<=4096 eight-byte SFX entries, exact lengths/no trailing bytes.

The take grammar matches `jam-recorder.ts::validateJamTake` plus `.pixa`'s normalized canonical
JSON equality. Exact fields, Unicode-scalar text, bounded distinct participants/lanes/recipes,
strict per-lane sequence and time ordering, valid events/MIDI/pads/waveforms/recipe indices,
and deterministic inter-lane ordering are mandatory. Nonces are primitive strings of exactly
16lowercase hex characters, INCLUDING all-zero (matching the existing validator). Number/array
coercion, duplicate JSON keys, alternate numeric spellings, whitespace, noncanonical order and
unknown fields reject. Recipe validation and declaration-order hash reuse `StudioPatch`; hashes
are never treated as blob CIDs. Every PIX must validate and be192x144. SFX duration1..600000ms
is authored, not derived from FPS. The old four-byte SFX draft rejects.

Native `publish_pixa({server,bytesB64})->{cid,bytes}` bounds base64 before decoding and follows
the existing actor's persistent bounded blob stage/promotion/flush path. Cancellation owns the
inline concurrency lease until actor completion; the response rechecks exact unlocked UI
generation and server incarnation. Memory-only/failing publication cannot report a saved CID.
Content publication creates NO Studio export record, index entry, expiry or receipt. The main
integration must separately validate exact held bytes/CID and save SetExport through normal
Studio intent/source durability, after successful promotion.

## Local visual GIF contract

Native `export_studio_gif({server,bytesB64})` validates the ENTIRE PIXA package, even the audio
which GIF does not use. It encodes a local visual-only, opaque192x144 GIF with infinite repeat;
this does not preserve audio, peer timing authority or synchronization. Palette roles resolve to
their explicitly authored fallback RGB, not the exporting machine's ambient theme.

The existing pure-Rust `image::GifEncoder` receives one expanded RGBA frame at a time.
For frame index i, centisecond delay is
`floor((i+1)*100/fps)-floor(i*100/fps)`; carrying the remainder prevents per-frame rounding
drift at12/24FPS. GIF's centisecond timing is an explicit approximation, not exact sub-10ms
presentation. The bounded writer refuses before extending beyond32MiB and bounds requested
capacity growth. It remembers failed writes including the encoder's otherwise unreportable
Drop-time trailer, so a truncated-at-cap GIF cannot report success.

Detached encoding retains its own cancellation/inline keepalive even when the caller disappears.
After encoding, the same unlocked generation and exact server incarnation are held through
filesystem publication/reveal. A stale, cancelled, removed or replaced scope at that final
admission boundary writes/reveals nothing. The commit guard linearizes an admitted write/reveal
before a concurrently completing lock; it does not promise retroactive cancellation once that
guarded commit has started.
The renderer supplies no filename/path: `Mewtual-flipnote.gif` uses the existing create-new safe
Downloads suffix/reservation helper. A synced staging file is atomically renamed onto its own
reserved final filename; failure cleans up staging/reservation and never overwrites an older
export. Reveal failure reports a saved-file warning, not a false write failure. No directory-flush
or hardware/power-loss durability guarantee is added to this local export.

## Verification boundary

Focused tests cover shared byte/hash vectors and16shared Rust/TS malformed verdicts; all
framing prefixes, nonce coercion, canonicalization, recipe/event/lane bounds,999frames/4096SFX,
authored duration, persistent actor publication/readback/cancellation, all24FPS timing and999GIF
frames, declared RGB, output cap including final trailer, stale session/server and cancellation,
safe suffix/nooverwrite, and reveal-failure truthfulness. Native command handlers, policy,
main-window capabilities and frontend review ledger move together; permission-chain tests pass.

Six Rust validator tests, one actor publication test, four native GIF tests,18frontend PIXA
tests and9command-security tests pass. Root app and native all-target strict Clippy, formatting,
Svelte check0errors/0warnings, local Git-Bash ambient gate and diff checks pass. The dependency
change is only a direct `serde_json` dependency already in both resolved graphs; no version
changes occurred.

Read-only design and actual-diff implementation review found no remaining BLOCKER/HIGH/MEDIUM/
LOW in this bounded checkpoint; the reviewer independently checked the16shared TypeScript
verdicts and diff whitespace. Main owns the
complete combined root/native/frontend suites and UI/playback/SetExport integration. This
checkpoint alone cannot close Gate6. The frame core's native sound DTO finding has since been
closed by the integrated shared fallible converter and its reviewed ordinary/provisional tests.
