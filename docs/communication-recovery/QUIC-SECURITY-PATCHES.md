# Compatible QUIC security patches

The Root and native workspaces retain libp2p0.56 and their existing Quinn
versions (Root0.11.9, native0.11.11). Root moves libp2p-quic0.13.0 to0.13.1;
native already used0.13.1. Both move quinn-proto0.11.18 to0.11.19 and
quinn-udp0.5.14 to0.5.16. The committed locks are checked with `--locked`.
No unused direct dependency or advisory exemption is added.

These patches address certificate-expiry panic, excess implicit streams,
oversized Retry tokens and a truncated-datagram receive loop. These lower
transport boundaries precede application membership validation. The UDP
advisory excludes Windows/Linux receive paths; the compatible patch is retained
for other supported platforms. Sources: [QUIC fix](https://github.com/libp2p/rust-libp2p/commit/212f3774af048e2cecfb2e6b1e08477685e52b22),
[stream limits](https://rustsec.org/advisories/RUSTSEC-2026-0523.html),
[Retry handling](https://rustsec.org/advisories/RUSTSEC-2026-0524.html),
[UDP receive loop](https://rustsec.org/advisories/RUSTSEC-2026-0538.html).

Root's feature graph gains default bloom/log features through the new direct
quinn-proto edge in libp2p-quic0.13.1. Cargo adds fastbloom0.17.0, libm0.2.16
and siphasher1.0.4. Native already had this feature closure. Both retain Tokio
and rustls-ring. Unrelated package blocks and dependency edges are preserved;
only orphaned old Windows bindings are removed. Actual production feature
graphs are captured before/after under catcoms-net and mewtual-desktop.

The new test uses the actual production transport with fixed test identities,
router mapping disabled and a UDP-only listener. It requires authenticated
QuicV1 connection snapshots, exact peers/protocol/payloads in both request
directions, then withdrawal after endpoint loss and refusal of a stale
connected-only request. It is functional compatibility evidence, not upstream
attack reproduction, physical-network acceptance or joined actor drainage.

Focused execution and full-suite results are recorded in HANDOVER. The thirteen
Hickory/PRUNE advisories require a separate libp2p migration; this patch cannot
close the dependency gate or enable Chat V2.
