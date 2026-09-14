# Gate 4: Closing-overlay runtime activation

Status: proposal for user adversarial review, 2026-09-15. Review base: `aa0a81f34af6707ab05fd2a437412c38b79a82ff`. HANDOFF-002 is closed; the bounded
core/store transaction at `62f06d4` is accepted. **Gate 4 is still active and incomplete.**
The user clarified to finish Gate 4 before starting Gate 5. This checkpoint adds diagnostic
profiling and this runtime proposal; it does not add an actor/native overlay command.

## Why direct wiring is insufficient

The accepted [handoff design](GATE4-OVERLAY-HANDOFF-REVIEW.md) explicitly excludes running the
256-operation synchronous batch in the live actor. `StudioDispatch` moves the sole Server and
vault lease into a blocking worker, then awaits its return. This prevents executor blocking,
but it still holds the actor, numeric-server ordering, UI commit and vault custody for the
whole operation. Putting `Server::handoff_studio_overlay` in that arm would preserve durability
while delaying unrelated actor work and lock teardown.

This is more than a signing loop. `EpochIntentState::decode` validates the overlay, and
`local_draft` reconstructs it again. The handoff loads/validates metadata repeatedly, restores
the pristine successor, performs typed admission and signing for every operation, verifies the
completed source and traverses inventory. Moving only one signature loop leaves substantial
work under custody. A timeout cannot revoke a blocking worker's ownership or refund its slot.

The opt-in [profile](../crates/catcoms-app/src/store/epoch_studio/tests/rotation/overlay/handoff/performance.rs)
measures actual Index and Flipnote paths at 1, 32 and 256 accepted operations. It separates
record read/unseal/decode, draft reconstruction, private candidate generation, inventory,
warm durable handoff and completed retry after reopen. Results and limitations are recorded in
[P1-PERFORMANCE](P1-PERFORMANCE.md). These are measurements, not latency acceptance thresholds.

## First implementation boundary: detached inspection

Start by making a retained draft readable without reconstructing it while the actor holds
the vault. This also supplies the capture/currency boundary needed for later local acceptance
and handoff. It introduces no write authority and does not enable native overlay Save.

1. **Capture under the existing Ready/lease path.** Require a live numeric server, known channel,
   actual current device membership, the complete target, actor incarnation and mounted vault.
   Reserve from `registry_catchup::preparation_pool()` before reading: the existing four slots
   are shared with Registry/Studio source preparation, not duplicated for overlays. Authenticate
   one bounded intent record without calling its expensive decoder. Capture its complete-wrapper
   digest, physical size, mount identity, full server/group/type/logical/channel target, local
   device and current group context. Validate the parent directory and regular-file restrictions
   used by ordinary intent reads. Capture absence explicitly; missing and unreadable differ.
2. **Release actor and native guards before reconstruction.** The job owns only authenticated
   zeroizing plaintext, public context and its original permit. It owns no Server, store,
   device/MLS secret, source writer or editable installed epoch. On the blocking worker, use the
   complete existing intent/overlay decoder and ordered reconstruction. Check the complete target
   even for Completed/floor-only metadata, and the active branch's author before returning its
   local projection. Ordinary pending intents are not local acceptance. Prepared is still a
   retained draft; inspection must not complete or cancel it. Completed-only metadata is not a
   fabricated local draft or settlement receipt.
3. **Reacquire custody and check currency.** Before delivering a result, recheck the original
   numeric server, actor incarnation, mount, group/device context, membership, channel and latest
   request. Reauthenticate the actual bounded intent wrapper and require the same full digest and
   physical size; do not decode/reconstruct it again. For captured absence require continuing
   absence. A changed record, deletion, remount or replaced request discards the result. An
   unchanged logical key, basis fingerprint or visible projection alone cannot pass this check.
4. **Retain ownership through native delivery.** Carry the original shared permit through queued
   work, running work, ready result and conversion. Cancellation, timeout and remount do not refund
   it until the last actual worker/result owner drops it. Use the existing native cancellation,
   request/session/instance fences and the actor delivery handshake; recheck after conversion.
   One request token and original UI session generation span both custody visits. The second visit
   cannot silently start a fresh session/request that legitimizes an obsolete first result.

The entire input is bounded by the existing intent record ceiling (5 MiB + 1024 plaintext bytes,
plus sealed framing). The existing 2 MiB base, combined 64 KiB metadata, 256-operation and typed
projection limits remain simultaneous. No second seed pool, plaintext history cache or persisted
inspection record is introduced. The input and reconstructed output may coexist only within the
same owned preparation slot; drop input and intermediate graphs as soon as reconstruction and
stamp creation finish. Native conversion must account for its encoded output as well as retained
typed input; its concrete byte limit and maximal accepted-shape tests are part of the implementation
review before that command is enabled. This proposal does not assert a measured heap ceiling.

Return a separate local-draft inspection result, containing its basis, accepted count, Active or
Prepared transfer state and full conflict-preserving projection. No current phase, publication,
receipt or editable epoch is invented. An absent local draft says nothing about pending ordinary
intents, completed handoff records or shared settlement. Keep draft display separate from ordinary
`StudioRead` and the awaiting-tenure preview cache. Command names/JSON become callable only when
the implementation and its tests are entered in [FLIPNOTE-UI-HOOKS](FLIPNOTE-UI-HOOKS.md).

## Following write boundary: preparation, signing, durable commit

The inspection result is never an append or handoff capability. Preserve the existing acceptance
and handoff provenance and recheck them from fresh live state. In particular:

