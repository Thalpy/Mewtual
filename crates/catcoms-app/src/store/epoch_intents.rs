//! Persist-before-edit local intents. Receipt-covered retirement is store-internal and must
//! follow the source/recovery durability barriers, never a live document's marker.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;

use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::{
    epoch::{MAX_DOMAIN_OP_BYTES, MAX_INTENT_LEDGER_BYTES},
    DomainOp, IntentLedger, LocalIntent, LogicalDocument,
};

use super::epoch_budget::{
    EpochStorageBudget, Footprint, Replacement, StorageRecord, StorageScope, WritePurpose,
};
use super::epoch_recovery::inventory::{is_link, regular_file};
use super::epoch_recovery::AuthenticatedEpochFileBytes;
use super::*;

pub(super) const RECORD_DOMAIN: &[u8] = b"catcoms/epoch-intent-store/v1";
const MAX_RECORD_BYTES: usize = MAX_INTENT_LEDGER_BYTES + 1024;
pub(super) const MAX_SEALED_BYTES: usize = MAX_RECORD_BYTES + 40;
/// Conservative vault-wide intent ceiling: sealed final files PLUS unpublished siblings and the
/// full replacement copy at peak. Framing counts too; this is stricter than a payload-only cap.
pub const MAX_VAULT_INTENT_BYTES: u64 = 64 * 1024 * 1024;

mod retirement;

/// Read-only replay data, not authority to edit or proof an intent is final. There is deliberately
/// no public constructor, mutation/retirement method, or content-bearing Debug implementation.
pub struct EpochIntentState {
    ledger: IntentLedger,
}

impl std::fmt::Debug for EpochIntentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochIntentState")
            .field("pending", &self.ledger.len())
            .finish_non_exhaustive()
    }
}

impl EpochIntentState {
    /// Stable, author-derived operation ids and their original replay instructions.
    pub fn pending(&self) -> impl ExactSizeIterator<Item = (&[u8; 32], &LocalIntent)> {
        self.ledger.pending()
    }

    fn encode(&self, scope: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let ledger = Zeroizing::new(self.ledger.encode().map_err(invalid)?);
        let mut e = Encoder::new();
        e.put_bytes(scope).map_err(invalid)?;
        e.put_bytes(&ledger).map_err(invalid)?;
        Ok(Zeroizing::new(e.finish()))
    }

    pub(super) fn decode(
        bytes: &[u8],
        scope: &[u8],
        document: &LogicalDocument,
    ) -> Result<Self, AppError> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(invalid("record exceeds its bound"));
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes().map_err(invalid)? != scope {
            return Err(invalid("wrong sealed scope"));
        }
        let ledger = IntentLedger::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?;
        d.finish().map_err(invalid)?;
        if ledger.document() != document {
            return Err(invalid("wrong ledger scope"));
        }
        Ok(Self { ledger })
    }
}

/// One vault-wide intent budget, derived only from completed intent-covering inventory. A private
/// mount/generation token also rejects duplicate budgets and stale scan results after any intent
/// write or cleanup attempt. Other P1 storage still requires the separate per-server budget and
/// sole coordinator. A token is only an in-process freshness check, not persisted authority.
pub struct EpochIntentBudget {
    generation: Arc<()>,
    records: BTreeMap<[u8; 32], u64>,
    // Temporary siblings consume metadata slots even when empty. Never grow this namespace
    // past the scanner's rail; the coordinator must also admit the other families' metadata.
    record_slots: usize,
    bytes: u64,
    ready: bool,
}

impl std::fmt::Debug for EpochIntentBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochIntentBudget")
            .field("bytes", &self.bytes)
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

