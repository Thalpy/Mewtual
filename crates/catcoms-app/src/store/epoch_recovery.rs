//! Durable P1 recovery-slot transitions, not the settlement transaction itself.
//!
//! A single sealed replacement contains both retained snapshots, the staged snapshot and its
//! original deadline. No caller can persist only the eviction while forgetting the incoming
//! version. Mutations require exclusive access to the non-cloneable, session-locked ServerStore
//! and reload the record, so a stale view cannot overwrite a newer transition.
//!
//! This prerequisite is deliberately not wired to remote input, settlement pruning or UI yet.
//! The accounted save adapter requires a complete trusted EpochStorageBudget, including all
//! temporary siblings. Inventory bootstrap and the multi-record settlement coordinator are not
//! wired yet; the unaccounted primitive alone is not a server-wide quota. Like held blobs, these
//! records currently remain after `remove_server`. Cleanup/retention must be integrated explicitly.

use std::io::Read;

use catcoms_replication::{
    epoch::MAX_RECOVERY_SLOTS_BYTES, LogicalDocument, RecoverySlots, RecoverySnapshot,
    RecoveryTransition, ReplError,
};
use catcoms_rt::Clock;

use super::epoch_budget::{
    EpochStorageBudget, Footprint, Replacement, StorageRecord, StorageScope, WritePurpose,
};
use super::*;

pub(super) mod cleanup;
pub(super) mod inventory;

// Scope <= 501 bytes, completion metadata <= 73 bytes, and length framing. This is a local
// persistence format, independent of the protocol's signed records and Automerge encodings.
const MAX_RECORD_BYTES: usize = MAX_RECOVERY_SLOTS_BYTES + 1024;
const MAX_SEALED_BYTES: usize = MAX_RECORD_BYTES + 24 + 16;
const RECORD_DOMAIN: &[u8] = b"catcoms/epoch-recovery-store/v1";

// Never derive Debug: authenticated plaintext contains private recovery projections.
pub(super) struct AuthenticatedEpochFileBytes {
    pub(super) plain: Zeroizing<Vec<u8>>,
    pub(super) physical_bytes: u64,
}

/// One requested recovery transition. Callers construct snapshots from a sealed epoch; this
/// persistence boundary validates their generic schema, not the consumer's domain projection.
pub enum EpochRecoveryAction {
    /// Retain a version, or stage it behind the existing two-version eviction warning.
    Stage(RecoverySnapshot),
    /// Authorize only the exact pair named by a persisted warning.
    Acknowledge {
        /// Previously retained version the warning says will be removed.
        oldest_snapshot: [u8; 32],
        /// Staged replacement named by the same warning.
        staged_snapshot: [u8; 32],
    },
    /// Promote a staged version if its persisted seven-day deadline has elapsed.
    AdvanceTime,
}

impl std::fmt::Debug for EpochRecoveryAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Recovery contains private content. Generic command logging must not print projections.
        f.write_str(match self {
            Self::Stage(_) => "Stage(<private recovery snapshot>)",
            Self::Acknowledge { .. } => "Acknowledge(<snapshot ids>)",
            Self::AdvanceTime => "AdvanceTime",
        })
    }
}

/// A read-only view of one durable record. Possession is not a permit to prune an epoch: the
/// future settlement transaction must also persist its receipt, checkpoint, gate and intents.
#[derive(Default)]
pub struct EpochRecoveryState {
    slots: RecoverySlots,
    // Remember exactly the most recent completed eviction. Without this, a post-rename flush
    // error could make an acknowledgement impossible to retry: its staged slot is already gone.
    completed_eviction: Option<([u8; 32], [u8; 32])>,
}

impl std::fmt::Debug for EpochRecoveryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochRecoveryState")
            .field("retained", &self.slots.retained().len())
            .field("staged", &self.slots.staged().is_some())
            .finish_non_exhaustive()
    }
}

impl EpochRecoveryState {
    fn footprint(&self, physical_bytes: u64) -> Result<Footprint, AppError> {
        // The staged blob's length prefix and timestamp belong to the settlement reserve too.
        // The slot-presence tag exists in both forms and remains ordinary record overhead.
        let settlement = if let Some(staged) = self.slots.staged() {
            let bytes = Zeroizing::new(staged.encode().map_err(invalid)?);
            bytes.len() as u64 + 4 + 8
        } else {
            0
        };
        let content = physical_bytes
            .checked_sub(settlement)
            .ok_or_else(|| invalid("inconsistent staged length"))?;
        Ok(Footprint {
            content,
            protocol: 0,
            settlement,
        })
    }

    /// Newest-first materializations available to Restore/Export when those actions are wired.
    pub fn retained(&self) -> impl ExactSizeIterator<Item = &RecoverySnapshot> {
        self.slots.retained()
    }

    /// Incoming version held until the warning is acknowledged or its deadline elapses.
    pub fn staged(&self) -> Option<&RecoverySnapshot> {
        self.slots.staged()
    }

    /// The original warning and deadline, including after restart. Reading never evicts.
    pub fn eviction_pending(&self) -> Result<Option<RecoveryTransition>, AppError> {
        self.slots.eviction_pending().map_err(invalid)
    }

