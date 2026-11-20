//! Original native invite-intent preparation and isolated durable coverage.
//! No key, SavedInvite, Add admission or invite publication is reachable here.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use catcoms_sync::owner_v2::{
    PreparedV2FounderInviteData, V2FounderInviteRequestData, FOUNDER_INVITE_DATA_BYTES,
    MAX_FOUNDER_INVITE_LIFETIME_MS,
};
mod journal;
use journal::InviteIntentJournal;

pub(super) struct PendingInvite {
    request: V2FounderInviteRequestData,
    // Retain exact public bytes so journal/publication never reconstructs the
    // plan later from a digest and potentially changed transport state.
    routing_plan: Vec<u8>,
    original: Option<PreparedV2FounderInviteData>,
    journal: InviteIntentJournal,
    // Last: the retained spec/preimage and all refusal overlap remain paid.
    fee: CatalogReadReservation,
}
const INVITE_BYTES: usize = FOUNDER_INVITE_DATA_BYTES
    + journal::JOURNAL_BYTES
    + 4 * std::mem::size_of::<PendingInvite>()
    + 4096;

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> PreparedV2TextHost<T, R> {
    #[cfg(test)]
    pub(in crate::store) fn fail_partial_invite_write_for_test() {
        journal::fail_partial_write_for_test();
    }
    /// Private owning native call-through. The route commitment is produced by
    /// the actual Runtime's transport reservation, never supplied by the UI.
    /// Returned hash/expiry are unsigned historical DATA, not a Saved invite.
    pub(in crate::store) fn prepare_native_invite_intent_data(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2FounderInviteRequestData,
        routing_plan: &[u8],
        keys: &mut NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<([u8; 32], u64), AppError> {
        keys.preflight(&self.origin)?;
        if routing_plan.is_empty() || routing_plan.len() > 1024 {
            return Err(refused()); // before hash/copy/draw
        }
        let route = catcoms_replication::chat::join_bindings::JoinPayloadKindData::RoutingPlan
            .commitment_data(routing_plan)
            .map_err(|_| refused())?;
        if route != request.public_routing_plan
            || request.token == [0; 32]
            || request.public_routing_plan == [0; 32]
            || request.lifetime_ms == 0
            || request.lifetime_ms > MAX_FOUNDER_INVITE_LIFETIME_MS
            || self
                .invite
                .as_ref()
                .is_some_and(|p| p.request != request || p.routing_plan != routing_plan)
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
                routing_plan: routing_plan.to_vec(),
                original: None,
                journal: InviteIntentJournal::default(),
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
        // Mandatory exact durable lookup precedes ANY new Clock/RNG draw.
        // A saved unsigned intent is DATA only: it never adopts a cold signer.
        let hit = {
            let owner = self.owner.borrow_current()?;
            self.member
                .as_mut()
                .ok_or_else(refused)?
                .with_invite_parent(store, &owner, |guard| {
                    keys.preflight(&self.origin)?;
                    p.journal.lookup(guard, request, routing_plan)
                })?
        };
        if let Some(result) = hit {
            keys.preflight(&self.origin)?;
            return Ok(result);
        }
        // This is outside every Current/Core writer. A late refusal leaves the
        // SAME original in this host before the RuntimeLoan is restored.
        self.owner
            .prepare_invite_data(store, request, &keys.original, &mut p.original)?;
        let PendingInvite {
            original, journal, ..
        } = p.as_mut();
        let (_, bytes) = original
            .as_ref()
            .ok_or_else(refused)?
            .unsigned_data()
            .map_err(AppError::from)?;
        journal.prepare_data(store, request, routing_plan, bytes, rng)?;
        let owner = self.owner.borrow_current()?;
        let result = self
            .member
            .as_mut()
            .ok_or_else(refused)?
            .with_invite_parent(store, &owner, |guard| {
                keys.preflight(&self.origin)?;
                journal.cover(guard, request, routing_plan, bytes)
            })?;
        keys.preflight(&self.origin)?;
        Ok(result)
    }
}