impl EpochIntentBudget {
    /// Includes intent finals and temporaries across every server in the vault, even unknown
    /// orphan ownership.
    /// Recovery-only/two-family inventories cannot bootstrap this budget. No disk writes occur.
    pub fn from_inventory(inventory: &EpochStorageInventory) -> Result<Self, AppError> {
        if !inventory.coverage().includes_intents() {
            return Err(invalid("inventory does not cover intents"));
        }
        let mut records = BTreeMap::new();
        let mut bytes = 0u64;
        for entry in inventory
            .records()
            .filter(|e| e.kind == EpochRecordKind::Intents)
        {
            let size = entry.record.footprint.total().map_err(invalid)?;
            bytes = bytes
                .checked_add(size)
                .ok_or_else(|| invalid("vault intent limit reached"))?;
            records.insert(entry.record.id, size);
        }
        for orphan in inventory
            .orphans()
            .filter(|o| o.kind() == EpochRecordKind::Intents)
        {
            bytes = bytes
                .checked_add(orphan.bytes())
                .ok_or_else(|| invalid("vault intent limit reached"))?;
        }
        if bytes > MAX_VAULT_INTENT_BYTES {
            return Err(invalid("vault intent limit reached"));
        }
        Ok(Self {
            generation: inventory.intent_generation.clone(),
            record_slots: records.len()
                + inventory
                    .orphans()
                    .filter(|o| o.kind() == EpochRecordKind::Intents)
                    .count(),
            records,
            bytes,
            ready: true,
        })
    }

    /// Failed reconciliation leaves the old budget unusable. Scan again after cleanup; never
    /// subtract the cleanup progress counter or trust a saved counter from before restart.
    pub fn reconcile(&mut self, inventory: &EpochStorageInventory) -> Result<(), AppError> {
        self.ready = false;
        *self = Self::from_inventory(inventory)?;
        Ok(())
    }

    /// Observed physical intent occupancy. It is not free space or an editing permit.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    fn preflight(
        &mut self,
        generation: &Arc<()>,
        id: [u8; 32],
        old: Option<u64>,
        next: u64,
        sync_only: bool,
    ) -> Result<u64, AppError> {
        if !self.ready
            || !Arc::ptr_eq(generation, &self.generation)
            || self.records.get(&id).copied() != old
        {
            self.ready = false;
            return Err(invalid("vault intent inventory must be reconciled"));
        }
        // The old final and ALL orphan attempts remain charged until replacement succeeds.
        if old.is_none()
            && !sync_only
            && self.record_slots >= super::epoch_budget::MAX_ACCOUNTED_RECORDS
        {
            return Err(invalid("vault intent inventory has too many records"));
        }
        if self
            .bytes
            .checked_add(if sync_only { 0 } else { next })
            .is_none_or(|peak| peak > MAX_VAULT_INTENT_BYTES)
        {
            return Err(invalid("vault intent limit reached"));
        }
        self.bytes
            .checked_sub(old.unwrap_or(0))
            .and_then(|v| v.checked_add(next))
            .ok_or_else(|| invalid("vault intent accounting mismatch"))
    }
}

impl ServerStore {
    /// Load saved replay instructions. Only an absent file means empty. A loaded intent does not
    /// authorize a current device to impersonate its original author when replaying it.
    pub fn load_epoch_intents(
        &self,
        server: u64,
        document: &LogicalDocument,
    ) -> Result<EpochIntentState, AppError> {
        self.read_epoch_intent_record(&scope_bytes(server, document)?, document)
            .map(|(state, _)| state)
    }

