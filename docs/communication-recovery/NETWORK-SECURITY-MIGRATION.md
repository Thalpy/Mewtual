# Network security dependency migration

Latest recorded wire evidence: both explicit contracts pass on Linux and
Windows in CI43561112497 at66b9a2f3. Earlier full4935 native/checks pass on both
platforms and Windows Root passes; Linux Root reached its outer120-minute bound
while tests progressed. Later native factory changes require their own full
validation. These are scoped results, not complete Chat V2 acceptance.

Status: implementation under validation. Chat V2 remains disabled. This targets
the remaining dependency obligations; it does not supply the missing native or
joined source.

Both workspaces resolve libp2p 0.57.1, actual production gossipsub 0.50.1 and
Hickory proto/net/resolver 0.26.3. All five direct libp2p edges move together,
including native and test consumers; an unused repaired package beside an active
old dependency would not repair the runtime. The two obsolete Hickory advisory
exceptions are removed. No new exception or DNS/WebSocket feature removal is used.
[Published dependencies](https://docs.rs/crate/libp2p/0.57.1/source/Cargo.toml),
[PRUNE advisory](https://rustsec.org/advisories/RUSTSEC-2026-0534.html).

## Compatibility choices

The repaired gossipsub default is 100 subscriptions per peer/request. Mewtual's
existing accepted limit is 2000, with real 100/101/2000-topic delivery tests.
Construction explicitly retains both finite 2000 limits using the upstream
`MaxCountSubscriptionFilter`. Both the protobuf-payload limit and aggregate
SUB/control limit are explicitly 65,536 bytes. The aggregate limit is raised
from the new 16,384-byte default to preserve the accepted subscription inventory;
it is still finite. The local upstream correction partitions the initial hello
within both limits; raising the packet cap would conceal the compatibility defect.
An unlimited filter would undermine the boundedness correction. Existing tests
also require rejection above the bound, duplicate acceptance and progress after
unsubscribe. GRAFT inspection does not independently prove PRUNE boundedness.
[Upstream changes](https://docs.rs/crate/libp2p-gossipsub/0.50.1/source/CHANGELOG.md).

AutoNAT now forks an injected `SeedableRng`. A zero-state adapter that ignores
the supplied seed, or cloning the same stream, cannot satisfy that behavior.
The new RT `SeededProtocolRng` wraps pinned ChaCha20 0.10 and bridges only this
upstream rand-core 0.10 API. The ordinary application rand-core 0.6 seam is
unchanged. Production obtains one 32-byte seed through `OsCryptoRng`; supplied
test seeds remain deterministic. There is no ambient Default, Clone, Copy or
state-bearing Debug. The temporary seed is zeroizing; this is not a claim that
all upstream TLS/RNG state is wiped or deterministically injected.
[Actual fork](https://docs.rs/crate/libp2p-autonat/0.16.0/source/src/v2/server/behaviour.rs),
[RNG contract](https://docs.rs/rand_core/0.10.1/rand_core/).

UPnP struct events project the same external address into the existing global
UPnP ownership key; the new local field does not silently change expiry semantics.
Only the three request-response Codec implementations stop using `async_trait`,
matching the new native async/RPIT contract. Other async traits retain it. The
boxed DNS-over-TCP/WebSocket/QUIC composition, Noise authentication, connection
limits and dial policy are preserved. Smart dialing is not enabled.
[Codec contract](https://docs.rs/crate/libp2p-request-response/0.30.0/source/src/codec.rs).

## TLS provider boundary

Published QUIC 0.14 and libp2p TLS 0.7 require AWS-LC; their supported features
offer no ring-only selection. Their actual client/server configuration, verifier
and certificate signing paths choose that provider explicitly. WebSocket 0.46
chooses ring explicitly. This migration therefore adds the AWS-LC native build
and hybrid key exchange to QUIC while preserving ring WebSocket TLS.
[QUIC dependencies](https://docs.rs/crate/libp2p-quic/0.14.0/source/Cargo.toml),
[Peer TLS configuration](https://docs.rs/crate/libp2p-tls/0.7.0/source/src/lib.rs),
[WebSocket TLS](https://docs.rs/crate/libp2p-websocket/0.46.0/source/src/tls.rs).

Net installs no process-global provider. The native updater's existing ring
initializer remains separate. Public web-PKI validation for WebSocket must never
be replaced with libp2p's custom peer-certificate verifier. Relay/rendezvous
infrastructure constructors still use metered Noise/TCP and ring/WebSocket;
their constructors do not add a QUIC listener. The ordinary mesh retains QUIC.
Hybrid handshakes add work and memory: the AutoNAT RNG adapter neither funds that
work nor establishes a complete process-memory bound.

## Verification status

All three new RT regressions pass: exact original seed consumption, deterministic
streams and distinct parent-advancing forks. All 67 RT tests pass (0.01s).
All 174 affected Net tests pass
(0.93s), including the five gossip acceptance cases. The six direct vendor
regressions pass (0.01s); all nine vendor protocol tests pass (0.70s).
The complete 159-case vendor library suite also passes (8.80s). Both actual
shipping dependency/license audits pass: advisories, bans, licences and sources.
No new advisory exception was added. Root formatting, ambient-dependency and
store raw-filesystem gates pass.
New tests use isolated processes to construct both providers in both orders,
require exactly one child test to execute and observe no global installation.
Real WebSocket TLS tests use fixed 2020–2100 public test certificates and known
test seeds, checking successful exchange, DNS localhost, untrusted roots and
wrong names. Production roots are not modified. Real QUIC checks cover actual
bidirectional requests, withdrawal and rejection of a wrong expected peer.

Read-only design and actual-diff review found no remaining production blocker.
The source-integrity gate verifies all 46 files against the checksum-pinned
published archive, allows only the five documented original-file changes, and
checks that both shipping locks use the same corrected runtime. The dedicated
old-peer gate builds an unchanged public-API example against the pinned previous
stack, records source/lock/binary hashes and requires bidirectional actual
request/reply and near-60KiB signed gossip at 2,000 subscriptions. Bounded
republication establishes eventual transport receipt, not application signing or
one-shot delivery. It requires path withdrawal and an exact Unreachable result
after the old Mesh drops while its process remains alive.

The first overlapping-bulk probe failed in both old/current and a current/current
control before completing its observations. A subsequent current/current run
passed; no production cause has been established. The functional gate now starts
bulk only after the exact bidirectional RR observations. That phased old/current
run passes. The earlier failures remain a load/fairness acceptance gap, not an
excuse to increase a protocol timeout or claim a wire fix from test ordering.
Adversarial review also required a positive live-path barrier before controlled
Drop, explicit post-Drop withdrawal admission and unconditional cleanup of both
owned children. Their exact-source rerun passes, including PRESENT before Drop
and an old process still alive at the current peer's Unreachable observation.
The cleanup failure-injection self-test passes. Review and short re-review found
no remaining blocker, high or medium finding.

All3,441 frontend tests pass (118.37s), type checks report0 errors/warnings and
the production build passes (51.69s). Actual Windows native production checking
passes (7m05s). Linux native build, startup, strict lint and all full shipping
backend suites remain required. No physical-network, browser/audio or
final Chat V2 acceptance follows from these intermediate checks.

Root strict Clippy was run and fails with610 existing App errors. There is no
warning suppression, unused public export or advisory waiver in this checkpoint.
Strict Clippy for both changed RT/Net crates, including all targets/features,
passes (54.69s).
The native PC-close and media-refusal regressions also pass on the combined
stack(6.45s and8.42s). Actual Windows rebuilding(4m13s) and the real application
startup gate pass. Dedicated compatibility CI43547079658 passes on Linux and
Windows; full shipping run43547127334 remains in progress. No all-green claim
follows: strict App debt and unfinished native/joined/transport consumers remain.
The pinned predecessor is retained independently of PR branch history by
`compat/network-security-predecessor-944115c2`; the gate still verifies its full
commit SHA, not a movable tag.

## Subscription retry amplification follow-up (14 November)

The exact939ea602 Windows peer job43553131480 completed QUIC authentication and
bidirectional RR, then both endpoints timed out waiting for gossip. Its two
logs contain no target subscription/publication observations. This is an open
acceptance failure, not proof of a particular production cause or a flaky test.

A separate confirmed amplification existed in the current actor: EVERY incoming
Subscribed event synchronously retried the whole held-publication queue. Both
upstream versions sign a complete payload before checking subscriber presence.
An unrelated subscription burst could repeatedly sign up to 8 MiB on one actor.
The callback now extracts only the exact canonical announced topic. Unmatched
originals keep their order, Bytes ownership and byte charge. Malformed and
uppercase aliases select nothing. The independent periodic retry is unchanged.
No wire limits, deadlines, payload sizes or receipt assertions were reduced.

All 177 Net tests pass (0.99s), including three new queue regressions; strict Net
all-target/all-feature Clippy passes (11.62s). The new unchanged-source old-peer
gate also passes locally: both actual runtimes adopted 2,000 topics, observed the
target and received exact RR and near 60 KiB signed gossip. It still starts bulk
after RR without waiting for a diagnostic subscription flag. Existing 15s/45s
bounds are unchanged. This successful run does not establish the CI failure's
cause or sustained-load fairness. An example-only fixed scalar subscriber now
records adoption, post-sign attempts, successful publication, first-hold causes
and actual gossip receipt without emitting full per-event logs or payloads.
The predecessor's manifest/lock remain unchanged; its only addition remains
the identical example.

For the earlier `4935e25` runtime, full native tests/checks now pass on both OSes,
and Linux frontend 3,441/check/build pass. The `939ea602` member-storage workflow
43553131305 passes on both platforms after correcting genuine cohort counts.
Root full tests are still running; these results are not full sign-off for the
new retry correction. Native command and Chat V2 activation remain unfinished.

## Separate wire and startup-pressure contracts (14 November)

The next Windows run 43556397080 passed QUIC/RR but exposed the predecessor's
retry amplification directly: the old endpoint adopted 1,129 topics, had not
adopted the target, and recorded 53,783 signed attempts with no successful
publication. The current endpoint adopted 2,000 and published 563 payloads. The
old implementation cannot be retroactively corrected by this dependency update.
These observations and the synchronous whole-queue retry call path identify a
predecessor starvation problem; they do not prove every stalled receive detail.

The fixture now states two different contracts explicitly:

- Old/new wire compatibility waits for actual target-topic adoption before bulk
  publication. It still requires real bidirectional RR, exact signed near 60 KiB
  gossip, and live-process Mesh withdrawal. This is a deliberate scope change:
  it does not claim mixed-version startup resilience.
- Current/current pressure starts with no server subscriptions. The client must
  prove exactly 128 originals held for NoPeers, with target absent and exactly
  7,864,320 held bytes. It sends no replacement originals. A second actual RR
  must complete BEFORE the controller releases subscriptions. The server then
  admits 2,000 subscriptions while continuing to service traffic, and must receive
  a serial from those original 128 messages. Both original 15s exchange limits and
  one 45s controller deadline per pair remain unchanged.

The pressure client's signing oracle requires 256..=1,280 attempts: initial 128,
retained target retries, and the conservative allowance for every 2s timer pass
inside 15s. This both checks the oracle is active and rejects wholesale retry per
unrelated subscription even on a fast host. Hold observations publish their full
tuple before a Release count, which the pressure-client task consumes with Acquire; the Python controller reads its completed marker.
Both pressure processes are proven to use the same current binary/stack; their
proof is separate from the unchanged-source predecessor proof.

Both real scenarios pass locally, including the final pressure RR and signing
range. The pressure client records 256 signed attempts, 128 successful local
publications and 128 first holds. An actual pinned old/old negative control
reached the same held/RR barriers, then failed the signed-attempt-bound assertion;
its earlier helper did not print the actual count, and no number is claimed.
Strict Net all-target/all-feature lint and cleanup/script checks pass. All 177
Net unit tests remain passing; production code is unchanged since 77d34d08.
This proves bounded retry work and traffic progress in this fixture, not delivery
of every original, chat durability, another group's fairness or physical networks.

After receiving an original client message, the pressure server now sends an
additional exact RR acknowledgement. The client must handle and serve it through
its original Mesh actor, and the server must receive the reply. Both endpoints
require this causal barrier before completing the phase or sampling the signing
range. It is separate from the earlier pre-release pressure RR: receiving early
server gossip alone cannot prove the client's retry batch has returned. Both
scenarios pass after this final review correction, with unchanged deadlines.
