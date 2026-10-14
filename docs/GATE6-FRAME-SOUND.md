# Gate 6 Flipnote sound state

Isolated core checkpoint based on `c3515d2b6283f1eee782f35f840c85a21f475931`.
This adds bounded typed state, not native activation, audio playback or blob publication.

## Records and effective views

`FlipnoteFrameProjection.sound.records` retains a sorted map from physical property to a
`FrameRegister<FlipnoteOp>`: original source (full author, nonce, derived operation id, asserted
timestamp), actual Automerge selected value and every concurrent alternative. All sources,
canonical bodies and keys are revalidated before serialization. Debug is content-redacted.

| Property | Contract |
| --- | --- |
| `s/<sfx id>` | Mutable SetSfx register; sequential editing keeps the id. Every Set requires a live frame and effective local recipe at the sender frontier. |
| `q/<sfx id>/<op id>` | Immutable deletion; any retained deletion suppresses every Set alternative and prevents same-id reuse. |
| `p/<recipe hash>` | Mutable SetPatch/RemovePatch register. ANY live Remove wins even when the raw AM winner is Set. Later Set consumes observed same-property predecessors only. |
| `x/<export id>/<op id>` | Immutable export candidate, smallest derived op id selects a same-id collision. Replacement uses a new export id. |
| `z/<export id>/<op id>` | Immutable export deletion, suppressing all same-id candidates. |
| `h/score` | Mutable optional score id; explicit None preserves unlink provenance. This grants no authority about the other object's existence, kind, channel or readiness. |

Single-letter keys keep the existing 99-byte reader rail. The same record framing as art
(version, author, asserted timestamp, absent origins, complete DomainOp) binds values to their
signed causal mutation. Existing P1 authentication, exact predecessor checking, operation marker,
checkpoint/recovery preflight and durable ownership remain the only admission path.

`sound.sfx()` and `sound.exports()` return all effective nondeleted ids, including overflow;
`sound.patches()` returns effective nonremoved recipes, including overflow. These return borrowed
source/value records, not authored summaries. The full records map must accompany diagnostics
and recovery: effective getters intentionally omit removals, losing exports and raw alternatives.
`sound.score()` returns the complete optional header register.

Deterministic active prefixes are 4096 SFX, 64 recipes and 8 exports (design 2.10), each ascending
by id/hash. `sound_readiness()` reports overflow, missing/deleted/over-cap frames, missing or
overflow recipes, and effective conflicts. It does not resolve a linked score; the application
must verify that score's trusted type/channel, current content and combined patch union before
export/playback. Concurrent valid edits retain references and become unready rather than losing
evidence. RemoveSfx remains possible when its frame/recipe has gone. Local growth refuses beyond
the cap; trimming remains possible. Authored SetSfx `duration_ms` is mandatory 1..600000,
never guessed from FPS. The unreleased missing-duration draft rejects.

## Checkpoint and reference compatibility

Existing no-sound art payload/checkpoint bytes are unchanged (the art golden tests pass).
A nonempty retained sound map adds trailer version byte1, count, then sorted property/register
values: length-framed key, conflict count, exact selected record, sorted alternative records.
An empty trailer rejects; absence means NO sound evidence, not merely no audible sound.

Full recovery retains every value/removal/overflow reference. Seeds omit whole removed slots,
not tombstones alone, and share art's four-value/1024-conflict-field budget. Export candidate
conflicts count as one field per id; removed groups and overflow disappear only into the existing
recovery-before-replacement barrier. Deletion permanence means retained causal history, not an
unbounded lifetime ledger after compaction. Historical replay must remain conservative when a
compacted deletion cannot be proven absent. Post-seed explicit registers replace fallback values;
inherited source ids cannot be reused as fresh edits.

Reference enumeration includes every retained export candidate, even losing/deleted/overflow
values, plus complete signed operation history. Recipe hashes and linked score ids are NOT blob
CIDs. Expiry is metadata and cannot release retained evidence. Actual `.pixa` length/CID/format
validation and durable blob-before-reference ordering are application/native obligations.

## Verification and pending integration

Eleven focused Rust sound tests pass, covering authored duration/schema, causal-only recipe/frame
authority, concurrent deletion, raw-Set/remove-wins, unseen removal, exact predecessor sets,
post-seed fallback/source reuse, exact byte caps and4097-SFX/9-export bounds, export collisions,
signed owned epoch restart/recovery, malformed trailer/source reuse and complete retained CID
holds. Final scoped Studio suite passes210/210, including art golden vectors. Formatting,
diff checks and scoped strict Clippy pass. Frontend fixture tests10/10 and Svelte check0 errors/
0 warnings pass. The complete root/native/frontend suites are primary-owned after integration.

One old unsupported-family assertion intentionally changed: explicit score unlink and absent
reusable recipe removal are now valid typed intents. Unknown/malformed properties, missing SFX/
export targets and forged same-property predecessors still reject. Art golden vectors remain.

Read-only design and actual-diff review reported no remaining core blocker/high; one MEDIUM
integration condition remains below. Main owns full
root/native/frontend suites after integration, plus native DTO/consumer coverage. In particular,
unchanged native `projection_content` currently omits sound on remotely accepted Flipnote
changes: this is an OPEN integration defect, not excused by local sound Save refusing. Resolve
it before combined acceptance. No unfinished command is registered by this checkpoint.
