# Reply-assisted member reconnect

This feature retains a successful reply callback as a restart route after authenticated P2P
admission. If A can dial B's listener but B cannot dial A, A retains that outbound direction.
B's inbound source port is never treated as a listener. Both members retain the authenticated
group policy and signed member descriptors; no private address is added to PEX.

## Implementation boundaries

- [`member_finalization.rs`](../../crates/catcoms-sync/src/member_finalization.rs) implements the
  additive connected-only exchange, current-member endpoint proofs and pending admission
  candidates. Existing request authentication, descriptor validation and roster checks apply.
- [`member_reconnect.rs`](../../apps/desktop/src-tauri/src/member_reconnect.rs) completes available
  proofs, saves the exact actor incarnation's core snapshot, then saves its local network hints.
  The native bridge invokes capture after registration, on existing network/event wakeups and
  on its discovery cadence. Close runs the capture pre-pass before freezing actors.
- [`store.rs`](../../crates/catcoms-app/src/store.rs) writes network record version 6, retaining
  the version-5 `MemberMesh` provenance tag and adding separate mesh-retention bounds. This preserves the existing transport seed, listener port,
  sequence reservation and bounded last-good routes.
- The actor exposes permission and candidate queries. Candidate records are separate from
  operational proof. The sync dial boundary still checks the current roster, descriptor,
  local MLS active state and shared endpoint scheduler.

## Authority and protocol

Request kind 26 carries one existing self-signed public descriptor. Its signed transcript binds
the current group, epoch, policy digest, both actual transport peers and request nonce. The signed
response additionally binds the exact request-body hash. A descriptor must identify the signing
current member and the actual endpoint. No new field accepts a remote private dial address.

The frame cap is 4,096 bytes. The existing descriptor shape allows eight addresses of at most
256 bytes each and an Ed25519 public key/signature. Its maximum encoded shape is 2,232 bytes;
the corresponding authenticated request is 2,490 bytes and response 2,485 bytes. The encoder
regression pins these values without truncating signed descriptors.

Each outgoing exchange has a two-second injected-clock deadline. Serving is limited to once per
second per current device, with at most 512 rate entries. Native capture tries at most two selected
outbound route targets, twice each. Opposite owners stagger outside their actor loops so the waiting
owner can serve inbound requests; after a collision, one side leaves a full request-deadline window
before retrying. The enclosing close deadline still bounds the complete pre-pass.

Pending admission candidates are capped at 512 and keyed by exact device and transport peer. They
come from an accepted direct MLS admission, a helper that verified the new Add, or the joiner's
verified Welcome from its named inviter. They grant no proof or dial authority. The first pending
endpoint is retained across cached-Welcome retries, and current signed descriptors supersede it.
Removal, a contradictory descriptor, self-identity and conflicting existing claims invalidate it.
Unknown Noise endpoints and infrastructure are not mandatory close obligations.

Authenticated P2P mode is necessary but insufficient: the local MLS instance must still be active
and its device must remain in the roster. The immutable informational mode may remain P2P after
local removal. Finalization, serving and local-hint use then fail closed. Missing policy remains
`LegacyUnverified`; this feature does not turn old disabled records into P2P consent. Dedicated
mode remains unsupported by the policy feature and gains no member-mesh behavior here.

## Persistence and close

The core snapshot must save successfully before `MemberMesh` authority is written to the network
record. Every native write rechecks the exact registry incarnation. A restored `MemberMesh` hint
is installed only when the restored core snapshot independently permits active P2P membership.
This is an ordering barrier between individual files, not an atomic multi-file transaction.

Only the local transport's successful outbound Noise listener evidence can create a new private
hint. At most eight members with two routes each are sealed, within an 8 KiB encoded-route
budget. Legacy single-contact authority remains capped at two routes for that contact. Empty
observations retain prior last-good routes. An unfinished
identified member target keeps close pending after other successful persistence work is preserved.
An already-sealed route can cover that target only while a unique current roster descriptor still
claims it and the saved route remains retained. Unknown transport peers cannot hold close pending.

The admission-candidate map is session-local. The durable retry permission is the core P2P policy
plus the sealed network record; a finished hint requires the signed descriptor and local direction
evidence. Ordinary close defers while an observed admitted route still lacks that proof. A process
crash before the barrier finishes does not imply restart-safe reachability or a save acknowledgement.
The feature cannot reconnect two peers when neither has a usable permitted listener direction.

Version 6 decodes v1-v5 records under their existing provenance and original route-count bounds.
It does not reinterpret an empty old route list as continuing permission. All newly saved network
records use v6, including disabled and legacy records. Older binaries cannot decode these v6
records: downgrading is unsupported without restoring a compatible backup. A decode/read failure
in the current loader fails closed; it does not become a missing record or regenerate identity. Public member discovery and canonical signed retained history remain separate from
these local private hints; no custom history-routing envelope or dedicated fallback is introduced.

## Review corrections

Adversarial review identified and corrected local-removal permission, symmetric sole-owner waits,
replacement at a full rate-map capacity, snapshot-before-network ordering, unfinished proof being
reported as complete, and unknown infrastructure being mistaken for mandatory member work.
The candidate map and cached-route exception remain bounded and roster checked. Existing helpers
and a code-holder's temporary reply record alone cannot grant continuing member authority.

## Integrated verification

At integration code checkpoint `45ac997`, the complete native suite passed: 285 tests,
including all six member-reconnect regressions and seven admission-storage regressions.
The real TCP callback case passed in both restart orders, with A still unable to listen.
The five core member-finalization tests passed in the 295-test sync run before the final
equivalent lint cleanup; strict core Clippy and native callback tests passed after that cleanup.
The network v5 fixture passed with one selected test.

