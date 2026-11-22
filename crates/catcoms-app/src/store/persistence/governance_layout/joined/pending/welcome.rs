//! Covers possible MLS entry on the original warm owner, and keeps successful
//! recovery historical until the native source can install it.
use super::*;
use catcoms_mls::persist::{
    text_current_add::TextCurrentAddLimits, TextCurrentSdkGraphReservation,
    TextCurrentSdkRestoreAdmission, TextCurrentSdkRestoreLimits,
};
use catcoms_sync::{RecoveredV2JoinedOutcome, V2PendingJoinContextData};
use critical_namespaces::joined_pin::welcome as codec;

const ENTRY: &str = "chat-critical-namespaces.joined-welcome-entry.bin";
const OUTCOME: &str = "chat-critical-namespaces.joined-welcome-outcome.bin";
const FRAME_FEE: usize = 12 * (codec::ENTRY_FRAME + codec::OUTCOME_FRAME) + 4 * 1024 * 1024;

fn paid_frames(store: &ServerStore) -> Result<CatalogReadReservation, AppError> {
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    policy.current.archive.pending.pool.try_reserve(FRAME_FEE)
}

pub(super) fn require_absent(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, ENTRY)?)?;
    absent(&fixed(store, false, OUTCOME)?)
}

pub(super) struct WarmWelcome {
    pub(super) delivery: Zeroizing<Vec<u8>>,
    entry: Zeroizing<Vec<u8>>,
    outcome: Option<Zeroizing<Vec<u8>>>,
    held_entry: Option<CreatedStage>,
    held_outcome: Option<CreatedStage>,
    _fee: CatalogReadReservation,
}

impl WarmWelcome {
    pub(super) fn prepare(
        store: &ServerStore,
        required: &PreparedPinnedPending,
        key: &[u8; 32],
        pending_frame: &[u8],
        delivery: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        require_absent(store)?;
        let fee = paid_frames(store)?;
        required.validate_delivery_data(delivery)?;
        let entry = codec::seal_entry(key, required.context_data(), pending_frame, delivery, rng)?;
        Ok(Self {
            delivery: Zeroizing::new(delivery.to_vec()),
            entry,
            outcome: None,
            held_entry: None,
            held_outcome: None,
            _fee: fee,
        })
    }

    pub(super) fn cover_entry(&mut self, store: &ServerStore) -> Result<(), AppError> {
        if self.held_outcome.is_none() {
            absent(&fixed(store, false, OUTCOME)?)?;
        }
        let path = fixed(store, false, ENTRY)?;
        write_held(store, &path, &self.entry, &mut self.held_entry)
    }

    pub(super) fn require_entry(&self, store: &ServerStore) -> Result<(), AppError> {
        exact_retained(
            &fixed(store, false, ENTRY)?,
            self.held_entry.as_ref().ok_or_else(denied)?,
            &self.entry,
        )
    }

    pub(super) fn prepare_outcome(
        &mut self,
        store: &ServerStore,
        required: &mut PreparedPinnedPending,
        key: &[u8; 32],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.require_entry(store)?;
        if self.outcome.is_none() {
            let context = required.context_data();
            let result = required.finish_delivery(key, &self.delivery, rng)?;
            self.outcome = Some(codec::seal_outcome(key, context, &self.entry, result, rng)?);
        }
        Ok(())
    }

    pub(super) fn cover_outcome(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let raw = self.outcome.as_ref().ok_or_else(denied)?;
        let path = fixed(store, false, OUTCOME)?;
        self.require_entry(store)?;
        write_held(store, &path, raw, &mut self.held_outcome)?;
        self.require_entry(store)
    }

    pub(super) fn require_outcome(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_entry(store)?;
        exact_retained(
            &fixed(store, false, OUTCOME)?,
            self.held_outcome.as_ref().ok_or_else(denied)?,
            self.outcome.as_ref().ok_or_else(denied)?,
        )
    }
}

fn write_held(
    store: &ServerStore,
    path: &Path,
    bytes: &[u8],
    receiver: &mut Option<CreatedStage>,
) -> Result<(), AppError> {
    ensure_native_claim(path, receiver)?;
    sync_directory(&store.dir).map_err(io)?;
    ensure(
        path,
        bytes,
        receiver,
        ArchiveOutputMode::Original,
        #[cfg(test)]
        0,
        #[cfg(test)]
        0,
    )?;
    sync_directory(&store.dir).map_err(io)?;
    exact_retained(path, receiver.as_ref().ok_or_else(denied)?, bytes)
}

pub(super) struct RecoveredWelcome {
    entry: OpenedArchiveObject,
    outcome: Option<(OpenedArchiveObject, RecoveredV2JoinedOutcome)>,
    _fee: CatalogReadReservation,
}

impl RecoveredWelcome {
    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        let (file, bytes) = &self.entry;
        exact_held(
            &fixed(store, false, ENTRY)?,
            file,
            session_id(file).map_err(io)?,
            bytes,
        )?;
        let path = fixed(store, false, OUTCOME)?;
        if let Some(((file, bytes), _)) = &self.outcome {
            exact_held(&path, file, session_id(file).map_err(io)?, bytes)
        } else {
            absent(&path)
        }
    }

    pub(super) fn outcome_data(
        &self,
    ) -> Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)> {
        self.outcome
            .as_ref()
            .map(|(_, result)| (result.governing_head_data(), result.epoch_data()))
    }
}

struct RestorePayment {
    limits: TextCurrentSdkRestoreLimits,
    _fee: CatalogReadReservation,
}

impl TextCurrentSdkGraphReservation for RestorePayment {
    fn limits(&self) -> TextCurrentSdkRestoreLimits {
        self.limits
    }
}

pub(super) fn recover(
    store: &ServerStore,
    key: &[u8; 32],
    pending_frame: &[u8],
    pending: &RecoveredPinnedPending,
) -> Result<Option<RecoveredWelcome>, AppError> {
    let frame_fee = paid_frames(store)?;
    let entry_path = fixed(store, false, ENTRY)?;
    let outcome_path = fixed(store, false, OUTCOME)?;
    let Some(entry) = read(&entry_path, codec::ENTRY_FRAME)? else {
        absent(&outcome_path)?;
        return Ok(None);
    };
    let (request, context) = pending.pending_data();
    let delivery = codec::open_entry(key, context, pending_frame, &entry.1)?;
    request
        .validate_first_join_delivery_data(&delivery)
        .map_err(|_| denied())?;
    let outcome = if let Some(outcome) = read(&outcome_path, codec::OUTCOME_FRAME)? {
        let sealed = codec::open_outcome(key, context, &entry.1, &outcome.1)?;
        let restore = TextCurrentAddLimits::governed_text_v2();
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(restore.restore_typed_bytes)?;
        let admission = TextCurrentSdkRestoreAdmission::new(RestorePayment {
            limits: TextCurrentSdkRestoreLimits {
                max_typed_bytes: restore.restore_typed_bytes,
                max_work: restore.restore_work,
            },
            _fee: fee,
        })?;
        let recovered = RecoveredV2JoinedOutcome::open(&sealed, key, request, &delivery, admission)
            .map_err(|_| denied())?;
        Some((outcome, recovered))
    } else {
        None
    };
    let value = RecoveredWelcome {
        entry,
        outcome,
        _fee: frame_fee,
    };
    value.require(store)?;
    Ok(Some(value))
}
