//! One selected warm join with its original private graph and transport.
catcoms_rt::redacted_debug! {
    [T: MeshTransport, R: CryptoRngCore] NativeTextJoinedRuntime<T, R>;
}

use super::super::join_tcp;
use super::super::native_ui::{
    UiAdmissionOwner, UiCandidate, UiCapture, UiPreparation, UiRetirement,
};
use super::*;
use crate::store::persistence::governance_layout::joined::{
    rejoin::{
        RejoinCatalogPreparation, RejoinIndexOwner, RejoinPendingOwner, RejoinPrefixOwner,
        RejoinReceivedOwner, RejoinWelcomeOwner,
    },
    JoinedRuntimeInputs,
};
use crate::store::NativeTextJoinedRegistrationData;
use catcoms_replication::chat::{owner_facts::OwnerFactId, owner_header::OwnerHeaderId};
use catcoms_sync::{ClosedV2JoinedNode, SelectedRejoinRemoval, MAX_V2_FIRST_ADD_PROOF_BYTES};
use std::sync::atomic::{AtomicBool, AtomicU64};
use zeroize::Zeroizing;

mod rejoin_exchange;
pub use rejoin_exchange::{NativeTextRejoinDelivery, NativeTextRejoinExchange};

enum RecoveredJoinedMembership {
    Active(OwnerHeaderId, OwnerFactId, u64),
    Removed(SelectedRejoinRemoval),
}

struct PaidGoverningProof {
    bytes: Zeroizing<Vec<u8>>,
    _fee: CatalogReadReservation,
}

impl AsRef<[u8]> for PaidGoverningProof {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

pub struct NativeTextJoinedRuntime<T: MeshTransport, R: CryptoRngCore> {
    node: Option<ClosedV2JoinedNode<T, R>>,
    inputs: Option<JoinedRuntimeInputs<T, R>>,
    storage: NativeTextJoinedStorage,
    _fee: CatalogReadReservation,
}

/// Authenticated cold result for the original joined vault. A retired row
/// retains the physical owner and a drain observation for the original TCP
/// actor. It has no live MLS node, Source or native signing cut.
pub enum NativeTextJoinedRecovery<R: CryptoRngCore> {
    Active(
        crate::store::NativeTextVault,
        crate::store::NativeTextSources<catcoms_net::MeshService, R>,
    ),
    Removed(NativeTextRemovedJoinedVault),
}

/// Exclusive retired membership storage. The recorded labels are historical
/// data. A rejoin writer must recheck the held Remove at its own commit edge.
pub struct NativeTextRemovedJoinedVault {
    ui: UiAdmissionOwner,
    key_source: catcoms_rt::NativeChatSourceOwner,
    index: upgrade_source::owning::Store,
    registration: NativeTextJoinedRegistrationData,
    removal: SelectedRejoinRemoval,
    tcp_drain: catcoms_net::NativeTcpDrain,
    rejoin_catalog: Option<Box<RejoinCatalogPreparation>>,
    rejoin_pending: Option<RejoinPendingOwner>,
    rejoin_tcp: Option<Arc<join_tcp::Transport>>,
    rejoin_received: Option<RejoinReceivedOwner>,
    rejoin_welcome: Option<Box<RejoinWelcomeOwner>>,
    rejoin_prefix: Option<Box<RejoinPrefixOwner>>,
    rejoin_index: Option<RejoinIndexOwner>,
}

catcoms_rt::redacted_debug! {
    [R: CryptoRngCore] NativeTextJoinedRecovery<R>;
    NativeTextRemovedJoinedVault;
}

impl Drop for NativeTextRemovedJoinedVault {
    fn drop(&mut self) {
        // Close the retired UI and independent key source before their
        // original physical vault and file custody can leave this owner.
        self.ui.close_first();
        self.key_source.close();
    }
}

impl NativeTextRemovedJoinedVault {
    /// Capture the original retired vault's paid UI generation. The
    /// candidate must be finished outside native or Store guards.
    pub fn prepare_rejoin_ui(
        &self,
        expected: u64,
        credit: crate::store::NativeUiInvocationMetadata,
    ) -> UiPreparation {
        self.ui.prepare_capture(expected, credit)
    }

