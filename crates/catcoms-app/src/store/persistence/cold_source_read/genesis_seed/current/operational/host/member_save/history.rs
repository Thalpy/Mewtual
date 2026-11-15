//! Blocking history preparation on this exact Host and namespace5 context.
//! Initializing an empty context or finishing already admitted recovery is
//! setup, never a message save, signature, publication or remote receipt.
use super::*;
use crate::store::persistence::v2_member_outbox::{
    MemberHistoryPage, MemberHistoryRead, MEMBER_HISTORY_MAX_ITEMS, MEMBER_OUTBOX_HISTORY_FEE,
};

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn prepare_member_history_with_access(
        &mut self,
        layout: &V2CurrentLayoutStore,
        read: MemberHistoryRead,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<MemberHistoryPage, V2MemberSaveRefusal> {
        access
            .require(&self.origin)
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }
        if read.maximum == 0
            || read.maximum > MEMBER_HISTORY_MAX_ITEMS
            || read.end.is_some_and(|end| read.start > end)
        {
            return Err(V2MemberSaveRefusal::Stale);
        }
        let store = layout
            .checked_store()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        let mount = store
            .member_outbox_mount()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        // This fee covers the owned context/group scratch before locking the
        // cell, then follows the returned page. No uncharged request copy.
        let payment = mount
            .pay(MEMBER_OUTBOX_HISTORY_FEE)
            .map_err(|_| V2MemberSaveRefusal::Busy)?;
        let key = member_outbox_key(store).map_err(|_| V2MemberSaveRefusal::Failed)?;
        let (context, signer) = {
            let owner = self
                .owner
                .borrow_current()
                .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
            let context = member_outbox_context(&owner, mount.capability().current_layout_data())
                .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
            (context, owner.source().data().context.spec_data().signer)
        };
        let id = context.id();
        let cell = mount.cell(id).map_err(|_| V2MemberSaveRefusal::Busy)?;
        let mut guard = cell.lock_recovering();
        if matches!(guard.custody, Some(MemberOriginalCustody::Running { .. })) {
            return Err(V2MemberSaveRefusal::Busy);
        }
        // As in save preparation, the cell's named request-scratch allowance
        // covers context DATA after this lock. If history_page returns an
        // error and releases its payment first, env's scratch remains paid and
        // leaves before the guard unlocks for a successor's scratch allocation.
        let mut env = SaveEnvironment {
            layout,
            store,
            mount: &mount,
            key: &key,
            id,
            signer,
            context: Some(context),
            lookup_fee: None,
            rng,
        };
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }
        // Never turn absent/busy/uncertain state into an invented empty page.
        // A first empty context gets a genuine authenticated empty head. An
        // uncertain owner finishes only its original admitted recovery work.
        let owner = ensure_owner(&mut env, &mut guard.owner)?;
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }
        owner
            .history_page(&key, read.start, read.end, read.maximum, payment)
            .map_err(|error| refusal_of(&error))
    }
}