    fn apply(
        &mut self,
        action: EpochRecoveryAction,
        now_ms: u64,
    ) -> Result<RecoveryTransition, AppError> {
        let before = self.slots.eviction_pending().map_err(invalid)?;
        let result = match action {
            EpochRecoveryAction::Stage(snapshot) => self.slots.stage(snapshot, now_ms),
            EpochRecoveryAction::Acknowledge {
                oldest_snapshot,
                staged_snapshot,
            } if before.is_none()
                && self.completed_eviction == Some((oldest_snapshot, staged_snapshot)) =>
            {
                // A matching retry does not promote anything, but the caller must still save
                // this state again to repair a previous committed-but-not-durable outcome.
                Ok(RecoveryTransition::Unchanged)
            }
            EpochRecoveryAction::Acknowledge {
                oldest_snapshot,
                staged_snapshot,
            } => self
                .slots
                .acknowledge_eviction(oldest_snapshot, staged_snapshot),
            EpochRecoveryAction::AdvanceTime => self.slots.advance_time(now_ms),
        }
        .map_err(invalid)?;
        if result == RecoveryTransition::Promoted {
            self.completed_eviction = match before {
                Some(RecoveryTransition::EvictionPending {
                    oldest_snapshot,
                    staged_snapshot,
                    ..
                }) => Some((oldest_snapshot, staged_snapshot)),
                _ => None,
            };
        }
        Ok(result)
    }

    fn encode(
        &self,
        scope: &[u8],
        document: &LogicalDocument,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.check_scope(document)?;
        let mut e = Encoder::new();
        e.put_bytes(scope).map_err(invalid)?;
        let slots = Zeroizing::new(self.slots.encode().map_err(invalid)?);
        e.put_bytes(&slots).map_err(invalid)?;
        match self.completed_eviction {
            None => {
                e.put_u8(0);
            }
            Some((oldest, staged)) => {
                e.put_u8(1);
                e.put_bytes(&oldest).expect("fixed hash fits");
                e.put_bytes(&staged).expect("fixed hash fits");
            }
        }
        let bytes = Zeroizing::new(e.finish());
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(invalid("recovery record exceeds its bound"));
        }
        Ok(bytes)
    }

    fn decode(bytes: &[u8], scope: &[u8], document: &LogicalDocument) -> Result<Self, AppError> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(invalid("recovery record exceeds its bound"));
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes().map_err(invalid)? != scope {
            return Err(invalid(
                "recovery record belongs to another document or server",
            ));
        }
        let slots = RecoverySlots::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?;
        let completed_eviction = match d.get_u8().map_err(invalid)? {
            0 => None,
            1 => Some((
                d.get_bytes()
                    .map_err(invalid)?
                    .try_into()
                    .map_err(invalid)?,
                d.get_bytes()
                    .map_err(invalid)?
                    .try_into()
                    .map_err(invalid)?,
            )),
            _ => return Err(invalid("unknown recovery completion tag")),
        };
        d.finish().map_err(invalid)?;
        let state = Self {
            slots,
            completed_eviction,
        };
        state.check_scope(document)?;
        // A completed pair is evidence only while its replacement is still retained and the
        // old version is gone. It cannot bless arbitrary acknowledgements from a corrupt record.
        if let Some((oldest, staged)) = completed_eviction {
            let ids = state
                .retained()
                .map(RecoverySnapshot::id)
                .collect::<Result<Vec<_>, _>>()
                .map_err(invalid)?;
            if oldest == staged || ids.contains(&oldest) || !ids.contains(&staged) {
                return Err(invalid("inconsistent recovery completion"));
            }
        }
        Ok(state)
    }

    fn check_scope(&self, document: &LogicalDocument) -> Result<(), AppError> {
        for snapshot in self.slots.retained().chain(self.slots.staged()) {
            if snapshot.doc_type != document.doc_type
                || snapshot.logical_key != document.logical_key
            {
                return Err(invalid(ReplError::EpochScope));
            }
        }
        Ok(())
    }
}

/// Result returned only after the complete sealed replacement has been saved successfully.
#[derive(Debug)]
pub struct EpochRecoveryUpdate {
    /// What changed; a matching retry may report Unchanged after re-saving successfully.
    pub transition: RecoveryTransition,
    /// The full saved view, including a warning that must be resurfaced after restart.
    pub state: EpochRecoveryState,
}

