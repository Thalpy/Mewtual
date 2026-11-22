//! One selected warm join with its original private graph and transport.
use super::*;
use crate::store::persistence::governance_layout::joined::JoinedRuntimeInputs;
use catcoms_replication::chat::{owner_facts::OwnerFactId, owner_header::OwnerHeaderId};
use catcoms_sync::ClosedV2JoinedNode;

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
            if self.pending.is_some() || outcome.is_none() {
                return Err(refused());
            }
            Ok(())
        });
        self.bind_runtime(checked, (transport, rng, clock))
    }

    fn bind_runtime<T: MeshTransport, R: CryptoRngCore>(
        self,
        checked: Result<(), AppError>,
        inputs: JoinedRuntimeInputs<T, R>,
    ) -> Result<NativeTextJoinedRuntime<T, R>, (Self, AppError)> {
        let prepared = checked.and_then(|()| {
            let allowance = ClosedV2JoinedNode::<T, R>::allocation_bytes()
                + 2 * std::mem::size_of::<NativeTextJoinedRuntime<T, R>>();
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

impl<T: MeshTransport, R: CryptoRngCore> NativeTextJoinedRuntime<T, R> {
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
