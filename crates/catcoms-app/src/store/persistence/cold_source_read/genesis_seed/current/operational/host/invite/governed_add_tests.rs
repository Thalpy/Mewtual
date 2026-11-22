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
            GovernedAddHeaderRecoveryState, GovernedAddRecoveryState, PreparedGovernedAddFence,
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
        let node = self.owner.governed_node_for_test(store)?;
        node.materialize_admission_data()?;
        let admitted = node.signed_admission_data()?.clone();
        let owner = self.owner.borrow_current()?;
        let outcome = self.owner.governed_outcome_data(store)?;
        let read_record = |fence: &PreparedGovernedAddFence| {
            let mut reader = read_fence(fence, &owner, selected)?;
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
        let mut before_record = read_record(&fence)?;
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
        fence.prepare_governing_record(store, owner.source(), &outcome, &admitted, rng)?;
        for _ in 0..2 {
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                fence.cover_governing_record(&guard, selected)?;
                assert!(before_record
                    .require_governing_current(&guard, selected)
                    .is_err());
                Ok(())
            })?;
        }
        let mut saved_record = read_record(&fence)?;
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            assert_eq!(
                saved_record.require_governing_current(&guard, selected)?,
                Some(&admitted)
            );
            Ok(())
        })?;
        Ok(())
    }
}