impl ServerStore {
    /// Observe this document's authenticated physical record for inventory reconciliation. This
    /// is ONE inventory entry, not a complete server inventory: the caller must also include all
    /// other documents, protocol records, and temporary/orphan files before admitting any write.
    pub fn epoch_recovery_inventory_record(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<Option<StorageRecord>, AppError> {
        let scope = scope_bytes(server, document)?;
        let (state, size) = self.read_epoch_recovery_record(&scope, document)?;
        size.map(|size| {
            state
                .footprint(size)
                .map(|footprint| recovery_record(&scope, footprint))
        })
        .transpose()
    }

    /// Accounted variant of `update_epoch_recovery`, for the future settlement coordinator. The
    /// supplied non-cloneable budget must cover the COMPLETE local server inventory. Both the
    /// old file's pool split and owner are verified before admission; no guessed future deletion
    /// creates headroom. In particular a first/second retained recovery version can be refused at
    /// the content cap even if a later multi-record settlement could free history.
    ///
    /// Any write error or abandoned reservation closes this budget until full reconciliation.
    /// This does not bootstrap an inventory, prune content, or authorize checkpoint installation.
    #[allow(clippy::too_many_arguments)]
    pub fn update_epoch_recovery_accounted(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        action: EpochRecoveryAction,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
    ) -> Result<EpochRecoveryUpdate, AppError> {
        self.update_epoch_recovery_accounted_with_writer(
            server,
            document,
            action,
            clock,
            rng,
            budget,
            atomic_write,
        )
    }

    // Store-internal writer seam shared by typed registry staging; it exercises the same
    // admission/commit order with exact disk failures. No public caller can inject a writer.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_epoch_recovery_accounted_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        action: EpochRecoveryAction,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<EpochRecoveryUpdate, AppError> {
        let scope = scope_bytes(server, document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let (mut state, old_bytes) = match self.read_epoch_recovery_record(&scope, document) {
            Ok(record) => record,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        let id = *blake3::hash(&scope).as_bytes();
        let observed = old_bytes
            .map(|size| {
                state
                    .footprint(size)
                    .map(|footprint| recovery_record(&scope, footprint))
            })
            .transpose()?;
        budget
            .verify_record(&storage_scope, id, observed)
            .map_err(invalid)?;
        let transition = state.apply(action, clock.now_ms())?;
        let plain = state.encode(&scope, document)?;
        // XChaCha20-Poly1305 adds a 24-byte nonce and a 16-byte tag. The full copy, not only
        // positive growth over the old file, must fit while atomic_write prepares its sibling.
        let record = recovery_record(&scope, state.footprint(plain.len() as u64 + 40)?);
        let reservation = budget
            .reserve(
                &storage_scope,
                Replacement {
                    record,
                    scratch_bytes: 0,
                    purpose: WritePurpose::Settlement,
                },
            )
            .map_err(invalid)?;
        let sealed = match self.keys.db_key().and_then(|key| seal(&key, &plain, rng)) {
            Ok(sealed) => sealed,
            Err(error) => {
                reservation.cancel_before_write();
                return Err(error.into());
            }
        };
        writer(&self.epoch_recovery_path(&scope), &frame(&sealed))?;
        reservation.commit();
        Ok(EpochRecoveryUpdate { transition, state })
    }

    /// Load a bounded, authenticated recovery record. Absence alone means empty; corruption,
    /// a wrong scope, a non-regular file or a failed read is never treated as empty recovery.
    pub fn load_epoch_recovery(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<EpochRecoveryState, AppError> {
        let scope = scope_bytes(server, document)?;
        self.read_epoch_recovery(&scope, document)
    }

    /// Low-level, unaccounted persistence, retained for bootstrap/local tooling. Use
    /// `update_epoch_recovery_accounted` when connecting this to P1 settlement. This writes even on a
    /// matching retry/no-op so a previous post-rename directory-flush failure can be repaired.
    ///
    /// On any error, do not install a checkpoint, prune history, or report successful eviction.
    /// CommittedButNotDurable means the replacement may already be visible; reload and retry
    /// the same action. A successful write provides the existing store's file-sync/atomic-rename
    /// guarantee, plus parent-directory sync on Unix, not immunity to storage-device failure.
    /// The Clock's wall time is used because this seven-day deadline must survive process exit.
    pub fn update_epoch_recovery(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        action: EpochRecoveryAction,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
    ) -> Result<EpochRecoveryUpdate, AppError> {
        self.update_epoch_recovery_with_writer(server, document, action, clock, rng, atomic_write)
    }

    // The extra argument is the deterministic disk-failure seam; the production API has no
    // caller-supplied writer and always uses the existing atomic persistence primitive.
    #[allow(clippy::too_many_arguments)]
    fn update_epoch_recovery_with_writer(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        action: EpochRecoveryAction,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
    ) -> Result<EpochRecoveryUpdate, AppError> {
        let scope = scope_bytes(server, document)?;
        let mut state = self.read_epoch_recovery(&scope, document)?;
        let transition = state.apply(action, clock.now_ms())?;
        let plain = state.encode(&scope, document)?;
        let sealed = seal(&self.keys.db_key()?, &plain, rng)?;
        writer(&self.epoch_recovery_path(&scope), &frame(&sealed))?;
        Ok(EpochRecoveryUpdate { transition, state })
    }

    fn epoch_recovery_path(&self, scope: &[u8]) -> PathBuf {
        // BLAKE3 is a local filename derivation only, not a protocol record/change identity.
        // Full framing and the domain distinguish this key from other vault record namespaces.
        self.dir
            .join("servers")
            .join(format!("{}.recovery", blake3::hash(scope).to_hex()))
    }

    fn read_epoch_recovery(
        &self,
        scope: &[u8],
        document: &LogicalDocument,
    ) -> Result<EpochRecoveryState, AppError> {
        self.read_epoch_recovery_record(scope, document)
            .map(|(state, _)| state)
    }

    fn read_epoch_recovery_record(
        &self,
        scope: &[u8],
        document: &LogicalDocument,
    ) -> Result<(EpochRecoveryState, Option<u64>), AppError> {
        let path = self.epoch_recovery_path(scope);
        match self.read_epoch_recovery_plain(&path)? {
            None => Ok((EpochRecoveryState::default(), None)),
            Some(bytes) => Ok((
                EpochRecoveryState::decode(&bytes.plain, scope, document)?,
                Some(bytes.physical_bytes),
            )),
        }
    }

    // Shared bounded authentication path for addressed reads and inventory discovery. Absence
    // is optional here; the scanner treats disappearance of an enumerated file as a failed scan.
    fn read_epoch_recovery_plain(
        &self,
        path: &Path,
    ) -> Result<Option<AuthenticatedEpochFileBytes>, AppError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(AppError::Io(error.to_string())),
        };
        if !inventory::regular_file(&metadata) || metadata.len() > MAX_SEALED_BYTES as u64 {
            return Err(invalid("recovery file is not a bounded regular file"));
        }
        // A length check alone is racy. Read through a fixed limit even if a file grows between
        // stat and open. The installation OS lock excludes other conforming process writers.
        let file = File::open(path).map_err(|e| AppError::Io(e.to_string()))?;
        if !inventory::regular_file(&file.metadata().map_err(|e| AppError::Io(e.to_string()))?) {
            return Err(invalid("recovery file is not regular"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_SEALED_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Io(e.to_string()))?;
        if bytes.len() > MAX_SEALED_BYTES {
            return Err(invalid("recovery file exceeds its bound"));
        }
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&bytes)?)?);
        Ok(Some(AuthenticatedEpochFileBytes {
            plain,
            physical_bytes: bytes.len() as u64,
        }))
    }
}

