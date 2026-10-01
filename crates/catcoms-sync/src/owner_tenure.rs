//! Independent local evidence for P1 receipt authority. The current owner's key alone cannot
//! distinguish A -> B -> A. This state observes applied MLS transitions and is saved in the SAME
//! vault-sealed snapshot as that MLS group; a receipt never supplies its own tenure evidence.
use super::*;

/// Captured immediately before a synchronous MLS mutation, never from a network body.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Position {
    owner: Option<DeviceId>,
    /// The committer's leaf identity: index plus a digest over index, signature key and credential.
    ///
    /// Carried because `owner` and `epoch` alone cannot distinguish a same-commit remove-and-re-add
    /// of the committer from an ordinary same-owner commit. A rejoined device keeps its `DeviceId`,
    /// so without this a witness preserves the old tenure start while the rejoining device computes
    /// a new one, and the two disagree about who may issue a receipt. `catcoms-mls`'s M-1 refuses
    /// that commit shape outright; this is the other half, so every participant computes the same
    /// value for every shape that IS allowed.
    leaf: Option<(u32, [u8; 32])>,
    /// The committer's full signature key, captured here because a Remove takes it out of the
    /// group: after the transition there is nothing left to read it from, and an archived witness
    /// (CORE-005) needs the key itself, not the `DeviceId` hashed from it. Never persisted; a
    /// restored position reads it from the same live group it is checked against.
    owner_key: Option<[u8; 32]>,
    epoch: u64,
}
impl Position {
    pub(super) fn of(group: &ServerGroup) -> Self {
        let owner = group.designated_committer();
        Self {
            owner,
            leaf: group.designated_committer_leaf(),
            owner_key: owner.and_then(|id| committer_key(group, &id)),
            epoch: group.epoch(),
        }
    }
}

fn committer_key(group: &ServerGroup, owner: &DeviceId) -> Option<[u8; 32]> {
    group.member_signature_key(owner)?.try_into().ok()
}

/// The CORE-005 cap on one archived witness, framing included. The encoding below is a fixed 80
/// bytes behind a four-byte length, and the assertion keeps any future field honest about it.
pub const MAX_HISTORICAL_OWNER_WITNESS_BYTES: usize = 128;
const WITNESS_BYTES: usize = 32 + 8 + 32 + 8;
const _: () = assert!(4 + WITNESS_BYTES <= MAX_HISTORICAL_OWNER_WITNESS_BYTES);

/// One owner tenure this device **positively observed** from start to retirement (CORE-005).
///
/// Agent 3's historical report admission needs to know that a removed owner really held office,
/// and when. A self-signed receipt cannot say so, and neither can anything this device only
/// imported or inferred. So a witness is minted in exactly one place, `OwnerTenure::applied`, and
/// only when a contiguous MLS step ends a tenure whose start this device held as `Observed` and
/// begins a different one. There is no public constructor, no wire form and no import path.
///
/// One per group. A later positively observed retirement replaces it; a gap, an `Imported` or
/// `Unknown` departure, or an ordinary same-owner commit leaves it alone. That is the bounded
/// history the CORE-005 review accepted, not an audit trail.
///
/// The application must not read this off a live `ChannelSync`. It reaches it only through
/// [`ChannelSync::with_durable_owner_history`], under a [`DurableOwnerSnapshot`] that exists only
/// after the snapshot carrying this witness was durably saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchivedOwnerTenure {
    owner_key: [u8; 32],
    start: u64,
    tenure_id: [u8; 32],
    retired_at: u64,
}

