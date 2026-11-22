//! Real native governed Add integration fixture.
use super::super::super::owner::V2TextOwnerRef;
use super::*;

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn cover_governed_add_fence_for_test(
        &mut self,
        layout: &V2CurrentLayoutStore,
        packet: &catcoms_sync::V2JoinRequestPacketData<'_>,
        replacement: &catcoms_sync::V2JoinRequestPacketData<'_>,
        routing_plan: &[u8],
        keys: &NativeFounderKeys,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &RequestCancellation,
        cuts: [catcoms_rt::NativeChatPreparedKeyCut; 3],
        mut inject: impl FnMut(&str, bool),
    ) -> Result<(), AppError> {
        use crate::store::persistence::v2_text::{
            GovernedAddHeaderRecoveryState, GovernedAddRecoveryState,
            GovernedAddWelcomeRecoveryState, HeldGovernedAddFence, PreparedGovernedAddFence,
            SelectedV2TextHead,
        };
        self.reserve_governed_add(layout, packet, routing_plan, keys, rng, negative)?;
        assert!(self
            .reserve_governed_add(layout, replacement, routing_plan, keys, rng, negative)
            .is_err());
        inject("invite.raw.v1", true);
        assert!(self
            .reserve_governed_add(layout, packet, routing_plan, keys, rng, negative)
            .is_err());
        inject("invite.raw.v1", false);
        self.reserve_governed_add(layout, packet, routing_plan, keys, rng, negative)?;
        self.require_member_quiescent_for_add(layout)?;
        let store = layout.checked_store()?;
        let original = self.governed_add.as_mut().ok_or_else(refused)?;
        assert!(original
            .sdk
            .as_ref()
            .ok_or_else(refused)?
            .supports_routing_data());
        let owner = self.owner.borrow_current()?;
        let original_seed = owner.source().seed().observe_original_seed_data();
        let resources = original.parent.resources().clone();
        let selected = original.parent.selected_data().ok_or_else(refused)?;
        let fence = &mut original.fence;
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
        let read_fence = |fence: &PreparedGovernedAddFence,
                          owner: &V2TextOwnerRef<'_>,
                          selected: &SelectedV2TextHead| {
            let mut reader =
                fence.prepare_recovery_read_for_test(store, owner.source(), selected)?;
            store.with_v2_text_current(owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, selected.resources())?;
                reader.capture(&guard, selected)
            })?;
            reader.authenticate(store, owner.source())?;
            Ok::<_, AppError>(reader)
        };
        let mut before_entry = read_fence(fence, &owner, selected)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert_eq!(
                before_entry.require_request_current(&guard, selected)?,
                Some(packet.packet_bytes_data())
            );
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
        let mut after_entry = read_fence(fence, &owner, selected)?;
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
        inject("text.add.outcome.v2", true);
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(after_entry
                .require_outcome_current(&guard, selected)
                .is_err());
            Ok(())
        })?;
        let mut with_outcome = read_fence(fence, &owner, selected)?;
        with_outcome.prepare_outcome_read(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            with_outcome.capture_outcome(&guard, selected)
        })?;
        assert!(with_outcome
            .authenticate_outcome(store, owner.source())
            .is_err());
        inject("text.add.outcome.v2", false);
        self.execute_governed_add_sdk(layout, packet, routing_plan, keys, cuts, rng, negative)?;
        let original = self.governed_add.as_mut().ok_or_else(refused)?;
        let resources = original.parent.resources().clone();
        let selected = original.parent.selected_data().ok_or_else(refused)?;
        let fence = &mut original.fence;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let mut verified = read_fence(fence, &owner, selected)?;
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

        fence.prepare_header_attempt(store, owner.source(), rng)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            keys.preflight(&self.origin)?;
            fence.cover_header_attempt(&guard, selected)?;
            verified.capture_header(&guard, selected)
        })?;
        verified.authenticate_header(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert_eq!(
                verified.require_header_current(&guard, selected)?,
                GovernedAddHeaderRecoveryState::PossiblyEntered
            );
            keys.preflight(&self.origin)?;
            fence.cover_header_attempt(&guard, selected)
        })?;
        self.sign_governed_header(layout, packet, routing_plan, keys, rng, negative)?;
        let original = self.governed_add.as_mut().ok_or_else(refused)?;
        let selected = original.parent.selected_data().ok_or_else(refused)?;
        let resources = original.parent.resources().clone();
        let fence = &mut original.fence;
        let raw = self
            .owner
            .cached_governed_header_raw(store)?
            .ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let mut foreign = raw;
        foreign[0] ^= 1;
        assert!(fence
            .prepare_raw_header(store, owner.source(), &outcome, &foreign, rng)
            .is_err());
        fence.prepare_raw_header(store, owner.source(), &outcome, &raw, rng)?;
        for _ in 0..2 {
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                assert!(fence.cover_header_attempt(&guard, selected).is_err());
                fence.cover_raw_header(&guard, selected)?;
                assert!(verified.require_header_current(&guard, selected).is_err());
                Ok(())
            })?;
        }
        let mut fresh = read_fence(fence, &owner, selected)?;
        fresh.prepare_outcome_read(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            fresh.capture_outcome(&guard, selected)
        })?;
        fresh.authenticate_outcome(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            fresh.capture_header(&guard, selected)
        })?;
        fresh.authenticate_header(store, owner.source())?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert_eq!(
                fresh.require_header_current(&guard, selected)?,
                GovernedAddHeaderRecoveryState::SignatureRecovered(raw)
            );
            Ok(())
        })?;
        let admitted = self.owner.materialize_governed_header(store)?;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let read_record = |fence: &PreparedGovernedAddFence,
                           owner: &V2TextOwnerRef<'_>,
                           selected: &SelectedV2TextHead| {
            let mut reader = read_fence(fence, owner, selected)?;
            reader.prepare_outcome_read(store, owner.source())?;
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                reader.capture_outcome(&guard, selected)
            })?;
            reader.authenticate_outcome(store, owner.source())?;
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                reader.capture_header(&guard, selected)
            })?;
            reader.authenticate_header(store, owner.source())?;
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                reader.capture_governing_record(&guard, selected)
            })?;
            reader.authenticate_governing_record(store, owner.source())?;
            Ok::<_, AppError>(reader)
        };
        let mut before_record = read_record(fence, &owner, selected)?;
        let read_welcome = |fence: &PreparedGovernedAddFence,
                            owner: &V2TextOwnerRef<'_>,
                            selected: &SelectedV2TextHead| {
            let mut reader = read_record(fence, owner, selected)?;
            store.with_v2_text_current(owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                reader.capture_welcome(&guard, selected)
            })?;
            assert!(reader
                .authenticate_welcome(store, owner.source(), replacement)
                .is_err());
            reader.authenticate_welcome(store, owner.source(), packet)?;
            Ok::<_, AppError>(reader)
        };
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(before_record
                .require_governing_current(&guard, selected)?
                .is_none());
            Ok(())
        })?;
        assert!(fence
            .prepare_governing_record(
                store,
                owner.source(),
                &outcome,
                owner.source().data().header,
                rng
            )
            .is_err());
        for _ in 0..2 {
            self.cover_governed_history(layout, packet, routing_plan, keys, rng, negative)?;
        }
        let original = self.governed_add.as_ref().ok_or_else(refused)?;
        let selected = original.parent.selected_data().ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        let mut saved_record = read_record(&original.fence, &owner, selected)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(before_record
                .require_governing_current(&guard, selected)
                .is_err());
            assert_eq!(
                saved_record.require_governing_current(&guard, selected)?,
                Some(&admitted)
            );
            Ok(())
        })?;
        drop((before_record, saved_record, fresh, verified));

        let mut changed = self.owner.freeze_governed_welcome(store)?;
        *changed.last_mut().unwrap() ^= 1;
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let p = self.governed_add.as_mut().ok_or_else(refused)?;
        assert!(p
            .fence
            .prepare_welcome_intent(store, owner.source(), packet, &outcome, &changed, rng)
            .is_err());
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let mut before_intent = read_welcome(&p.fence, &owner, selected)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert_eq!(
                before_intent.require_welcome_current(&guard, selected)?,
                GovernedAddWelcomeRecoveryState::GoverningOnly
            );
            Ok(())
        })?;
        for _ in 0..2 {
            self.cover_governed_welcome_intent(layout, packet, routing_plan, keys, rng, negative)?;
        }
        let p = self.governed_add.as_ref().ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let mut intent_only = read_welcome(&p.fence, &owner, selected)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(before_intent
                .require_welcome_current(&guard, selected)
                .is_err());
            assert_eq!(
                intent_only.require_welcome_current(&guard, selected)?,
                GovernedAddWelcomeRecoveryState::IntentCovered
            );
            Ok(())
        })?;
        drop(before_intent);
        self.sign_governed_welcome(layout, packet, routing_plan, keys, rng, negative)?;
        let welcome_raw = self
            .owner
            .cached_governed_welcome_raw(store)?
            .ok_or_else(refused)?;
        self.sign_governed_welcome(layout, packet, routing_plan, keys, rng, negative)?;
        assert_eq!(
            self.owner.cached_governed_welcome_raw(store)?,
            Some(welcome_raw)
        );
        let p = self.governed_add.as_ref().ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        inject("text.add.raw-welcome.v2", true);
        let mut attempt_only = read_welcome(&p.fence, &owner, selected)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(intent_only
                .require_welcome_current(&guard, selected)
                .is_err());
            assert_eq!(
                attempt_only.require_welcome_current(&guard, selected)?,
                GovernedAddWelcomeRecoveryState::PossiblyEntered
            );
            Ok(())
        })?;
        drop(intent_only);
        inject("text.add.raw-welcome.v2", false);
        let mut exact = read_welcome(&p.fence, &owner, selected)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(attempt_only
                .require_welcome_current(&guard, selected)
                .is_err());
            assert_eq!(
                exact.require_welcome_current(&guard, selected)?,
                GovernedAddWelcomeRecoveryState::SignatureRecovered(welcome_raw)
            );
            Ok(())
        })?;
        drop(attempt_only);
        let read_response = |reader: &mut HeldGovernedAddFence,
                             owner: &V2TextOwnerRef<'_>,
                             selected: &SelectedV2TextHead| {
            reader.prepare_response_read(store, owner.source())?;
            store.with_v2_text_current(owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                reader.capture_response(&guard, selected)
            })?;
            reader.authenticate_response(store, owner.source())
        };
        read_response(&mut exact, &owner, selected)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(exact.require_response_current(&guard, selected)?.is_none());
            Ok(())
        })?;
        for _ in 0..2 {
            self.cover_governed_response(layout, packet, routing_plan, keys, rng, negative)?;
        }
        let p = self.governed_add.as_ref().ok_or_else(refused)?;
        let owner = self.owner.borrow_current()?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        let response_bytes = self
            .owner
            .governed_response_data(store)?
            .encode_packet_data()?;
        let mut response = read_welcome(&p.fence, &owner, selected)?;
        read_response(&mut response, &owner, selected)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert!(exact.require_response_current(&guard, selected).is_err());
            assert_eq!(
                response.require_response_current(&guard, selected)?,
                Some(response_bytes.as_slice())
            );
            Ok(())
        })?;
        drop((exact, response));
        let archive =
            self.cover_governed_archive(layout, packet, routing_plan, keys, rng, negative)?;
        assert_eq!(
            self.cover_governed_archive(layout, packet, routing_plan, keys, rng, negative)?,
            archive
        );
        let head = self.stage_governed_head(layout, packet, routing_plan, keys, rng, negative)?;
        assert_eq!(
            self.stage_governed_head(layout, packet, routing_plan, keys, rng, negative)?,
            head
        );
        let p = self.governed_add.as_ref().ok_or_else(refused)?;
        let selected = p.parent.selected_data().ok_or_else(refused)?;
        assert_ne!(head, selected.head_id_data());
        let owner = self.owner.borrow_current()?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            selected.validate_under_current_guard(&guard)
        })?;
        Ok(())
    }
}