fn recovery_record(scope: &[u8], footprint: Footprint) -> StorageRecord {
    let id = *blake3::hash(scope).as_bytes();
    StorageRecord {
        id,
        document: id,
        footprint,
    }
}

pub(super) fn scope_bytes(server: u64, document: &LogicalDocument) -> Result<Vec<u8>, AppError> {
    // LogicalDocument has public fields. Revalidate even when a caller bypassed its constructor.
    LogicalDocument::new(
        document.server_id.clone(),
        document.doc_type,
        document.logical_key.clone(),
    )
    .map_err(invalid)?;
    let mut e = Encoder::new();
    e.put_bytes(RECORD_DOMAIN).expect("constant domain fits");
    e.put_u64(server);
    e.put_bytes(&document.server_id).map_err(invalid)?;
    e.put_u16(document.doc_type.tag());
    e.put_bytes(&document.logical_key).map_err(invalid)?;
    Ok(e.finish())
}

fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("epoch recovery: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_replication::{
        epoch::{MAX_RECOVERY_SNAPSHOT_BYTES, RECOVERY_GRACE_MS},
        RecoveryReason,
    };
    use catcoms_rt::ManualClock;
    use catcoms_wire::DocType;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    const SERVER: u64 = 7;
    const SECRET: &[u8] = b"epoch-recovery-test";

    fn document() -> LogicalDocument {
        LogicalDocument::new(
            b"test-group".to_vec(),
            DocType::StudioObject,
            b"moon-cat".to_vec(),
        )
        .unwrap()
    }

    fn snapshot(epoch: u64) -> RecoverySnapshot {
        RecoverySnapshot {
            doc_type: DocType::StudioObject,
            logical_key: b"moon-cat".to_vec(),
            epoch,
            base_close_record_hash: None,
            reason: RecoveryReason::Excluded,
            projection: b"private recovery pixels".to_vec(),
            tombstones: vec![],
            elements: vec![],
            conflicts: vec![],
            applied_ops: vec![],
        }
    }

    fn open(root: &Path) -> ServerStore {
        ServerStore::open(root, SECRET, &mut ChaCha20Rng::seed_from_u64(19)).unwrap()
    }

    fn update(
        store: &mut ServerStore,
        action: EpochRecoveryAction,
        now: u64,
    ) -> EpochRecoveryUpdate {
        store
            .update_epoch_recovery(
                SERVER,
                &document(),
                action,
                &ManualClock::new(now),
                &mut ChaCha20Rng::seed_from_u64(now),
            )
            .unwrap()
    }

    fn stage(store: &mut ServerStore, epoch: u64, now: u64) -> EpochRecoveryUpdate {
        update(store, EpochRecoveryAction::Stage(snapshot(epoch)), now)
    }

    fn acknowledgement(warning: RecoveryTransition) -> EpochRecoveryAction {
        match warning {
            RecoveryTransition::EvictionPending {
                oldest_snapshot,
                staged_snapshot,
                ..
            } => EpochRecoveryAction::Acknowledge {
                oldest_snapshot,
                staged_snapshot,
            },
            _ => panic!("expected an eviction warning"),
        }
    }

    fn epochs(state: &EpochRecoveryState) -> Vec<u64> {
        state.retained().map(|snapshot| snapshot.epoch).collect()
    }

    #[test]
    fn reopen_preserves_all_three_slots_and_the_original_deadline() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        assert!(
            store
                .load_epoch_recovery(SERVER, &document())
                .unwrap()
                .retained()
                .len()
                == 0
        );
        stage(&mut store, 1, 10);
        drop(store);
        let mut store = open(root.path());
        assert_eq!(
            epochs(&store.load_epoch_recovery(SERVER, &document()).unwrap()),
            vec![1]
        );
        stage(&mut store, 2, 20);
        drop(store);
        let mut store = open(root.path());
        let warning = stage(&mut store, 3, 30).transition;
        let path = store.epoch_recovery_path(&scope_bytes(SERVER, &document()).unwrap());
        let ciphertext = fs::read(path).unwrap();
        assert!(!ciphertext
            .windows(b"private recovery pixels".len())
            .any(|w| w == b"private recovery pixels"));
        drop(store);
        let mut store = open(root.path());
        let state = store.load_epoch_recovery(SERVER, &document()).unwrap();
        assert_eq!(epochs(&state), vec![2, 1]);
        assert_eq!(state.staged().unwrap().epoch, 3);
        assert_eq!(state.eviction_pending().unwrap(), Some(warning));
        assert_eq!(stage(&mut store, 3, 50_000).transition, warning);
        let early = update(
            &mut store,
            EpochRecoveryAction::AdvanceTime,
            30 + RECOVERY_GRACE_MS - 1,
        );
        assert_eq!(early.transition, RecoveryTransition::Unchanged);
        assert_eq!(early.state.eviction_pending().unwrap(), Some(warning));
        let promoted = update(
            &mut store,
            EpochRecoveryAction::AdvanceTime,
            30 + RECOVERY_GRACE_MS,
        );
        assert_eq!(promoted.transition, RecoveryTransition::Promoted);
        drop(store);
        let store = open(root.path());
        let state = store.load_epoch_recovery(SERVER, &document()).unwrap();
        assert_eq!(epochs(&state), vec![3, 2]);
        assert!(state.eviction_pending().unwrap().is_none());
    }

    #[test]
    fn acknowledgement_is_exact_and_cannot_authorize_the_next_eviction() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 1, 10);
        stage(&mut store, 2, 20);
        let warning = stage(&mut store, 3, 30).transition;
        let promoted = update(&mut store, acknowledgement(warning), 31);
        assert_eq!(epochs(&promoted.state), vec![3, 2]);
        assert_eq!(
            update(&mut store, acknowledgement(warning), 32).transition,
            RecoveryTransition::Unchanged
        );
        let next = stage(&mut store, 4, 40).transition;
        assert!(store
            .update_epoch_recovery(
                SERVER,
                &document(),
                acknowledgement(warning),
                &ManualClock::new(41),
                &mut ChaCha20Rng::seed_from_u64(9)
            )
            .is_err());
        let state = store.load_epoch_recovery(SERVER, &document()).unwrap();
        assert_eq!(state.eviction_pending().unwrap(), Some(next));
        assert_eq!(epochs(&state), vec![3, 2]);
        assert_eq!(
            epochs(&update(&mut store, acknowledgement(next), 42).state),
            vec![4, 3]
        );
    }

    #[test]
    fn failed_writes_do_not_publish_stage_acknowledgement_or_expiry() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 1, 10);
        stage(&mut store, 2, 20);
        let fail = |_: &Path, _: &[u8]| Err(AppError::Io("injected full disk".into()));
        assert!(store
            .update_epoch_recovery_with_writer(
                SERVER,
                &document(),
                EpochRecoveryAction::Stage(snapshot(3)),
                &ManualClock::new(30),
                &mut ChaCha20Rng::seed_from_u64(0),
                fail
            )
            .is_err());
        drop(store);
        let mut store = open(root.path());
        assert!(store
            .load_epoch_recovery(SERVER, &document())
            .unwrap()
            .staged()
            .is_none());
        let warning = stage(&mut store, 3, 30).transition;
        for action in [acknowledgement(warning), EpochRecoveryAction::AdvanceTime] {
            assert!(store
                .update_epoch_recovery_with_writer(
                    SERVER,
                    &document(),
                    action,
                    &ManualClock::new(30 + RECOVERY_GRACE_MS),
                    &mut ChaCha20Rng::seed_from_u64(0),
                    fail
                )
                .is_err());
            drop(store);
            store = open(root.path());
            assert_eq!(
                store
                    .load_epoch_recovery(SERVER, &document())
                    .unwrap()
                    .eviction_pending()
                    .unwrap(),
                Some(warning)
            );
        }
    }

    #[test]
    fn post_rename_flush_failure_can_retry_each_action_after_reopen() {
        for action_kind in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            stage(&mut store, 1, 10);
            stage(&mut store, 2, 20);
            let warning = if action_kind == 0 {
                RecoveryTransition::EvictionPending {
                    oldest_snapshot: snapshot(1).id().unwrap(),
                    staged_snapshot: snapshot(3).id().unwrap(),
                    deadline_ms: 30 + RECOVERY_GRACE_MS,
                }
            } else {
                stage(&mut store, 3, 30).transition
            };
            let action = || match action_kind {
                0 => EpochRecoveryAction::Stage(snapshot(3)),
                1 => acknowledgement(warning),
                _ => EpochRecoveryAction::AdvanceTime,
            };
            let writer = |path: &Path, bytes: &[u8]| {
                atomic_write_with_hook_and_sync(
                    path,
                    bytes,
                    |_, _| {},
                    |_| Err(std::io::Error::other("injected directory flush failure")),
                )
            };
            assert!(matches!(
                store.update_epoch_recovery_with_writer(
                    SERVER,
                    &document(),
                    action(),
                    &ManualClock::new(if action_kind == 0 {
                        30
                    } else {
                        30 + RECOVERY_GRACE_MS
                    }),
                    &mut ChaCha20Rng::seed_from_u64(0),
                    writer
                ),
                Err(AppError::CommittedButNotDurable(_))
            ));
            drop(store);
            let mut store = open(root.path());
            let mut writes = 0;
            let saved = store
                .update_epoch_recovery_with_writer(
                    SERVER,
                    &document(),
                    action(),
                    &ManualClock::new(30 + RECOVERY_GRACE_MS),
                    &mut ChaCha20Rng::seed_from_u64(1),
                    |path, bytes| {
                        writes += 1;
                        atomic_write(path, bytes)
                    },
                )
                .unwrap();
            assert_eq!(
                writes, 1,
                "a matching retry must flush again, not return from RAM"
            );
            if action_kind == 0 {
                assert_eq!(saved.state.eviction_pending().unwrap(), Some(warning));
            } else {
                assert_eq!(epochs(&saved.state), vec![3, 2]);
                assert!(saved.state.staged().is_none());
            }
            drop(store);
            let store = open(root.path());
            assert_eq!(
                epochs(&store.load_epoch_recovery(SERVER, &document()).unwrap()),
                epochs(&saved.state)
            );
        }
    }

    #[test]
    fn scope_binds_local_server_group_document_type_and_full_key() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 1, 10);
        let source_scope = scope_bytes(SERVER, &document()).unwrap();
        let source = store.epoch_recovery_path(&source_scope);
        let mut group = document();
        group.server_id.push(0);
        let mut kind = document();
        kind.doc_type = DocType::StudioIndex;
        let mut key = document();
        key.logical_key.push(0);
        for (server, doc) in [
            (SERVER + 1, document()),
            (SERVER, group),
            (SERVER, kind),
            (SERVER, key),
        ] {
            let target = store.epoch_recovery_path(&scope_bytes(server, &doc).unwrap());
            assert_ne!(source, target);
            fs::copy(&source, &target).unwrap();
            assert!(
                store.load_epoch_recovery(server, &doc).is_err(),
                "a valid vault seal is not target authority"
            );
        }
        let mut wrong = snapshot(2);
        wrong.logical_key.push(0);
        assert!(store
            .update_epoch_recovery(
                SERVER,
                &document(),
                EpochRecoveryAction::Stage(wrong),
                &ManualClock::new(0),
                &mut ChaCha20Rng::seed_from_u64(2)
            )
            .is_err());
        assert_eq!(
            epochs(&store.load_epoch_recovery(SERVER, &document()).unwrap()),
            vec![1]
        );
    }

    #[test]
    fn malformed_or_oversized_files_are_not_treated_as_empty_recovery() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let path = store.epoch_recovery_path(&scope_bytes(SERVER, &document()).unwrap());
        for bytes in [vec![], vec![0; 23], vec![0; 40]] {
            fs::write(&path, bytes).unwrap();
            assert!(store.load_epoch_recovery(SERVER, &document()).is_err());
            assert!(store
                .update_epoch_recovery(
                    SERVER,
                    &document(),
                    EpochRecoveryAction::Stage(snapshot(1)),
                    &ManualClock::new(0),
                    &mut ChaCha20Rng::seed_from_u64(1)
                )
                .is_err());
        }
        File::create(&path)
            .unwrap()
            .set_len(MAX_SEALED_BYTES as u64 + 1)
            .unwrap();
        assert!(store.load_epoch_recovery(SERVER, &document()).is_err());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(store.load_epoch_recovery(SERVER, &document()).is_err());
    }

    #[test]
    fn max_sized_snapshot_roundtrips_and_one_extra_byte_never_replaces_it() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut max = snapshot(1);
        max.projection.clear();
        let overhead = max.encode().unwrap().len();
        max.projection
            .resize(MAX_RECOVERY_SNAPSHOT_BYTES - overhead, 0x42);
        assert_eq!(max.encode().unwrap().len(), MAX_RECOVERY_SNAPSHOT_BYTES);
        update(&mut store, EpochRecoveryAction::Stage(max.clone()), 10);
        max.projection.push(0);
        assert!(store
            .update_epoch_recovery(
                SERVER,
                &document(),
                EpochRecoveryAction::Stage(max),
                &ManualClock::new(20),
                &mut ChaCha20Rng::seed_from_u64(20)
            )
            .is_err());
        drop(store);
        let store = open(root.path());
        let held = store.load_epoch_recovery(SERVER, &document()).unwrap();
        assert_eq!(
            held.retained().next().unwrap().encode().unwrap().len(),
            MAX_RECOVERY_SNAPSHOT_BYTES
        );
    }

    #[test]
    fn serialized_competing_stages_cannot_overwrite_the_staged_slot() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 1, 10);
        stage(&mut store, 2, 20);
        let shared = std::sync::Mutex::new(store);
        std::thread::scope(|scope| {
            let joins: Vec<_> = [3, 4]
                .into_iter()
                .map(|epoch| {
                    let shared = &shared;
                    scope.spawn(move || {
                        shared
                            .lock()
                            .unwrap()
                            .update_epoch_recovery(
                                SERVER,
                                &document(),
                                EpochRecoveryAction::Stage(snapshot(epoch)),
                                &ManualClock::new(30),
                                &mut ChaCha20Rng::seed_from_u64(epoch),
                            )
                            .is_ok()
                    })
                })
                .collect();
            assert_eq!(
                joins
                    .into_iter()
                    .map(|join| join.join().unwrap())
                    .filter(|accepted| *accepted)
                    .count(),
                1
            );
        });
        assert_eq!(
            shared
                .lock()
                .unwrap()
                .load_epoch_recovery(SERVER, &document())
                .unwrap()
                .retained()
                .len(),
            2
        );
    }

    #[test]
    fn plaintext_record_rejects_trailing_bytes_and_invalid_completion_proof() {
        let doc = document();
        let scope = scope_bytes(SERVER, &doc).unwrap();
        let mut state = EpochRecoveryState::default();
        state
            .apply(EpochRecoveryAction::Stage(snapshot(1)), 10)
            .unwrap();
        let mut bytes = state.encode(&scope, &doc).unwrap();
        bytes.push(0);
        assert!(EpochRecoveryState::decode(&bytes, &scope, &doc).is_err());
        state.completed_eviction = Some(([1; 32], [2; 32]));
        assert!(
            EpochRecoveryState::decode(&state.encode(&scope, &doc).unwrap(), &scope, &doc).is_err()
        );
    }

    #[test]
    fn diagnostic_debug_never_includes_recovery_content() {
        let mut state = EpochRecoveryState::default();
        state
            .apply(EpochRecoveryAction::Stage(snapshot(1)), 10)
            .unwrap();
        let action = format!("{:?}", EpochRecoveryAction::Stage(snapshot(1)));
        let result = format!(
            "{:?}",
            EpochRecoveryUpdate {
                transition: RecoveryTransition::Promoted,
                state
            }
        );
        assert!(!action.contains("pixels"));
        assert!(!result.contains("pixels"));
        assert!(!result.contains("projection"));
    }

    #[test]
    fn maximal_scope_fits_the_record_allowance_and_roundtrips() {
        let doc = LogicalDocument::new(vec![1; 256], DocType::StudioObject, vec![2; 192]).unwrap();
        let scope = scope_bytes(u64::MAX, &doc).unwrap();
        assert_eq!(scope.len(), 501);
        let state = EpochRecoveryState::default();
        let encoded = state.encode(&scope, &doc).unwrap();
        assert!(encoded.len() < MAX_RECORD_BYTES - MAX_RECOVERY_SLOTS_BYTES);
        assert_eq!(
            EpochRecoveryState::decode(&encoded, &scope, &doc)
                .unwrap()
                .retained()
                .len(),
            0
        );
    }

    // These fixtures contain exactly one recovery file and no other P1 artifacts. A real caller
    // cannot replace complete inventory discovery with this one-document helper.
    fn inventory_budget(store: &ServerStore) -> EpochStorageBudget {
        EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &document().server_id).unwrap(),
            store
                .epoch_recovery_inventory_record(SERVER, &document())
                .unwrap(),
        )
        .unwrap()
    }

    fn accounted(
        store: &mut ServerStore,
        budget: &mut EpochStorageBudget,
        action: EpochRecoveryAction,
        now: u64,
    ) -> EpochRecoveryUpdate {
        store
            .update_epoch_recovery_accounted(
                SERVER,
                &document(),
                action,
                &ManualClock::new(now),
                &mut ChaCha20Rng::seed_from_u64(now),
                budget,
            )
            .unwrap()
    }

    #[test]
    fn accounted_updates_charge_physical_bytes_and_keep_staged_reserve_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut budget = inventory_budget(&store);
        let mut warning = RecoveryTransition::Unchanged;
        for epoch in 1..=3 {
            warning = accounted(
                &mut store,
                &mut budget,
                EpochRecoveryAction::Stage(snapshot(epoch)),
                epoch * 10,
            )
            .transition;
            let record = store
                .epoch_recovery_inventory_record(SERVER, &document())
                .unwrap()
                .unwrap();
            assert_eq!(budget.usage(), record.footprint);
            let physical =
                fs::metadata(store.epoch_recovery_path(&scope_bytes(SERVER, &document()).unwrap()))
                    .unwrap()
                    .len();
            assert_eq!(record.footprint.total().unwrap(), physical);
            assert_eq!(record.footprint.settlement > 0, epoch == 3);
        }
        let held = budget.usage();
        drop(store);
        let mut store = open(root.path());
        let mut budget = inventory_budget(&store);
        assert_eq!(budget.usage(), held);
        assert_eq!(
            store
                .load_epoch_recovery(SERVER, &document())
                .unwrap()
                .eviction_pending()
                .unwrap(),
            Some(warning)
        );
        let saved = accounted(&mut store, &mut budget, acknowledgement(warning), 31);
        assert_eq!(epochs(&saved.state), vec![3, 2]);
        assert_eq!(budget.usage().settlement, 0);
        assert_eq!(
            budget.usage(),
            store
                .epoch_recovery_inventory_record(SERVER, &document())
                .unwrap()
                .unwrap()
                .footprint
        );
    }

    #[test]
    fn content_cap_refuses_before_writer_without_borrowing_future_history_deletions() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let filler = StorageRecord {
            id: [9; 32],
            document: [9; 32],
            footprint: Footprint {
                content: epoch_budget::CONTENT_ALLOWANCE_BYTES,
                ..Footprint::default()
            },
        };
        let mut budget = EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &document().server_id).unwrap(),
            [filler],
        )
        .unwrap();
        let mut writes = 0;
        let result = store.update_epoch_recovery_accounted_with_writer(
            SERVER,
            &document(),
            EpochRecoveryAction::Stage(snapshot(1)),
            &ManualClock::new(10),
            &mut ChaCha20Rng::seed_from_u64(10),
            &mut budget,
            |_, _| {
                writes += 1;
                Ok(())
            },
        );
        assert!(result.unwrap_err().to_string().contains("storage limit"));
        assert_eq!(writes, 0);
        assert!(!budget.requires_reconciliation());
        assert!(store
            .epoch_recovery_inventory_record(SERVER, &document())
            .unwrap()
            .is_none());
    }

    #[test]
    fn accounted_save_failure_requires_actual_inventory_before_a_retry() {
        for after_rename in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let mut budget = inventory_budget(&store);
            let action = || EpochRecoveryAction::Stage(snapshot(1));
            let result = store.update_epoch_recovery_accounted_with_writer(
                SERVER,
                &document(),
                action(),
                &ManualClock::new(10),
                &mut ChaCha20Rng::seed_from_u64(10),
                &mut budget,
                |path, bytes| {
                    if after_rename {
                        atomic_write_with_hook_and_sync(
                            path,
                            bytes,
                            |_, _| {},
                            |_| Err(std::io::Error::other("injected flush failure")),
                        )
                    } else {
                        Err(AppError::Io("injected write failure".into()))
                    }
                },
            );
            assert!(result.is_err());
            assert!(budget.requires_reconciliation());
            let mut retried_writes = 0;
            assert!(store
                .update_epoch_recovery_accounted_with_writer(
                    SERVER,
                    &document(),
                    action(),
                    &ManualClock::new(10),
                    &mut ChaCha20Rng::seed_from_u64(10),
                    &mut budget,
                    |_, _| {
                        retried_writes += 1;
                        Ok(())
                    }
                )
                .is_err());
            assert_eq!(retried_writes, 0);
            let record = store
                .epoch_recovery_inventory_record(SERVER, &document())
                .unwrap();
            assert_eq!(record.is_some(), after_rename);
            // The test has no orphan sibling; a production reconciliation must include it if
            // present, or wait until a successful durable cleanup before releasing its bytes.
            budget
                .reconcile(
                    &StorageScope::new(SERVER, &document().server_id).unwrap(),
                    record,
                )
                .unwrap();
            accounted(&mut store, &mut budget, action(), 10);
            assert!(!budget.requires_reconciliation());
            assert_eq!(
                budget.usage(),
                store
                    .epoch_recovery_inventory_record(SERVER, &document())
                    .unwrap()
                    .unwrap()
                    .footprint
            );
        }
    }

    #[test]
    fn equal_length_but_wrong_pool_inventory_cannot_save_and_corrupt_reads_close_admission() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for epoch in 1..=3 {
            stage(&mut store, epoch, epoch * 10);
        }
        let mut wrong = store
            .epoch_recovery_inventory_record(SERVER, &document())
            .unwrap()
            .unwrap();
        wrong.footprint.content += wrong.footprint.settlement;
        wrong.footprint.settlement = 0;
        let mut budget = EpochStorageBudget::from_inventory(
            StorageScope::new(SERVER, &document().server_id).unwrap(),
            [wrong],
        )
        .unwrap();
        let mut writes = 0;
        assert!(store
            .update_epoch_recovery_accounted_with_writer(
                SERVER,
                &document(),
                EpochRecoveryAction::AdvanceTime,
                &ManualClock::new(40),
                &mut ChaCha20Rng::seed_from_u64(10),
                &mut budget,
                |_, _| {
                    writes += 1;
                    Ok(())
                }
            )
            .is_err());
        assert_eq!(writes, 0);
        assert!(budget.requires_reconciliation());
        let mut budget = inventory_budget(&store);
        let path = store.epoch_recovery_path(&scope_bytes(SERVER, &document()).unwrap());
        fs::write(path, [0; 40]).unwrap();
        assert!(store
            .update_epoch_recovery_accounted(
                SERVER,
                &document(),
                EpochRecoveryAction::AdvanceTime,
                &ManualClock::new(40),
                &mut ChaCha20Rng::seed_from_u64(10),
                &mut budget
            )
            .is_err());
        assert!(budget.requires_reconciliation());
    }

    #[test]
    fn another_documents_staged_recovery_and_orphan_bytes_keep_their_reserve() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for epoch in 1..=3 {
            stage(&mut store, epoch, epoch * 10);
        }
        let mut budget = inventory_budget(&store);
        let mut other_doc = document();
        other_doc.logical_key = b"other-cat".to_vec();
        let mut other_snapshot = snapshot(1);
        other_snapshot.logical_key = other_doc.logical_key.clone();
        let result = store.update_epoch_recovery_accounted(
            SERVER,
            &other_doc,
            EpochRecoveryAction::Stage(other_snapshot),
            &ManualClock::new(40),
            &mut ChaCha20Rng::seed_from_u64(10),
            &mut budget,
        );
        assert!(result.unwrap_err().to_string().contains("another document"));
        let current = store
            .epoch_recovery_inventory_record(SERVER, &document())
            .unwrap()
            .unwrap();
        let orphan = StorageRecord {
            id: [9; 32],
            document: current.document,
            footprint: Footprint {
                settlement: epoch_budget::SETTLEMENT_RESERVE_BYTES - current.footprint.settlement,
                ..Footprint::default()
            },
        };
        budget
            .reconcile(
                &StorageScope::new(SERVER, &document().server_id).unwrap(),
                [current, orphan],
            )
            .unwrap();
        // Replacing even the same bytes needs another physical copy; no early orphan refund.
        assert!(store
            .update_epoch_recovery_accounted(
                SERVER,
                &document(),
                EpochRecoveryAction::AdvanceTime,
                &ManualClock::new(40),
                &mut ChaCha20Rng::seed_from_u64(10),
                &mut budget
            )
            .unwrap_err()
            .to_string()
            .contains("storage limit"));
        assert_eq!(
            budget.usage().settlement,
            epoch_budget::SETTLEMENT_RESERVE_BYTES
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_cannot_redirect_a_recovery_read_or_update() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let target = root.path().join("unrelated");
        fs::write(&target, b"preserve me").unwrap();
        let path = store.epoch_recovery_path(&scope_bytes(SERVER, &document()).unwrap());
        std::os::unix::fs::symlink(&target, path).unwrap();
        assert!(store.load_epoch_recovery(SERVER, &document()).is_err());
        assert!(store
            .update_epoch_recovery(
                SERVER,
                &document(),
                EpochRecoveryAction::Stage(snapshot(1)),
                &ManualClock::new(0),
                &mut ChaCha20Rng::seed_from_u64(1)
            )
            .is_err());
        assert_eq!(fs::read(target).unwrap(), b"preserve me");
    }
}
