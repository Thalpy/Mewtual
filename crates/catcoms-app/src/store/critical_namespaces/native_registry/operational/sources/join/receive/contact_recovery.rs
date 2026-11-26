//! The native row revalidates stored consent before any live peer approval
//! or numeric dial. Saved bytes are never a live membership observation.
use super::super::reconnect::{NativeTextJoinedContactApprovalProof, NativeTextJoinedReconnect};
use super::history_discovery::HistoryContactScope;
use super::*;
use crate::store::persistence::v2_contact_approvals::{
    ApprovedContactData, ContactApprovalScope, CONTACT_APPROVAL_FEE,
};
use catcoms_sync::{V2MemberContactData, V2_MEMBER_CONTACT_BYTES};

catcoms_rt::redacted_debug! { NativeTextJoinedSavedContactApprovals; }

pub struct NativeTextJoinedSavedContactApprovals {
    selection: NativeTextJoinedRowSelection,
    scope: ContactApprovalScope,
    contacts: Vec<ApprovedContactData>,
    generation: RequestCancellation,
    _fee: CatalogReadReservation,
}

impl NativeTextJoinedSavedContactApprovals {
    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Store recurring approval after the exact signed contact connected.
    /// The native caller orders the physical save against UI lock.
    pub fn remember_member_contact(
        self,
        selection: &NativeTextJoinedRowSelection,
        raw: [u8; V2_MEMBER_CONTACT_BYTES],
        proof: NativeTextJoinedContactApprovalProof,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !selection.same_original(&proof.candidate.row)
            || proof.candidate.digest != blake3::hash(&raw)
            || !proof.connection.is_live()
            || !proof.consent.is_current_for(selection)
        {
            return Err(Refusal::Stale);
        }
        let contact = ApprovedContactData::new(raw, proof.outbound.then_some(proof.candidate.host))
            .map_err(|_| Refusal::Failed)?;
        let transport = self
            .original
            .tcp
            .upgrade()
            .ok_or(Refusal::Unavailable)?
            .capture_attached_dial()
            .map_err(|_| Refusal::Unavailable)?;
        self.with_member_receiver(selection, caller, |source| {
            let expected = source.verify_saved_contact(&contact)?;
            if expected.peer != proof.candidate.contact.peer
                || expected.admission != proof.candidate.contact.admission
                || transport
                    .connection_lease(expected.peer)
                    .is_none_or(|live| !live.is_same(&proof.connection))
            {
                return Err(Refusal::Stale);
            }
            let _fee = source
                .metadata
                .reserve_join_node(CONTACT_APPROVAL_FEE)
                .map_err(|_| Refusal::Busy)?;
            if caller.is_cancelled()
                || !proof.connection.is_live()
                || !proof.consent.is_current_for(selection)
            {
                return Err(Refusal::Cancelled);
            }
            selection
                .retire_contact_approvals(false)
                .map_err(|_| Refusal::Busy)?;
            source
                .store
                .write_contact_approval(
                    source.contact_approval_scope(selection)?,
                    contact,
                    proof.candidate.host,
                    rng,
                )
                .map_err(|_| Refusal::Failed)?;
            // The consent is already durable. If the live peer table is
            // contended, the worker can install it on a fresh capture.
            let _ = selection.remember_governing_peer(expected);
            Ok(())
        })
    }

    /// Forget standing consent under the same native lock-ordered save.
    /// Already approved one-time contacts may finish their current session.
    pub fn forget_member_contact_approvals(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.with_member_receiver(selection, caller, |source| {
            let _fee = source
                .metadata
                .reserve_join_node(CONTACT_APPROVAL_FEE)
                .map_err(|_| Refusal::Busy)?;
            let scope = source.contact_approval_scope(selection)?;
            selection
                .retire_contact_approvals(true)
                .map_err(|_| Refusal::Busy)?;
            source
                .store
                .clear_contact_approvals(scope, rng)
                .map_err(|_| Refusal::Failed)
        })
    }