- Closing basis creation uses the authenticated actual source, matching saved signed close and
  independently observed current-owner tenure. The renderer supplies no close, seed, receipt,
  author or tenure authority. Ordinary failed Apply is not silently converted to local acceptance.
- Move typed reconstruction, ordered change preparation and exact checkpoint/recovery preflight
  off actor/store custody. A new private prepared-change type must bind every original envelope,
  author, nonce, timestamp, order, actual source/intent version and authorization context. It is
  minted only by complete validation, never by decoding renderer-authored deltas. Expose no
  generic "sign these bytes" API. Device and MLS secrets stay with the sole actor.
- Signing uses finite turns on the private candidate, rechecking current incarnation/membership/
  owner/tenure/MLS context before each turn. Start with one bounded operation per turn; measure
  that unit on maximal accepted input. One operation alone is not proof of a latency bound.
  Return heavy validation, source assembly and final manifest work to the owned worker. Keep the
  candidate and all signed output private until the accepted durable transaction completes.
- Before Prepared, reacquire exclusive custody and reauthenticate full actual source and intent
  wrappers, including required-metadata links and complete channel identity. A changed source,
  branch or authority cancels the candidate. Reuse verified data only under those exact stamps;
  do not remove the common source-write fence or make the reference inventory use the pure
  validation cache. HANDOFF-002's complete reference traversal remains mandatory.
- Retain the accepted Prepared -> whole Source -> Completed transaction, full signed digests,
  retry floor, reference protection, replacement peaks and interruption resolution. No durable
  signed prefix or per-entry retirement. If any barrier needs detached expensive work between
  visits, Prepared remains a durable per-document hold and all common writer/publication paths
  keep their existing fences. The next visit checks actual evidence, never a worker's claim that
  a prior write must have completed. After Completed, use normal page/tail publication; do not
  enlarge the ordinary two-packet initial Save window to send the batch.

This is the next write-design obligation, not permission to expose a partially implemented
native Save. Existing actor preparation scheduling must remain fair to authoritative discovery,
receive and other servers. Coalesce one target's work, pace retries, and do not turn repeated UI
Reads or peer traffic into unbounded preparation attempts. Refusals retain local work and require
fresh context; they never imply successful handoff, delivery, settlement or disposal.

## Required evidence before accepting the inspection implementation

| Case | Required independent observation |
|---|---|
| Index and Flipnote reopen | Real store-accepted branches; original count/order/timestamps and full projection; canonical records unchanged. |
| No draft / failed Save / completed-only | Separate results from real records; no promotion, new acceptance, invented projection or false settlement. |
| Scope/author/corruption | Authenticated but wrong channel or author reaches its exact check; malformed/truncated/nonregular records error rather than look absent. |
| Supersession | Change only actual intent bytes after capture, preserving displayed content/basis where possible; reject at the full-wrapper check. Test deletion and absent-to-present separately. |
| Live authority/lifecycle | Distinct tests for device/group/membership/channel, actor replacement, numeric server and remount, with earlier checks held valid. |
| Resource ownership | Four shared slots occupied by real jobs/results; cancellation of a paused parser retains its slot; completion and final delivery release it exactly once. No additional overlay-only pool. |
| Actor progress | Pause an actual detached reconstruction; unrelated actor work and an authoritative checkpoint operation still complete without releasing its retained slot. |
| Native final delivery | Convert real actor-produced local drafts, then independently invalidate session/request/delivery; final check rejects the otherwise complete value. No ordinary-view substitute. |
| Accepted limits | Maximum admitted seed/metadata/operation and output shapes, simultaneous input/output accounting, overflow refusal without partial output, and retained data unchanged. |

Use isolated mutations for full-wrapper currency, target/author and final-delivery guards where
another failure could mask their removal. Require the intended executed assertion failure, exact
source restoration and passing restored regression. The diagnostic profile is not a substitute
for these tests or for signing/commit cost qualification.

## Work that still closes Gate 4

After the read/preparation boundary: reviewed native local acceptance and automatic handoff;
bounded manual inspection/copy/export/disposition for noneligible branches; conservative stale-base
handling and separately reviewed preview-based local work; remaining repeated-owner tenure
integration; runtime signed fault repair; then combined end-to-end acceptance and required suites.
Preserve accepted work through every refusal and interruption. The broader CI failures recorded
in HANDOVER remain acceptance items, independent of HANDOFF-002's closure.

## Adversarial review message

```text
Please review the Gate 4 overlay runtime proposal and its diagnostic profiling checkpoint.
Start with docs/GATE4-OVERLAY-RUNTIME-REVIEW.md, the latest HANDOVER entry, and the new
Closing-overlay section of docs/P1-PERFORMANCE.md. Inspect the actual profile code as well.

HANDOFF-002 is closed and the corrected bounded core/store handoff remains accepted.
Challenge the proposed capture -> detached reconstruction -> current-record recheck -> native
delivery boundary: complete scope/author binding, absence handling, live membership/context,
full-wrapper currency, shared permit ownership through cancellation and conversion, and no
promotion of a local draft into write/receipt authority. Check the same original session and
request must span both native custody visits. Identify missing evidence before implementation.

Check that the profiling fixtures exercise successful real decoding, typed candidate creation,
durable handoff and exact reopen/retry, and distinguish batched setup from the measured paths.
The debug measurements are not release latency, maximal-byte coverage or actor fairness proof.
No actor/native overlay command is implemented in this checkpoint. The later signing/commit
split and manual/preview lifecycle still require their own implementation and acceptance.

Return PASS for this bounded proposal/instrumentation, or numbered findings with concrete
failure paths and required corrections. A PASS does not close Gate 4 or enable native Save.
```
