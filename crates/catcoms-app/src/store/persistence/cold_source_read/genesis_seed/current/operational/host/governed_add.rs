//! Payments for the original native governed Add candidate. The candidate
//! keeps this debit for as long as its SDK graph or pending output owns it.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::persistence::v2_text::{
    prepare_sdk_admission, PreparedGovernedAddFence, PreparedGovernedRemoveFence,
    PreparedV2TextParentCapture, TextHeadIdData,
};
use crate::store::ServerStore;
use catcoms_mls::persist::text_current_add::TextCurrentAddAdmission;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_sync::{
    owner_v2::V2InstalledFounderAddData, V2JoinRequestPacketData, V2MemberAddRequestData,
    MAX_V2_JOIN_REQUEST_PACKET_BYTES,
};
use std::sync::Arc;

pub(super) struct PendingGovernedAdd {
    // Physical write handles drain before the associated selected capture.
    pub(super) fence: PreparedGovernedAddFence,
    pub(super) removal: Option<Box<PreparedGovernedRemoveFence>>,
    pub(super) parent: Arc<PreparedV2TextParentCapture>,
    pub(super) sdk: Option<TextCurrentAddAdmission>,
    core: Option<catcoms_replication::chat::V2GeneralCandidate>,
    pub(super) installed: Option<(TextHeadIdData, V2InstalledFounderAddData)>,
    request: Vec<u8>,
    fee: CatalogReadReservation,
}

impl PendingGovernedAdd {
    pub(super) fn require_selected_member_head(
        &mut self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
        replay: &catcoms_replication::chat::owner_state::transition::ReplayMath,
    ) -> Result<(), AppError> {
        let head = self.require_selected_replay(replay)?;
        if let Some(removal) = &mut self.removal {
            return removal.require_selected(guard, self.installed.ok_or_else(refused)?);
        }
        self.fence.require_selected_head2(
            guard,
            self.parent.selected_data().ok_or_else(refused)?,
            head,
        )
    }

    pub(super) fn require_selected_review_anchor(
        &mut self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
        replay: &catcoms_replication::chat::owner_state::transition::ReplayMath,
        anchor: (OwnerHeaderId, u64),
    ) -> Result<(), AppError> {
        let selected = (replay.header().id(), replay.mls_epoch_data());
        if anchor.1 == 0 || anchor.1 > selected.1 {
            return Err(refused());
        }
        self.require_selected_member_head(guard, replay)?;
        if anchor == selected {
            return Ok(());
        }
        if let Some(removal) = &mut self.removal {
            removal.require_warm_review_parent_anchor(guard, anchor)?;
        } else {
            self.fence
                .require_warm_review_parent_anchor(guard, anchor)?;
        }
        self.require_selected_member_head(guard, replay)
    }

    pub(super) fn require_selected_replay(
        &self,
        replay: &catcoms_replication::chat::owner_state::transition::ReplayMath,
    ) -> Result<TextHeadIdData, AppError> {
        let (head, installed) = self.installed.ok_or_else(refused)?;
        if replay.header().id() != installed.header
            || replay.header().spec().state != installed.state
            || replay.mls_epoch_data() != installed.epoch
        {
            return Err(refused());
        }
        Ok(head)
    }

