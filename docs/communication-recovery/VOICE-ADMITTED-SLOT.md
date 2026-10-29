# Admitted native Slot and operation31

29 October 2026. Implementation draft in an isolated worktree. Native code,
layout guards and native tests are **uncompiled/unrun**. V2 remains OFF.

This is inert capacity custody. `Reserved` means that the original native Slot
owns BOTH a genuine neutral `RootRoundPreclaim` and an allocator-excluded
`AdmittedParentClaim`. It grants no peer/role, Availability, RoundGate, Action,
signing, Ready, SDP, RTC or media permission. There is no consume API here.

## Separate physical profile and fixed grammar

The original admitted physical Row selects a separate `AdmittedRoute`. Existing
routes and request decoders reject operation31 before counter debit. Admitted
routes reject every Existing operation1–30 before counter debit. Only the common
cryptographic framing understands the new operation number; it does not select a
fallback body grammar or provide profile authority.

Operation31 is Control, its logical ID is the exact physical Session, and request
ciphertext is exactly26 bytes. The ten-byte plaintext is version12, one kind
byte, then an unsigned64-bit big-endian generation. Kinds are:

| Kind | Byte | Generation |
| --- | --- | --- |
| SetupAck | 0 | Exactly0 |
| Register | 1 | Nonzero |
| Observe | 2 | Nonzero |
| Cancel | 3 | Nonzero |

SetupAck only returns the original `browser_setup` contribution. Its trusted
retained driver must send it after the actual original ECDH/HKDF/setup promises
return. The ACK itself proves no SDK/general return. It creates no Slot, task,
registration or ordinal. Exact repeats are noncreating. The production driver
sender is a separate integration; this leaf supplies pure codecs only.

The response is59 plaintext bytes,75 ciphertext bytes: version12, closed state
byte, requested/canonical/cancellation-floor u64s, explicit incarnation-presence
byte and32 identity bytes. State values0–8 are Unstarted, Preparing, Waiting,
Reserved, Closing, Returned, Closed, Uncertain and Superseded. A present all-zero
random identity is preserved. SetupAck's g0 validation is separate from the
nonzero-generation rules. These observations never prove that no Root round was
minted, that unknown work was refused, or that browser SDK/physical owners drained.

`voice-ipc-admitted-admission.ts` and `voice-ipc-admitted-frame.ts` are distinct
pure helpers. The old frontend framing remains closed1–30. Shared request vectors
pin the same native/browser version, kinds and full u64 values.

## Original task and affine results

Register requires current UI generation, server instance, actual NativeActive
Handle and its exact cached typed admitted key. The final UI→Coordinator→Row
fence is repeated when installing the Slot, queue invocation and Reserved
candidate. Native never converts an admitted key into `SelectedHandoffOwner` or
uses the old Root HIGH admission/lookup path.

One separately paid Slot Box and permanent task/native hold belong to the original
Row. Funding is outside the Box and survives its actual deallocation. Every
funding alias uses `Arc::into_inner`, with no raw Arc/Weak or backedge. No task or
abort handle is retained in the Slot. The Host's existing bounded cadence wakes
retries; a fast Capacity callback cannot requeue itself without another granted
turn. Observe is noncreating and may only finish already-paid negative recovery.

The canonical invocation exists before the Actor publisher can enqueue. The SAME
sealed `VoiceActivationControl` and its send fence carry the actual grant,
activation, optional original admitted observer and generation. The whole affine
result is staged even after cancellation. `Some(registration)` survives a later
capacity error; retries use its same observer/incarnation. A genuine pre-enqueue
refusal preserves its returned `AdmissionRpcPermit`; accepted Work owns its fee
through actual argument/callback return. Entered callback loss/panic is Uncertain,
even after Work returns, and can never authorize another `None` registration.

Once Root supplies a neutral candidate, local ledger/UI/Coordinator/Row Busy
retains that SAME candidate and any already-reserved native claim. Both owners
must match the exact registration, physical identity, typed key, ledger and g
before Reserved. A successor is checked g+1 only after actual old-generation
capacity/claim/Work reclamation. Cancellation validates the ordinal before changing
any floor and cancels admission only, not the underlying call or sibling media.
An exact-next Cancel whose Register never entered advances only the negative
canonical/floor and cannot create work. The same checked through-generation rule
applies before the first Slot. Late Register cannot revive a canceled generation;
skips and wrapping ordinals refuse. A canceled in-flight observation stays Closing
until actual return, and ordinary cancellation preserves the original registration
for the next generation even if Root or final attachment observes the new floor.

Task destruction closes first, transfers the SAME native position to original
negative custody when owners remain, and leaves owed results/Work in the Slot.
Recovery needs no new task/RPC/payment. Heavy owner destruction is outside all
native guards; a final marker follows actual affine owner destruction. Host
shutdown closes every Slot and joins its original Work/position before physical
idle. Work/Task return is not a Tokio-allocation or NoMint certificate.

The physical-history owner is implemented separately. Its actual Row collection
must still require the exhaustive admitted Slot drain predicate. This leaf does
not implement a never-started/ActuallyCollected receipt or its final ACK, and must
not weaken that independent retained-history requirement.

## Inventory and verification

The candidate task/Slot allowance is8KiB and the original Actor contribution4KiB.
Production guards include the concrete task, Slot/Box construction, finisher,
reclamation/return markers, parent preparation, actual publisher and Actor graph.
The HTTP/control and enclosing future inventories include the new path. These
are **not measured acceptance claims** until native compilation executes them;
no existing term may be cut to fit. Original group/process/position caps are
unchanged. Physical Row and admitted parent have their separate existing fees.

Twenty-five focused Node tests pass: eight new admitted cases plus the existing
framing/negotiation cases. Targeted strict TypeScript and leaf-only Rust formatting
pass. Native tests are drafted for genuine Actor/Root registration retry under
shared32-slot pressure, monotone cancellation, task-abort recovery under full
ordinary bytes, actual reclamation unwind, and authenticated TCP cross-profile,
version/length/nonce and same-incarnation successor behavior. Further regressions
pin lost-before-entry next-generation cancellation, the transient Closing snapshot,
and actual cancellation between acceptance/enqueue or native claim/final attachment.
They are unrun.

The primary's early actual-diff review found the canceled-Waiting snapshot and
lost-next-entry cleanup defects; this snapshot fixes both and awaits re-review.
The independently owned paid RowRef/SessionRef prerequisite is not yet adopted.
Native measurements/tests/lint and complete
backend/frontend/integration gates remain pending with the primary. Operational
RoundGate binding/role trim/consumption, bilateral control, admitted KDF/tags8/9,
Ready, RTC/media, real browser driver integration and activation remain unfinished.