impl ArchivedOwnerTenure {
    fn mint(group: &ServerGroup, owner_key: [u8; 32], start: u64, retired_at: u64) -> Self {
        Self {
            owner_key,
            start,
            tenure_id: catcoms_replication::tenure_id(&group.group_id(), &owner_key, start),
            retired_at,
        }
    }
    /// The retired owner's full signature key.
    pub fn owner_key(&self) -> &[u8; 32] {
        &self.owner_key
    }
    /// The MLS epoch the tenure began at, as this device observed it.
    pub fn start(&self) -> u64 {
        self.start
    }
    /// `tenure_id(group id, owner key, start)`: what that owner's receipts carry.
    pub fn tenure_id(&self) -> &[u8; 32] {
        &self.tenure_id
    }
    /// The first MLS epoch at which the owner no longer held office.
    pub fn retired_at(&self) -> u64 {
        self.retired_at
    }
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(WITNESS_BYTES);
        bytes.extend_from_slice(&self.owner_key);
        bytes.extend_from_slice(&self.start.to_be_bytes());
        bytes.extend_from_slice(&self.tenure_id);
        bytes.extend_from_slice(&self.retired_at.to_be_bytes());
        bytes
    }
    /// Restore checks everything that can be checked against the live group: framing, the
    /// derived id (which binds the group, the key and the start together), and
    /// `start < retired_at <= epoch`. Any failure refuses the whole tail rather than dropping the
    /// witness, so corruption cannot pass for "no history".
    fn decode(bytes: &[u8], group: &ServerGroup, epoch: u64) -> Result<Self, SyncError> {
        if bytes.len() != WITNESS_BYTES {
            return Err(SyncError::Malformed);
        }
        let owner_key: [u8; 32] = bytes[..32].try_into().map_err(|_| SyncError::Malformed)?;
        let start = u64::from_be_bytes(bytes[32..40].try_into().map_err(|_| SyncError::Malformed)?);
        let tenure_id: [u8; 32] = bytes[40..72].try_into().map_err(|_| SyncError::Malformed)?;
        let retired_at =
            u64::from_be_bytes(bytes[72..80].try_into().map_err(|_| SyncError::Malformed)?);
        let witness = Self::mint(group, owner_key, start, retired_at);
        if witness.tenure_id != tenure_id || start >= retired_at || retired_at > epoch {
            return Err(SyncError::Malformed);
        }
        Ok(witness)
    }
}

pub(super) struct OwnerTenure {
    position: Position,
    start: Option<u64>,
    /// Set only by the v1 snapshot migration, for a `start < epoch` whose continuity this build
    /// cannot establish. Persisted, so a save/reload cycle cannot launder it into a fully observed
    /// value and silently grant the authoring authority the migration withheld.
    imported: bool,
    /// The last positively observed retirement (CORE-005). Saved in this same tail, so it is
    /// atomic with the MLS state and the current tenure by construction.
    archive: Option<ArchivedOwnerTenure>,
}

/// What is known about the current owner's tenure start, and how well.
///
/// The two consumers of this value have **opposite failure directions**, which is why an enum
/// replaced a bare `Option<u64>`. Verification wants the value present wherever it is sound, because
/// a missing local value makes `complete_checkpoint_head_scoped` accept a proof's own claim.
/// Authoring wants it absent unless fully observed, because signing as the owner on a tenure this
/// build cannot verify is the thing that must not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedOwnerTenure {
    /// Fully observed under the leaf-aware rule, or migrated from a provably safe v1 state.
    Observed(u64),
    /// A v1 `start < epoch`, with no leaf-continuity evidence. Sound for VERIFICATION and
    /// fail-closed for AUTHORING.
    Imported(u64),
    /// No evidence at all.
    Unknown,
}

impl OwnerTenure {
    /// A locally available founding group is unambiguous. A post-Welcome group is not: its
    /// current epoch is the join epoch, not necessarily the epoch when its owner took office.
    pub(super) fn new(group: &ServerGroup) -> Self {
        let mut state = Self::unknown(group);
        if state.position.epoch == 0 && state.position.owner.is_some() {
            state.start = Some(0);
        }
        state
    }

    /// Legacy snapshots and new joins cannot manufacture knowledge from the current leaf rank.
    pub(super) fn unknown(group: &ServerGroup) -> Self {
        Self {
            position: Position::of(group),
            start: None,
            imported: false,
            archive: None,
        }
    }

    /// A device that has just joined via Welcome, and is the committer of the group it joined.
    ///
    /// The inference is about **current continuous membership**, not about history: this device's
    /// current continuous membership in this group began at this epoch, and a tenure is an
    /// uninterrupted run as designated committer, so its *current* tenure cannot have begun before
    /// its current membership did. If it is the committer now, its current tenure started here. That
    /// holds for a returning device too, which is why the earlier phrasing - "was not a member at
    /// any earlier epoch" - was wrong and this one is not.
    ///
    /// Deliberately **not** available to `unknown`, which also serves legacy snapshots where the
    /// device may have been committer for an unknown number of prior epochs.
    ///
    /// Without this, a device joining into a recycled low leaf becomes the designated committer with
    /// `None` and can never issue a receipt, while every witness knows the answer.
    pub(super) fn joined(group: &ServerGroup, device: &MlsDevice) -> Self {
        let mut state = Self::unknown(group);
        if group.designated_committer() == Some(device.device_id()) {
            state.start = Some(state.position.epoch);
        }
        state
    }