    /// Read one saved envelope with BOTH inventories checked before a replay decision. This is
    /// not a write/flush permit and never creates missing data. The replay coordinator checks the
    /// original author and typed semantics, then uses the normal two-barrier edit path.
    pub(super) fn checked_epoch_replay_intent(
        &self,
        server: u64,
        document: &LogicalDocument,
        intent_id: &[u8; 32],
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<LocalIntent, AppError> {
        let state = self.checked_epoch_replay_state(server, document, budget, intents)?;
        let found = state
            .pending()
            .find(|(id, _)| *id == intent_id)
            .map(|(_, intent)| intent.clone());
        found.ok_or_else(|| invalid("saved replay intent is missing"))
    }

    /// Checked ledger snapshot for selection only. Its ids are not a continuing write permit:
    /// replay reopens the actual records and repeats both inventory checks on every attempt.
    pub(super) fn checked_epoch_replay_state(
        &self,
        server: u64,
        document: &LogicalDocument,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<EpochIntentState, AppError> {
        let scope = scope_bytes(server, document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let (state, old) = match self.read_epoch_intent_record(&scope, document) {
            Ok(loaded) => loaded,
            Err(error) => {
                budget.invalidate();
                intents.ready = false;
                return Err(error);
            }
        };
        let id = *blake3::hash(&scope).as_bytes();
        let observed = old
            .map(|n| storage_record(server, document, &scope, n))
            .transpose()?;
        if let Err(error) = budget.verify_record(&storage_scope, id, observed) {
            intents.ready = false;
            return Err(invalid(error));
        }
        intents.preflight(&self.intent_generation, id, old, old.unwrap_or(0), true)?;
        Ok(state)
    }

    /// Repair uncertainty for an unchanged authenticated ledger without replaying any intent.
    /// The exclusive coordinator also flushes its matching saved source before claiming a
    /// maintenance no-op. This does not retire entries or infer that an operation is receipted.
    pub(super) fn flush_checked_epoch_intents(
        &self,
        server: u64,
        document: &LogicalDocument,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<(), AppError> {
        self.checked_epoch_replay_state(server, document, budget, intents)?;
        let scope = scope_bytes(server, document)?;
        let (_, bytes) = self.read_epoch_intent_record(&scope, document)?;
        if let Some(bytes) = bytes {
            let record = storage_record(server, document, &scope, bytes)?;
            let reservation = budget
                .reserve_sync(
                    &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                    record,
                )
                .map_err(invalid)?;
            sync_intent(&self.epoch_intent_path(&scope), bytes)?;
            reservation.commit();
        }
        Ok(())
    }

    /// Save one local intent before applying or gossiping the edit. The actual local MLS device,
    /// not a payload identity, supplies its author; it must still belong to the document's group.
    /// Type-specific semantic validation must run before calling this envelope-storage adapter.
    /// Exact retries sync the authenticated unchanged final file and its parent without another
    /// copy, so an earlier post-rename failure remains retryable at the cap. Failure returns no
    /// edit permit; retry/reconcile before editing. There is no standalone public removal API: markers/acks alone
    /// cannot retire durable intents.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_epoch_intent(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        operation: DomainOp,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<EpochIntentState, AppError> {
        self.prepare_epoch_intent_with_io(
            server,
            document,
            operation,
            device,
            group,
            rng,
            budget,
            intents,
            atomic_write,
            sync_intent,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_epoch_intent_with_io(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        operation: DomainOp,
        device: &MlsDevice,
        group: &ServerGroup,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<EpochIntentState, AppError> {
        let scope = scope_bytes(server, document)?;
        // Bound public Vec fields before encode can copy an arbitrarily large caller input.
        if operation.body.len() > MAX_DOMAIN_OP_BYTES || operation.logical_key.len() > 192 {
            return Err(invalid("operation exceeds its bound"));
        }
        if document.server_id != group.group_id()
            || group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("intent author is not a current local member"));
        }
        self.hold_creative_operation(document, &operation);
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let (mut state, old) = match self.read_epoch_intent_record(&scope, document) {
            Ok(value) => value,
            Err(error) => {
                budget.invalidate();
                intents.ready = false;
                return Err(error);
            }
        };
        let id = *blake3::hash(&scope).as_bytes();
        let observed = old
            .map(|n| storage_record(server, document, &scope, n))
            .transpose()?;
        if let Err(error) = budget.verify_record(&storage_scope, id, observed) {
            intents.ready = false;
            return Err(invalid(error));
        }
        let count = state.ledger.len();
        state
            .ledger
            .prepare(device.device_id(), operation)
            .map_err(invalid)?;
        if state.ledger.len() == count {
            // `prepare` compares all author/body bytes for an existing id. The loaded file is
            // therefore already the exact required ledger, possibly including newer intents.
            let record = observed.ok_or_else(|| invalid("missing retry record"))?;
            let bytes = old.expect("an observed record has a physical size");
            intents.preflight(&self.intent_generation, id, old, bytes, true)?;
            let reservation = budget
                .reserve_sync(&storage_scope, record)
                .map_err(invalid)?;
            intents.ready = false;
            self.intent_generation = Arc::new(());
            sync(&self.epoch_intent_path(&scope), bytes)?;
            reservation.commit();
            intents.generation = self.intent_generation.clone();
            intents.ready = true;
            return Ok(state);
        }
        let plain = state.encode(&scope)?;
        let next = plain.len() as u64 + 40;
        let final_bytes = intents.preflight(&self.intent_generation, id, old, next, false)?;
        let record = storage_record(server, document, &scope, next)?;
        let reservation = budget
            .reserve(
                &storage_scope,
                Replacement {
                    record,
                    scratch_bytes: 0,
                    purpose: WritePurpose::Ordinary,
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
        // Poison BOTH budgets before I/O, including a caught writer panic. Rotate freshness even
        // on failure so a second budget or an older completed scan cannot bypass reconciliation.
        intents.ready = false;
        self.intent_generation = Arc::new(());
        writer(&self.epoch_intent_path(&scope), &frame(&sealed))?;
        reservation.commit();
        if old.is_none() {
            intents.record_slots += 1;
        }
        intents.records.insert(id, next);
        intents.bytes = final_bytes;
        intents.generation = self.intent_generation.clone();
        intents.ready = true;
        Ok(state)
    }

    fn epoch_intent_path(&self, scope: &[u8]) -> PathBuf {
        self.dir
            .join("servers")
            .join(format!("{}.intents", blake3::hash(scope).to_hex()))
    }

    fn read_epoch_intent_record(
        &self,
        scope: &[u8],
        document: &LogicalDocument,
    ) -> Result<(EpochIntentState, Option<u64>), AppError> {
        let parent = fs::symlink_metadata(self.dir.join("servers"))
            .map_err(|e| AppError::Io(e.to_string()))?;
        if !parent.is_dir() || is_link(&parent) {
            return Err(invalid("parent is not a regular directory"));
        }
        match self.read_epoch_intent_plain(&self.epoch_intent_path(scope))? {
            None => Ok((
                EpochIntentState {
                    ledger: IntentLedger::new(document.clone()),
                },
                None,
            )),
            Some(bytes) => Ok((
                EpochIntentState::decode(&bytes.plain, scope, document)?,
                Some(bytes.physical_bytes),
            )),
        }
    }

    pub(super) fn read_epoch_intent_plain(
        &self,
        path: &Path,
    ) -> Result<Option<AuthenticatedEpochFileBytes>, AppError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AppError::Io(e.to_string())),
        };
        if !regular_file(&metadata) || metadata.len() > MAX_SEALED_BYTES as u64 {
            return Err(invalid("intent file is not bounded and regular"));
        }
        let file = File::open(path).map_err(|e| AppError::Io(e.to_string()))?;
        if !regular_file(&file.metadata().map_err(|e| AppError::Io(e.to_string()))?) {
            return Err(invalid("opened intent file is not regular"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_SEALED_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Io(e.to_string()))?;
        if bytes.len() > MAX_SEALED_BYTES {
            return Err(invalid("intent file exceeds its bound"));
        }
        Ok(Some(AuthenticatedEpochFileBytes {
            plain: Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&bytes)?)?),
            physical_bytes: bytes.len() as u64,
        }))
    }
}

pub(super) fn scope_bytes(server: u64, document: &LogicalDocument) -> Result<Vec<u8>, AppError> {
    // Reuse the existing scope validation, but not its filename namespace.
    super::epoch_recovery::scope_bytes(server, document)?;
    let mut e = Encoder::new();
    e.put_bytes(RECORD_DOMAIN).expect("constant fits");
    e.put_u64(server);
    e.put_bytes(&document.server_id).map_err(invalid)?;
    e.put_u16(document.doc_type.tag());
    e.put_bytes(&document.logical_key).map_err(invalid)?;
    Ok(e.finish())
}

pub(super) fn storage_record(
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    bytes: u64,
) -> Result<StorageRecord, AppError> {
    Ok(StorageRecord {
        id: *blake3::hash(scope).as_bytes(),
        document: *blake3::hash(&super::epoch_recovery::scope_bytes(server, document)?).as_bytes(),
        footprint: Footprint {
            content: bytes,
            ..Footprint::default()
        },
    })
}

fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("epoch intent: {error}"))
}

// Re-sync only: no new ciphertext, nonce, staging file, or free-space requirement. Mounted-store
// exclusion protects the authenticated file between read and flush; hostile concurrent local path
// replacement is outside this guarantee. Keep regular-file checks at the actual open too.
pub(super) fn sync_intent(path: &Path, expected_bytes: u64) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|e| AppError::Io(e.to_string()))?;
    if !regular_file(&metadata) || metadata.len() != expected_bytes {
        return Err(invalid("retry file changed"));
    }
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| AppError::Io(e.to_string()))?;
    let metadata = file.metadata().map_err(|e| AppError::Io(e.to_string()))?;
    if !regular_file(&metadata) || metadata.len() != expected_bytes {
        return Err(invalid("opened retry file changed"));
    }
    file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;
    sync_directory(
        path.parent()
            .ok_or_else(|| invalid("missing intent parent"))?,
    )
    .map_err(|e| AppError::Io(e.to_string()))
}

#[cfg(test)]
mod tests;
