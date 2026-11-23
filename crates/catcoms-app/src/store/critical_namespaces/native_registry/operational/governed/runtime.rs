//! One selected warm join with its original private graph and transport.
use super::super::join_tcp;
use super::*;
use crate::store::persistence::governance_layout::joined::JoinedRuntimeInputs;
use catcoms_replication::chat::{owner_facts::OwnerFactId, owner_header::OwnerHeaderId};
use catcoms_sync::{ClosedV2JoinedNode, MAX_V2_FIRST_ADD_PROOF_BYTES};
use std::sync::atomic::{AtomicBool, AtomicU64};
use zeroize::Zeroizing;

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

impl NativeTextJoinedStorage {
    /// Bind the transport before exchanging the covered request. A retry of
    /// the owning runtime keeps the same transport and private request.
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
        if self.storage.pending.is_some() || negative.is_cancelled() {
            return Err(refused());
        }
        self.storage
            .index
            .attach_recovered_joined_node(&mut self.inputs, &mut self.node)?;
        let node = self
            .node
            .as_ref()
            .filter(|node| node.was_recovered_data())
            .ok_or_else(refused)?;
        let (head, own, epoch) = node.selected_join_data().map_err(|_| refused())?;
        if self.storage.recovered_tcp.is_some() {
            let receipt = self
                .storage
                .recovered_join_registration_data()?
                .ok_or_else(refused)?;
            if receipt.selected_data() != (head, own, epoch)
                || receipt.transport_data().0 != node.local_peer_data()
            {
                return Err(refused());
            }
            self.storage.index.require_registered_native_join(receipt)?;
        }
        if negative.is_cancelled()
            || self.storage.recovered_join_outcome_data()? != Some((head, epoch))
        {
            return Err(refused());
        }
        Ok((head, own, epoch))
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
        mut self,
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
        if self.storage.row_count_data()? != 1 {
            return Err(refused());
        }
        self.complete_recovery(negative)?;
        let receipt = self
            .storage
            .recovered_join_registration_data()?
            .ok_or_else(refused)?;
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
        Ok((vault, sources))
    }
}
