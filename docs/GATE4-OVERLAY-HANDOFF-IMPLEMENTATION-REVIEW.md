# Gate 4: Closing overlay handoff implementation

Status: implemented and pushed at `bf37cc48c45d09be6b63e97322171e506768fbf4`, 2026-09-14.
The dedicated normal and mutation checks pass; implementation acceptance awaits user adversarial
review. The user accepts the corrected design at `dd2fbc0` and closes HANDOFF-001; that design PASS
does not cover this implementation or full Gate 4. Base: `dd2fbc01ebb8a692964eec0df19dd201b225c936`.

## Implemented boundary

`Server::handoff_studio_overlay` is an explicit Rust adapter under exclusive Server/store custody.
It obtains actual membership, device and independently observed tenure from the live sync instance.
No actor/native command schedules it, and it returns no packet batch. A new transfer requires the
real installed adjacent successor, its exact opening receipt and seed, zero signed operations and
complete typed base equality. Current owner/tenure verification is independent of local metadata.
Each accepted envelope is signed into a private candidate in original sequence and timestamp using
the existing policy, typed writer/validator and admission limits. Index PutObject checks the actual
referenced Flipnote source. Any refusal retains the full branch.

The store preflights all three replacement peaks with existing ordinary budgets, then persists
Prepared, replaces the complete signed source once, and authenticates/flushes that actual source
before persisting Completed. A private capability binds the candidate, complete intent contents
and actual source bytes captured before Prepared. Source/intent changes cannot reuse that capability.
Full author/domain envelopes and complete signed-operation hashes must match; visible values,
markers and receipt hints cannot complete a transfer. Original pending ledger entries remain.

After restart, all-exact evidence completes without reapplying and without requiring new-edit
authority. Fault remains Fault. An absent manifest in the same physical destination returns
durably to Active; a fresh attempt must pass all normal eligibility checks. Partial/conflicting
evidence, missing source and different physical epochs retain Prepared. This implementation chooses
the design's permitted return-to-Active route instead of reproducing an interrupted candidate.

## Metadata and local source dependency

Inner intent extension version 2 retains a mandatory complete target, optional Active/Prepared
branch, optional completed acknowledgement with its own matching target, and monotonic retry floor.
The ordinary ledger codec and outer extension tag stay unchanged. Checked version-1 bases migrate
on a later write; reading alone preserves exact v1 bytes. Combined metadata is bounded before seed
allocation/replay. All branch/acknowledgement targets must equal the enclosing target and outer
logical scope. Completed entries can outlive legitimate ledger retirement.

`StudioOverlaySave` distinguishes Local draft from HandedOff. Completed retry compares the complete
request target before source lookup, acknowledgement or sync reservation/callback. Both target
encodings count against the existing metadata limit. Rollover replaces one compact acknowledgement
and advances the floor; forgetting an old acknowledgement cannot admit that request after rewind.
Floor-only metadata retains the target too.

The implementation adds a **local source-to-intent dependency** to enforce the design's missing-file
requirement after restart: an optional trailing `u8(1)` in the authenticated Studio source wrapper.
The common writer sets it on replacements with overlay metadata and retains it through later
rotation/adoption. A linked source requires the matching intent extension even if its ledger is
empty. Explicit/cold reads, detached capture/install and warm source authentication check it.
The byte consumes ordinary content budget; old unlinked wrappers remain byte-compatible and old
readers reject linked wrappers. This dependency is not a receipt, signed operation or network field.
It is an implementation addition to the design's field list and warrants explicit review.

## Enforcement paths

| Path | Fence |
|---|---|
| `rotate_studio_owner_with_io`, including frozen-owner takeover | Resolve before source flush, owner journal, recovery or retirement side effects |
| `adopt_studio_checkpoint_with_io` | Resolve before sealing/adoption/recovery; the shared finish half inherits that custody |
| `save_studio_source` / `save_studio_source_reusing` | Converge on `save_studio_source_checked`; authenticate current intents and source, retain every old signed change, and refuse destructive replacement while Prepared |
| Checked candidate write | Private capability binds exact Prepared metadata, candidate and source-before bytes; ordinary callers cannot introduce protected overlay operations |
| Same-epoch packet/page ingest and receipt/Fault recording | May preserve the complete existing signed source; cannot erase evidence or inject protected branch operations without the handoff capability |
| Ordinary Apply/retry and receipt/manual retirement | Prepared must resolve first; Active annotations remain held; Completed leaves ordinary entries eligible under their original rules |
| Generic page/current-tail/seed source service | Prepared is a per-document hold; a completed record is flushed before serving |
| Source replacement/read after intent-file loss | Persistent source dependency refuses missing metadata, including after restart and later rotation |

