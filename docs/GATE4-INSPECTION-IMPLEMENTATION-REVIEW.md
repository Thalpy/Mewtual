# Gate 4: detached local-draft inspection implementation

Status: user **PASS** for read-only inspection; **INSPECTION-TEST-001 closed** by user
re-review, 2026-09-15. Reviewed `0b28f06...c47ae0b`; original code/test head:
`d5ca2ff1516852f15edf7fe5b09db07c61fcda56`, base:
`0b28f06b8a2076a345f06dfea1220787c262650b`. Production changes are at `bf7ceed`;
`d55c05d` and `d5ca2ff` extend/correct tests and documentation; `c47ae0b` adds evidence only.
The user inspected code, actual CI logs and all ten new mutation/restoration logs, verified
artifact hashes and found no blocking production defect. They did not run Cargo locally.
HANDOFF-002 and all prior closures stand. Gate 4 remains active; Gate 5 is untouched.

## INSPECTION-TEST-001 correction

Correction code/test head: `d38df93f2ba49b20b632870d38d353e69e2d60d8`, against
`c47ae0b61b9b8adc0ab5e4bf798199413bfff73c`. Only the regression, mutation harness and
documentation change. Execution evidence is in HANDOVER.
The focused local regression and size-only mutation/restored test pass. GitHub run 34953124247
also passes all 10 store/actor tests, two fixture tests and four mutation/restoration pairs.
The added mutation fails at `inspection.rs:182`, then the restored two-kind test passes.

The original growing-record fixture rejects even if currency checks only physical size. The
new regression replaces one ordinary pending envelope's fixed-width nonce after capture,
through the existing authenticated record writer. Both records must fully decode with equal
actual sizes, unequal plaintext digests, identical accepted metadata, target, author, basis,
count and complete draft projection. Fresh inspection succeeds, the old stamp rejects, and
inspection/currency checks leave durable bytes unchanged. Both Index and Flipnote run.

The added isolated mutation retains physical-size comparison while removing digest comparison.
It must execute the new test and fail at the specific obsolete-stamp assertion, restore source
bytes exactly, then pass restored source. No production change is requested or made. See the
latest HANDOVER for execution status. The user accepts this correction and closes the finding
without further changes; the production PASS stands.

`studio_overlay_read` returns a separate read-only local draft or absent result for Index and
Flipnote. It never resolves Prepared, fabricates a receipt or enables durable native Save.
The exact JSON and UI behavior are in [FLIPNOTE-UI-HOOKS](FLIPNOTE-UI-HOOKS.md).

## Ownership and currency

- The existing actor Control Ready/lease path checks a known channel and actual current local
  membership. It reserves one of the existing four Registry/Studio preparation slots before
  authenticating the bounded intent wrapper. Parent/file restrictions match ordinary reads.
- Detached work owns zeroizing authenticated plaintext, public context and that original
  permit. Full production decoding, target/author checks and ordered reconstruction occur
  outside actor/store custody. Input and intermediate graphs drop before the ready result.
- Completion checks sync incarnation, group/device/owner/MLS context, mount, numeric server,
  complete target and the complete authenticated plaintext wrapper digest plus physical size.
  Absence must remain absence. The completion read does not replay the draft.
- The original native operation registration, UI session generation, actor instance and latest
  view request span both custody visits. No guard is held while awaiting reconstruction.
- The existing actor delivery handshake keeps the checked state stable through conversion.
  The last retained result/delivery guard owns the preparation slot. Cancellation/expiry does
  not refund a running worker or retained result. A monotonic five-second validity deadline
  can expire during synchronous conversion; final native checks suppress the converted value.

The existing 32-MiB encoded JSON limit now uses a counting writer, avoiding an extra full
serialized buffer just to measure it. The typed projection and JSON value still coexist during
conversion under the same retained slot. This is not a measured process heap ceiling or a
release latency qualification. The original input/seed/metadata/operation ceilings remain.

## Regression evidence to inspect

See the latest [HANDOVER](HANDOVER.md) entry for executed commands, results and GitHub runs.

| Boundary | Fixture and independent observation |
|---|---|
| Real draft | Shared public-adapter fixture creates eligible signed history, seals it and accepts a local edit through the existing store path. Actual spawned actors produce both native drafts; full projection, author, count, basis and unchanged records are checked. |
| Currency | Original growing-record case plus a same-size authenticated ordinary-envelope replacement: different plaintext digest, identical accepted metadata, target, author, basis, count and projection. Old stamp rejects; fresh inspection succeeds; checks preserve bytes. Presence/deletion, scope, corruption, nonregular files and remount are separate cases. |
| Metadata | Wrong local author and same-logical-object/wrong-channel records reach detached validation. Prepared and Completed reads leave durable bytes unchanged. Ordinary pending intent records produce no draft. |
| Resource lifetime | Four captures fill the same preparation implementation; a cancelled paused worker retains its slot, as do ready results and expired retained delivery guards. The production actor job is checked against the actual shared pool. |
| Actor progress | Pause reconstruction of a real retained draft. Another watched document crosses the actual checkpoint writer before releasing the worker, then the original draft still delivers. |
| Native context | Actual successful reconstruction precedes session-generation change, newer request or instance replacement between visits. Separate tests first convert a real actor draft, then invalidate delivery, session or latest request. |
| Shapes and limits | Both kinds at 256 accepted operations; canonical codec fixtures at 64 Index objects with retained alternatives and 999 Flipnote frames with all 1024 conflict fields; bounded sealed input, exact UTF-8/escape byte counts and the 32-MiB encoded-output edge. Codec-sized fixtures do not establish signed admission or heap/latency qualification. |
| Mutations | Remove the full currency comparison, retain size comparison alone, or remove target, author, final inspection validity, or original native-context preservation. Each run must execute one test and fail at its named assertion, restore source bytes exactly, then pass restored source. |

New code changes no durable format, source-write fence, handoff transaction or publication
window. The required-metadata reference protection accepted under HANDOFF-002 stays in place.
The native account-read command policy, main-window capability and session-gate audit chain
include the command. Frontend layout and the mockup are separately owned.

## Remaining Gate 4 work

Read-only inspection is one part of block 2. Still required: detached write preparation,
bounded signing and durable native local acceptance/automatic handoff; manual copy/export and
disposition, stale-base and preview-based local-work handling, repeated-owner tenure; runtime
signed fault repair; combined full Gate 4 acceptance and required suites. Existing broad-CI
failures are recorded separately in HANDOVER. This checkpoint does not close the gate.

## Historical message for correction re-review (finding now closed)

```text
Please re-review INSPECTION-TEST-001 at d38df93f2ba49b20b632870d38d353e69e2d60d8
against c47ae0b61b9b8adc0ab5e4bf798199413bfff73c on PR #26, Create-suite-2.
Read the latest execution evidence in HANDOVER (subsequent documentation updates only).
Inspect the new same-size store regression and added size-only mutation in
.github/scripts/check-studio-inspection-mutations.py.

Require successful authentication and complete decoding of both records, equal actual sizes
with different plaintext digests, unchanged accepted metadata/target/author/basis/count/full
projection, rejection of the old stamp, success of a fresh inspection, and unchanged records
after read/checks. Confirm the size-only mutation fails at the intended executed assertion,
restores byte-for-byte and passes restored source; incidental failures do not count.

Return PASS to close INSPECTION-TEST-001 or concrete remaining findings. Production inspection
already passed; no production correction was requested. Prior closures stand. Native Save,
automatic handoff, remaining lifecycle/repair work and full Gate 4 acceptance remain separate.
```
