//! One normal3 transaction retained inside the affine selected owner. The
//! mutex is still held across actual IO: exclusive Rust borrowing alone would
//! not poison the state when a file effect unwinds. Poison requires cold open.
use super::{admission, preparation, rename, wal, *};
use crate::store::{
    critical_namespaces::native_registry::preparation::NativeRegistryHandler,
    persistence::native_registry::lifecycle::upgrade::{index::IndexCommitIo, UpgradeInventory},
    V2CurrentLayoutStore,
};
use catcoms_crypto::seal;
use catcoms_rt::CryptoRngCore;
use std::sync::Mutex;
use zeroize::Zeroizing;

#[derive(Default)]
pub(in crate::store::critical_namespaces::native_registry) struct Cell {
    pending: Option<Transaction>,
}
impl Cell {
    pub(in crate::store::critical_namespaces::native_registry) fn require_idle(
        &self,
    ) -> Result<(), AppError> {
        if self.pending.is_some() {
            return Err(refused());
        }
        Ok(())
    }
}

struct Transaction {
    io: IndexCommitIo,
    frame: Zeroizing<Vec<u8>>,
    _selected: Box<data::Index>,
    _candidate: Box<data::Index>,
    request: rename::Request,
    result: rename::Metadata,
    claim: Option<admission::Claim>,
    // Original physical exclusion outlives every file/buffer, even if the
    // affine Store closes before an already-completed caller drops its debt.
    _session: catcoms_storage::VaultPhysicalWorkLease,
    _payment: admission::PaymentTail,
}

/// No reuse API or public physical loan. The future native caller must place
/// this in its outer retirement slot BEFORE leaving any native commit guard,
/// and drop it outside native/cell/catalog guards on every error/unwind path.
/// That native consumer is intentionally not implemented by this storage slice.
pub(in crate::store::critical_namespaces::native_registry) struct CompletedDebt {
    _transaction: Transaction,
    // LAST, not a Drop body on CompletedDebt: field destruction must actually
    // finish before JOIN/new reads can observe the original claim as Idle.
    _ack: RetirementAck,
}
struct RetirementAck(Option<admission::Claim>);
impl Drop for RetirementAck {
    fn drop(&mut self) {
        if let Some(claim) = self.0.take() {
            claim.finish_after_retirement();
        }
    }
}

pub(in crate::store::critical_namespaces::native_registry) const METADATA_BYTES: usize = 8
    * (std::mem::size_of::<Mutex<Cell>>()
        + std::mem::size_of::<Transaction>()
        + std::mem::size_of::<CompletedDebt>()
        + std::mem::size_of::<RetirementAck>()
        + std::mem::size_of::<IndexCommitIo>()
    + std::mem::size_of::<crate::store::persistence::native_registry::lifecycle::upgrade::index::IndexStageCleanup>()
        + std::mem::size_of::<wal::VerifiedWal>())
    + 1024;