    /// An optional paid snapshot of locally approved contacts for the
    /// installed row. Every use must recheck the actual selected prefix.
    pub fn read_member_contact_approvals(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: &RequestCancellation,
    ) -> Result<NativeTextJoinedSavedContactApprovals, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.with_member_receiver(selection, caller, |source| {
            let fee = source
                .metadata
                .reserve_join_node(CONTACT_APPROVAL_FEE)
                .map_err(|_| Refusal::Busy)?;
            let scope = source.contact_approval_scope(selection)?;
            let generation = selection
                .observe_contact_approvals()
                .map_err(|_| Refusal::Busy)?;
            let contacts = source
                .store
                .read_contact_approvals(scope)
                .map_err(|_| Refusal::Failed)?;
            if generation.is_cancelled() {
                return Err(Refusal::Stale);
            }
            Ok(NativeTextJoinedSavedContactApprovals {
                selection: selection.clone(),
                scope,
                contacts,
                generation,
                _fee: fee,
            })
        })
    }

    /// Restore one original consent. Passive contacts can become approved on
    /// an existing authenticated connection. They cannot nominate an address.
    pub fn prepare_saved_member_contact(
        self,
        selection: &NativeTextJoinedRowSelection,
        saved: &NativeTextJoinedSavedContactApprovals,
        index: usize,
        caller: RequestCancellation,
    ) -> Result<Option<NativeTextJoinedReconnect>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        if !selection.same_original(&saved.selection) || saved.generation.is_cancelled() {
            return Err(Refusal::Stale);
        }
        let contact = saved.contacts.get(index).ok_or(Refusal::Stale)?;
        let caller = HistoryContactScope::constrain(saved.generation.clone(), caller);
        let peer = self.with_member_receiver(selection, &caller, |source| {
            if source.contact_approval_scope(selection)? != saved.scope {
                return Err(Refusal::Stale);
            }
            source.verify_saved_contact(contact)
        })?;
        if let Some(host) = contact.outbound {
            return self
                .prepare_member_contact(selection, contact.raw(), host, caller)
                .map(|reconnect| Some(reconnect.as_restored()));
        }
        let transport = self
            .original
            .tcp
            .upgrade()
            .ok_or(Refusal::Unavailable)?
            .capture_attached_dial()
            .map_err(|_| Refusal::Unavailable)?;
        if transport
            .connection_lease(peer.peer)
            .is_some_and(|connection| connection.is_live())
        {
            self.with_member_receiver(selection, &caller, |source| {
                if source.contact_approval_scope(selection)? != saved.scope
                    || transport
                        .connection_lease(peer.peer)
                        .is_none_or(|value| !value.is_live())
                {
                    return Err(Refusal::Stale);
                }
                selection
                    .install_restored_peer(peer, &caller)
                    .map_err(|_| Refusal::Busy)
            })?;
        }
        Ok(None)
    }
}

impl<R: CryptoRngCore> ReceiverSource<'_, R> {
    fn contact_approval_scope(
        &self,
        selection: &NativeTextJoinedRowSelection,
    ) -> Result<ContactApprovalScope, V2MemberSaveRefusal> {
        let (head, own, epoch) = self
            .node
            .selected_join_data()
            .map_err(|_| V2MemberSaveRefusal::Stale)?;
        let row = selection.original.row;
        let mut stable =
            blake3::Hasher::new_derive_key("mewtual/chat/registered-contact-approvals/v1");
        stable.update(&row.id.to_be_bytes());
        stable.update(&row.identity);
        stable.update(selection.original.member_outbox.as_bytes());
        stable.update(&self.registration.request);
        stable.update(own.as_bytes());
        Ok(ContactApprovalScope {
            row: *stable.finalize().as_bytes(),
            head,
            epoch,
        })
    }

    fn verify_saved_contact(
        &self,
        stored: &ApprovedContactData,
    ) -> Result<ApprovedMemberPeer, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let contact = V2MemberContactData::decode(stored.raw()).map_err(|_| Refusal::Failed)?;
        self.node
            .verify_member_contact(self.prefix, &contact)
            .map_err(|_| Refusal::Stale)?;
        Ok(ApprovedMemberPeer {
            peer: stored.peer(),
            admission: contact.admission_data(),
        })
    }
}