There are no network awaits under vault custody. Normal initial Save publication remains bounded
to at most two packets. Future delivery of transferred operations uses existing authenticated paging.

## Evidence

Execution results are recorded in [HANDOVER](HANDOVER.md): 202 local application Studio tests
pass with one optional profile ignored, and all 98 selected replication Studio tests pass. The
[handoff workflow](https://github.com/Thalpy/Mewtual/actions/runs/34885437799) passes 22 normal
tests, nine isolated mutations and nine restored regressions. Its
[job log](https://github.com/Thalpy/Mewtual/actions/runs/34885437799/job/104114827694) identifies
PR merge checkout `e37caec550a1e4c80b8b4fc1a5b90bc1366d0c99` for code head `bf37cc4`.
Artifact `studio-handoff-mutations` contains the individual failing and restored test logs.
Subsequent documentation updates do not change the tested Rust or harness files.

The new `studio_overlay_handoff` tests use real signed Closing sources,
installed checkpoints, authenticated store records and actual replacement/retirement paths.
Coverage includes whole-branch order and signatures, crash/reopen barriers, absent/partial/conflicting
manifests, actual current-author/old-owner separation, source changes after Prepared, replacement
and publication fences, Fault/adoption/takeover, migration/target retention, real completed retry
after receipt retirement, rollover plus rewind, physical limits and later replacement peaks.

[The mutation harness](../.github/scripts/check-studio-handoff-mutations.py) requires one executed
failure at the intended assertion, unique anchors, byte-exact restoration and passing restored
regressions. It covers completed channel binding, signed digest, source version, replacement,
publication, missing metadata, retry floor, acceptance order and later source capacity.
[The workflow](../.github/workflows/studio-handoff.yml) publishes the mutation/restoration logs.
Compilation failures and zero-test filters do not count as evidence.

The existing overlay foundation workflow also passes 12 tests plus five mutations/restored
regressions. Native and two-client workflows pass. Broad CI remains failed for the recorded
queue-classification test, Linux desktop unused-code errors and Rustls dependency advisory;
HANDOVER distinguishes those unchanged-source failures from the handoff checks.

## Still outside this checkpoint

Actor scheduling/detached preparation and measured signing custody, native overlay commands,
manual inspect/copy/export/disposition, stale-base reconciliation and preview-based overlays remain
open. No durable overlay UI Save is enabled. Repeated-owner tenure work, signed fault repair and
combined Gate 4 acceptance remain separately tracked. The existing signed-history replay rules,
including ordinary failed-Save NoEvidence, are unchanged.

## Message for adversarial implementation review

```text
Please adversarially review the Closing overlay handoff implementation in PR #26.
Base: dd2fbc01ebb8a692964eec0df19dd201b225c936
Tested code head: bf37cc48c45d09be6b63e97322171e506768fbf4
Compare: https://github.com/Thalpy/Mewtual/compare/dd2fbc01ebb8a692964eec0df19dd201b225c936...bf37cc48c45d09be6b63e97322171e506768fbf4
Read docs/GATE4-OVERLAY-HANDOFF-IMPLEMENTATION-REVIEW.md and docs/HANDOVER.md.
HANDOFF-001 is closed and the corrected design is accepted; this request covers implementation.

Review the three durability barriers, current-owner/pristine-successor eligibility, complete
signed-operation evidence, sequence/timestamp preservation, actual source/intent rechecks,
restart resolution, normal replacement/publication paths and existing quota/reference accounting.

Pay particular attention to the new persistent local source-to-intent dependency: missing metadata
must not expose an unfinished batch after restart or after a later source replacement. Check that
the private candidate capability cannot be replaced by an ordinary writer or caller-supplied IDs.

Check completed retry after real receipt retirement, full channel binding before any acknowledgement
or sync, v1 migration, Active plus older Completed, retry floor retention and old retry after rewind.
Inspect normal test results and isolated mutation/restoration logs rather than relying on comments.

Return PASS for this bounded core/store implementation or specific findings with independently
targeted assertions. This does not request actor/native activation or full Gate 4 acceptance.
```