/// A CAS on local metadata, not an invocation-receipt protocol. Exact retained
/// retries share the frozen candidate and draw no RNG. After completion/cold
/// open, a stale revision refuses even if the desired name is now visible.
/// Only current revision + same name is a no-change observation.
pub(in crate::store::critical_namespaces::native_registry) fn rename_metadata(
    cell: &Mutex<Cell>,
    current: &V2CurrentLayoutStore,
    handler: &admission::Handler,
    request: &rename::Request,
    rng: &mut impl CryptoRngCore,
) -> Result<(rename::Metadata, Option<CompletedDebt>), AppError> {
    if request.name.len() > super::super::data::MAX_NAME {
        return Err(refused());
    }
    let mut cell = cell.try_lock().map_err(|_| refused())?;
    handler.require()?;
    let store = current.checked_store()?;
    if let Some(pending) = &cell.pending {
        if &pending.request != request {
            return Err(refused());
        }
    } else {
        handler.require_idle_cached()?; // also excludes finite completed debt
        let inventory = UpgradeInventory::capture(store, handler.debit(), handler.pool())?;
        if inventory.seed().is_some()
            || inventory.has_upgrade_outputs()
            || inventory.index_stage().is_some()
            || inventory.catalog_stage().is_some()
        {
            return Err(refused());
        }
        let (selected, _) = preparation::decode_selected_data(store, &inventory)?;
        let parent = *blake3::hash(inventory.index()).as_bytes();
        let plan = rename::prepare(selected, request, parent)?;
        let rename::Plan::Change {
            selected,
            candidate,
            result,
        } = plan
        else {
            let rename::Plan::NoChange(result) = plan else {
                unreachable!()
            };
            // No sealing/RNG/effect. Reprove the same observation under the
            // actual catalog mutex before reporting current metadata.
            let guard = store
                .pending_join_catalog
                .try_lock()
                .map_err(|_| refused())?;
            if guard.as_ref().ok_or_else(refused)?.pending.is_some() {
                return Err(refused());
            }
            handler.require_idle_cached()?;
            inventory.require(store)?;
            return Ok((result, None));
        };
        let key = Zeroizing::new(store.keys.db_key()?);
        let derived = Zeroizing::new(blake3::derive_key(preparation::INDEX_KEY, key.as_slice()));
        let plain = Zeroizing::new(candidate.encode_data()?);
        let frame = Zeroizing::new(crate::store::frame(&seal(&derived, &plain, rng)?));
        let proof = wal::verify_warm_rename(store, &inventory, &candidate, &frame)?;
        let io = IndexCommitIo::prepare(store, inventory, proof, &frame)?;
        // All allocations/preparation precede the writer and final claim.
        let mut transaction = Transaction {
            io,
            frame,
            _selected: selected,
            _candidate: candidate,
            request: rename::Request {
                id: request.id,
                identity: request.identity,
                expected_revision: request.expected_revision,
                name: request.name.clone(),
            },
            result,
            claim: None,
            _session: store._session.retain_for_physical_work(),
            _payment: handler.payment_tail(),
        };
        let guard = store
            .pending_join_catalog
            .try_lock()
            .map_err(|_| refused())?;
        let catalog = guard.as_ref().ok_or_else(refused)?;
        if catalog.pending.is_some()
            || !matches!(store.current_layout_policy.as_ref()
            .and_then(|policy| policy.native_registry.as_ref()),
            Some(NativeRegistryHandler::Lifecycle(actual)) if actual.same(handler))
        {
            return Err(refused());
        }
        current.cached_store()?;
        transaction.io.require_unstarted_parent(store)?;
        #[cfg(test)]
        super::super::operational::upgrade_source::owning::observe_edge(
            super::super::operational::upgrade_source::owning::Edge::BeforeMetadataAdmission,
        );
        transaction.claim = Some(handler.claim()?);
        // Infallible move after admission, before unlocking or any file effect.
        cell.pending = Some(transaction);
        drop(guard);
    }
    let transaction = cell.pending.as_mut().ok_or_else(refused)?;
    transaction.io.commit(store, &transaction.frame)?;
    #[cfg(test)]
    super::super::operational::upgrade_source::owning::observe_edge(
        super::super::operational::upgrade_source::owning::Edge::AfterMetadataCommit,
    );
    transaction.io.require_complete(store, &transaction.frame)?;
    // The original claim stays ACTIVE across the final proof and all outgoing
    // resource destruction. Nothing below publishes Idle under a guard.
    let guard = store
        .pending_join_catalog
        .try_lock()
        .map_err(|_| refused())?;
    if guard.as_ref().ok_or_else(refused)?.pending.is_some() {
        return Err(refused());
    }
    current.cached_store()?;
    transaction.io.require_complete(store, &transaction.frame)?;
    transaction
        .claim
        .as_ref()
        .ok_or_else(refused)?
        .require_active()?;
    let mut transaction = cell.pending.take().ok_or_else(refused)?;
    let result = transaction.result;
    let ack = RetirementAck(transaction.claim.take());
    drop(guard);
    drop(cell);
    Ok((
        result,
        Some(CompletedDebt {
            _transaction: transaction,
            _ack: ack,
        }),
    ))
}

#[cfg(test)]
mod tests;
