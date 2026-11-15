# Native acceptance fixture corrections (12 November; pending CI)

## Later full-run SDK preparation refusal

Full native run43522590967 passed on Windows but failed on Linux: the original
SDK preparation unwrap returned Protocol(Capacity), followed by17 failed-shutdown
exclusion cases. A constructor-only fixture retry now retains the same physical
position and request for at most32 attempts with sanctioned Clock sleeps of10ms.
The actual parent mutex oracle proves refusal without changing SDK preparation
or installed-obligation flags, and releases its guard before assertions.
Allocation, attachment, final authorization and Release are unchanged.

The primary scenario passes locally(6.49s), both closing-fence cases pass(13.96s),
and native checking and formatting pass. Static design/diff review is clear.
The next full Linux native run remains required. The former failed-media
parent-without-key timeout remains unresolved; improved diagnostics are not a
production correction. Chat V2 remains off.

The broad native runs have genuine unresolved failures. They cannot be counted
as passes because later cases fail through the shared shutdown gate. A static
audit confirmed three independent fixture contract errors; it did not establish
the cause of the Linux control-step timeout.

- Negative media Resolve/Close/Resolve can encounter local Index/row Capacity
  before changing state. The fixture now forces actual Index exclusion, releases
  it before any wait, and retries only Capacity on the same original Session and
  Begin within 32 observations/one second. It preserves the no-admission/no-Root
  assertions and original negative result.
- PC preflight can encounter a busy server registry before Actor work. The
  fixture forces that real barrier, releases it, then retries only this local
  refusal using the same Context/Route. No PC/key/Ready is recreated.
- Joining a browser media attempt returns void, not proof of fresh Current. The
  sibling fixture now requires its actual authenticated Current observation to
  increase on the same retained plan, with its peer still open. Existing Session,
  key, bootstrap, counter and cleanup assertions remain.

The Linux timeout now reports a bounded fixture step name, control-frame count,
closed callback enum and existing original-scope diagnostic observations. It
does not query an Actor, resolve a replacement scope, log SDP/key/payload data or
extend any timeout. The cause must be diagnosed from execution; passing a
different case cannot establish that this failure is inherited or harmless.

No shipping voice policy or protocol code is changed. Test-only native barriers
hold the actual index/server registry; they issue no capability or admission.
The native Index barrier exercises trusted-scope negative operations. The
browser regression separately uses authenticated stand-in replies to prove
Current retry behavior. These are complementary checks; they do not force
contention during an authenticated HTTP Current on the original native Session.
That combined boundary remains a coverage gap.
No platform stack, resource cap, protocol deadline or cleanup assertion is
relaxed. The focused CI option runs four exact nonempty real native/WebCrypto
cases, all six native command-ACL controls and the separate native compile check.
Complete root/native/frontend,
strict/audit and physical audio/network acceptance remain separate required
gates. No local Cargo or physical-media claim is made.
