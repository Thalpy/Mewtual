//! Original native invite-intent preparation. No key, journal write, Saved
//! receipt, Add admission or invite publication is reachable from this phase.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use catcoms_sync::owner_v2::{
    PreparedV2FounderInviteData, V2FounderInviteRequestData, FOUNDER_INVITE_DATA_BYTES,
    MAX_FOUNDER_INVITE_LIFETIME_MS,
};

pub(super) struct PendingInvite {
    request: V2FounderInviteRequestData,
    original: Option<PreparedV2FounderInviteData>,
    // Last: the retained spec/preimage and all refusal overlap remain paid.
    fee: CatalogReadReservation,
}
const INVITE_BYTES: usize =
    FOUNDER_INVITE_DATA_BYTES + 4 * std::mem::size_of::<PendingInvite>() + 4096;

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Private owning native call-through. The route commitment is produced by
    /// the actual Runtime's transport reservation, never supplied by the UI.
    /// Returned hash/expiry are unsigned historical DATA, not a Saved invite.
    pub(in crate::store) fn prepare_native_invite_intent_data(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2FounderInviteRequestData,
        keys: &mut NativeFounderKeys,
    ) -> Result<([u8; 32], u64), AppError> {
        keys.preflight(&self.origin)?;
        if request.token == [0; 32]
            || request.public_routing_plan == [0; 32]
            || request.lifetime_ms == 0
            || request.lifetime_ms > MAX_FOUNDER_INVITE_LIFETIME_MS
            || self.invite.as_ref().is_some_and(|p| p.request != request)
        {
            return Err(refused());
        }
        let store = layout.checked_store()?;
        // Reject an unsupported family before minting a member verifier or
        // excluding founder Create. This cached filter grants no permission;
        // the actual held4 proof still runs below on every preparation/retry.
        if !store
            .current_layout_policy
            .as_ref()
            .and_then(|p| p.native_registry.as_ref())
            .is_some_and(|handler| handler.is_governed())
        {
            return Err(refused());
        }
        self.ensure_member_lane(layout)?;
        {
            let owner = self.owner.borrow_current()?;
            self.member
                .as_mut()
                .ok_or_else(refused)?
                .require_invite_parent(store, &owner)?;
        }
        if self.invite.is_none() {
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(INVITE_BYTES)?;
            fee.validate(pool, INVITE_BYTES)?;
            self.invite = Some(Box::new(PendingInvite {
                request,
                original: None,
                fee,
            }));
        }
        let p = self.invite.as_mut().ok_or_else(refused)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        p.fee.validate(pool, INVITE_BYTES)?;
        // This is outside every Current/Core writer. A late refusal leaves the
        // SAME original in this host before the RuntimeLoan is restored.
        self.owner
            .prepare_invite_data(store, request, &keys.original, &mut p.original)?;
        let owner = self.owner.borrow_current()?;
        self.member
            .as_mut()
            .ok_or_else(refused)?
            .require_invite_parent(store, &owner)?;
        keys.preflight(&self.origin)?;
        let (spec, bytes) = p
            .original
            .as_ref()
            .ok_or_else(refused)?
            .unsigned_data()
            .map_err(AppError::from)?;
        Ok((*blake3::hash(bytes).as_bytes(), spec.expires_at_ms))
    }
}
