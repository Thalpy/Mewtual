//! The selected replacement's private files and Core graph. The registry
//! receipt has no constructor into this owner.
use super::*;
use crate::store::persistence::governance_layout::joined::pending::welcome::{
    member_add::MemberAdds, RecoveredMemberPrefix,
};
use crate::store::persistence::governance_layout::joined::JoinedRuntimeInputs;
use catcoms_rt::MeshTransport;
use catcoms_sync::ClosedV2JoinedNode;
use std::sync::Arc;

pub(in crate::store) struct InstalledRejoin {
    original: (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
    replacement: NativeTextJoinedRegistrationData,
    row: NativeTextJoinedRegistryRowData,
    pending: RejoinPendingOwner,
    received: RejoinReceivedOwner,
    welcome: Box<RejoinWelcomeOwner>,
    prefix: Box<RejoinPrefixOwner>,
    index: RejoinIndexOwner,
    additions: MemberAdds,
    // Retain the original authenticated Index3 and its physical mount while
    // using the successor. Commands can never infer authority from a new
    // file captured at an old pathname.
    inventory: RetainedNativeRejoinInventory,
}

impl InstalledRejoin {
    fn require_files(&self, store: &ServerStore) -> Result<(), AppError> {
        self.inventory.inventory.require(store)?;
        store.require_recovered_joined_removal(self.original.0, self.original.1)?;
        absent(&fixed(store, false, REJOIN_STAGE)?)?;
        let binding = match &self.pending {
            RejoinPendingOwner::Warm(warm) => &warm.binding,
            RejoinPendingOwner::Cold(cold) => &cold.binding,
        };
        if binding.context != self.replacement.context
            || !self.pending.matches_cookie(&self.index.pending_cookie)
            || !self.pending.matches_cookie(&self.prefix.pending_cookie)
        {
            return Err(denied());
        }
        let attempt = fixed(store, false, ATTEMPT)?;
        let pending = fixed(store, false, PENDING)?;
        require_held_rejoin_files(
            [attempt.as_path(), pending.as_path()],
            self.pending.files(binding)?,
        )?;
        self.received.require(&fixed(store, false, RECEIVED)?)?;
        self.welcome
            .entry
            .require(&fixed(store, false, SDK_ATTEMPT)?)?;
        self.welcome
            .outcome
            .as_ref()
            .ok_or_else(denied)?
            .require(&fixed(store, false, OUTCOME)?)?;
        self.prefix
            .frame
            .as_ref()
            .ok_or_else(denied)?
            .require(&fixed(store, false, PREFIX)?)?;
        self.index.record.require(&fixed(store, false, INDEX)?)?;
        self.inventory.inventory.require(store)
    }

    pub(in crate::store) fn row(
        &self,
        store: &ServerStore,
        original: NativeTextJoinedRegistrationData,
    ) -> Result<NativeTextJoinedRegistryRowData, AppError> {
        if self.replacement != original {
            return Err(denied());
        }
        // Row identity is immutable even during an exact governing retry
        // or after a selected self-Remove. Projection checks availability
        // separately so neither can strand a covered transition.
        self.require_files(store)?;
        Ok(self.row)
    }

    pub(in crate::store) fn prior_registration(&self) -> NativeTextJoinedRegistrationData {
        self.original.0
    }

    pub(in crate::store) fn capture_prefix(
        &self,
        store: &ServerStore,
        original: NativeTextJoinedRegistrationData,
    ) -> Result<RecoveredMemberPrefix, AppError> {
        self.row(store, original)?;
        self.additions.require_current()?;
        Ok(RecoveredMemberPrefix::capture(&self.prefix.core))
    }

    pub(in crate::store) fn encode_successor(
        &self,
        store: &ServerStore,
        original: NativeTextJoinedRegistrationData,
        request: &catcoms_sync::V2MemberAddRequestData,
    ) -> Result<Vec<u8>, AppError> {
        self.row(store, original)?;
        let outcome = self.welcome.outcome.as_ref().ok_or_else(denied)?;
        let encoded = self.additions.encode_selected_successor(
            store,
            self.replacement.context,
            &outcome.frame,
            request,
        )?;
        self.row(store, original)?;
        Ok(encoded)
    }

    pub(in crate::store) fn cover_successor<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        node: &mut ClosedV2JoinedNode<T, R>,
        delivery: &[u8],
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            crate::store::critical_namespaces::joined_pin::welcome::RetiredJoinedMembership,
            Result<(), AppError>,
        ),
        AppError,
    > {
        self.require_files(store)?;
        self.require_node(node)?;
        let core = Arc::get_mut(&mut self.prefix.core).ok_or_else(denied)?;
        let outcome = self.welcome.outcome.as_ref().ok_or_else(denied)?;
        let retired = self.additions.cover(
            (store, self.replacement.context),
            &outcome.frame,
            (&mut core.core, node),
            delivery,
            caller,
            rng,
        )?;
        Ok((retired, self.require_files(store)))
    }

