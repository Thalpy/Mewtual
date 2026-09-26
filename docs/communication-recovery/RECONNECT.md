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
- [`store.rs`](../../crates/catcoms-app/src/store.rs) adds network record version 5 and the
  `MemberMesh` provenance tag. This preserves the existing transport seed, listener port,
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
hint. At most two routes are sealed. Empty observations retain prior last-good routes. An unfinished
identified member target keeps close pending after other successful persistence work is preserved.
An already-sealed route can cover that target only while a unique current roster descriptor still
claims it and the saved route remains retained. Unknown transport peers cannot hold close pending.

The admission-candidate map is session-local. The durable retry permission is the core P2P policy
plus the sealed network record; a finished hint requires the signed descriptor and local direction
evidence. Ordinary close defers while an observed admitted route still lacks that proof. A process
crash before the barrier finishes does not imply restart-safe reachability or a save acknowledgement.
The feature cannot reconnect two peers when neither has a usable permitted listener direction.

Version 5 decodes older v1-v4 records under their existing restrictive provenance. It does not
reinterpret an empty old route list as continuing permission. A v4 decoder cannot interpret the
new tag. Public member discovery and canonical signed retained history remain separate from
these local private hints; no custom history-routing envelope or dedicated fallback is introduced.

## Review corrections

Adversarial review identified and corrected local-removal permission, symmetric sole-owner waits,
replacement at a full rate-map capacity, snapshot-before-network ordering, unfinished proof being
reported as complete, and unknown infrastructure being mistaken for mandatory member work.
The candidate map and cached-route exception remain bounded and roster checked. Existing helpers
and a code-holder's temporary reply record alone cannot grant continuing member authority.

## Required integrated verification

These commands are verification requirements, not a claim that this document's revision has run
them. Final native TCP verification is pending root integration.

```text
cargo test -p catcoms-sync member_finalization::tests
cargo test -p catcoms-app member_mesh_net_v5
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib member_reconnect_regressions -- --test-threads=1
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib reply_admission_fetches_signed_records_before_its_first_snapshot
```

The regressions cover signed endpoint/policy substitution, snapshot restore, actual local removal,
maximum frame sizes, pending-candidate retirement, failed-save/incarnation barriers, unknown peer
exclusion, unfinished admission proof, prior durable-route coverage and concurrent actor waits.
The TCP reproduction gives A no listener, admits B through A's outbound callback, closes both,
reopens separate vaults and transports in both start orders, and retrieves signed history retained
while B was offline. Its assertions reject B acquiring an inbound ephemeral listener hint.
