# Network security dependency migration

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
