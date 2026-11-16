# Historical KeyPackage labels for retained owner outcomes

The source provides a narrow prerequisite for the inactive covering owner Add. It does not yet
implement that transaction or enable Chat V2.

`MlsDevice::key_package_identity` extracts full signer, SDK KeyPackage reference and the existing
credential-type/TLS digest from an existing `KeyPackage`. Actual `add_member_observed` and own-leaf
credential comparison share the same bounded digest implementation. Their original domain and
bytes are unchanged. The data type cannot manufacture a `LocalMembershipChange`, fresh join or
accepted membership operation.

`MlsDevice::retained_key_package_identity` returns the same **plain, unauthenticated labels** from
an original canonical wire package without checking its signatures or lifetime. It must never
admit a new package. Its intended historical consumer must independently verify the pinned owner's
original signed accepted reference, full subject, typed credential, exact retained request, Commit,
Welcome and fact chain before trusting those labels. No such consumer is claimed by this slice.

This distinction matters because the actual locked OpenMLS **0.9.2** `KeyPackageIn::validate`
calls `Lifetime::validate`, which reads `SystemTime::now`. The ordinary identification parser
therefore also checks current validity. Reusing it to restore an accepted result can strand the
original response after expiry. It remains unchanged, as does every new-admission policy check.
OpenMLS 0.8.1 is only the older compatibility-fixture writer, not the current runtime version.

The historical extractor checks the <=32 KiB frame before SDK parsing; requires fixed MLS 1.0,
KeyPackage wire format and the pinned suite; fully consumes one SDK message; and compares the
canonical KeyPackage **body** with the original. It uses the SDK's own `make_key_package_ref` on
that body, including the KP signature, under the same device crypto provider. The outer message
header is not part of the reference. A changed signature can still yield data, but changes the
reference and therefore cannot match the original independently verified owner certificate.

The existing 4 KiB credential-content cap precedes credential TLS encoding. SDK parsing and
`unverified_credential` can first allocate or clone input-derived data; canonical body and SDK
RefHash also make bounded copies. These must enter the transaction's actual simultaneous invoice.
The helper makes no whole-SDK-heap, active-host-quota or before-every-clone claim.

Five regressions are authored and **unexecuted** while compilation/tests/builds are paused: actual
Add/selected-member/restore identity agreement; already-expired signed data with ordinary parser
refusal; changed signature/reference; wrong-header/size refusal before SDK decode plus trailing
refusal; and credential cap refusal before TLS encoding. A genuinely expired-after-success retained
outcome/reply-loss/cold-reopen fixture belongs to the still-open covering consumer acceptance; a
ManualClock advanced into the future cannot simulate the SDK's wall-clock check.
