# Unsigned V2 Add data from the original replay model

Local Core checkpoint, 7 November. The reviewed children and parent are now declared.
All258 Core chat tests pass in101.26s after a19.53s compile, including this slice's six
new tests. Independent actual-source review is clear after a max64 fixture-key collision
was corrected before execution. No owner key, actual MLS mutation, Source/Current
constructor, durable Add receipt, native dispatch or transport is enabled.

`ReplayMath::prepare_admission_data` returns immutable `UnsignedAdmissionStepData`: a
HeaderSpec, complete canonical public image, existing owner-header/v2 signing bytes and a
domain-separated local intent commitment. There is no public constructor, decoder, Clone,
mutable model/image/spec getter, signing callback or externally supplied next model.
Inputs and output are mathematical DATA; neither raw Admission labels nor successful math
grant permission to use a founder key or open a member Source.

The pre-sign and signed replay paths share one fixed Add change validator. It requires
Add origin, exact same group/Profile, current before epoch and checked after=before+1,
remaining sequence space, fewer than64 current slots and a nonduplicate full subject.
The slot is inserted in canonical Admission-CID order into an explicitly64-capacity image
vector. It has empty accepted frontier, tip BODY equal to its actual Admission CID and
the exact Profile/Admission-context empty root. No current Header CID is placed recursively
in its own image. The signed path still performs every genuine successor signature,
Profile/header/body and complete proposed-image check before using that shared change;
only the actual signed Header supplies the installed new slot's tip HEADER.

Preparing data never changes the original model. Repeating under the same model produces
identical bytes/commitment. Any intervening global successor changes parent/preimage even
if it appended another Admission's log and left the Add's epoch unchanged; the stale signed
Header cannot advance that newer model. Every unrelated slot retains its own exact tip.
Remove/rejoin produces a fresh A2 incarnation for the same key; no frozen A1 is reopened.
The64 current-slot limit does not impose a lifetime admission limit.

The separate `UNSIGNED_ADMISSION_DATA_INVOICE` is64KiB. Its compile-time named expression
includes the actual unsigned/change/spec/frame layouts, one64-slot public image vector,
two maximum public images, bounded Admission/Header/signing/group scratch and hashes.
The actual caller must independently prepay it before preparation; complete ReplayMath1MiB,
input ownership, original SDK/source/control/crypto costs and durable operation obligations
remain separate. The signed replay's existing named1MiB expression additionally counts the
new fixed shared-change layout without increasing its cap. No actual Root payer or whole
allocator/RSS claim is supplied by these constants.

Five transition regressions and one actual general-verifier regression are authored. They
cover independent received-image/signing-byte equality, repeated intent stability, real
Header signing/replay and exact host selection; founder/foreign/duplicate/epoch and signed
predecessor/kind binding refusals; stale Add after another Append; canonical64-slot fullness
and same-key rejoin; checked arithmetic boundary instrumentation; and a real member action
after genuine Core selected Admission without another owner Header. The extreme sequence
test injects a private arithmetic boundary and does not claim to replay2^64 Headers. Test
HOST selections are not App Saved receipts or actual MLS Add observations. No build/test
has been run by this source-only agent.

Next actual producer must construct the Admission ONLY from an opaque successful
`LocalMembershipChange` and its same isolated MLS candidate, exact commit/Welcome/KP and
credential, independently trusted original V2 pin/G/current prefix, prepaid operation and
mandatory-control space, and original Source/native generation. Owner signing must follow
the real durable governing intent; raw caller Admission input cannot authorize it. A real
saved transaction must cover MLS state, V2 Header/image, consumed invite and outgoing
Welcome/commit before publishing or constructing the destination member owner. Destination
must match the original selected-KP Welcome and acquire its own real Source/Current and
retained graph/runtime payments. Existing Genesis-only and v1 checkpoint wrappers cannot
be relabelled. Those SDK, persistence, invitation, Root/UI and outbox consumers remain open.
