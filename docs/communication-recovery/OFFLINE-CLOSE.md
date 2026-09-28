# Independent close with unfinished member verification

Orderly close saves current local state without initiating member-finalization network requests.
It captures already-proven outbound listener evidence, saves the authenticated policy and retry
correlations, then uses the existing actor shutdown barrier to checkpoint accepted history and
freeze the actor. An unavailable peer leaves saved retry work. Local snapshot/network-record write
failures, a missing mount or a changed server incarnation still refuse close.

The regular capture worker retains its bounded proof attempts and fair selection. Its typed result
separates `Saved` (including a pending-observed-peer count) from local storage errors. Correlations
without a fresh observation remain stored but are not included in that count. `Saved` is not a claim
of present connectivity or delivery. Initial admission and unlock disclose when a group with other
members has no saved outgoing route: recovery may depend on another member connecting inbound or
on discovery. An empty new founder does not receive that warning.

The common admission helper already performs the connected, signed two-way finalization before
PEX. A successful reply admission therefore supplies both endpoint proofs while the peer is
available. The existing real-TCP test now explicitly asserts those proofs before the unchanged
immediate close/restart route checks, in both restart orders. No extra finalization pass was added
to make that fixture pass. If the peer leaves before verification, close preserves the obligation
and discloses the missing outgoing route; it cannot manufacture a usable route.

Signed member descriptors already survive in the sealed group snapshot. An optional versioned
tail now also retains descriptor-missing, authoritatively admitted device/transport correlations.
It contains no listener addresses, connection state or live endpoint proof. Version 1 permits at
most 512 entries, each two length-prefixed 32-byte identities, for at most 36,869 inner bytes.
The decoder checks the version, size, count, canonical device order, unique peers and complete
frame. Restore applies current P2P permission, MLS membership and descriptor-conflict pruning
through the existing admission-candidate checks. A restored candidate only becomes eligible for
connected-only verification after a future actual outbound observation; it cannot cause dialing.

Snapshots without the new tail remain readable. Malformed or unknown tails fail closed. Older
readers that require end-of-input after the durable-chat frame reject new snapshots, even when
the new tail has zero entries. Once a new client writes the snapshot, an old client cannot reopen
it. Rollback compatibility is not promised. The network-record format and route bounds are unchanged.

Regressions added for core snapshot compatibility, malformed tails and authority preservation,
and for real native `freeze_servers` after an admitted peer disappears. The native case records an
actual outbound TCP/Noise observation, durably accepts a chat through the native send barrier,
refuses unmounted-store and actual checkpoint-replacement failures, then closes and opens a fresh vault. Exact history and the pending
correlation must survive with no promoted listener. The UI test executes the production restore
function to verify the disclosure. Rust execution is recorded by the integrating agent; these
tests are not a six-client or physical-NAT acceptance claim.
