# Control7 Query/Support operational slice

29 October 2026. Focused implementation, OFF. Twelve focused tests PASS at snapshot
`34336b2f` (7.96s, Windows, `cargo test -p catcoms-sync --all-features --lib
admitted_control -j1 -- --test-threads=1 --nocapture`), including actual restricted
B→D replacement and MLS epoch/rejoin refusal. They also passed atc5c8b7d8; the
first10 passed atf60d6bab. Early actual-path review corrected three submissions
and the final original-helper proof. The separate full Group accounting correction
below now passes focused checks and final read-only review with no blocker/high/
medium. Strict lint and complete integration remain primary-owned pending gates.

First compilation found adapter visibility/type/test-trait issues; the next found
one missing Result annotation. After those corrections,4 tests passed and6 failed
at fixture setup because the restricted transport omitted `request_connected`.
Delegating only permitted edges to the actual Mem driver produced the10-test
green run. The added four-node fixture first needed a member-count type correction,
then all12 passed. No rejection/positive assertion or deadline was weakened.

Measured new graphs are record6992/8192, table1344/4096, ledger3888/8192,
wrapper1096/4096, actual execution future224, prepared88, shared completion272 and
scheduler inline104. The original enclosing Group304, ExistingHelper216,
ExistingSource184, BTree tuple40 and Resources Counts128/bucket tuple160/service24
exposed an inherited1024-byte fixed-fee gap even without Control7's fields.
Opaque paid owners now retain that SAME outside Group sponsor across escaped
ledgers and actual body/header return; all three counter-removal paths retire
real sparse/empty backing under the pool lock. The complete source-corrected
H8 invoice measures3750 and prepays4096, without changing caps. Revised Group224
and BTree tuple48 reflect those new owners. Runtime42, Resources60, helper26,
source32, rt authority11 and route9 focused checks pass; actual Root transport69
passes at4b9d2241. These filters overlap; their counts are not a unique total.
[The exact accounting proof, red evidence and limits](VOICE-GROUP-LEDGER-CUSTODY.md)
remain separate from the Query protocol's positive execution evidence.

The production kind28 dispatcher selects a separate Control7 signed/envelope
grammar after authenticating its current connected requester. Existing Body,
SdpProfile, Source activity and packet factories remain closed. An unjoined
current member may answer protocol Query and forward valid targeted Control7
one hop under current group policy. It owns no call/key/watcher/Availability
promise. Support advertises only implemented forwarding bit0 (mask1); encrypted8
and Ready9 remain refused, and no other Control7 operation has a local consumer.
The existing local VoiceRouting switch remains OFF by default.

Trusted Root APIs separately ensure/refresh/observe endpoint support and direct
helper support. Endpoint selection uses the SAME original admitted Presence's
Watcher FreshPeer and immutable local join/key. A direct-helper probe uses an
actual connected bound route, with a distinct handle/observation type. The wire
query has no helper endpoint tuple. Discovery tags1–3/FreshPeer alone never prove
support. One current protocol query per peer is stored under the SAME residence.
Refresh prepays/signs q2 first, then atomically closes q1 before q2 can publish;
later Source capacity refusal keeps q2's original bytes, ordinal and deadline.
Ensure/lost-reply retry coalesces; missing or late replies create nothing.

Every query combines its immutable signed15s wall expiry with a checked original
15s monotonic deadline. Every Source or helper original has THREE submissions,
500ms minimum retry separation, and preserves attempts through refusal, loss and
actual Lower destruction. Helpers rotate through current authenticated direct
support observations rather than permanently choosing the first two. A Lower
retains the exact selected helper query/Support proof; q2/End/epoch/reconnect
invalidates that original. Target/helper lookup scans borrowed bounded current
membership/descriptor data and captures actual connection incarnations; it never
dials or materializes a new complete route roster.

Queries, responses and opaque forwarding records each prepay a FRESH affine8KiB
group-bound credit. Every alias retains that SAME payer outside the record Arc
through body/header deallocation, including superseded external handles. Current
query table32 is distinct from lifetime occupancy: unchanged512KiB group/2MiB
process caps imply at most64/256 actual original records across all rooms/Roots,
with less usable capacity after infrastructure/other work. Production const
guards pin those ratios; reducing the fee or raising caps requires another affine
count mechanism. Separate paid fixed backing bounds Source32/helper64/cache32.
Exact duplicates retain one original signed response, payer and remaining budget.
Shared Forward4, routes64, metadata512 and completion backlog8 remain aggregate
across both families. Paid Control completion indirection preserves the old inline
variant footprint. New-family and complete enclosing Group/Resources graph guards
pass focused execution. The new Group infrastructure fee is4KiB even while OFF;
the separate first-Root route-registry fee is another4KiB within the same caps.

Authenticated Control service, including duplicates, additionally consumes shared
process burst64/refill8 and group burst16/refill2 per250ms; periods preserve
remainder and backward time cannot refill. Existing outer verification/requester
and original-issuer Setup budgets remain independent. Final native origin debit
uses an already-prepaid row, with no pruning/allocation after final owner checks.
Membership, exact signed epoch/policy/room, original request binding, call consent
and actual connected route gates are repeated before positive publication/send.
The final request-authority callback has the same trusted-adapter contract as the
existing native ingress seam: actual native/Mem driver lifetime observers are pure.
This does not claim deadlock safety for arbitrary reentrant custom implementations
of the transport RequestAuthority trait.

Focused regressions drive real run_once/kind28 dispatch over an in-memory transport
that forbids A–C requests and exposes no A–C connection lease, with B joining
nothing. They also pin q2 under Forward pressure/late q1, exact duplicate cache,
malicious flood bounds, wrong room/old grammar/expiry, retained superseded-owner
pressure/actual return, three original submissions, and discrete service budgets.
The replacement case forces D below B in identity order, holds B's actual four
Forward credits, and requires the SAME signed Query to traverse A→D→C and produce
authenticated endpoint Support. The old wrap algorithm could repeat B ahead of D;
the corrected cursor considers other helpers before last-used fallback. A real
MLS Add closes old query/Lower before a new native call can issue a new query;
a current authenticated outer wrapper cannot repair the old inner epoch/query.
The intake-oracle negative cases are structural enforcement checks; the topology
case is the actual production dispatch path. TCP integration remains primary-owned.

The strong registry remains an actual paid owner after the last external alias
returns. Ordinary Actor housekeeping, ingress and preparation prune sole rows
before reserving shared lanes. Actual driver/byte/preparation owners still prevent
refund during that production collection. A canceled upper may return its Gate
authority while a responder keeps its independent Lower payer; collecting that
sole row cannot return the responder's loan. Root transport tests now exercise
both cases and retain exact original zero/refund/attempt oracles.

This slice is not complete V2. Availability, bilateral Request/Grant/Commit,
neutral6/6 consumption, immutable RoundGate/native parent consumption, operational
agreement/Ready/SDP/ICE/RTC/capture/audio and activation rollout remain unfinished.
