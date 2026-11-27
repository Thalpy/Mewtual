//! The warm native founder's selected Remove transaction.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::persistence::v2_text::{prepare_sdk_admission, PreparedGovernedRemoveFence};
use catcoms_mls::persist::text_current_add::TextCurrentAddAdmission;
use catcoms_replication::chat::{
    owner_header::{OwnerBodyId, OwnerHeaderId},
    V2GeneralCandidate,
};
use catcoms_rt::{NativeChatKeyEnds, NativeChatPreparedKeyCut, RequestCancellation};

pub(super) struct PendingGovernedRemoval {
    fence: Box<PreparedGovernedRemoveFence>,
    sdk: Option<TextCurrentAddAdmission>,
    core: Option<V2GeneralCandidate>,
}

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn governed_roster_native(
        &mut self,
        layout: &V2CurrentLayoutStore,
        keys: &NativeFounderKeys,
        negative: &RequestCancellation,
    ) -> Result<crate::store::NativeTextRoster, AppError> {
        use crate::store::persistence::v2_text::PreparedV2TextParentCapture;
        use crate::store::{NativeTextRoster, NativeTextRosterMember};
        let store = layout.checked_store()?;
        let require = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        require()?;
        let owner = self.owner.borrow_current()?;
        let genesis = owner.source().data();
        let own = genesis.admission.id();
        if let Some(current) = self.governed_add.as_mut() {
            let verifier = self
                .member
                .as_ref()
                .ok_or_else(refused)?
                .selected_invite_verifier()?;
            let replay = verifier.replay_data();
            let resources = current.parent.resources().clone();
            return store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                current.require_selected_member_head(&guard, replay)?;
                require()?;
                let roster =
                    NativeTextRoster::from_selected(store, replay, own)?.with_founder_removal();
                require()?;
                Ok(roster)
            });
        }
        let mut capture = PreparedV2TextParentCapture::prepare(store, owner.source())?;
        let resources = capture.resources().clone();
        store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            capture
                .capture(&guard)?
                .validate_idle_genesis_member_under_current_guard(&guard)?;
            require()?;
            NativeTextRoster::prepare(
                store,
                std::iter::once(NativeTextRosterMember {
                    admission: own,
                    signer: genesis.admission.spec().subject,
                    founder: true,
                    you: true,
                }),
            )
        })
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn after_governed_remove_selection_for_test(hook: Box<dyn FnOnce()>) {
        super::governed_add::AFTER_GOVERNED_HEAD_SAVE.with(|slot| {
            assert!(slot.replace(Some(hook)).is_none());
        });
    }

    pub(in crate::store) fn remove_governed_member_native(
        &mut self,
        layout: &V2CurrentLayoutStore,
        target: OwnerBodyId,
        keys: &NativeFounderKeys,
        (sdk_cut, header_cut, ends): (
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatKeyEnds,
        ),
        rng: &mut impl catcoms_rt::CryptoRngCore,
        negative: &RequestCancellation,
    ) -> Result<(OwnerHeaderId, u64), AppError> {
        let store = layout.checked_store()?;
        let require_live = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            keys.preflight(&self.origin)
        };
        require_live()?;
        self.require_member_quiescent_for_add(layout)?;
        let member = self.member.as_mut().ok_or_else(refused)?;
        member.require_quiescent_for_add()?;
        // Reserve capacity before entering SDK or durable Remove. The paid lane
        // holds at most one cut per bounded governing membership step.
        if member.retired_history.len() >= catcoms_sync::MAX_V2_GOVERNING_JOIN_STEPS {
            return Err(refused());
        }
        let prior = self.governed_add.as_mut().ok_or_else(refused)?;
        let installed = prior.installed.ok_or_else(refused)?;
        let verifier = member.governed_add_verifier()?;
        prior.require_selected_replay(verifier.replay_data())?;
        let before = prior.parent.selected_data().ok_or_else(refused)?;
        let resources = prior.parent.resources();
        if let Some(removal) = &mut prior.removal {
            if removal.matches_admission(target) && self.governed_removal.is_none() {
                let owner = self.owner.borrow_current()?;
                return store.with_v2_text_current(&owner, |admission| {
                    let guard = store.lock_v2_text_current_group(admission, resources)?;
                    require_live()?;
                    removal.require_selected(&guard, installed)?;
                    require_live()?;
                    Ok((installed.1.header, installed.1.epoch))
                });
            }
        }
        if self.governed_removal.is_none() {
            let replay = verifier.replay_data();
            let slot = replay.slot(target).ok_or_else(refused)?;
            if slot.image.subject == replay.profile().spec().founder {
                return Err(refused());
            }
            let proof = self.owner.governed_fetch_verifier(store, verifier)?;
            let owner = self.owner.borrow_current()?;
            let fence = PreparedGovernedRemoveFence::prepare(
                store,
                owner.source(),
                before,
                installed,
                target,
                proof,
                rng,
            )?;
            let sdk = prepare_sdk_admission(store)?;
            self.governed_removal = Some(Box::new(PendingGovernedRemoval {
                fence: Box::new(fence),
                sdk: Some(sdk),
                core: None,
            }));
        }
        let pending = self.governed_removal.as_mut().ok_or_else(refused)?;
        if !pending.fence.matches_admission(target) {
            return Err(refused());
        }
        if !pending.fence.selection_entered() {
            let owner = self.owner.borrow_current()?;
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, resources)?;
                require_live()?;
                pending.fence.cover_sdk_entry(&guard)?;
                require_live()
            })?;
            self.owner.execute_governed_removal(
                store,
                verifier,
                target,
                &keys.original,
                (&mut pending.sdk, sdk_cut, ends),
            )?;
            let outcome = self.owner.governed_removal_outcome(store)?;
            pending.fence.prepare_outcome(store, &outcome, rng)?;
            let (owner, mut primitive) = self.owner.borrow_removal_signing(store, verifier)?;
            store.with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, resources)?;
                require_live()?;
                pending.fence.cover_signing_attempt(&guard)?;
                require_live()?;
                primitive.sign_header_native(&keys.original, header_cut, ends)?;
                require_live()
            })?;
            let header = self.owner.materialize_governed_removal(store)?;
            let outcome = self.owner.governed_removal_outcome(store)?;
            pending
                .fence
                .prepare_archive(store, &header, &outcome, rng)?;
        }

        if pending.core.is_none() {
            pending.core = Some(
                self.owner
                    .derive_governed_removal_selection(store, verifier)?,
            );
        }
        let (owner, installation) = self.owner.borrow_removal_install(store, verifier)?;
        let fingerprint = installation.expected_data();
        let head = store.with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, resources)?;
            require_live()?;
            installation
                .check_host_selection_data(verifier, pending.core.as_ref().ok_or_else(refused)?)?;
            let head = pending.fence.select(&guard, fingerprint)?;
            #[cfg(test)]
            super::governed_add::AFTER_GOVERNED_HEAD_SAVE.with(|hook| {
                let hook = hook.borrow_mut().take();
                if let Some(hook) = hook {
                    hook();
                }
            });
            require_live()?;
            installation.finish_host_selected(verifier, &mut pending.core)?;
            Ok(head)
        })?;
        let pending = self
            .governed_removal
            .take()
            .expect("selected original Remove");
        let mut retired = self.owner.take_retired_governed_removal()?;
        let previous = prior.removal.replace(pending.fence);
        prior.installed = Some((head, fingerprint));
        let frozen = retired.take_frozen_data().ok_or_else(refused)?;
        member.retired_history.push(frozen);
        if let Some(mut close) = retired.take_close_completion() {
            let _ = close.try_complete();
        }
        drop((retired, previous));
        Ok((fingerprint.header, fingerprint.epoch))
    }
}
