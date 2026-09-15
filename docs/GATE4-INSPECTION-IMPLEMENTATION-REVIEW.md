# Gate 4: detached local-draft inspection implementation

Status: implementation and regression checkpoint, 2026-09-15. Code/test head:
`d5ca2ff1516852f15edf7fe5b09db07c61fcda56`, base:
`0b28f06b8a2076a345f06dfea1220787c262650b`. Production changes are at `bf7ceed`;
`d55c05d` and `d5ca2ff` extend/correct tests and documentation. Subsequent evidence updates
change documentation only. The user passes the base profiling/design checkpoint
without findings. HANDOFF-002 and all prior closures stand. This checkpoint requires a separate
user adversarial implementation review. Gate 4 remains active; Gate 5 is untouched.

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
| Currency | Add an ordinary pending intent while preserving displayed local content, count and basis; only full-wrapper currency changes. Presence/deletion, numeric/group/target/author scope, corrupt files, nonregular files and identical bytes after remount are separate cases. |
| Metadata | Wrong local author and same-logical-object/wrong-channel records reach detached validation. Prepared and Completed reads leave durable bytes unchanged. Ordinary pending intent records produce no draft. |
| Resource lifetime | Four captures fill the same preparation implementation; a cancelled paused worker retains its slot, as do ready results and expired retained delivery guards. The production actor job is checked against the actual shared pool. |
| Actor progress | Pause reconstruction of a real retained draft. Another watched document crosses the actual checkpoint writer before releasing the worker, then the original draft still delivers. |
| Native context | Actual successful reconstruction precedes session-generation change, newer request or instance replacement between visits. Separate tests first convert a real actor draft, then invalidate delivery, session or latest request. |
| Shapes and limits | Both kinds at 256 accepted operations; canonical codec fixtures at 64 Index objects with retained alternatives and 999 Flipnote frames with all 1024 conflict fields; bounded sealed input, exact UTF-8/escape byte counts and the 32-MiB encoded-output edge. Codec-sized fixtures do not establish signed admission or heap/latency qualification. |
| Mutations | Remove only full-wrapper currency, target, author, final inspection validity, or original native-context preservation. Each run must execute one test and fail at its named assertion, restore source bytes exactly, then pass restored source. |

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

## Message for adversarial review

```text
Please review detached local-draft inspection at d5ca2ff1516852f15edf7fe5b09db07c61fcda56
against 0b28f06b8a2076a345f06dfea1220787c262650b on PR #26, Create-suite-2.
Compare: https://github.com/Thalpy/Mewtual/compare/0b28f06b8a2076a345f06dfea1220787c262650b...d5ca2ff1516852f15edf7fe5b09db07c61fcda56
Read docs/GATE4-INSPECTION-IMPLEMENTATION-REVIEW.md, the latest HANDOVER evidence,
and FLIPNOTE-UI-HOOKS.md, then inspect the actual store/app/native code and test logs.

Challenge full-wrapper currency despite unchanged visible content, complete target/author
binding, Prepared/Completed/absent behavior, the original session/request across both custody
visits, cancellation and permit ownership, actor checkpoint progress while reconstruction is
paused, and final delivery after successful native conversion. Check the byte-limit and maximal
shape fixtures without treating codec fixtures as signed-admission or heap/latency evidence.
Inspect isolated mutation failures and restored passes; compilation errors and empty filters
do not count. Check the native command permission chain as well as the Rust guards.

Return PASS for this bounded implementation or numbered findings with concrete failing paths
and required corrections. Prior review closures stand. Native overlay Save, automatic handoff,
manual/preview lifecycle, signed repair and full Gate 4 acceptance remain separate work.
```
