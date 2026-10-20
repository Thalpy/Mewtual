# Gate 6 application/native activation design

This is the next integration boundary after the reviewed core score checkpoint `78c79faa`.
No native control is enabled merely because its core operation codec exists. Implementation and
focused regressions precede activation, followed by independent actual-diff review and full suites.

## Explicit family, unchanged owners

- Existing Studio commands gain an optional `objectKind` (`flipnote` or `score`). Omission keeps
  the existing flipnote meaning; unknown kinds and a kind without an object refuse. Create defaults
  to flipnote for compatibility and explicitly selects the matching initial header and Index kind.
  Kind participates in exact retry/custody/target checks, not a guess from operation bodies.
- The accounted sealed Studio source wrapper preserves all legacy bytes and adds an explicit score
  extension byte2 after its optional intent-link byte1. Score's core restart unit already requires
  its own trailing family2 tag, even when pristine. Index rejects the score extension; wrong-kind
  source access refuses before an intent write. Source keys remain the existing group/type/object
  namespace, so an art/score collision cannot overwrite or create a second interpretation.
- Studio-only head/page/seed queries gain explicit score scope selector `0x8010` in their existing
  u16 scope field. This is NOT a new `DocType`: P1 logical/physical type and key derivation remain
  StudioObject16. Existing Index15/art16 queries stay byte-identical; old peers reject the new
  selector, and callers never downgrade score requests into art. The entire query, including this
  selector, remains signed/request-bound under the existing Studio domains and membership gates.
- Receipts, clocks, RNG, budgets, receive leases, intent-before-source durability and
  recovery-before-replacement stay with their existing owners. No second persistence system,
  scheduler or transport is introduced.

## Content and references

Native Score content retains optional authored headers, every raw selected/conflicting slot and
an explicit effective removal flag, plus derived readiness. Defaults have no invented source.
Frame sound content likewise exposes all retained sound records and derived readiness: accepted
remote sound must never disappear from ordinary, awaiting-tenure or historical diagnostics.
The frontend validates the complete closed shape before rendering, retaining conflict/removal and
overflow evidence separately from the playable/exportable view.

Typed operation inspection checks the complete envelope against the held family before returning
no blob references. Vault intent inspection, which has no family authority, may use an explicit
static union selector for score-only operation names/header fields; it fully validates the chosen
grammar, never parser-fallbacks and grants no edit/fetch authority. Common title/recipe operations
have no CIDs in either family. All retained art export CIDs, not merely eight display entries,
remain protected through source, seed, intents, drafts and complete recovery.

Local `.pixa` publication requires native actual-byte validation and durable blob staging/promotion.
The separate export record accepts only the exact held CID/length and validated package; a failed
record save leaves a retryable/orphan blob, never a falsely successful record. Existing bounded
blob transfer supplies readback; format validation remains separate from availability. Source
scope/generation checks fence async UI export/playback work. GIF is local visual output only;
its bounded encoding/save contract is independently reviewed before its command is registered.

The held-package rule covers new Unconfirmed local authoring too, not only ordinary Open Save.
Shared Flow S admission derives an owned Frame-PIX or Export-PIXA reference from the exact
operation, validates/promotes those bytes at S1b, holds the CID throughout detached planning,
and rechecks verifying content-addressed presence and length at S3 before the intent barrier.
Successful/uncertain writes transfer protection to conservative durable-reference holds before
the transient owner is released; cancelled or stale plans release their hold without accepting.
Export-specific reasons are `ExportUnpublished`, `ExportInvalid` and `ExportMissing`, each with
the NewTicket action. These are local typed IPC diagnostics, not a persisted or wire format.
An exact retry of already accepted work remains an acknowledgement before media admission and
does not require the media or a fresh preview again. Unconfirmed provenance remains explicitly
non-transferable; this correction grants no native Closing Save/P5 or handoff authority.

## Recovery and activation

Historical score replay remains manual: absence of a reusable coordinate cannot prove that a
different intervening clear never occurred. Explicit per-item restore/copy names historical source
ids, previews against the complete current fingerprint, and authors fresh ordinary operations
under current membership/capacity/lifecycle checks. It never replays old signed authority. Missing
recipes, conflicts, out-of-current-range cells and cap overflow stay visible and cleanable.

Index dependencies are different from automatic replay of an object's own edits. The exact
validated `PutObject.kind` selects an expected art/Score target in Index replay, P2 classification,
H1/H5 handoff and historical Object restore/copy probes. The sealed wrapper, typed restart unit,
group, channel and nonempty/installed work then independently establish the source. No Index
winner/hint retags storage, and there is no parser fallback. Same-id claims of both families
retain two distinct obligations: one physical wrapper cannot satisfy both. An unavailable or
wrong-family historical pointer becomes bounded manual work; transient local I/O stays retryable.
H5 and copy Apply recheck source existence, rather than trusting a previous preview. Existing
documented header-only copy/H5 checks remain header-only; H1 still full-loads the typed source.
An Index refresh preserves an already opened Score graph just as it preserves opened art.

Cross-document copy requires the same channel and the same object family. Score-to-Score can
use the manual planner; art-to-Score, Score-to-art and Index-to-object refuse before fresh intent
creation. Score operation replay remains manual even though safe Index pointers can be replayed.

Manual native choices are `scoreCell {step,row,value}`, `scorePatch {patch,value}` and
`scoreHeader {field,value}` where `value` is a historical source operation id, never a supplied
musical value. Header fields are title/bpm/bpb/steps. Restore is additive: no existing register
(including a retained clear or conflict) is overwritten; identical unconflicted content is
Unchanged. Retained destination-history removals block resurrection. Copy explicitly resolves
a selected alternative/removal through the ordinary fresh-operation Save gate. Recipes must be
recovered before notes, and note extent uses current authored steps. Copy may introduce the
same visible overlap/conflict readiness state as an ordinary edit. Superseded 2026-10-20
(docs/design-score-layers.md): Score export/playback no longer refuses on readiness; what cannot
play is skipped and counted, never silently presented as the whole score. Source/destination family, full group and channel remain
mandatory; manual recovery does not grant old membership or automatic replay authority.

Required tests include legacy wrapper/query byte compatibility; wrong/unknown family and object
collision rejection; score create/edit/reopen, exact retry and scoped two-member exchange; source
and recovery enumeration; remote sound ordinary/awaiting/recovery DTO coverage; manual recovery;
native malformed/maximal package validation and publication-before-record ordering; frontend
closed-shape validation, retry ownership and lifecycle cancellation; and final combined suites.