    /// Observe the actual post-call group, including when a helper returned an error AFTER
    /// merging. An error before mutation/no-op establishes no new tenure. Adds can replace the
    /// owner through recycled low leaf slots, just as Remove can.
    pub(super) fn applied(&mut self, before: Position, group: &ServerGroup) {
        let after = Position::of(group);
        if after == before {
            return;
        }
        // `computed` distinguishes a start this call DERIVED from one it merely carried forward.
        // Only a derived start may clear the import flag, and getting that wrong is how an
        // unverifiable v1 tenure would launder itself into a fully observed one: the preserve arm
        // returns `self.start`, which is `Some`, so treating "start is present" as "start is fresh"
        // promoted every `Imported` server on its next same-owner commit. An adversarial review
        // demonstrated exactly that, one member-add after the upgrade.
        let (start, computed) = if after.owner.is_none()
            || before.epoch.checked_add(1) != Some(after.epoch)
        {
            (None, false) // an unobserved gap might include A -> B -> A
        } else if before.owner.is_some() && after.owner.is_some() && before.owner != after.owner {
            (Some(after.epoch), true)
        } else if before.owner.is_some() && before.owner == after.owner && before.leaf != after.leaf
        {
            // The same `DeviceId` on a DIFFERENT leaf identity across one contiguous step: the
            // committer's membership restarted, so this is a new tenure even though the owner did
            // not change. Without this arm a witness would preserve the old start here while the
            // rejoining device computed a new one, and the two would disagree.
            //
            // `catcoms-mls`'s M-1 refuses the one commit shape that can produce this inside a single
            // commit, so in a compliant group this arm is reached only by shapes M-1 permits. It is
            // kept rather than assumed away because the two rules protect different things: M-1
            // stops the ambiguous commit being accepted, and this makes the computation agree for
            // every shape that is.
            //
            // A self-update does NOT reach here: `designated_committer_leaf` excludes the HPKE
            // encryption key, so rotating keys while keeping the credential leaves the digest equal.
            (Some(after.epoch), true)
        } else if self.position == before {
            (self.start, false) // same owner preserves knowledge OR the lack of it
        } else {
            (None, false)
        };
        // CORE-005. Archive the departing tenure only when this step DERIVED a new one AND the
        // tenure it ends was fully observed here: the saved position is exactly `before`, a start
        // is held, and it was not imported. Anything less - a gap, an import, no start - neither
        // mints nor replaces, so an earlier witness survives it.
        if computed && self.position == before && !self.imported {
            if let (Some(key), Some(start)) = (before.owner_key, self.start) {
                self.archive = Some(ArchivedOwnerTenure::mint(group, key, start, after.epoch));
            }
        }
        self.position = after;
        self.start = start;
        // The flag survives everything except a start this call derived itself, and cannot survive
        // the start disappearing.
        //
        // The second half is not cosmetic: `encode` writes the flag beside an empty start, and
        // `decode` refuses that combination, so an `Imported` state that lost its start through a
        // gap would serialise into a snapshot this build cannot reopen - the whole server, not just
        // the tenure. Clearing it here keeps the two consistent by construction.
        self.imported = self.imported && !computed && start.is_some();
    }

    /// The archived witness, under the same stale-position refusal as `observed`.
    pub(super) fn archived(&self, group: &ServerGroup) -> Option<ArchivedOwnerTenure> {
        if self.position != Position::of(group) {
            return None;
        }
        self.archive
    }

    pub(super) fn observed(&self, group: &ServerGroup) -> ObservedOwnerTenure {
        if self.position != Position::of(group) {
            return ObservedOwnerTenure::Unknown;
        }
        match (self.start, self.imported) {
            (Some(start), false) => ObservedOwnerTenure::Observed(start),
            (Some(start), true) => ObservedOwnerTenure::Imported(start),
            (None, _) => ObservedOwnerTenure::Unknown,
        }
    }

