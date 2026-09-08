//! Independent local evidence for P1 receipt authority. The current owner's key alone cannot
//! distinguish A -> B -> A. This state observes applied MLS transitions and is saved in the SAME
//! vault-sealed snapshot as that MLS group; a receipt never supplies its own tenure evidence.
use super::*;

/// Captured immediately before a synchronous MLS mutation, never from a network body.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Position {
    owner: Option<DeviceId>,
    epoch: u64,
}
impl Position {
    pub(super) fn of(group: &ServerGroup) -> Self {
        Self {
            owner: group.designated_committer(),
            epoch: group.epoch(),
        }
    }
}

pub(super) struct OwnerTenure {
    position: Position,
    start: Option<u64>,
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
        }
    }

    /// Observe the actual post-call group, including when a helper returned an error AFTER
    /// merging. An error before mutation/no-op establishes no new tenure. Adds can replace the
    /// owner through recycled low leaf slots, just as Remove can.
    pub(super) fn applied(&mut self, before: Position, group: &ServerGroup) {
        let after = Position::of(group);
        if after == before {
            return;
        }
        let start = if after.owner.is_none() || before.epoch.checked_add(1) != Some(after.epoch) {
            None // an unobserved gap might include A -> B -> A
        } else if before.owner.is_some() && after.owner.is_some() && before.owner != after.owner {
            Some(after.epoch)
        } else if self.position == before {
            self.start // same owner preserves knowledge OR the lack of it
        } else {
            None
        };
        self.position = after;
        self.start = start;
    }

    pub(super) fn start(&self, group: &ServerGroup) -> Option<u64> {
        (self.position == Position::of(group))
            .then_some(self.start)
            .flatten()
    }

    /// Strict versioned tail, capped at 57 bytes: v, observed epoch, optional owner bytes32,
    /// optional start bytes8. Snapshot callers add the existing four-byte length framing.
    pub(super) fn encode(&self, group: &ServerGroup) -> Result<Vec<u8>, SyncError> {
        if self.position != Position::of(group) {
            // An omitted integration hook must fail closed, not persist fresh MLS with stale
            // authority. Reading start() also refuses in this case, including after unwind.
            return Err(SyncError::Malformed);
        }
        let mut e = Encoder::new();
        e.put_u8(1);
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
        Ok(e.finish())
    }

    /// Only authenticated local snapshot bytes may enter; not a wire proof or a join transfer.
    pub(super) fn decode(bytes: &[u8], group: &ServerGroup) -> Result<Self, SyncError> {
        if bytes.len() > 57 {
            return Err(SyncError::Malformed);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| SyncError::Malformed)? != 1 {
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
        d.finish().map_err(|_| SyncError::Malformed)?;
        let position = Position { owner, epoch };
        if position != Position::of(group)
            || start.is_some_and(|start| owner.is_none() || start > epoch)
        {
            return Err(SyncError::Malformed);
        }
        Ok(Self { position, start })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// All synchronous MLS mutations go through this seam. Observation follows the actual group
    /// before the caller propagates an error: serialization can fail after a successful merge.
    /// A panic still fails closed via start()/snapshot's position checks; no publication permit
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

    /// Independently observed start of the CURRENT owner tenure, if known. None is not epoch
    /// zero: legacy snapshots/new joins can remain unknown indefinitely through same-owner
    /// commits. This is local evidence only, not a durable-publication permit or a remote proof.
    /// A receipt publisher must first flush the matching MLS snapshot and owner decision, then
    /// recheck this value and current membership at its actual signing/submission boundary.
    pub fn observed_owner_tenure_start(&self) -> Option<u64> {
        self.owner_tenure.start(&self.group)
    }
}

#[cfg(test)]
mod tests;
