//! One retained index transaction per physical mount. Only historical DATA is
//! produced here: even a covered Pending birth is not an SDK, signing, network
//! or backend-installation grant. The actual founder operation must own those
//! independent authorities and its original generated values.
use super::*;
use crate::store::persistence::native_registry::operational as io;
use catcoms_rt::CryptoRngCore;
use std::sync::atomic::{AtomicBool, Ordering};
use zeroize::Zeroizing;

#[derive(Default)]
pub(super) struct IndexCell {
    pending: Option<Transaction>,
    // Bounded DATA cache, published under this same cell before the completed
    // debt leaves. This survives a lost result without granting another effect.
    last: Option<CommittedPendingData>,
    // No fresh index copies while a completed debt still owns the previous
    // maximum buffers/files. An Arc payer extends time, not reserved capacity.
    retiring: Option<Arc<AtomicBool>>,
}
pub(super) const CELL_METADATA_BYTES: usize = std::mem::size_of::<IndexCell>()
    + std::mem::size_of::<Transaction>()
    + std::mem::size_of::<CompletedIndexDebt>()
    + std::mem::size_of::<RetirementAck>()
    + 128;
impl IndexCell {
    pub(super) fn require_idle(&self) -> Result<(), AppError> {
        if self.pending.is_some()
            || self
                .retiring
                .as_ref()
                .is_some_and(|done| !done.load(Ordering::Acquire))
        {
            return Err(refused());
        }
        Ok(())
    }
}

/// A frozen, bounded Pending-only input. This private storage role cannot be
/// obtained from a renderer or used to activate a row. The production network
/// plan will supply these exact original values, never recreate them on retry.
pub(super) struct PendingBirth {
    identity: [u8; 32],
    name: String,
    is_dm: bool,
    birth: data::Birth,
    network_frame: Zeroizing<Vec<u8>>,
}

struct Transaction {
    io: io::IndexCommitIo,
    frame: Zeroizing<Vec<u8>>,
    selected: data::Index,
    candidate: data::Index,
    result: CommittedPendingData,
    retirement: Arc<AtomicBool>,
    // Populated once, at final admission. No release-on-Drop: uncertain IO or
    // unwinding must never announce Idle to JOIN.
    claim: Option<interlock::IndexClaim>,
    // Actual file/buffer field destruction precedes this test observation.
    #[cfg(test)]
    retirement_observation: Option<io::RetirementObservation>,
    // LAST: a completed debt may be moved out after the mount's final Arc. Its
    // original files and copies must still precede the actual payment's drop.
    _fee: Arc<payment::IndexPayment>,
}

