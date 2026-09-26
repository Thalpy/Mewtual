# Authenticated P2P policy feature

This feature adds an immutable communication-policy pin. New `Server::found` groups initialize
an owner-signed `PeerToPeer` policy before export. Raw `ChannelSync::new`, historical snapshots,
and v2 invitations remain `LegacyUnverified`; membership or a local device role does not establish
permission for newly added durable member-route capabilities.

The versioned policy binds its MLS group id, mode and optional dedicated-service identity. Its
digest excludes the endorsement envelope so the current MLS designated committer can re-endorse
the same body after succession. First adoption requires the actual current MLS owner. A policy
already accepted and saved inside the vault keeps its exact digest even if its issuer leaves.
Unsupported policy versions, modes and malformed present snapshot tails reject rather than
silently becoming legacy. `Dedicated` is reserved in the authenticated format but rejected by the
sync layer until its discovery and serving restrictions exist. This feature adds no dedicated
creation option or dedicated security contract.

Invites with a policy use a distinct v3 signature domain and bind the complete signed envelope;
policy-free invites preserve exact v2 encoding. The inviter-signed, encrypted routing transfer
carries a current-owner endorsement of the same policy digest. Join verifies both bindings against
the actual Welcome group. An initialized group rejects policy-free or conflicting invitations
before consuming their nonce or modifying membership. Companion admission carries the verified
policy in its owner-signed routing transfer as well.

The sealed synchronizer snapshot appends a length-prefixed policy frame after observed owner
tenure: `version:u8 = 1`, then a length-prefixed policy encoding, or empty bytes for unresolved
legacy state. Further extensions belong after this frame. The actor tracks a process-local policy
revision in `SnapshotNeeded`, including received endorsements that change no rendered document or
membership count.

The core legacy-migration API is explicit and owner-authorized; this feature exposes no migration
UI or native command. Its caller must call `initialize_group_policy`, save the resulting snapshot,
then call `publish_group_policy`. Until that last call the candidate pin is included in snapshots
but remains inactive for member-mesh permissions and admission. A bounded owner-signed control frame
propagates the immutable decision; native discovery passes retry publication only after this
post-save call or after restoring a sealed pin. Remote nodes independently verify the current
owner and reject replacement bodies. The retries keep at most one pending policy frame. They do
not claim guaranteed migration while the owner is unreachable. After migration, old invitations
must be replaced. Older applications that do not understand v3 cannot redeem the new invites.
Verified received policy may authorize in-memory P2P behavior, but native must save the exact core
policy snapshot before writing standing reconnect authority or reporting restart safety. A failed
recipient save followed by restart restores unresolved legacy behavior; network metadata alone
cannot recreate policy authority. Founding likewise must not report durable route permissions
merely because an in-memory founder has initialized P2P policy.

One admission limitation remains explicit. MLS assigns new members to vacant lower leaves, which
can make the joining member the new designated committer. The old committer cannot provide that
future owner's endorsement. Until the admission protocol carries an authoritative succession
proof, policy-bound invitations and companion admissions fail before MLS Add when the current
designated committer occupies a leaf above zero. Minting returns a specific authority-unavailable
error. Existing pinned members can continue to use and restore the group. A joiner's self-signature
over an offered body is not treated as proof that it matches the group's earlier immutable pin.

Focused regression coverage lives in `catcoms-mls/tests/group_policy.rs`,
`catcoms-sync/src/group_policy/tests.rs`, the app actor migration test and the existing product
found/invite/join conversation test. Execution results belong in the feature integration report.
