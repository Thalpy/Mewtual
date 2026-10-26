# Design: Welcome joins and expired KeyPackage leaves

Branch `mls-leaf-lifetime`, based on `chat-update-implementation` (openmls 0.8.1). It must also
hold on PR #57 (openmls 0.9.2).

## 1. The bug, verified against the openmls source

Every leaf that enters the tree through a KeyPackage carries that KeyPackage's `Lifetime`
(`LeafNodeSource::KeyPackage`). openmls's default is 84 days (`DEFAULT_KEY_PACKAGE_LIFETIME_SECONDS`,
same in 0.8.1 and 0.9.2), and `device.rs` mints every KeyPackage with that default. A leaf keeps its
KeyPackage lifetime until its owner replaces it, through an Update proposal or its own commit path.

openmls compares lifetimes with `SystemTime::now()` in these places that reach us:

| Where | What is checked | 0.8.1 | 0.9.2 |
|---|---|---|---|
| Joining from a Welcome (`StagedWelcome::new_from_welcome` -> `PublicGroup::from_ratchet_tree`) | **every** leaf in the tree whose source is KeyPackage | `public_group/mod.rs:290` | `public_group/mod.rs:308` |
| Processing or building a commit with an Add (`validate_add_proposals`) | the **added** KeyPackage only | `validation.rs:408` | `validation.rs:405` |
| `KeyPackageIn::validate` (the joiner's KeyPackage off the wire, at the committer) | that KeyPackage | `key_package_in.rs:196` | `key_package_in.rs:195` |

(An Update proposal's leaf is checked too, `validation.rs:490`; nothing sends one today.) The error
is `LeafNodeValidationError::Lifetime(NotCurrent)` in 0.8.1 and `Lifetime(Expired{..})` or
`NotValidYet` in 0.9.2.

Membership is single-serializer (`SyncConfig::max_committer_rank == 0`). `add_members` and
`remove_members` always carry a path, so the designated committer's own leaf becomes
`LeafNodeSource::Commit` (no lifetime) at its first commit. Nobody else ever commits, and the app has
no MLS Update flow, so **every other member's leaf keeps its join-time lifetime forever**. 84 days
after the first such member joined, every Welcome join to the group fails, for every joiner. The
committer has already merged the Add and consumed the invite by then (section 5).

Second failure, same root cause: a member that was offline for longer than an added KeyPackage's
lifetime cannot replay that Add from the commit log (`max_commit_log` = 256 records). openmls
re-checks the added KeyPackage against today's clock and refuses the commit, so the member stays
at the old epoch (section 6).

## 2. Options

### A. Joiner skips the lifetime check on the Welcome's ratchet tree (chosen)

openmls ships this exact switch for this exact problem: `StagedWelcome::build_from_welcome(..)`
`.skip_lifetime_validation().build()`, which sets `LeafNodeLifetimePolicy::Skip`. Its validation
code cites RFC 9420 §7.3 and openmls issue #1810: the RFC recommends the check but acknowledges
that it breaks for leaves that are never updated. The API is identical in 0.8.1 and 0.9.2.

What `Skip` turns off: only the `not_before`/`not_after` comparison for leaves **in the received
tree**. What still runs, unchanged (`validate_leaf_node_inner` and its callers):

- the GroupInfo signature, tree hash, parent hashes and leaf signatures (the tree is authentic);
- leaf capabilities, credential types and key uniqueness;
- our own checks: Welcome signed by the invite's inviter or the granted owner (`finish_join`,
  `finish_device_join`), group id, ciphersuite;
- the Add-time lifetime check on the joiner's KeyPackage, by the committer when it builds the Add
  and by every existing member when it processes it. Admitting a stale KeyPackage stays refused.

Security cost. The in-tree check never protected anyone already in the group: existing members
keep encrypting path secrets to a stale leaf whatever a joiner decides. It only made the joiner
refuse to stand where existing members already stand. An expired KeyPackage leaf means "this
member has not rotated its HPKE leaf key for a long time", a post-compromise-security hygiene
signal, not an authenticity failure. The joiner now accepts the same exposure existing members
have. Two smaller consequences:

- A malicious inviter could already hand a joiner a fabricated tree of its own making (a fork
  containing only the inviter); it can now also put KeyPackage leaves of any age in it, such as
  old leaves of removed members. Our joiner does not check leaf credentials at join, before or
  after this change.
- The joiner's own leaf no longer bounds the Welcome's age at 84 days. The join flow's own
  request/response deadlines bound it instead, and the joiner's KeyPackage private key is
  consumed by the first attempt.

That is the whole delta, and it is the smallest one that makes joins work. "Verify first, then
skip" is not possible: openmls deletes the KeyPackage private key before it validates the tree
(0.8.1 `creation.rs:608`, 0.9.2 `:709`), and it would accept exactly the same trees.

Fixes groups that are already broken, with no action from existing members. No wire change, no
persistence change.

### B. Member Update proposals, committed by the designated committer

This would refresh leaves (source becomes `Update`, no lifetime) and improve post-compromise
security, so it is worth having on its own merits. It is not a fix for this bug:

- **There is no MLS proposal path today.** The "proposal/commit split" in this codebase is
  app-level: members send signed requests (`CTRL_REMOVE_REQUEST`, `CTRL_ADD_REQUEST`,
  `CTRL_DEVICE_ADD`) and the committer builds the commit itself. That cannot carry an Update: only
  the leaf's owner can sign its new leaf, and RFC 9420 lets a commit carry another member's Update
  only **by reference** (openmls rejects an Update by value as the committer's own,
  `CommitterIncludedOwnUpdate`). So every member must hold the proposal in its openmls proposal
  store before the commit arrives. `process_incoming` drops proposals today (`Incoming::Other`,
  never `store_pending_proposal`), and the full proposal/commit split is the deferred 6d-2
  (ARCHITECTURE §4c).
- **Catch-up breaks.** `CommitRecord` carries only the commit. A member replaying a by-reference
  commit from the commit log without the proposal cannot apply it, so the record format and
  `KIND_COMMIT_CATCHUP` would have to change.
- **Mixed versions split.** Older builds do not store proposals, so they fail at the first
  refresh commit and are stranded at that epoch.
- **Storing proposals widens the committer's input.** openmls's commit builder consumes the whole
  proposal store by default, so every later Add or Remove would also commit whatever members
  queued; the store would have to be filtered to Updates from their own leaf.
- **It cannot repair a group whose stale member is offline.** That leaf stays expired until its
  owner returns.

Recorded as a follow-up for post-compromise security, to land with 6d-2.

### C. Longer or never-expiring lifetimes for new KeyPackages

- Does nothing for existing leaves, which keep their signed 84-day lifetime, so it does not fix
  existing groups.
- Weakens the Add-time protection (a stale KeyPackage becomes admissible) for every new member.
- RFC 9420 asks applications to cap the total lifetime. openmls 0.8.1 and 0.9.2 define a cap
  (`Lifetime::has_acceptable_range`, 84 days + 1 hour) but do not enforce it. A later openmls that
  does would refuse our KeyPackages at Add time.
- Its one real benefit is the replay case in section 6 for future Adds.

Not adopted; listed in section 6 as the owner's decision.

## 3. Change

- `ServerGroup::join` uses `StagedWelcome::build_from_welcome(provider, &join_config(), welcome)?`
  `.skip_lifetime_validation().build()?.into_group(provider)?`, with a comment naming what is
  skipped and what still runs. Nothing else changes: not `create`, not KeyPackage minting, not
  `process_incoming`.
- **Admission floor (from the design review, H1).** The joiner chooses its own KeyPackage
  lifetime. One that is valid now but ends a minute later is admitted by the committer, then
  refused permanently by every member that processes the Add later: offline members, and anyone
  replaying the commit log. They stay at the old epoch until re-invited. This predates this
  change, but it is the same failure. `MlsDevice::parse_key_package(bytes, now_ms)`, the one parse
  every admission path in `catcoms-sync` uses, now also requires `not_after >= now + 60 days`
  (`MIN_ADMITTED_KEY_PACKAGE_LIFETIME_MS`, `InviteError::KeyPackageExpiresTooSoon`). `now_ms`
  comes from the caller's `Clock`. Honest KeyPackages carry 84 days and are admitted within an hour
  of minting, so 24 days of slack absorb a slow joiner clock. Only `not_after` is checked: a
  `not_before` of "just now" makes members whose clocks lag refuse the Add only until their clocks
  pass it, which catch-up retries outlive. A `not_before` margin would also refuse every honest
  KeyPackage in tests whose injected clock sits in 1970 while openmls stamps real time.
- Tests (`group/leaf_lifetime_tests.rs`), deterministic, with lifetimes that are already in the
  past and no sleeping:
  1. **Regression**: a Welcome whose tree holds an expired KeyPackage leaf joins, and the joiner
     agrees with the group on the epoch, the exporter (`channel_secret`) and a decrypted message.
     Fixture: the founder's leaf is created with an expired lifetime, and the founder adds through
     openmls's `add_members_without_update`, so its leaf stays KeyPackage-sourced in the Welcome's
     tree. The test asserts that the tree really holds an expired leaf.
  2. **The fixture is load-bearing**: the same Welcome is refused by openmls's default join
     (`Verify`). It runs on a duplicate of the joiner's device, because a join attempt consumes
     the KeyPackage's private material.
  3. **Production shape**: three parties. The expired leaf belongs to a member who is neither the
     joiner nor the signer, and the Welcome comes from a path-carrying production `add_member`.
  4. **The kept protection**: `add_member` refuses a KeyPackage whose lifetime has already ended
     (and one not yet valid), the group does not advance, and a valid Add still succeeds after.
  5. **The admission floor**: one second short of 60 days is refused, exactly 60 days is
     admitted, an honest KeyPackage clears it even three weeks late, and an already-expired one is
     refused on the wire.
  6. Break-it checks: reverting `join` to `Verify` fails tests 1 and 3; removing the floor fails
     test 5.
- Docs: THREAT-MODEL dated section; HANDOVER entry; INTERFACES notes on `join` and
  `parse_key_package`.

## 4. Compatibility

- **Wire:** unchanged; still standard MLS.
- **Persistence:** unchanged (`MlsGroupJoinConfig` is untouched; the lifetime policy is not
  stored).
- **Mixed versions:** the join fix is entirely on the joiner. A joiner on this build joins any
  group; a joiner on an older build still fails on a group with an expired leaf, and still uses
  up the invite and leaves a leaf behind (section 5). Existing members' versions do not matter.
  The admission floor is on the committer: an older committer still admits a short-lived
  KeyPackage. Honest KeyPackages from any build pass it.
- **Alpha data:** groups already past 84 days become joinable as soon as the joiner runs this
  build. Nothing stored needs migrating.
- **PR #57 (openmls 0.9.2):** same API, same behaviour. #57 adds `create_with_lifetime`,
  `key_package_leaf_not_after` and test modules next to `join`; this change edits only the body of
  `join` and adds its test module at the end of `group.rs`, so the two are expected to merge
  without conflict (a trial merge is recorded in the PR). After both land, #57's never-expiring
  fixture lifetimes are no longer needed for joins; they remain correct and are left alone, and
  this test module's local founder helper can switch to #57's `create_with_lifetime`.

## 5. Recovery of already-broken groups

- New joins: work once the joiner has this build. Nothing to do on the group side.
- Each join that already failed left a leaf behind. The committer merged the Add and consumed the
  invite before the joiner's openmls refused the tree, and the refused attempt deleted the
  joiner's KeyPackage private key, so that leaf is a member nobody can operate. It shows in the
  roster, and it never rotates. The owner removes it through the ordinary removal path, and the
  person needs a fresh invite. This PR does not add an automatic cleanup: telling a "never
  arrived" leaf from a quiet member needs evidence the protocol does not have.

## 6. Residual: replaying an Add after its KeyPackage expired

A member offline for longer than an added KeyPackage's remaining lifetime, whose gap is still
inside a peer's commit log, cannot apply that Add: openmls has no production switch to skip the
check on `process_message` (the `skip_validation` flags are test-only). The admission floor makes
that remaining lifetime at least 60 days (about 84 for honest joiners) instead of whatever the
joiner chose. The member stays at the old epoch. Remedy today: re-invite.

It is also misreported. If later records follow the refused Add in the same catch-up answer, they
are buffered as "future" and the exchange returns `Stranded`, recording a `MembershipChainGap`
that blames log retention. If the refused Add is the newest record, the exchange returns
`Verified { applied: 0 }`. A distinct outcome for "offered at my epoch but refused" is a
follow-up; it belongs with the sync-layer outcome types, not this MLS change.

Options for the owner:

- lengthen new KeyPackage lifetimes (option C): fixes future Adds only, with the costs above;
- ask openmls for a process-time lifetime policy;
- the deferred snapshot rejoin.

This PR does not test the replay itself: building an Add that is valid when built and expired
when replayed needs the wall clock to move, and openmls reads the system clock directly.

## 7. Design review (AGENTS.md skeleton, read-only)

No blocker. Findings and what was done:

| Severity | Finding | Outcome |
|---|---|---|
| HIGH | A joiner-chosen short `not_after` strands every member that processes the Add later | Fixed: admission floor in `parse_key_package` (section 3), test 5 |
| MEDIUM | Section 6 wrongly said `MembershipChainGap` does not cover the replay failure; it misreports it | Section 6 corrected; distinct outcome is a follow-up |
| MEDIUM | Fixture did not match the production Welcome shape | Test 3 added |
| LOW | "Exactly two places"; delta understated (fabricated trees, Welcome age); old-build joiners still burn invites | Sections 1, 2A, 4 corrected |
| LOW | Kept-protection test: no wire-path check, no follow-up valid Add, substring match | Wire path and follow-up Add added; openmls errors arrive as strings through `MlsError::Protocol`, so the match stays on text, now `Lifetime` |
| LOW | Local test helpers duplicate #57's | Kept local to avoid merge conflicts; switch after both land |
| LOW | Trial merge stated as done at design time | Reworded |
