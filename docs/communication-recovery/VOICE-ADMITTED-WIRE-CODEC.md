# Private admitted wire codec checkpoint

29 October 2026. This leaf implements the syntax/vector portion of the reviewed
[bilateral admission proposal](VOICE-BILATERAL-ADMISSION-PROPOSAL.md). It is private,
inactive and has no production signing or operational consumer. The exact inactive
allocations below were agreed for this staged leaf and are pinned in the proposal.
Operational wire/consumer allocation, compatibility and activation gates remain open;
this leaf is not an interoperability or activation approval.

## Implemented boundary

`catcoms-sync::voice_v2::admitted_wire` retains the exact canonical signed input as a
borrowed slice. `SignedAdmitted` exposes immutable header/body projections and hashes
that same original slice. Decode verifies issuer/key binding and the outer Ed25519
signature before a Request verifies its one nested signed Availability. That nested
decoder requires the fixed Availability operation before body/signature work and cannot
recurse into another Request. Availability must be positive and match exact scope,
epoch, HIGH identity/join/key/incarnation/generation and LOW target.

The separate closed body tags are control7, encrypted8 and Ready9. The admission profile
is1; only the separate admitted SDP grammar accepts profile2. Old Body1-6, SDP profile1,
Source and dispatch enums/encoders remain unchanged. No envelope decoder, forwarding
validator or route-permit issuer for the new family is introduced.

Agreement A is the proposal's exact503-byte raw shape, with explicit absent-policy
zero bytes, positive generations and actual ordered LOW/HIGH labels. `RoundRef` derives
BLAKE3 over the new round domain plus A. The789-byte transcript embeds R, A and both
endpoint Ed25519/P256 public bytes. It emits the fixed830-byte salt preimage, SHA256
salt and839-byte directional info; it derives no key. Point validation establishes
uncompressed shape, key-reference hash and DeviceId hash only. It deliberately does not
establish curve validity or original private-key ownership.

## Exact agreed inactive allocations

Control format1 and admission profile1 are closed. Query kind0 is protocol support and
kind1 is availability. Support permits only mask0x0007: bit0 is control7 forwarding,
bit1 encrypted8 forwarding and bit2 Ready9 forwarding. Zero/partial support is an
observation, never reservation, key ownership or consent. Busy retry hints are bounded
to0..1000ms for scheduling only and never renew attempts/deadlines. All generations and
advert serials must be nonzero; MAX is parsed as a value, not advanced or wrapped here.
Present all-zero random fields are valid.

Terminal decision0 is Unknown,1 is Accepted and2 is Rejected. Reason0 means no rejection;
rejected reasons1..5 are Cancelled, Expired, Superseded, AuthorityChanged and
ProtocolRejected; AuthorityChanged includes policy/epoch. Observed state0..5 is
HistoricalOnly, Negotiating, Established, Closing, Expired and Collected;6 is permitted
only for Unknown. Unknown requires absent
GD/R, zero decision metadata/result digest and observed6. Accepted requires present GD/R
and reason0; accepted-wall0 is valid. Rejected requires a reason and zero accepted-wall;
R cannot be present without GD, and rejected results cannot observe Negotiating or
Established.

The immutable Recorded digest is BLAKE3 over
`"mewtual/voice/admission/terminal/v1\0"` followed by exactly263 raw bytes:
`format1 | profile1 | group32 | policy_present1 | policy32 | room16 | epoch8 |
issuer32 | target32 | RD32 | GD_present1 | GD32 | R_present1 | R32 | decision1 |
reason1 | accepted_wall8`. Absent policy/GD/R bytes must be zero. Present zero digests
remain present. The current observed state and the signed wrapper's id/issued/expires
are excluded. Historical accepted-wall is not compared with a later wrapper's wall
window, so wrapper clock rollback cannot rewrite immutable history. A newly signed
wrapper still has a distinct signed-object hash. Unknown has a separate type variant
and no recorded-digest conversion.

## Bounds and verification scope

Closed signed-object ceilings are control2048, encrypted8864 and Ready693 bytes. The
encrypted variants retain existing confirmation/Bye/8KiB-SDP plaintext bounds plus the
16-byte AEAD tag. Decode borrows ciphertext/capability data and uses fixed at-most-two
helper storage. Re-encode explicitly allocates one exact canonical output copy.

Tests structurally frame the maximum policy-pinned Request1142 into direct1148 and
one-helper1544 bytes using the existing envelope and permit grammar. That test-local
framing proves byte widths and rejection by actual old ingress; it does not prove live
new-family forwarding or create an admitted envelope/Source adapter. The48KiB existing
envelope ceiling is unchanged.

Focused all-feature library tests pass26/26 on Windows. Their compiled inline layouts
are Signed592, Body352, Control344, Header208, Request328, Encrypted320, Ready344,
Agreement512, Endpoint136 and Recorded152 bytes. Wire A503 is distinct from its512-byte
in-memory struct. Neither fixed wire widths nor these inline measurements constitute
the proposal's complete prepaid graph inventory. A paid future consumer must count
retained canonical bytes, all simultaneous struct/encoder/hash/signature-library/IPC/SDK
copies and actual transition frames. Leaf formatting and strict all-target/all-feature
crate Clippy pass. Independent static adversarial review/re-review has no remaining findings.
The primary owns the complete backend/native/frontend suites; those have not run for
this isolated leaf, and no full-suite result is claimed here.

Focused verification commands (all passed; one shared Rust target, at most two jobs):

```text
rustfmt --edition 2021 --check crates/catcoms-sync/src/voice_v2/admitted_wire.rs
cargo test -p catcoms-sync --all-features --lib admitted_wire -j2 -- --nocapture
cargo clippy -p catcoms-sync --all-targets --all-features -j2 -- -D warnings
git diff --check HEAD
```

The focused run includes all26 current tests and the Request capability's original-input
pointer assertion;1351 unrelated sync tests were filtered out. Local logs are
`target/admitted-wire-focused.log` and `target/admitted-wire-clippy.log`. No full suite,
native/Tauri build, startup or frontend command ran in this isolated leaf. Those remain
the primary's integration gates; this change adds no native/startup/frontend consumer.

## Unfinished operational gates

No current query owner, challenge ordering, presence/membership/epoch proof, helper
compatibility/consent, replay/monotonic deadline state, Root fresh-activation profile fence,
paid neutral/native/browser preclaim, committed RoundGate or original-key ownership is
implemented by this codec. Agreement assembly and Commit checking only validate signed
relationships; they are not current native publication or a deadline-consumption step.

Production signing, new Source/helper routing, native/profile/Ready/KDF consumers,
terminal collection/Forget/link recovery, actual64KiB SDP-sidecar ownership, complete
inventory, full integration and physical acceptance remain open. The proposed16KiB
agreement/terminal allowance is not allocated or measured here. Existing local IPC29/30
is unchanged; proposed31/32 is not implemented by this leaf. V2 remains OFF.