    /// Strict versioned tail: v, observed epoch, optional owner bytes32, optional start bytes8, the
    /// committer leaf identity, and the import flag. Snapshot callers add the four-byte framing.
    pub(super) fn encode(&self, group: &ServerGroup) -> Result<Vec<u8>, SyncError> {
        if self.position != Position::of(group) {
            // An omitted integration hook must fail closed, not persist fresh MLS with stale
            // authority. Reading `observed()` also refuses in this case, including after unwind.
            return Err(SyncError::Malformed);
        }
        let mut e = Encoder::new();
        // v2 carries the committer's leaf identity and the import flag. v1 carried neither, and a v1
        // record cannot be upgraded in place: see `decode`.
        //
        // v3 is v2 plus one archived witness, and it is written ONLY when a witness exists. Every
        // state without one keeps its exact v2 bytes, so no existing snapshot changes, and each
        // state still has a single encoding: v3 never carries an empty witness.
        e.put_u8(if self.archive.is_some() { 3 } else { 2 });
        e.put_u64(self.position.epoch);
        e.put_bytes(
            self.position
                .owner
                .as_ref()
                .map_or(&[], |id| id.as_bytes().as_slice()),
        )
        .map_err(|_| SyncError::Malformed)?;
        e.put_bytes(
            &self
                .start
                .map_or_else(Vec::new, |epoch| epoch.to_be_bytes().to_vec()),
        )
        .map_err(|_| SyncError::Malformed)?;
        e.put_bytes(&match self.position.leaf {
            None => Vec::new(),
            Some((index, digest)) => {
                let mut bytes = index.to_be_bytes().to_vec();
                bytes.extend_from_slice(&digest);
                bytes
            }
        })
        .map_err(|_| SyncError::Malformed)?;
        e.put_u8(u8::from(self.imported));
        if let Some(archive) = &self.archive {
            e.put_bytes(&archive.encode())
                .map_err(|_| SyncError::Malformed)?;
        }
        Ok(e.finish())
    }

