# V2 join binding codecs

The Core `chat::join_bindings` module implements cryptographic DATA formats for
the next production join vertical. No caller registers them on transport,
accepts a native join, mounts namespace6 or constructs an operational source.
Chat V2 remains OFF and incomplete. The wider work and design constraints are
in [the production-join plan](CHAT-V2-PRODUCTION-JOIN.md).

The invite contains a complete genesis-anchor tuple, inviter key, nonzero nonce,
expiry, immutable policy commitment and public routing-plan commitment. The
request binds the exact invite/anchor, joiner key and original request token,
plus THREE different package facts: exact wire-bytes commitment, OpenMLS hash
reference and MLS credential digest. A valid request signature alone does not
prove those last two agree with the actual package; the SDK must check them.

Welcome binds the exact request, invite, anchor and package, governing Add Header
and Admission, actual Welcome bytes, policy, original public routing plan and
separate exact sealed routing transfer. Its DATA consistency check requires an
Add origin, exact subject/reference/credential and the Profile/Header/body/tenure
links. The Welcome signer equals that Header issuer. Current Core requires the
Profile founder as governing issuer; an ordinary member/forwarder is not one.
An independently pinned complete prefix, real MLS outcome and covering save
remain mandatory. This helper does not verify the complete genesis/state math
or choose a selected parent.

All three semantic domains and their version2 fields are distinct; a common
tagged envelope contains the exact semantic frame and64-byte signature. Complete
envelopes are at most1024 bytes. Framing, widths and EOF precede the only
variable-size decode copy (group, at most256 bytes). Invalid syntax refuses
before calling the signer. Decoding is syntax only: every representation must
verify its own signature before duplicate or link processing, including one
with the same semantic ID as a valid frame. Debug formatting omits bearer and
identity/request contents.

Public body commitments distinguish KeyPackage wire, policy, public plan, sealed
transfer and Welcome with different domain tags. Each hashes its tag, u64
big-endian byte length and exact bytes, refusing empty or greater-than512KiB
input first. This bound does not supply network, SDK or physical-memory payment.
The separate body-check helpers compare actual bytes to signed commitments;
they parse/decrypt no package, policy, route or Welcome. Expiry is an
admission-time observation; an exact completed Welcome may arrive after expiry.
The App's [initial founder packet](CHAT-V2-INVITE-ISSUER.md#invitation-packet)
uses these checks and separately parses the bound policy and reservation.

The first11 focused cases passed with real signatures. Review found a masked
cross-root negative test; its new table coherently re-signs invite/request/
Welcome while retaining the original Profile/Add/Header, and independently
tests anchor group/founder/Profile/tenure. All12 new cases passed in the full
727-case replication unit suite (389.91s). All52 integration cases passed;
the single named ten-thousand-operation cost case remains user-deferred. The
unfiltered command initially started that case; it was stopped and the
remaining integration targets were rerun with its exact exclusion. Doc tests
completed with zero cases. Tests also
cover every-byte substitution/truncation, future versions, trailers, bounds,
signature variants sharing IDs, subject/KP/credential/admitter swaps, founder
Admission substitution, changed transfers and a real Legacy-decoder positive
control followed by refusal of the V2 bytes. This is not an MLS join test.

Final strict replication Clippy passed (all targets/features). Complete current
Root/native validation remains pending. Explicit native check
passed (1m38s). Root strict Clippy retains the same564 inherited errors. Both
dependency audits passed. All3480 frontend tests passed after this Core change
(73.23s); ambient/raw-fs/format gates passed.
Read-only design and diff reviews have no remaining B/H/M/L after corrections.