    pub(in crate::store) fn prepare_tcp(
        &self,
        store: &ServerStore,
    ) -> Result<Zeroizing<[u8; 32]>, AppError> {
        self.row(store, self.replacement)?;
        let RejoinPendingOwner::Cold(pending) = &self.pending else {
            return Err(denied());
        };
        let k = Zeroizing::new(store.keys.db_key()?);
        let attempt = open_frame(&k, ATTEMPT_PURPOSE, &pending.attempt.1)?;
        let (_, invite) = decode_saved_attempt(&attempt)?;
        let original = open_pending(
            &pending.binding,
            &invite,
            &k,
            pending_limits()?,
            &pending.pending.1,
        )?;
        if original.transport.identity_data()? != self.replacement.transport {
            return Err(denied());
        }
        let (seed, _) = original.transport.sealed_record_data();
        self.require_files(store)?;
        Ok(Zeroizing::new(*seed))
    }

    pub(in crate::store) fn attach<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        inputs: &mut Option<JoinedRuntimeInputs<T, R>>,
        slot: &mut Option<ClosedV2JoinedNode<T, R>>,
    ) -> Result<(), AppError> {
        self.require_files(store)?;
        if slot.is_none() {
            let graph = self.welcome.recovered.as_deref_mut().ok_or_else(denied)?;
            *slot = Some(
                graph
                    .take_closed_recovered_node(&self.prefix.core.core, inputs)
                    .map_err(|_| denied())?,
            );
        }
        let node = slot.as_mut().ok_or_else(denied)?;
        self.require_node(node)?;
        // A2 has its own continuation namespace. Never replay the journal
        // sealed against the first admission's Outcome onto this graph.
        let core = Arc::get_mut(&mut self.prefix.core).ok_or_else(denied)?;
        let outcome = self.welcome.outcome.as_ref().ok_or_else(denied)?;
        if self
            .additions
            .recover(
                store,
                self.replacement.context,
                &outcome.frame,
                &mut core.core,
                node,
            )?
            .is_some()
        {
            return Err(crate::AppError::ChatV2MembershipRemoved);
        }
        self.row(store, self.replacement).map(drop)
    }

    fn require_node<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ClosedV2JoinedNode<T, R>,
    ) -> Result<(), AppError> {
        if !node.was_recovered_data()
            || node.original_join_data() != self.replacement.selected_data()
            || node.local_peer_data() != self.replacement.transport_data().0
        {
            return Err(denied());
        }
        Ok(())
    }
}

impl ServerStore {
    pub(in crate::store) fn with_installed_rejoin<T>(
        &self,
        lookup: impl FnOnce(&InstalledRejoin) -> Result<T, AppError>,
    ) -> Result<Option<T>, AppError> {
        self.installed_rejoin
            .try_lock()
            .map_err(|_| denied())?
            .as_deref()
            .map(lookup)
            .transpose()
    }

    pub(in crate::store) fn with_installed_rejoin_mut<T>(
        &self,
        lookup: impl FnOnce(&mut InstalledRejoin) -> Result<T, AppError>,
    ) -> Result<Option<T>, AppError> {
        self.installed_rejoin
            .try_lock()
            .map_err(|_| denied())?
            .as_deref_mut()
            .map(lookup)
            .transpose()
    }

    pub(in crate::store) fn recover_installed_rejoin(
        &self,
        inventory: UpgradeInventory,
        original: (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedRegistrationData, AppError> {
        let require_current = || {
            if caller.is_cancelled() {
                return Err(denied());
            }
            Ok(())
        };
        require_current()?;
        let pool = &self
            .governance_layout_policy
            .as_ref()
            .ok_or_else(denied)?
            .current
            .archive
            .pending
            .pool;
        let inventory = RetainedNativeRejoinInventory::retain(inventory, pool)?;
        let mut pending = None;
        let mut received = None;
        let mut welcome = None;
        let mut prefix = None;
        let mut index = None;
        let selected = self.recover_rejoin_index(
            &inventory.inventory,
            original,
            (
                &mut pending,
                &mut received,
                &mut welcome,
                &mut prefix,
                &mut index,
            ),
            caller,
            rng,
            require_current,
        )?;
        let prefix = prefix.ok_or_else(denied)?;
        let replacement = prefix.receipt.ok_or_else(denied)?.registration;
        let owner = InstalledRejoin {
            original,
            replacement,
            row: selected.row,
            pending: pending.ok_or_else(denied)?,
            received: received.ok_or_else(denied)?,
            welcome: welcome.ok_or_else(denied)?,
            prefix,
            index: index.ok_or_else(denied)?,
            additions: MemberAdds::default(),
            inventory,
        };
        owner.row(self, replacement)?;
        require_current()?;
        let mut slot = self.installed_rejoin.try_lock().map_err(|_| denied())?;
        if slot.is_some() {
            return Err(denied());
        }
        *slot = Some(Box::new(owner));
        Ok(replacement)
    }

    /// Distinguish absence from corruption. Presence is only an instruction
    /// to try exact cold recovery, so refuse broken links as ambiguity.
    pub(in crate::store) fn has_rejoin_index(&self) -> Result<bool, AppError> {
        self.has_rejoin_role(INDEX)
    }

    pub(in crate::store) fn has_rejoin_evidence(&self) -> Result<bool, AppError> {
        for role in [
            REJOIN_STAGE,
            ATTEMPT,
            PENDING,
            RECEIVED,
            SDK_ATTEMPT,
            OUTCOME,
            PREFIX,
            INDEX,
        ] {
            if self.has_rejoin_role(role)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn has_rejoin_role(&self, role: &str) -> Result<bool, AppError> {
        match fs::symlink_metadata(fixed(self, false, role)?) {
            Ok(metadata) if real_file(&metadata) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            _ => Err(denied()),
        }
    }
}
