//! A fresh authenticated member hint with provisional-only capacity and copied watch custody.
//! This path does not fetch a seed, validate its attribution or select an authoritative epoch.
use super::*;
use crate::receipt_head::{
    AuthenticatedCheckpointHint, CompletedCheckpointHead, PendingCheckpointHead,
};
use crate::StudioWatch;
use catcoms_replication::studio::StudioTarget;

pub struct PendingProvisionalStudioDiscovery<T: MeshTransport> {
    head: PendingCheckpointHead<T>,
    watch: StudioWatch,
    capacity: ProvisionalCheckpointCapacity,
    expires: u64,
}
pub struct CompletedProvisionalStudioDiscovery {
    head: CompletedCheckpointHead,
    watch: StudioWatch,
    capacity: ProvisionalCheckpointCapacity,
    expires: u64,
}
/// Opaque, volatile candidate supplied by an authenticated current member. Neither the receipt
/// signature nor any claimed current/historical ownership has been verified. It cannot be used
/// as a `RegistrySeedFetch`, installed checkpoint, editing authority or publication permit.
pub struct ProvisionalStudioHint {
    head: AuthenticatedCheckpointHint,
    watch: StudioWatch,
    _capacity: ProvisionalCheckpointCapacity,
    expires: u64,
}
/// Short-lived inspection of unconfirmed candidate metadata after all current-scope checks.
/// A copied receipt remains raw data and cannot reconstruct the private discovery context.
#[derive(Debug)]
pub struct ProvisionalStudioHintUse<'a> {
    pub target: StudioTarget,
    pub peer: PeerId,
    pub provider: DeviceId,
    pub receipt: &'a Receipt,
}
impl<T: MeshTransport> PendingProvisionalStudioDiscovery<T> {
    pub async fn fetch(self) -> CompletedProvisionalStudioDiscovery {
        CompletedProvisionalStudioDiscovery {
            head: self.head.fetch().await,
            watch: self.watch,
            capacity: self.capacity,
            expires: self.expires,
        }
    }
}
impl<T: MeshTransport> fmt::Debug for PendingProvisionalStudioDiscovery<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingProvisionalStudioDiscovery { .. }")
    }
}
impl fmt::Debug for CompletedProvisionalStudioDiscovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedProvisionalStudioDiscovery { .. }")
    }
}
impl fmt::Debug for ProvisionalStudioHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProvisionalStudioHint { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Separately acquire preview-eligible capacity and send a fresh head request. An ordinary
    /// authoritative Hint cannot be converted into this job or donated a reserved fourth slot.
    pub fn prepare_provisional_studio_discovery(
        &mut self,
        peer: PeerId,
        watch: &StudioWatch,
    ) -> Result<PendingProvisionalStudioDiscovery<T>, SyncError> {
        if !self.studio_watch_is_current(watch) {
            return Err(SyncError::Unauthorized);
        }
        let capacity = self.reserve_provisional_checkpoint_capacity()?;
        let expires = self
            .clock
            .monotonic_ms()
            .checked_add(FETCH_MS)
            .ok_or(SyncError::Malformed)?;
        let head = self
            .prepare_checkpoint_head(peer, CheckpointTarget::Studio(watch.target))?
            .retaining(capacity.keepalive());
        Ok(PendingProvisionalStudioDiscovery {
            head,
            watch: watch.copy_binding(),
            capacity,
            expires,
        })
    }
    /// None means no usable hint: absent receipt, owner-proof or repair responses need normal
    /// authoritative discovery on its bounded retry schedule. They never mint a selection here.
    pub fn complete_provisional_studio_discovery(
        &mut self,
        completed: CompletedProvisionalStudioDiscovery,
    ) -> Result<Option<ProvisionalStudioHint>, SyncError> {
        if !self.studio_watch_is_current(&completed.watch)
            || self.clock.monotonic_ms() >= completed.expires
        {
            return Err(SyncError::Unauthorized);
        }
        Ok(self
            .complete_checkpoint_hint(completed.head)?
            .map(|head| ProvisionalStudioHint {
                head,
                watch: completed.watch,
                _capacity: completed.capacity,
                expires: completed.expires,
            }))
    }
    /// Revocation is immediate; memory capacity remains held until every custodian releases it.
    /// Any later head preparation for this target revokes the candidate, even if it later fails.
    pub fn provisional_studio_hint_is_current(&self, hint: &ProvisionalStudioHint) -> bool {
        self.studio_watch_is_current(&hint.watch)
            && self.clock.monotonic_ms() < hint.expires
            && self.checkpoint_hint_is_current(&hint.head)
    }
    pub fn with_provisional_studio_hint<O>(
        &self,
        hint: &ProvisionalStudioHint,
        inspect: impl FnOnce(ProvisionalStudioHintUse<'_>) -> O,
    ) -> Result<O, SyncError> {
        if !self.provisional_studio_hint_is_current(hint) {
            return Err(SyncError::Unauthorized);
        }
        Ok(inspect(ProvisionalStudioHintUse {
            target: hint.watch.target,
            peer: hint.head.peer(),
            provider: hint.head.provider(),
            receipt: hint.head.receipt(),
        }))
    }
}