    pub(super) fn prepare(
        store: &ServerStore,
        owner: &super::super::owner::V2TextOwnerRef<'_>,
        parent: Arc<PreparedV2TextParentCapture>,
        packet: &V2JoinRequestPacketData<'_>,
        rng: &mut dyn catcoms_rt::CryptoRngCore,
        installed: Option<((TextHeadIdData, V2InstalledFounderAddData), [u8; 32])>,
    ) -> Result<Self, AppError> {
        let frame = packet.packet_bytes_data();
        let selected = parent.selected_data().ok_or_else(refused)?;
        if frame.is_empty() || frame.len() > MAX_V2_JOIN_REQUEST_PACKET_BYTES {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let bytes = 4 * std::mem::size_of::<Self>() + frame.len() + 4096;
        let fee = pool.try_reserve(bytes)?;
        fee.validate(pool, bytes)?;
        let sdk = prepare_sdk_admission(store)?;
        let fence = if let Some((installed, fetch_verifier)) = installed {
            PreparedGovernedAddFence::prepare_later(
                store,
                owner.source(),
                selected,
                packet,
                (installed, fetch_verifier),
                rng,
            )?
        } else {
            PreparedGovernedAddFence::prepare(store, owner.source(), selected, packet, rng)?
        };
        Ok(Self {
            fence,
            removal: None,
            parent,
            sdk: Some(sdk),
            core: None,
            installed: None,
            request: frame.to_vec(),
            fee,
        })
    }

    pub(super) fn require_request(
        &self,
        store: &ServerStore,
        packet: &V2JoinRequestPacketData<'_>,
    ) -> Result<(), AppError> {
        if !self.matches_request(packet) {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.fee.validate(
            pool,
            4 * std::mem::size_of::<Self>() + self.request.len() + 4096,
        )
    }

    pub(super) fn matches_request(&self, packet: &V2JoinRequestPacketData<'_>) -> bool {
        self.request == packet.packet_bytes_data()
    }
}

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn encode_selected_member_add(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: &V2MemberAddRequestData,
        keys: &NativeFounderKeys,
        caller: &catcoms_rt::RequestCancellation,
    ) -> Result<Vec<u8>, AppError> {
        let require_live = || {
            if caller.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        require_live()?;
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        let verifier = self
            .member
            .as_ref()
            .ok_or_else(refused)?
            .selected_invite_verifier()?;
        let pending = self.governed_add.as_mut().ok_or_else(refused)?;
        let head = pending.require_selected_replay(verifier.replay_data())?;
        let selected = pending.parent.selected_data().ok_or_else(refused)?;
        let frame = store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, pending.parent.resources())?;
            require_live()?;
            if let Some(removal) = &mut pending.removal {
                return removal.encode_selected_member_transition(
                    store,
                    &guard,
                    pending.installed.ok_or_else(refused)?,
                    request,
                );
            }
            pending
                .fence
                .encode_selected_member_add(store, &guard, selected, head, request)
        })?;
        require_live()?;
        Ok(frame)
    }

    pub(in crate::store) fn handle_received_join_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        (keys, cuts): (
            &NativeFounderKeys,
            [catcoms_rt::NativeChatPreparedKeyCut; 3],
        ),
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<crate::store::NativeTextSavedJoinResponse, AppError> {
        if self
            .governed_add
            .as_ref()
            .is_some_and(|pending| pending.removal.is_some() && pending.matches_request(packet))
        {
            return self.saved_governed_response(layout, packet, keys, negative);
        }
        if self.governed_removal.is_some() {
            return Err(refused());
        }
        let continuing_selection = self.governed_add.as_mut().is_some_and(|pending| {
            pending.matches_request(packet) && pending.fence.head_selection_entered()
        });
        if !continuing_selection {
            self.execute_governed_add_sdk(
                layout,
                packet,
                routing_plan,
                (keys, cuts),
                rng,
                negative,
            )?;
        }
        self.install_governed_add(layout, packet, routing_plan, keys, rng, negative)?;
        self.saved_governed_response(layout, packet, keys, negative)
    }

    pub(super) fn saved_governed_response(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        keys: &NativeFounderKeys,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<crate::store::NativeTextSavedJoinResponse, AppError> {
        let store = layout.checked_store()?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        p.require_request(store, packet)?;
        let (head, _) = p.installed.ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, p.parent.resources())?;
            require_live()?;
            let response = if let Some(removal) = &mut p.removal {
                removal.require_selected(&guard, p.installed.ok_or_else(refused)?)?;
                p.fence.cached_archived_response_data()?
            } else {
                p.fence.saved_response_data(&guard, selected, head)?
            };
            require_live()?;
            Ok(response)
        })
    }

    pub(super) fn install_governed_add(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<TextHeadIdData, AppError> {
        let store = layout.checked_store()?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        p.require_request(store, packet)?;
        if let Some((head, _)) = p.installed {
            let selected = p.parent.selected_data().ok_or_else(refused)?;
            let owner = self.owner.borrow_current()?;
            return store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, p.parent.resources())?;
                keys.preflight(&self.origin)?;
                p.fence.require_selected_head2(&guard, selected, head)?;
                keys.preflight(&self.origin)?;
                if negative.is_cancelled() {
                    return Err(refused());
                }
                Ok(head)
            });
        }
        if !p.fence.head_selection_entered() {
            self.stage_governed_head(layout, packet, routing_plan, keys, rng, negative)?;
        }
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let verifier = self
            .member
            .as_mut()
            .ok_or_else(refused)?
            .governed_add_verifier()?;
        if p.core.is_none() {
            p.core = Some(self.owner.derive_governed_core_selection(store, verifier)?);
        }
        let (owner, install) = self.owner.borrow_governed_install_parts(store)?;
        install.check_host_selection_data(verifier, p.core.as_ref().ok_or_else(refused)?)?;
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        let result = store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, p.parent.resources())?;
            require_live()?;
            install.check_host_selection_data(verifier, p.core.as_ref().ok_or_else(refused)?)?;
            let fingerprint = install.expected_data();
            let head = p.fence.select_head2(&guard, selected, fingerprint)?;
            #[cfg(test)]
            AFTER_GOVERNED_HEAD_SAVE.with(|hook| {
                let hook = hook.borrow_mut().take();
                if let Some(hook) = hook {
                    hook();
                }
            });
            require_live()?;
            install.finish_host_selected(verifier, &mut p.core)?;
            p.installed = Some((head, fingerprint));
            Ok(head)
        });
        if p.installed.is_some() {
            drop(self.owner.take_retired_governed_install()?);
        }
        result
    }

    pub(super) fn stage_governed_head(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<TextHeadIdData, AppError> {
        self.cover_governed_archive(layout, packet, routing_plan, keys, rng, negative)?;
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        p.fence
            .prepare_head_stage(store, owner.source(), selected, rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, p.parent.resources())?;
            keys.preflight(&self.origin)?;
            if negative.is_cancelled() {
                return Err(refused());
            }
            let head = p.fence.cover_head_stage(&guard, selected)?;
            keys.preflight(&self.origin)?;
            if negative.is_cancelled() {
                return Err(refused());
            }
            Ok(head)
        })
    }

    pub(super) fn cover_governed_archive(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<catcoms_replication::chat::cold_manifest::ArchiveHeadId, AppError> {
        self.cover_governed_response(layout, packet, routing_plan, keys, rng, negative)?;
        let store = layout.checked_store()?;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        p.fence
            .prepare_archive(store, owner.source(), selected, &outcome, rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, p.parent.resources())?;
            keys.preflight(&self.origin)?;
            if negative.is_cancelled() {
                return Err(refused());
            }
            let id = p.fence.cover_archive(&guard, selected)?;
            keys.preflight(&self.origin)?;
            if negative.is_cancelled() {
                return Err(refused());
            }
            Ok(id)
        })
    }

    pub(super) fn cover_governed_response(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.sign_governed_welcome(layout, packet, routing_plan, keys, rng, negative)?;
        let store = layout.checked_store()?;
        let preimage = self.owner.freeze_governed_welcome(store)?;
        self.owner.materialize_governed_response(store)?;
        let signed_header = self.owner.materialize_governed_header(store)?;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let response = self.owner.governed_response_data(store)?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        p.fence.prepare_response(
            store,
            owner.source(),
            &preimage,
            &outcome,
            (&signed_header, &response),
            rng,
        )?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, p.parent.resources())?;
            keys.preflight(&self.origin)?;
            if negative.is_cancelled() {
                return Err(refused());
            }
            p.fence.cover_response(&guard, selected)?;
            keys.preflight(&self.origin)?;
            if negative.is_cancelled() {
                return Err(refused());
            }
            Ok(())
        })
    }

    pub(super) fn sign_governed_welcome(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_governed_welcome_intent(layout, packet, routing_plan, keys, rng, negative)?;
        let store = layout.checked_store()?;
        let preimage = self.owner.freeze_governed_welcome(store)?;
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let resources = p.parent.resources();
        if self.owner.cached_governed_welcome_raw(store)?.is_none() {
            let owner = self.owner.borrow_current()?;
            p.fence
                .prepare_welcome_attempt(store, owner.source(), rng)?;
            let (owner, mut primitive) = self.owner.borrow_governed_signing_parts()?;
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, resources)?;
                require_live()?;
                p.fence.cover_welcome_attempt(&guard, selected)?;
                require_live()?;
                primitive.sign_welcome_native()?;
                require_live()
            })?;
        }
        let raw = self
            .owner
            .cached_governed_welcome_raw(store)?
            .ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        p.fence
            .prepare_raw_welcome(store, owner.source(), &preimage, &raw, rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, resources)?;
            p.fence.cover_raw_welcome(&guard, selected)?;
            require_live()
        })?;
        require_live()
    }

    /// Freeze and cover the exact response preimage from the original Cell
    /// and Admission before any Welcome key entry.
    pub(super) fn cover_governed_welcome_intent(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.cover_governed_history(layout, packet, routing_plan, keys, rng, negative)?;
        let store = layout.checked_store()?;
        let preimage = self.owner.freeze_governed_welcome(store)?;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let resources = p.parent.resources();
        p.fence
            .prepare_welcome_intent(store, owner.source(), packet, &outcome, &preimage, rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, resources)?;
            p.fence.cover_welcome_intent(&guard, selected)?;
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        })
    }

    pub(super) fn cover_governed_history(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.sign_governed_header(layout, packet, routing_plan, keys, rng, negative)?;
        let store = layout.checked_store()?;
        let header = self.owner.materialize_governed_header(store)?;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let resources = p.parent.resources();
        p.fence
            .prepare_governing_record(store, owner.source(), &outcome, &header, rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, resources)?;
            p.fence.cover_governing_record(&guard, selected)?;
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        })
    }

    /// Key use follows exact Outcome/Attempt coverage under the actual Current
    /// and original native source. The Sync loan caches the result in place.
    pub(super) fn sign_governed_header(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.reserve_governed_add(layout, packet, routing_plan, keys, rng, negative)?;
        let store = layout.checked_store()?;
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let resources = p.parent.resources().clone();
        if self.owner.cached_governed_header_raw(store)?.is_none() {
            let owner = self.owner.borrow_current()?;
            p.fence.prepare_header_attempt(store, owner.source(), rng)?;
            let (owner, mut primitive) = self.owner.borrow_governed_signing_parts()?;
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                require_live()?;
                p.fence.cover_header_attempt(&guard, selected)?;
                require_live()?;
                primitive.sign_header_native()?;
                require_live()
            })?;
        }

        let raw = self
            .owner
            .cached_governed_header_raw(store)?
            .ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        p.fence
            .prepare_raw_header(store, owner.source(), &outcome, &raw, rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            p.fence.cover_raw_header(&guard, selected)?;
            require_live()
        })?;
        require_live()
    }

    /// The same original runtime retains the request, paid SDK input, Entry,
    /// SDK Cell and outcome. Any SDK work follows the original durable Entry.
    /// The caller still owns the remaining signing and selection transaction.
    pub(super) fn execute_governed_add_sdk(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        (keys, cuts): (
            &NativeFounderKeys,
            [catcoms_rt::NativeChatPreparedKeyCut; 3],
        ),
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.reserve_governed_add(layout, packet, routing_plan, keys, rng, negative)?;
        self.require_member_quiescent_for_add(layout)?;
        let store = layout.checked_store()?;
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        let replay = self
            .member
            .as_ref()
            .ok_or_else(refused)?
            .governed_add_replay()?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        p.require_request(store, packet)?;
        require_live()?;
        self.owner.prepare_governed_node(store)?;
        self.owner.prepare_governed_sdk(
            store,
            packet.founder_add_data(),
            keys.original.clone(),
            &mut p.sdk,
            cuts,
        )?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let resources = p.parent.resources().clone();
        let owner = self.owner.borrow_current()?;
        p.fence.prepare_sdk_entry(store, owner.source(), rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            require_live()?;
            p.fence.cover_sdk_entry(&guard, selected)?;
            require_live()
        })?;
        self.owner
            .run_governed_sdk(store, packet.founder_add_data(), replay)?;
        let owner = self.owner.borrow_current()?;
        if !p.fence.has_prepared_outcome_data() {
            let outcome = self.owner.governed_outcome_data(store)?;
            p.fence
                .prepare_outcome(store, owner.source(), selected, packet, &outcome, rng)?;
        }
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            p.fence.cover_outcome(&guard, selected)?;
            require_live()
        })?;
        require_live()
    }

    pub(in crate::store) fn retire_add_for_closed_ui(
        &mut self,
        closed: &[Option<&catcoms_rt::NativeChatKeyDrain>],
    ) -> Result<(), AppError> {
        self.owner.retire_add_for_closed_ui(closed)
    }

    /// Whole native Source retirement has exclusive custody of this idle host.
    /// It must retire existing Add Ends before waiting for drained keys.
    pub(in crate::store) fn discard_add_for_closed_source(
        &mut self,
        closed: &catcoms_rt::NativeChatKeyDrain,
    ) -> Result<(), AppError> {
        self.owner.discard_add_for_closed_source(closed)
    }
}

#[cfg(test)]
std::thread_local! {
    pub(super) static AFTER_GOVERNED_HEAD_SAVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
