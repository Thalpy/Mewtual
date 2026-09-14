# Gate 4: Closing overlay handoff implementation

Status: user implementation review of `dd2fbc0...85e7179` requests changes for HANDOFF-002 (P2),
2026-09-14. The scanner correction is implemented and validation is in progress. The reviewer
found no additional defect in the transaction, exact signed evidence, live-owner eligibility,
restart/write/publication fences, completed binding or retry floors. Implementation acceptance
remains pending. HANDOFF-001 and the earlier findings stay closed; Gate 4 remains open.

## HANDOFF-002: reference inventory dependency

The linked source previously refused ordinary reads when its required intent metadata was
missing, but the pixel-reference scan discarded that dependency bit. A scan could therefore
declare its reference set complete while overlooking an accepted overlay-only pixel CID.

The corrected scan collects required targets from authenticated linked sources and actual
handoff-metadata targets from authenticated intent records. It matches numeric server, complete
group/type/logical scope and full target before installing references. Ordinary ledgers do not
satisfy the dependency. Only metadata from records already read in the normal bounded traversal
is retained; the one-body-per-step, record-count and byte limits are unchanged. Reference scans
continue to bypass the pure source-validation cache. Missing/mismatched metadata prevents the
completed reference installation; the public scan leaves deletion protection unknown.

The new pixel regression accepts a real stored PIX CID only in the overlay, independently checks
its absence from the pristine successor, overlay seed and recovery files, and interrupts before
the Source write. After deleting the intent file and reopening, it calls only the reference scan,
attempts protected deletion and checks that the original bytes remain. Restoring the original
metadata restores a complete pin set. A healthy-scan unreferenced CID proves real deletion works.
The second test supplies independently authenticated/decodable records with an ordinary ledger
or mismatched numeric server, group, logical key, type or channel, then restores the original.
The added mutation ignores only the final dependency-check result, retaining every other scan
guard and the deletion wrapper. Its required failure is the missing-metadata scan assertion.

Correction base: `85e71798b4197e8a7ef7d1361e4418bd7ecd32d4`. Results are recorded in HANDOVER.

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

The write-boundary crash matrix carries one accepted operation. Dependent multi-operation
replay and partial-manifest retention are separate tests; this is not an executed multi-operation
crash-prefix matrix. Injected I/O failures and vault reopens are not physical power-loss tests.

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

## Message for adversarial re-review

```text
Please re-review HANDOFF-002 (P2), the source-to-intent dependency in reference inventory.
Base: 85e71798b4197e8a7ef7d1361e4418bd7ecd32d4
Use the correction head and validation evidence in the latest docs/HANDOVER.md entry.

Check that linked-source requirements match actual authenticated handoff metadata by numeric
server, full group/type/logical scope and complete target before reference protection is installed.
Ordinary ledgers, missing files and mismatched metadata must leave reclamation disabled. Verify
that existing count/byte/per-step limits and reference-scan cache bypass remain intact.

Inspect the real stored overlay-only PIX regression: pristine successor and no recovery/base
copy, interruption before Source, deletion of metadata, fresh reopen, failed scan, actual protected
deletion retaining the bytes, and successful restoration of the original metadata. Check the
healthy-deletion control and all authenticated scope substitutions.

Inspect the isolated reference-dependency mutation and restored regression. Ignoring only the
final dependency result must expose a complete scan and deletion of the overlay-only bytes;
compilation failures, zero-test filters and unrelated errors must not count as detection.

Return PASS to close HANDOFF-002 and accept the corrected bounded core/store implementation,
or concrete remaining findings. Earlier closures stand. Actor/native activation and full Gate 4
acceptance remain outside this review. The existing write-crash matrix has one accepted operation;
multi-operation replay/partial-manifest cases are separate tests, not physical power-loss trials.
```
