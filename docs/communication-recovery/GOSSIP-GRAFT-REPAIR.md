# Unsolicited gossip GRAFT repair (13 November)

This is a compatible **GRAFT-only** repair. It does not complete gossip resource
security, resolve Hickory or enable Chat V2. The inherited PRUNE backoff-growth
issue remains HIGH and requires the separately reviewed migration below.

The locked0.49.4 source inserted every GRAFT topic into a connected peer's retained
subscription set, even when no subscription or local mesh existed. Transport
authentication and application-level signed chat do not prevent that lower-level
state growth. The [upstream advisory](https://github.com/libp2p/rust-libp2p/security/advisories/GHSA-g3g5-x568-qvqx)
has inconsistent affected-version metadata; its description and the actual source
are what motivated the regression. Audit silence is not evidence of immunity.

Both locks select libp2p-gossipsub0.49.5. Net's production import uses its direct
compatible minimum, preventing a later ordinary lock refresh from choosing an
older allowed0.49 release. This edge must move with libp2p during the0.57 upgrade,
not retain an unused older dependency island. Identity/core/transport versions,
packet caps, native authorization and timeout settings are unchanged.

The release ALSO changes its default incoming-subscription filter from unlimited
to2000 topics per peer and2000 entries per request. This protection is retained.
It is a count bound, not a complete process/global byte-budget claim; inputs past
that support bound are not credited as admitted. The existing transport subscribe
completion still means queued work, not remote wire receipt.

## Actual regressions

The first test negotiates Noise/TCP against the genuine product receiver. A valid
SUB receipt and256 unsolicited GRAFTs share one bounded RPC; the resulting real
`Subscribed` event is observed after synchronous processing, then retained topics
are inspected. It fails on0.49.4 with257 topics instead of1 and passes on0.49.5.
The initial handcrafted packet mistakenly used outer field6; protobuf ignored it,
so the valid SUB alone made that initial attempt pass. Correcting `RPC.control`
to field3 produced the actual pre-fix failure. That false precursor is not credited.

Other real TCP cases cover an oversized2001-entry request followed by an ordered
receipt, the2000-topic total bound, duplicate admission, overflow refusal and
capacity returning after unsubscribe. Later markers on the same stream establish
that preceding rejected frames were actually encountered. A separate test uses
TWO production `MeshService` peers and requires delivery of the actual payload at
100,101 and2000 topics; local subscribe success is insufficient.

All four cases pass locally on Windows in under a second after compilation.
Strict Net Clippy (all targets/features) and the complete186-test Net suite also
pass. Cross-platform exact-count CI and complete current-source root/native/
frontend validation remain distinct gates, recorded in HANDOVER/PR.
The shared result guard rejects zero tests, ignored substitutions, failures and
timeouts. No preexisting test or deadline is loosened.

## Still open

[PRUNE backoff growth](https://github.com/libp2p/rust-libp2p/security/advisories/GHSA-vrx6-jcp8-vm7m)
affects0.49.5: unknown-topic PRUNE can create backoff state that survives disconnect.
GRAFT passing is not a proof of PRUNE cleanup or complete bounded gossip state.
Nine Hickory findings also remain. Moving to released libp2p0.57.1 with gossipsub
>=0.50.1/Hickory0.26.3 needs old identity/address/envelope goldens, sanctioned RNG
and request-response API adapters, explicitly supported bounded topic policy,
mixed-version transport evidence and full gates. No major migration or clean-audit
claim is supplied here. Native Chat V2 ownership/installation/cold/joined/governance
work remains separate and unfinished.