    /// Bind the candidate to the canonical generation of this retired vault.
    /// This admits UI metadata only and issues no key or live chat source.
    pub fn admit_rejoin_ui(&self, candidate: &mut UiCandidate) -> Result<UiCapture, AppError> {
        self.ui.admit(candidate)
    }

    /// Move begun generation retirement outside the native guard before
    /// waiting for any admitted rejoin operation to leave.
    pub fn begin_rejoin_ui_retirement(&self, through: u64) -> Result<UiRetirement, AppError> {
        self.ui.begin_retirement(through)
    }

    /// Recheck the exact registry, original Welcome files and selected
    /// terminal cut before lending the historical coordinates. None means the
    /// original TCP actor still has a task or key to retire.
    pub fn removal_data(&self) -> Result<Option<SelectedRejoinRemoval>, AppError> {
        if !self.tcp_drain.is_drained() {
            return Ok(None);
        }
        self.index
            .require_recovered_native_join_removal(self.registration, self.removal)?;
        Ok(Some(self.removal))
    }

    /// Prepare the catalog boundary before any replacement key work. The
    /// original Current stays selected. Repeated calls retain the same
    /// candidate, and pending transport retirement consumes no RNG.
    pub fn prepare_rejoin_catalog(
        &mut self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<bool, AppError> {
        if !self.tcp_drain.is_drained() {
            return Ok(false);
        }
        if let Some(existing) = self.rejoin_catalog.as_ref() {
            self.index.require_original_rejoin_catalog(
                self.registration,
                self.removal,
                existing,
            )?;
        } else {
            let prepared =
                self.index
                    .prepare_retired_rejoin_catalog(self.registration, self.removal, rng)?;
            self.rejoin_catalog = Some(Box::new(prepared));
        }
        Ok(true)
    }

    /// Cover a local rejoin catalog candidate on disk. Current, the selected
    /// membership and the native index remain unchanged. The next opener can
    /// authenticate and reuse this exact complete candidate.
    pub fn stage_rejoin_catalog(&mut self, rng: &mut dyn CryptoRngCore) -> Result<bool, AppError> {
        if !self.prepare_rejoin_catalog(rng)? {
            return Ok(false);
        }
        let prepared = self.rejoin_catalog.as_mut().ok_or_else(refused)?;
        self.index.stage_retired_rejoin_catalog(prepared)?;
        Ok(true)
    }

    /// Cover an exact A2 Pending after authentication, original UI
    /// generation admission and actual retirement of the recovered A1 actor.
    /// It returns historical data. Network and Welcome are separate cuts.
    pub fn cover_rejoin_pending(
        &mut self,
        ui: &UiCapture,
        invite: (&crate::store::NativeTextInvitePacketData<'_>, [u8; 32], u64),
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<crate::store::NativeTextRejoinPendingReceipt>, AppError> {
        if !self.tcp_drain.is_drained() {
            return Ok(None);
        }
        let command = self.ui.require_capture(ui)?;
        let source = self.key_source.observe();
        let purpose = catcoms_rt::NativeChatKeyPurpose::JoinerDevice;
        let mut cut = self
            .key_source
            .prepare_cut(command, purpose)
            .map_err(|_| refused())?;
        // Ends outlive all storage guards, including refusal before a file
        // is covered and unwinding during the SDK or physical write.
        let mut ends = catcoms_rt::NativeChatKeyEnds::new();
        let authority = &self.ui;
        let require_current = || {
            if negative.is_cancelled() || !authority.is_current_cached(ui) {
                return Err(refused());
            }
            Ok(())
        };
        self.index
            .cover_rejoin_pending(
                (self.registration, self.removal),
                &mut self.rejoin_pending,
                invite,
                (&source, &mut cut, &mut ends),
                rng,
                require_current,
            )
            .map(Some)
    }

    /// Confirm the covered ID9 fingerprint from a fresh terminal mount. None
    /// means the original recovered TCP transport has yet to finish retiring.
    pub fn cover_selected_rejoin_catalog(&self) -> Result<Option<[u8; 32]>, AppError> {
        if !self.tcp_drain.is_drained() {
            return Ok(None);
        }
        self.index
            .cover_selected_rejoin_catalog(self.registration, self.removal)
            .map(Some)
    }

    /// Publish and cover the original Stage as Current. The old mount never
    /// becomes an A2 source. On success or after a restart, use the native
    /// cold opener to acquire the selected catalog in a fresh mount.
    pub fn select_rejoin_catalog(&mut self, rng: &mut dyn CryptoRngCore) -> Result<bool, AppError> {
        if !self.tcp_drain.is_drained() {
            return Ok(false);
        }
        if self.rejoin_catalog.is_none() && !self.prepare_rejoin_catalog(rng)? {
            return Ok(false);
        }
        let prepared = self.rejoin_catalog.as_mut().ok_or_else(refused)?;
        self.index.select_retired_rejoin_catalog(prepared)?;
        Ok(true)
    }
}

impl NativeTextJoinedStorage {
    /// Bind the transport before exchanging the covered request. A retry of
    /// the owning runtime keeps the same transport and private request.
    #[expect(
        clippy::result_large_err,
        reason = "A refused bind returns the original paid storage owner."
    )]
    pub fn bind_first_join_runtime<T: MeshTransport, R: CryptoRngCore>(
        self,
        transport: T,
        rng: R,
        clock: Box<dyn catcoms_rt::Clock + Send>,
    ) -> Result<NativeTextJoinedRuntime<T, R>, (Self, AppError)> {
        let checked = self.warm_join_outcome_data().and_then(|outcome| {
            if outcome.is_some() {
                return Err(refused());
            }
            Ok(())
        });
        self.bind_runtime(checked, (transport, rng, clock))
    }

    /// Bind already-funded runtime resources to the independently recovered
    /// selected outcome. The returned owner has no fresh Welcome authority.
    #[expect(
        clippy::result_large_err,
        reason = "A refused bind returns the original paid storage owner."
    )]
    pub fn bind_recovered_join_runtime<T: MeshTransport, R: CryptoRngCore>(
        self,
        transport: T,
        rng: R,
        clock: Box<dyn catcoms_rt::Clock + Send>,
    ) -> Result<NativeTextJoinedRuntime<T, R>, (Self, AppError)> {
        let checked = self.recovered_join_outcome_data().and_then(|outcome| {
            if self.pending.is_some()
                || outcome.is_none()
                || self.recovered_join_transport_data()?.is_some()
            {
                return Err(refused());
            }
            Ok(())
        });
        self.bind_runtime(checked, (transport, rng, clock))
    }

