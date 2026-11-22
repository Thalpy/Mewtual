//! Affine capture for the first accepted request on a mounted native vault.
use super::super::upgrade_source::owning::MetadataWork;
use super::*;
use crate::store::persistence::governance_layout::joined::{AcceptedFirstJoin, JoinedPendingIo};
use crate::store::NativeTextInvitePacketData;
use catcoms_replication::chat::join_bindings::GenesisAnchorBindingData;
use catcoms_rt::RequestCancellation;
use catcoms_sync::V2PendingJoinContextData;
use std::sync::Weak;

pub(super) type PendingJoin = Mutex<Option<JoinedPendingIo>>;

pub(super) fn invoice() -> usize {
    2 * std::mem::size_of::<NativeTextJoinedRequestPreparation>()
        + std::mem::size_of::<PendingJoin>()
        + 2 * std::mem::size_of::<usize>()
}

/// An original UI command and finite metadata slot. Keep the entire capture
/// outside native guards during IO or destruction, including on refusal.
pub struct NativeTextJoinedRequestPreparation {
    scope: Arc<Scope>,
    pending: Weak<PendingJoin>,
    command: NativeTextSourceCommand,
    work: Option<MetadataWork>,
    end: Option<retirement::PreparationEnd>,
    _control: Arc<retirement::Control>,
}

impl std::fmt::Debug for NativeTextJoinedRequestPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedRequestPreparation")
            .finish_non_exhaustive()
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSources<T, R> {
    /// Capture the actual mounted original without locking its book or opening
    /// files. A refused capture still owns the caller's UI command and fee.
    pub fn capture_join_request(
        &self,
        command: NativeTextSourceCommand,
    ) -> NativeTextJoinedRequestPreparation {
        let end = self.retirement.enter_preparation();
        let work = if end.is_some() && self.require_command(&command).is_ok() {
            self.work.metadata()
        } else {
            None
        };
        NativeTextJoinedRequestPreparation {
            scope: self.scope.clone(),
            pending: Arc::downgrade(&self.pending_join),
            command,
            work,
            end,
            _control: self.retirement.clone(),
        }
    }
}

impl NativeTextJoinedRequestPreparation {
    /// Cover the original request, leaving the affine warm Pending with the
    /// mount for a later exact retry. This receipt carries historical data.
    pub fn cover(
        self,
        invite: &NativeTextInvitePacketData<'_>,
        anchor: &GenesisAnchorBindingData,
        token: [u8; 32],
        now_ms: u64,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<([u8; 32], V2PendingJoinContextData), AppError> {
        let work = self.work.as_ref().ok_or_else(refused)?;
        let require_current = || {
            if self.end.is_none()
                || !Arc::ptr_eq(&self.scope, &self.command.scope)
                || !self.scope.is_open()
                || !self.command.ui.is_current_cached()
                || !work.is_live_cached()
                || negative.is_cancelled()
            {
                return Err(refused());
            }
            Ok(())
        };
        require_current()?;
        let original = self.pending.upgrade().ok_or_else(refused)?;
        let mut pending = original.try_lock().map_err(|_| refused())?;
        let accepted = AcceptedFirstJoin {
            invite,
            anchor,
            token,
            now_ms,
        };
        work.cover_accepted_first_join(&mut pending, accepted, require_current, rng)
    }
}
