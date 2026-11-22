//! Original native invitation preparation, signing and isolated durable coverage.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::native_text_invite::PreparedNativeTextInvitePacket;
use crate::store::persistence::v2_text::PreparedV2TextParentCapture;
use crate::store::{NativeTextInviteRecovery, NativeTextSavedInvite};
use catcoms_replication::chat::join_bindings::SignedInviteBindingData;
use catcoms_rt::{NativeChatKeyEnds, RequestCancellation};
use catcoms_sync::owner_v2::{
    PreparedV2FounderInviteData, V2FounderInviteRequestData, V2GenesisSeedData,
    FOUNDER_INVITE_DATA_BYTES, MAX_FOUNDER_INVITE_LIFETIME_MS,
};
mod journal;
use journal::{InviteIntentJournal, InviteRecoveryRead, InviteSigningJournal};

#[cfg(test)]
std::thread_local! {
    static AFTER_INVITE_KEY: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static AFTER_INVITE_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

pub(super) struct PendingInvite {
    request: V2FounderInviteRequestData,
    // Retain exact public bytes so journal/publication never reconstructs the
    // plan later from a digest and potentially changed transport state.
    routing_plan: Vec<u8>,
    original: Option<PreparedV2FounderInviteData>,
    // Signing file handles drain before the intent journal's physical lease.
    signing: InviteSigningJournal,
    journal: InviteIntentJournal,
    // Last: the retained spec/preimage and all refusal overlap remain paid.
    fee: CatalogReadReservation,
}
const INVITE_BYTES: usize = FOUNDER_INVITE_DATA_BYTES
    + journal::JOURNAL_BYTES
    + journal::SIGNING_JOURNAL_BYTES
    + 4 * std::mem::size_of::<PendingInvite>()
    + 4096;

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> PreparedV2TextHost<T, R> {
    #[cfg(test)]
    pub(in crate::store) fn fail_partial_invite_write_for_test() {
        journal::fail_partial_write_for_test();
    }
    #[cfg(test)]
    pub(in crate::store) fn after_invite_key_for_test(hook: Box<dyn FnOnce()>) {
        AFTER_INVITE_KEY.with(|slot| assert!(slot.replace(Some(hook)).is_none()));
    }
    #[cfg(test)]
    pub(in crate::store) fn after_invite_read_for_test(hook: Box<dyn FnOnce()>) {
        AFTER_INVITE_READ.with(|slot| assert!(slot.replace(Some(hook)).is_none()));
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
                signing: InviteSigningJournal::default(),
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

    /// The exact-request lookup and signature coverage share one RuntimeLoan.
    pub(in crate::store) fn save_native_invite_data(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2FounderInviteRequestData,
        routing_plan: &[u8],
        keys: &mut NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &RequestCancellation,
    ) -> Result<NativeTextSavedInvite, AppError> {
        self.prepare_native_invite_intent_data(layout, request, routing_plan, keys, rng)?;
        self.cover_native_invite_signature(layout, keys, rng, negative)
    }

    /// Independently inspect the files using the real current owner. This
    /// does not initialize the Host's member lane, intent or signing state.
    pub(in crate::store) fn inspect_native_invite_data(
        &self,
        layout: &V2CurrentLayoutStore,
        request: V2FounderInviteRequestData,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        negative: &RequestCancellation,
    ) -> Result<NativeTextInviteRecovery, AppError> {
        self.read_native_invite(layout, keys, negative, |read, genesis| {
            read.authenticate(request, routing_plan, genesis)
        })
    }

    /// Match the exact received invitation to the actual persisted signed
    /// original. This is a negative gate before a separate Add transaction.
    pub(in crate::store) fn require_received_invite(
        &self,
        layout: &V2CurrentLayoutStore,
        incoming: &SignedInviteBindingData,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        negative: &RequestCancellation,
    ) -> Result<(), AppError> {
        self.read_native_invite(layout, keys, negative, |read, genesis| {
            read.require_received_invite(incoming, routing_plan, genesis)
        })?;
        self.require_member_quiescent_for_add(layout)
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn cover_governed_add_fence_for_test(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &catcoms_sync::V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &RequestCancellation,
        [sdk_cut, header_cut, welcome_cut]: [catcoms_rt::NativeChatPreparedKeyCut; 3],
        mut inject_outcome: impl FnMut(bool),
    ) -> Result<(), AppError> {
        use crate::store::persistence::v2_text::{
            GovernedAddRecoveryState, PreparedGovernedAddFence,
        };
        self.require_received_invite(
            layout,
            packet.founder_add_data().invite,
            routing_plan,
            keys,
            negative,
        )?;
        let store = layout.checked_store()?;
        let sdk_admission = super::governed_add::prepare_sdk_admission(store)?;
        assert!(sdk_admission.supports_routing_data());
        let owner = self.owner.borrow_current()?;
        let original_seed = owner.source().seed().observe_original_seed_data();
        let mut parent = PreparedV2TextParentCapture::prepare(store, owner.source())?;
        let resources = parent.resources().clone();
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            parent.capture(&guard)?.genesis_member_anchor_data(&guard)?;
            Ok(())
        })?;
        let selected = parent.selected_data().ok_or_else(refused)?;
        let mut fence =
            PreparedGovernedAddFence::prepare(store, owner.source(), selected, packet, rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            keys.preflight(&self.origin)?;
            fence.cover(&guard, selected)
        })?;
        self.owner.prepare_governed_node(store)?;
        let owner = self.owner.borrow_current()?;
        original_seed.require_same_seed_data(owner.source().seed())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            keys.preflight(&self.origin)?;
            fence.cover(&guard, selected)?;
            assert!(fence.require_covered_sdk_entry(&guard, selected).is_err());
            Ok(())
        })?;
        let read_fence =
            |fence: &PreparedGovernedAddFence, owner: &super::super::owner::V2TextOwnerRef<'_>| {
                let mut reader =
                    fence.prepare_recovery_read_for_test(store, owner.source(), selected)?;
                store.with_v2_text_current(owner, |admission| {
                    let guard = store.lock_v2_text_current_group(admission, &resources)?;
                    reader.capture(&guard, selected)
                })?;
                reader.authenticate(store, owner.source())?;
                Ok::<_, AppError>(reader)
            };
        let mut before_entry = read_fence(&fence, &owner)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert_eq!(
                before_entry.require_recovery_state(&guard, selected)?,
                GovernedAddRecoveryState::ReservationOnly
            );
            Ok(())
        })?;
        fence.prepare_sdk_entry(store, owner.source(), rng)?;
        for _ in 0..2 {
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                keys.preflight(&self.origin)?;
                fence.cover_sdk_entry(&guard, selected)
            })?;
        }
        let mut after_entry = read_fence(&fence, &owner)?;
        after_entry.prepare_outcome_read(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(before_entry
                .require_recovery_state(&guard, selected)
                .is_err());
            assert_eq!(
                after_entry.require_recovery_state(&guard, selected)?,
                GovernedAddRecoveryState::PossiblyEntered
            );
            after_entry.capture_outcome(&guard, selected)
        })?;
        after_entry.authenticate_outcome(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(after_entry
                .require_outcome_current(&guard, selected)?
                .is_none());
            Ok(())
        })?;
        inject_outcome(true);
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(after_entry
                .require_outcome_current(&guard, selected)
                .is_err());
            Ok(())
        })?;
        let mut with_outcome = read_fence(&fence, &owner)?;
        with_outcome.prepare_outcome_read(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            with_outcome.capture_outcome(&guard, selected)
        })?;
        assert!(with_outcome
            .authenticate_outcome(store, owner.source())
            .is_err());
        inject_outcome(false);

        self.require_member_quiescent_for_add(layout)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            keys.preflight(&self.origin)?;
            fence.require_covered_sdk_entry(&guard, selected)
        })?;
        let replay = self
            .member
            .as_ref()
            .ok_or_else(refused)?
            .governed_add_replay_for_test()?;
        let node = self.owner.governed_node_for_test(store)?;
        node.prepare_add_data(
            packet.founder_add_data(),
            keys.original.clone(),
            sdk_admission,
            sdk_cut,
            header_cut,
            welcome_cut,
        )
        .map_err(|_| refused())?;
        node.execute_add_data_native(packet.founder_add_data())?;
        node.prepare_admission_data(replay)?;
        drop(node.retire_closed_add_native_data()?);

        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_for_test(store)?;
        fence.prepare_outcome(store, owner.source(), selected, packet, &outcome, rng)?;
        for _ in 0..2 {
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                keys.preflight(&self.origin)?;
                fence.cover_outcome(&guard, selected)
            })?;
        }
        let mut verified = read_fence(&fence, &owner)?;
        verified.prepare_outcome_read(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            verified.capture_outcome(&guard, selected)
        })?;
        verified.authenticate_outcome(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            let plaintext = verified
                .require_outcome_current(&guard, selected)?
                .ok_or_else(refused)?;
            assert!(plaintext.ends_with(outcome.result_data().secret_snapshot_data()));
            Ok(())
        })?;
        Ok(())
    }

    fn read_native_invite<U>(
        &self,
        layout: &V2CurrentLayoutStore,
        keys: &NativeFounderKeys,
        negative: &RequestCancellation,
        inspect: impl FnOnce(&InviteRecoveryRead<'_>, V2GenesisSeedData<'_>) -> Result<U, AppError>,
    ) -> Result<U, AppError> {
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        require_live()?;
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        let mut read = InviteRecoveryRead::prepare(store)?;
        let mut capture = PreparedV2TextParentCapture::prepare(store, owner.source())?;
        let resources = capture.resources().clone();
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.require_governed_native_invite_parent()?;
            capture.capture(&guard)?;
            let selected = capture.selected_data().ok_or_else(refused)?;
            selected.genesis_member_anchor_data(&guard)?;
            require_live()?;
            read.capture(&guard)
        })?;
        let status = inspect(&read, owner.source().data())?;
        #[cfg(test)]
        if let Some(hook) = AFTER_INVITE_READ.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.require_governed_native_invite_parent()?;
            let selected = capture.selected_data().ok_or_else(refused)?;
            selected.genesis_member_anchor_data(&guard)?;
            guard.require_governed_native_invite_parent()?;
            read.require_current(&guard)?;
            require_live()
        })?;
        require_live()?;
        Ok(status)
    }

    fn cover_native_invite_signature(
        &mut self,
        layout: &V2CurrentLayoutStore,
        keys: &mut NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &RequestCancellation,
    ) -> Result<NativeTextSavedInvite, AppError> {
        let origin = &self.origin;
        let owned = &self.owner;
        let require_live = |keys: &NativeFounderKeys| {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(origin)
        };
        require_live(keys)?;
        let store = layout.checked_store()?;
        let p = self.invite.as_mut().ok_or_else(refused)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        p.fee.validate(pool, INVITE_BYTES)?;
        let original = p.original.as_mut().ok_or_else(refused)?;
        let owner = owned.borrow_current()?;
        let member = self.member.as_mut().ok_or_else(refused)?;
        if original.raw_signature_data().is_err() {
            owned.preflight_invite_signing(store, original)?;
            p.signing.prepare_attempt(&p.journal, store, rng)?;
            member.with_invite_parent(store, &owner, |guard| {
                require_live(keys)?;
                p.signing.cover_attempt(&mut p.journal, guard)
            })?;
            // The marker's covering IO and its late Current checks must
            // complete before the key cut. Drain a partial End outside the
            // writers and before another RuntimeLoan can retry this original.
            let mut ends = NativeChatKeyEnds::new();
            member.with_invite_parent(store, &owner, |guard| {
                p.signing.require_attempt(&mut p.journal, guard)?;
                require_live(keys)?;
                owned.sign_invite(store, original, keys.invite_cut(), &mut ends)
            })?;
            drop(ends);
            #[cfg(test)]
            if let Some(hook) = AFTER_INVITE_KEY.with(|slot| slot.borrow_mut().take()) {
                hook();
            }
        }
        original.materialize_data().map_err(AppError::from)?;
        let signed = original.signed_data().map_err(AppError::from)?;
        p.signing.prepare_raw(
            &p.journal,
            store,
            original.raw_signature_data().map_err(AppError::from)?,
            rng,
        )?;
        // Pay and assemble outside writers. A packet assembly refusal must
        // still let this pass cover the exact raw already returned by Sync.
        let result = original
            .policy_bytes_data()
            .map_err(AppError::from)
            .and_then(|policy| {
                PreparedNativeTextInvitePacket::prepare(pool, signed, policy, &p.routing_plan)
            });
        member.with_invite_parent(store, &owner, |guard| {
            require_live(keys)?;
            p.signing.cover_raw(&mut p.journal, guard)
        })?;
        require_live(keys)?;
        result.map(PreparedNativeTextInvitePacket::into_saved)
    }
}