    /// Restore the same reserved identity from a completed and selected native
    /// joined row. The old Welcome is independently verified without SDK entry.
    #[expect(
        clippy::result_large_err,
        reason = "A refused bind returns the original paid storage owner."
    )]
    pub fn bind_recovered_native_runtime<R: CryptoRngCore>(
        mut self,
        rng: R,
        clock: Box<dyn catcoms_rt::Clock + Send>,
        negative: &RequestCancellation,
    ) -> Result<NativeTextJoinedRuntime<catcoms_net::MeshService, R>, (Self, AppError)> {
        let prepared = (|| {
            if self.pending.is_some() || negative.is_cancelled() {
                return Err(refused());
            }
            let receipt = self
                .recovered_join_registration_data()?
                .ok_or_else(refused)?;
            let require_original = || {
                if negative.is_cancelled() {
                    return Err(refused());
                }
                self.index.require_registered_native_join(receipt)
            };
            require_original()?;
            let allowance = runtime_invoice::<catcoms_net::MeshService, R>();
            let (_, fee) = self.index.joined_exchange_resources(allowance)?;
            if self.recovered_tcp.is_none() {
                let (_, owner_fee) = self.index.joined_exchange_resources(32 * 1024)?;
                self.recovered_tcp = Some(Arc::new(join_tcp::Transport::paid(owner_fee)));
            }
            let original = self.recovered_tcp.as_ref().ok_or_else(refused)?;
            let mut loan = original.checkout(
                receipt.transport_data(),
                || self.index.prepare_recovered_native_join_tcp(&receipt),
                require_original,
            )?;
            let transport = loan.take()?;
            loan.attached();
            Ok((transport, fee))
        })();
        match prepared {
            Ok((transport, fee)) => Ok(NativeTextJoinedRuntime {
                node: None,
                inputs: Some((transport, rng, clock)),
                storage: self,
                _fee: fee,
            }),
            Err(error) => Err((self, error)),
        }
    }

    #[expect(
        clippy::result_large_err,
        reason = "A refused bind returns the original paid storage owner."
    )]
    fn bind_runtime<T: MeshTransport, R: CryptoRngCore>(
        self,
        checked: Result<(), AppError>,
        inputs: JoinedRuntimeInputs<T, R>,
    ) -> Result<NativeTextJoinedRuntime<T, R>, (Self, AppError)> {
        let prepared = checked.and_then(|()| {
            let allowance = runtime_invoice::<T, R>();
            let (_, fee) = self.index.joined_exchange_resources(allowance)?;
            Ok(fee)
        });
        match prepared {
            Ok(fee) => Ok(NativeTextJoinedRuntime {
                node: None,
                inputs: Some(inputs),
                storage: self,
                _fee: fee,
            }),
            Err(error) => Err((self, error)),
        }
    }
}