Raw outputs: `logs/cr-final-native-tests.log`, `logs/cr-final-sync-tests.log`,
`logs/cr-member-net-store-final.log`, and `logs/cr-final-core-clippy.log`.
Reproduction commands (add the recorded locked/offline/serial settings for this machine):

```text
cargo test -p catcoms-sync member_finalization::tests
cargo test -p catcoms-app member_mesh_net_v6
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib member_reconnect_regressions -- --test-threads=1
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib reply_admission_fetches_signed_records_before_its_first_snapshot
```

The regressions cover signed endpoint/policy substitution, snapshot restore, actual local removal,
maximum frame sizes, pending-candidate retirement, failed-save/incarnation barriers, unknown peer
exclusion, unfinished admission proof, prior durable-route coverage and concurrent actor waits.
The TCP reproduction gives A no listener, admits B through A's outbound callback, closes both,
reopens separate vaults and transports in both start orders, and retrieves signed history retained
while B was offline. Its assertions reject B acquiring an inbound ephemeral listener hint.

The initial native fixture run had two failures: it tried to overwrite a prior-route fixture
through the admission writer, which now preserves existing route metadata, and it assumed
history was available immediately after an asynchronous catch-up request. The corrected
fixtures verify the sealed predecessor directly and advance injected retry deadlines while
real TCP actors recover history. All authority and direction assertions remain; no history
is injected. The final complete native suite passes. This is loopback TCP and independently
reopened stores, not physical NAT or OS process-kill acceptance.


## PR review R1: fair finalization work

The capture worker now computes completion over every currently admitted member with local
outbound Noise evidence. Its work budget is separate: two unfinalized members per pass, after
excluding current proofs. One actor-owned peer cursor rotates failed work and is advanced before
network awaits; cancellation cannot leave the same first two peers at the front. The cursor is
session-local and grants no connection authority. Queued canceled commands are skipped and an
executing finalization stops waiting when its native caller is canceled. Each exchange retains its
existing two-second core deadline and exact policy, epoch and endpoint checks.

The observed-peer command is bounded to the transport ledger's maximum of 640 observations. The
native capture loop checks the exact registry incarnation before work, and snapshot/net writes
retain their existing final checks. Watch wakes coalesce; ordinary discovery remains the quiet
retry cadence. Unselected eligible members keep close pending, except for the existing exemption
for a previously sealed direction whose current unique roster descriptor is still present.

Required focused verification (execution belongs to the integration owner):

```text
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib capture_worker_ -- --test-threads=1
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib member_reconnect_regressions -- --test-threads=1
```

The two new regressions execute the production watch worker using real outbound TCP Noise route
evidence and same-identity signed actor peers on the deterministic transport. They cover two
already-finalized predecessors and two failing predecessors, then successful service from the
third member. They do not claim that finalization fairness alone fixes route retention; that is
an independent bound addressed by R2.


## PR review R2: independent peer diversity and dial work

The shared policy in `catcoms-discovery::reconnect` bounds retained state at eight peers, two routes
per peer, 512 bytes per address and 8,192 encoded bytes in total. The total includes the one-byte
route count and, per route, the 32-byte peer id, both four-byte length prefixes and address bytes.
Selection scans at most the 640-entry transport evidence ledger plus 16 previously retained rows.
These are retention limits, not permission to attempt all retained addresses at once.

The native bridge ranks actual successful outbound observations newest first, then keeps
still-current, uniquely claimed old hints. One route for each retained member precedes second
transports. Refreshing B cannot remove C merely because B has TCP and QUIC. If more than eight
members have evidence, the most recent proven overlaps win; remaining peers keep the prior sealed
order. This is bounded recovery diversity, not a promise to retain every historical group member.
Private routes stay local and sealed. Inbound source ports and unverified claims never enter the
selection.

The runtime installer uses the same bounds and address validation. Each discovery pass considers
at most two eligible peers, with one actor-owned cursor advanced before scheduler/transport awaits.
A denied or failed first pair cannot monopolize later passes. Each peer's batch remains at most two
addresses, and all attempts still consume the existing process, group, peer, endpoint and prefix
gates. Repeated native hint installation does not reset the cursor.

The v6 decoder rejects peer-count, per-peer route-count, per-address length and aggregate encoded
byte overflow. The encoder normalizes arbitrary local route vectors with the same diversity policy,
so its output stays readable. v1-v5 parsing remains restricted to its original count and authority;
v5 hints migrate without changing seed, port, sequence, pending recovery or policy.

Required focused verification, not yet claimed executed for R2:

```text
cargo test -p catcoms-discovery reconnect_retention
cargo test -p catcoms-app member_mesh_net_v6
cargo test -p catcoms-app server_net_reconnect_routes_are_backward_compatible_and_bounded
cargo test -p catcoms-sync mesh_reconnect_retains_three_peers_and_rotates_two_peer_dial_passes
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib mesh_restart_keeps_private_alternative -- --test-threads=1
```

The native regression captures independently successful outbound TCP and QUIC connections to B,
retains C's private listener, then refreshes B through production persistence. B stops. A reopens its
vault and runs production cold restore without rendezvous, relay or advertised public addresses;
A must connect to C over the retained private direction and recover C's original signed message
identity, author and text through real TCP actors. Initial signed admission uses a deterministic
Hub with the same identities; no member proof, route authority or remote history is injected.
This is loopback integration coverage, not physical NAT testing. Transfer amplification for a large
archive is measured separately by the R4 reconciliation regression.