/// Already-covered files/buffers, moved out for destruction only AFTER catalog
/// and cell guards have ended. This is not a credential and has no reuse API.
pub(super) struct CompletedIndexDebt {
    _transaction: Transaction,
    // LAST: signal only AFTER actual original file/buffer destruction. There
    // is no callback, physical back-reference or authorization in this ack.
    _retirement: RetirementAck,
}
struct RetirementAck {
    done: Arc<AtomicBool>,
    _fee: Arc<payment::IndexPayment>,
}
impl Drop for RetirementAck {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
struct CommittedPendingData {
    invocation: [u8; 32],
    request: [u8; 32],
    input: [u8; 32],
    status: mount::NativeCreationStatusData,
}
impl PendingBirth {
    // Closed, length-delimited commitment to ALL the original storage input.
    // It is only a retained-hit comparison, never a caller authentication key.
    fn digest(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new();
        hash.update(b"catcoms/native-index2/pending-input/v1\0");
        for bytes in [
            self.identity.as_slice(),
            self.name.as_bytes(),
            &[u8::from(self.is_dm)],
            self.birth.invocation.as_slice(),
            self.birth.request.as_slice(),
            self.birth.signer.as_slice(),
            self.birth.network.as_slice(),
            self.network_frame.as_slice(),
        ] {
            hash.update(&(bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        *hash.finalize().as_bytes()
    }
}

impl mount::NativeIndexPhysical {
    /// Capture the exact parent and decode both schemas under this mount's
    /// existing maximum-overlap payment. Callers own the physical cell guard.
    fn capture_index(
        &self,
        cell: &IndexCell,
    ) -> Result<(io::IndexSnapshot, data::Index), AppError> {
        cell.require_idle()?;
        self.handler.require()?;
        let store = self.current.checked_store()?;
        let snapshot = io::IndexSnapshot::capture(store)?;
        if snapshot.stage().is_some() {
            return Err(refused());
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = upgrade::catalog_plain(&key, snapshot.catalog())?;
        let cap = upgrade::capability_data(&plain, &store.chat_vault_domain()?)?;
        capability::Capability::decode_exact(
            &upgrade::authenticate(
                &key,
                upgrade::MARKER_KEY,
                snapshot.marker(),
                super::super::preparation::MAX_MARKER_FRAME,
            )?,
            cap,
        )?;
        let index = data::Index::decode_data(
            &upgrade::authenticate(&key, upgrade::INDEX_KEY, snapshot.index(), data::MAX_FRAME)?,
            cap,
        )?;
        snapshot.require_selected(store)?;
        self.current.cached_store()?;
        Ok((snapshot, index))
    }

    // Return only bounded Copy projections. An owned decoded Index must never
    // escape this guard: otherwise another read/transaction could allocate
    // against its same fixed reservation before those old buffers dropped.
    pub(super) fn row_count_data(&self) -> Result<usize, AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        let (_snapshot, index) = self.capture_index(&cell)?;
        #[cfg(test)]
        after_decode();
        Ok(index.rows.len())
    }

    pub(super) fn lookup_creation_data(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<mount::NativeCreationStatusData>, AppError> {
        let cell = self.cell.try_lock().map_err(|_| refused())?;
        let (_snapshot, index) = self.capture_index(&cell)?;
        #[cfg(test)]
        after_decode();
        index
            .lookup_data(invocation, request)?
            .map(mount::status_data)
            .transpose()
    }

    /// Prepare/admit ONE Pending birth. An exact retained hit does no IO or
    /// RNG. An exact Pending disk hit returns historical status only, never
    /// live authority. Other historical states use the separate lookup API;
    /// they cannot validate a supplied Pending network frame they no longer hold.
    /// The caller must perform this lookup before generating its device/net
    /// plan as well; this storage check cannot undo earlier caller effects.
    pub(super) fn prepare_pending_birth(
        &self,
        input: &PendingBirth,
        rng: &mut impl CryptoRngCore,
    ) -> Result<mount::NativeCreationStatusData, AppError> {
        if input.name.len() > super::super::data::MAX_NAME
            || !(40..=data::MAX_NETWORK_FRAME).contains(&input.network_frame.len())
        {
            return Err(refused());
        }
        // Poison is deliberately held until cold recovery. Recovering a mutex
        // alone cannot prove whether an unwinding file effect completed. Never
        // clear the cell or read a fresh parent to manufacture Idle here.
        let mut cell = self.cell.try_lock().map_err(|_| refused())?;
        if let Some(transaction) = &cell.pending {
            return transaction.exact_pending(input);
        }
        if let Some(last) = &cell.last {
            if last.invocation == input.birth.invocation {
                if last.request != input.birth.request || last.input != input.digest() {
                    return Err(refused());
                }
                return Ok(last.status);
            }
        }
        if !self.handler.gate.idle() {
            return Err(refused());
        }
        cell.require_idle()?;
        cell.retiring = None; // Only a completed negative can be pruned here.
        let (snapshot, selected) = self.capture_index(&cell)?;
        if let Some(row) = selected.lookup_data(input.birth.invocation, input.birth.request)? {
            return exact_pending_row(row, input);
        }
        // This phase has no Stage, SDK or network effect. All allocation,
        // encoding and sealing happens BEFORE catalog/claim admission. If JOIN
        // overtakes preparation, the original snapshot refuses; its buffers are
        // dropped only after the final catalog guard has been released.
        let mut candidate =
            data::Index::decode_data(&selected.encode_data()?, selected.capability)?;
        candidate.generation = selected.generation.checked_add(1).ok_or_else(refused)?;
        candidate.parent = Some(*blake3::hash(snapshot.index()).as_bytes());
        let id = selected.last_issued_id.checked_add(1).ok_or_else(refused)?;
        candidate.last_issued_id = id;
        candidate.rows.push(data::Row {
            id,
            identity: input.identity,
            name: input.name.clone(),
            is_dm: input.is_dm,
            backend: data::Backend::Pending {
                birth: input.birth.clone(),
                network_frame: input.network_frame.clone(),
            },
        });
        if selected.successor_data(&candidate, candidate.parent.ok_or_else(refused)?)?
            != data::Change::Birth
        {
            return Err(refused());
        }
        let store = self.current.cached_store()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let frame = upgrade::sealed(
            &key,
            upgrade::INDEX_KEY,
            &candidate.encode_data()?,
            data::MAX_FRAME,
            rng,
        )?;
        let parent_catalog = *blake3::hash(snapshot.catalog()).as_bytes();
        let status = mount::status_data(candidate.rows.last().ok_or_else(refused)?)?;
        let mut prepared = Transaction {
            io: io::IndexCommitIo::prepare(snapshot, &frame)?,
            frame,
            selected,
            candidate,
            result: CommittedPendingData {
                invocation: input.birth.invocation,
                request: input.birth.request,
                input: input.digest(),
                status,
            },
            retirement: Arc::new(AtomicBool::new(false)),
            claim: None,
            #[cfg(test)]
            retirement_observation: None,
            _fee: self.handler.fee.clone(),
        };
        {
            let catalog_guard = store
                .pending_join_catalog
                .try_lock()
                .map_err(|_| refused())?;
            let catalog = catalog_guard.as_ref().ok_or_else(refused)?;
            if catalog.pending.is_some() || catalog.current.framed != parent_catalog {
                return Err(refused());
            }
            // Repeat the held complete parent tuple after preparation's large
            // reads/copies. Only then can the atomic negative claim be issued.
            prepared.io.require_unstarted_parent(store)?;
            let claim = self.handler.gate.claim()?;
            // NO fallible step, allocation, callback or IO between claim and
            // retained publication. Losing the caller can no longer lose debt.
            prepared.claim = Some(claim);
            cell.pending = Some(prepared);
        }
        Ok(status)
    }

    /// Finish exactly the mount-retained transaction. Results/unwinds never
    /// remove uncertain debt. A closed facade may finish this admitted DATA,
    /// but its CLOSED interlock is never reopened by completion.
    pub(super) fn finish_pending_birth(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<(mount::NativeCreationStatusData, CompletedIndexDebt), AppError> {
        self.handler.require()?;
        let mut cell = self.cell.try_lock().map_err(|_| refused())?;
        let transaction = cell.pending.as_mut().ok_or_else(refused)?;
        let row = transaction.original_row(invocation, request)?;
        let status = mount::status_data(row)?;
        // Every retained write is a checked Birth. Revalidate the relation in
        // memory; retry must never substitute a different decoded candidate.
        if transaction.selected.successor_data(
            &transaction.candidate,
            transaction.candidate.parent.ok_or_else(refused)?,
        )? != data::Change::Birth
        {
            return Err(refused());
        }
        let store = self.current.cached_store()?;
        {
            let catalog_guard = store
                .pending_join_catalog
                .try_lock()
                .map_err(|_| refused())?;
            if catalog_guard
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .is_some()
            {
                return Err(refused());
            }
            transaction.io.commit_index(store, &transaction.frame)?;
        }
        // Covering barriers succeeded. Claim release and whole-debt extraction
        // contain no IO/callback. Destruction is deferred to the outer caller.
        transaction.claim.take().ok_or_else(refused)?.finish()?;
        let result = transaction.result;
        let retirement = RetirementAck {
            done: transaction.retirement.clone(),
            _fee: transaction._fee.clone(),
        };
        let retired = transaction.retirement.clone();
        cell.last = Some(result); // Cache BEFORE releasing any original custody.
        cell.retiring = Some(retired);
        let completed = cell.pending.take().ok_or_else(refused)?;
        Ok((
            status,
            CompletedIndexDebt {
                _transaction: completed,
                _retirement: retirement,
            },
        ))
    }
}

impl Transaction {
    fn original_row(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<&data::Row, AppError> {
        let row = self.candidate.rows.last().ok_or_else(refused)?;
        let birth = row.backend.birth().ok_or_else(refused)?;
        if birth.invocation != invocation || birth.request != request {
            return Err(refused());
        }
        Ok(row)
    }
    fn exact_pending(
        &self,
        input: &PendingBirth,
    ) -> Result<mount::NativeCreationStatusData, AppError> {
        let row = self.original_row(input.birth.invocation, input.birth.request)?;
        exact_pending_row(row, input)
    }
}

// Use the SAME full comparison for retained and disk hits. Cache eviction or
// restart must never turn changed generated inputs into an accepted retry.
fn exact_pending_row(
    row: &data::Row,
    input: &PendingBirth,
) -> Result<mount::NativeCreationStatusData, AppError> {
    if row.identity != input.identity || row.name != input.name || row.is_dm != input.is_dm {
        return Err(refused());
    }
    match &row.backend {
        data::Backend::Pending {
            birth,
            network_frame,
        } if birth == &input.birth && network_frame == &input.network_frame => {
            mount::status_data(row)
        }
        _ => Err(refused()),
    }
}

#[cfg(test)]
thread_local! {
    static AFTER_DECODE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn after_decode() {
    let hook = AFTER_DECODE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
mod tests;