fn runtime_invoice<T: MeshTransport, R: CryptoRngCore>() -> usize {
    ClosedV2JoinedNode::<T, R>::allocation_bytes()
        + 2 * std::mem::size_of::<NativeTextJoinedRuntime<T, R>>()
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextJoinedRuntime<T, R> {
    /// Cover a signed governing Add for this independently reopened joined
    /// graph. The original Welcome and its registration remain historical.
    pub fn cover_recovered_member_add(
        &mut self,
        delivery: &[u8],
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(OwnerHeaderId, OwnerFactId, u64), AppError> {
        if self.storage.pending.is_some()
            || caller.is_cancelled()
            || self.storage.index.rejoin_catalog_selected()?
        {
            return Err(refused());
        }
        let node = self
            .node
            .as_mut()
            .filter(|node| node.was_recovered_data())
            .ok_or_else(refused)?;
        self.storage
            .index
            .cover_recovered_member_add(node, delivery, caller, rng)?;
        if caller.is_cancelled() {
            return Err(refused());
        }
        Ok(node.selected_join_data()?)
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::critical_namespaces::native_registry) fn check_recovered_member_add_frame_for_test(
        &self,
        wire: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(OwnerHeaderId, u64), AppError> {
        self.storage
            .index
            .actual_store_for_test()?
            .check_recovered_member_add_frame_for_test(
                self.node.as_ref().ok_or_else(refused)?,
                wire,
                rng,
            )
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::critical_namespaces::native_registry) fn recovered_member_add_request_for_test(
        &self,
    ) -> Result<catcoms_sync::V2MemberAddRequestData, AppError> {
        self.storage
            .index
            .actual_store_for_test()?
            .recovered_member_add_request_for_test(self.node.as_ref().ok_or_else(refused)?)
    }

    pub fn local_peer_data(&self) -> Option<catcoms_rt::PeerId> {
        self.node.as_ref().map(ClosedV2JoinedNode::local_peer_data)
    }
    /// Read the original signed governing proof from an attached first join.
    /// The exact parent and selected child identify this bounded history read.
    /// The returned historical data keeps its own credit through all clones.
    pub fn first_selected_governing_proof(
        &self,
        parent: OwnerHeaderId,
        child: OwnerHeaderId,
        negative: &RequestCancellation,
    ) -> Result<bytes::Bytes, AppError> {
        let node = self.node.as_ref().ok_or_else(refused)?;
        if negative.is_cancelled()
            || !matches!(node.selected_join_data(), Ok((head, _, 1)) if head == child)
        {
            return Err(refused());
        }
        let (mount, fee) = self.storage.index.joined_exchange_resources(
            MAX_V2_FIRST_ADD_PROOF_BYTES + 4 * std::mem::size_of::<PaidGoverningProof>(),
        )?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_V2_FIRST_ADD_PROOF_BYTES));
        self.storage.index.copy_selected_governing_proof(
            self.storage.pending.as_ref(),
            (parent, child),
            &mut bytes,
        )?;
        if negative.is_cancelled()
            || mount.is_cancelled()
            || node.selected_join_data().map_err(|_| refused())?.0 != child
        {
            return Err(refused());
        }
        Ok(bytes::Bytes::from_owner(PaidGoverningProof {
            bytes,
            _fee: fee,
        }))
    }

    /// Attach the closed restored graph once, then reprove its original held
    /// files on every exact retry. Native Source admission remains separate.
    pub fn complete_recovery(
        &mut self,
        negative: &RequestCancellation,
    ) -> Result<(OwnerHeaderId, OwnerFactId, u64), AppError> {
        match self.recover_joined_membership(negative)? {
            RecoveredJoinedMembership::Active(head, own, epoch) => Ok((head, own, epoch)),
            RecoveredJoinedMembership::Removed(_) => Err(AppError::ChatV2MembershipRemoved),
        }
    }

    fn recover_joined_membership(
        &mut self,
        negative: &RequestCancellation,
    ) -> Result<RecoveredJoinedMembership, AppError> {
        if self.storage.pending.is_some() || negative.is_cancelled() {
            return Err(refused());
        }
        let removed = self
            .storage
            .index
            .attach_recovered_joined_node(&mut self.inputs, &mut self.node)?;
        let node = self
            .node
            .as_ref()
            .filter(|node| node.was_recovered_data())
            .ok_or_else(refused)?;
        if self.storage.recovered_tcp.is_some() {
            let receipt = self
                .storage
                .recovered_join_registration_data()?
                .ok_or_else(refused)?;
            if receipt.selected_data() != node.original_join_data()
                || receipt.transport_data().0 != node.local_peer_data()
            {
                return Err(refused());
            }
            self.storage.index.require_registered_native_join(receipt)?;
        }
        let (initial_head, own, initial_epoch) = node.original_join_data();
        if negative.is_cancelled()
            || self.storage.recovered_join_outcome_data()? != Some((initial_head, initial_epoch))
        {
            return Err(refused());
        }
        if let Some(removal) = removed {
            if removal.1 != own || removal.2 <= initial_epoch {
                return Err(refused());
            }
            return Ok(RecoveredJoinedMembership::Removed(removal));
        }
        // During rejoin staging ID9 still owns the retired A1 row. Neither
        // a storage-only caller nor native startup may use an active A1.
        if self.storage.index.rejoin_catalog_selected()? {
            return Err(refused());
        }
        let (head, own, epoch) = node.selected_join_data().map_err(|_| refused())?;
        Ok(RecoveredJoinedMembership::Active(head, own, epoch))
    }

    /// Receive, cover, and pair the original Welcome. The returned data is
    /// historical. The native registry and ordinary-member save admission
    /// must still bind their own original Source before authoring.
    pub async fn complete_first_join(
        &mut self,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        negative: RequestCancellation,
    ) -> Result<(OwnerHeaderId, OwnerFactId, u64), AppError> {
        self.storage.require_exact_invite(invite)?;
        if negative.is_cancelled() {
            return Err(refused());
        }
        if self.storage.warm_join_outcome_data()?.is_none() {
            let (transport, rng, _) = self.inputs.as_mut().ok_or_else(refused)?;
            self.storage
                .exchange_first_join(invite, &*transport, negative.clone(), rng)
                .await?;
        }
        if negative.is_cancelled() {
            return Err(refused());
        }
        self.storage.index.attach_joined_node(
            self.storage.pending.as_mut().ok_or_else(refused)?,
            &mut self.inputs,
            &mut self.node,
        )?;
        let (head, own, epoch) = self
            .node
            .as_ref()
            .ok_or_else(refused)?
            .selected_join_data()
            .map_err(|_| refused())?;
        if negative.is_cancelled() || self.storage.warm_join_outcome_data()? != Some((head, epoch))
        {
            return Err(refused());
        }
        Ok((head, own, epoch))
    }
}

impl<R: CryptoRngCore> NativeTextJoinedRuntime<catcoms_net::MeshService, R> {
    /// Move this already covered and registered cold node into the same
    /// physical vault's newly authorized native Source collection.
    pub fn into_recovered_native_sources(
        self,
        session: (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>),
        ui_budget: crate::store::NativeUiInvocationBudget,
        policy: &crate::store::NativeTextSourcePolicy,
        negative: &RequestCancellation,
    ) -> Result<
        (
            crate::store::NativeTextVault,
            crate::store::NativeTextSources<catcoms_net::MeshService, R>,
        ),
        AppError,
    > {
        match self.into_recovered_joined_membership(session, ui_budget, policy, negative)? {
            NativeTextJoinedRecovery::Active(vault, sources) => Ok((vault, sources)),
            NativeTextJoinedRecovery::Removed(_) => Err(AppError::ChatV2MembershipRemoved),
        }
    }

    /// Separate an authenticated terminal selection from an active node.
    /// Missing or contradictory original state refuses both cases.
    pub fn into_recovered_joined_membership(
        mut self,
        session: (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>),
        ui_budget: crate::store::NativeUiInvocationBudget,
        policy: &crate::store::NativeTextSourcePolicy,
        negative: &RequestCancellation,
    ) -> Result<NativeTextJoinedRecovery<R>, AppError> {
        if self.storage.row_count_data()? != 1 {
            return Err(refused());
        }
        let selected = self.recover_joined_membership(negative)?;
        let receipt = self
            .storage
            .recovered_join_registration_data()?
            .ok_or_else(refused)?;
        if let RecoveredJoinedMembership::Removed(removal) = selected {
            // Observe the exact attached actor before dropping both owners.
            // Net can still be draining after the Service destructor.
            let tcp_drain = self
                .storage
                .recovered_tcp
                .as_ref()
                .ok_or_else(refused)?
                .observe_attached_drain()?;
            drop(self.node.take());
            drop(self.inputs.take());
            drop(self.storage.recovered_tcp.take());
            if negative.is_cancelled() {
                return Err(refused());
            }
            self.storage
                .index
                .require_recovered_native_join_removal(receipt, removal)?;
            let ui = UiAdmissionOwner::new(session.0, session.1, session.2, ui_budget)?;
            let key_source = ui.retired_key_source()?;
            let original = NativeTextRemovedJoinedVault {
                ui,
                key_source,
                index: self.storage.index,
                registration: receipt,
                removal,
                tcp_drain,
                rejoin_catalog: None,
                rejoin_pending: None,
                rejoin_tcp: None,
                rejoin_received: None,
                rejoin_welcome: None,
                rejoin_prefix: None,
                rejoin_index: None,
            };
            if negative.is_cancelled() {
                return Err(refused());
            }
            return Ok(NativeTextJoinedRecovery::Removed(original));
        }
        let native_tcp = self.storage.recovered_tcp.take().ok_or_else(refused)?;
        let budget = policy.prepare()?;
        let vault = crate::store::NativeTextVault::from_recovered_joined_index(
            self.storage.index,
            session,
            ui_budget,
        )?;
        let mut sources = vault
            .capture_native_sources(budget)
            .finish()
            .map_err(|_| refused())?;
        sources.install_recovered_joined(self.node.take(), receipt, native_tcp, self._fee)?;
        if negative.is_cancelled() {
            return Err(refused());
        }
        Ok(NativeTextJoinedRecovery::Active(vault, sources))
    }
}