    /// Only authenticated local snapshot bytes may enter; not a wire proof or a join transfer.
    ///
    /// **The v1 migration is the interesting part.** A v1 `start` was computed by the old `applied`,
    /// whose preserve branch could not see a same-commit remove-and-re-add, so a snapshot written
    /// after such a transition carries a **stale** start. Grafting the current leaf digest onto it
    /// would manufacture continuity evidence v1 never recorded. M-1 cannot repair that: it only
    /// stops such a transition being accepted *after* the upgrade, and an old build could have
    /// accepted one. Nor can corroboration: every pre-upgrade participant ran the same preserve
    /// branch and holds the same stale value, so a witnessed attestation would agree with the wrong
    /// answer.
    ///
    /// Exactly the states with `start == epoch` are provably safe. From the old `applied`: after any
    /// call `position.epoch == after.epoch`; the genuine-change branch sets `start = after.epoch`,
    /// so `start == epoch`; the preserve branch's value was fixed when `position.epoch` was
    /// `after.epoch - 1` and is never raised while preserving, so `start <= epoch - 1`. Therefore
    /// `start == epoch` implies the most recent applied step was a genuine `DeviceId` owner change,
    /// which is visible under BOTH the old and the new rule, so no hidden discontinuity lies there -
    /// and an earlier one is irrelevant, because that later genuine change reset the tenure. The
    /// founding `epoch == 0, start == Some(0)` is the same case.
    ///
    /// The rest are imported rather than discarded. "Promote only `start == epoch`, else Unknown"
    /// taken alone is too blunt: a founder at epoch 0 that has applied any commit holds
    /// `start = Some(0)` with `epoch > 0`, so every existing server's owner would lose the ability
    /// to issue receipts and rotate, and on a single-owner server would never regain it, since only
    /// a genuine owner change re-establishes a start. `Imported` keeps such a server verifying while
    /// refusing to let it author on evidence this build cannot check.
    pub(super) fn decode(bytes: &[u8], group: &ServerGroup) -> Result<Self, SyncError> {
        // 98 is the exact maximum valid v2 tail: version, epoch, a 32-byte owner, an 8-byte start, a
        // 36-byte leaf and the flag, with this encoder's framing. A loose cap was a silent deviation
        // from the design's number and bought nothing - `finish()` already refuses trailing bytes -
        // so the tight one is both correct and self-documenting. v3 adds exactly one framed
        // 80-byte witness.
        if bytes.len() > 98 + 4 + WITNESS_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut d = Decoder::new(bytes);
        let version = d.get_u8().map_err(|_| SyncError::Malformed)?;
        if !(1..=3).contains(&version) {
            return Err(SyncError::Malformed);
        }
        let epoch = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let owner = match d.get_bytes().map_err(|_| SyncError::Malformed)? {
            [] => None,
            bytes => Some(DeviceId::from_bytes(
                bytes.try_into().map_err(|_| SyncError::Malformed)?,
            )),
        };
        let start = match d.get_bytes().map_err(|_| SyncError::Malformed)? {
            [] => None,
            bytes => Some(u64::from_be_bytes(
                bytes.try_into().map_err(|_| SyncError::Malformed)?,
            )),
        };
        // A v2 record carries the leaf identity it was written with, and the position check below
        // requires it to equal the live group's - the same fail-closed rule the owner and epoch get.
        // A v1 record carries none, so the live value is used, which is sound ONLY because such a
        // record is either promoted on the provably safe `start == epoch` path or imported, never
        // treated as fully observed on the strength of a digest it never recorded.
        let (leaf, imported) = if version >= 2 {
            let leaf = match d.get_bytes().map_err(|_| SyncError::Malformed)? {
                [] => None,
                bytes if bytes.len() == 36 => {
                    let index = u32::from_be_bytes(
                        bytes[..4].try_into().map_err(|_| SyncError::Malformed)?,
                    );
                    let digest: [u8; 32] =
                        bytes[4..].try_into().map_err(|_| SyncError::Malformed)?;
                    Some((index, digest))
                }
                _ => return Err(SyncError::Malformed),
            };
            let imported = match d.get_u8().map_err(|_| SyncError::Malformed)? {
                0 => false,
                1 => true,
                _ => return Err(SyncError::Malformed),
            };
            (leaf, imported)
        } else {
            (
                group.designated_committer_leaf(),
                start.is_some_and(|start| start != epoch),
            )
        };
        // Legacy records have no witness, and nothing upgrades them into one: CORE-005's history
        // starts at the first retirement this build observes.
        let archive = if version == 3 {
            let bytes = d.get_bytes().map_err(|_| SyncError::Malformed)?;
            Some(ArchivedOwnerTenure::decode(bytes, group, epoch)?)
        } else {
            None
        };
        d.finish().map_err(|_| SyncError::Malformed)?;
        // The key is not persisted: it is read from the live group, which is sound because the
        // position must equal the live one anyway, and a `DeviceId` is a hash of that key.
        let owner_key = owner.and_then(|id| committer_key(group, &id));
        let position = Position {
            owner,
            leaf,
            owner_key,
            epoch,
        };
        if position != Position::of(group)
            || start.is_some_and(|start| owner.is_none() || start > epoch)
            || (imported && start.is_none())
        {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            position,
            start,
            imported,
            archive,
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// All synchronous MLS mutations go through this seam. Observation follows the actual group
    /// before the caller propagates an error: serialization can fail after a successful merge.
    /// A panic still fails closed via observed()/snapshot's position checks; no publication permit
    /// escapes this helper. The callback must not publish or snapshot its intermediate state.
    pub(super) fn with_observed_mls_transition<O>(
        &mut self,
        work: impl FnOnce(&mut Self) -> O,
    ) -> O {
        let before = Position::of(&self.group);
        let outcome = work(self);
        self.owner_tenure.applied(before, &self.group);
        outcome
    }

    /// The classified tenure state. Local evidence only, never a publication permit or a remote
    /// proof. A receipt publisher must first flush the matching MLS snapshot and owner decision,
    /// then recheck this value and current membership at its actual signing/submission boundary.
    pub fn observed_owner_tenure(&self) -> ObservedOwnerTenure {
        self.owner_tenure.observed(&self.group)
    }

    /// For **verification**: `Observed` and `Imported` both yield `Some`.
    ///
    /// Keeping `Imported` visible here is strictly safer than hiding it.
    /// `complete_checkpoint_head_scoped` accepts a proof's *claimed* tenure when the local value is
    /// absent, so surfacing a merely-imported value can only add refusals - never an acceptance that
    /// `Unknown` would have rejected.
    pub fn verification_owner_tenure_start(&self) -> Option<u64> {
        match self.observed_owner_tenure() {
            ObservedOwnerTenure::Observed(start) | ObservedOwnerTenure::Imported(start) => {
                Some(start)
            }
            ObservedOwnerTenure::Unknown => None,
        }
    }

    /// For **authoring**: only `Observed` yields `Some`.
    ///
    /// Signing as the owner is authoring, and doing it on a tenure this build cannot verify is the
    /// failure the `Imported` distinction exists to prevent.
    ///
    /// `observed_owner_tenure_start` was **removed rather than repointed.** Its doc promised an
    /// independently observed tenure, which would have become false for `Imported`, and leaving the
    /// name in place would let a future call site inherit the wrong semantics by default. Deleting
    /// it forces every site to declare which it wants, and the compiler enumerates them.
    pub fn authoring_owner_tenure_start(&self) -> Option<u64> {
        match self.observed_owner_tenure() {
            ObservedOwnerTenure::Observed(start) => Some(start),
            ObservedOwnerTenure::Imported(_) | ObservedOwnerTenure::Unknown => None,
        }
    }
}

#[cfg(test)]
mod tests;
