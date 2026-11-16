//! DISTINCT one-use PRIVATE candidate provider policy. This never unlocks an
//! ordinary restored provider or changes require_unrestricted's permanent fence.
//! The caller owns the real independent fee; these are negative limits only.
use super::*;
use serde::Serialize;
mod account;
mod epoch_keys;
#[cfg(any(test, feature = "test-utils"))]
pub(super) mod test_support;
use account::{OperationAccount, StackKey};

const LOADING: u8 = 0;
const PREPARED: u8 = 1;
const ADD: u8 = 2;
const OUTPUTS: u8 = 3;
const MERGING: u8 = 4;
const CLOSED: u8 = 5;
const FAILED: u8 = 6;
const MAX_PROVIDER_WRITES: usize = 64;
const MAX_ROW: usize = 512 * 1024;

/// CUMULATIVE independent operation allowance, additional to raw source/provider
/// and typed candidate restoration. Requested allocations are charged even when
/// later dropped. Provider bytes include table growth overlap, keys and values.
#[derive(Clone, Copy, Debug)]
pub struct TextAddOperationLimits {
    pub max_typed_bytes: usize,
    pub max_serialized_bytes: usize,
    pub max_provider_bytes: usize,
    pub max_work: usize,
}

pub(crate) struct Candidate {
    phase: AtomicU8,
    account: Mutex<OperationAccount>,
    group: [u8; 16],
    signer: [u8; 32],
    epoch: u64,
    own: u32,
}
impl fmt::Debug for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateTextAddProvider")
            .finish_non_exhaustive()
    }
}
impl Candidate {
    fn active(&self, phase: u8) -> Result<(), Error> {
        if self.phase.load(Ordering::Acquire) == phase {
            Ok(())
        } else {
            Err(Error)
        }
    }
    fn transition(&self, old: u8, new: u8) -> Result<(), Error> {
        self.phase
            .compare_exchange(old, new, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Error)
    }
    fn group_key<K: Serialize + ?Sized>(
        &self,
        row: Row,
        key: &K,
        a: &mut OperationAccount,
    ) -> Result<StackKey, Error> {
        a.key_entry()?;
        let mut actual = StackKey::new();
        actual.push(row.label())?;
        actual.json(key)?;
        actual.push(&[0, 1])?;
        if actual.bytes() != Key::new(row, &self.group, &self.signer)?.bytes() {
            return Err(Error);
        }
        Ok(actual)
    }
    fn epoch_key<G: Serialize + ?Sized, E: Serialize + ?Sized>(
        &self,
        group: &G,
        epoch: &E,
        own: u32,
        expected_epoch: u64,
        a: &mut OperationAccount,
    ) -> Result<StackKey, Error> {
        a.key_entry()?;
        let mut key = StackKey::new();
        key.push(crate::EPOCH_KEY_PAIRS_LABEL)?;
        key.json(group)?;
        key.json(epoch)?;
        key.json(&own)?;
        key.push(&[0, 1])?;
        let expected = self.expected_epoch_key(expected_epoch)?;
        if key.bytes() != expected.bytes() {
            return Err(Error);
        }
        Ok(key)
    }
    fn expected_epoch_key(&self, epoch: u64) -> Result<StackKey, Error> {
        let mut expected = StackKey::new();
        let group_key = Key::new(Row::Tree, &self.group, &self.signer)?;
        // Exact old persisted format is concatenated JSON(group), JSON(epoch),
        // JSON(index), NOT a JSON tuple or a new storage namespace.
        let group_json = &group_key.bytes()[Row::Tree.label().len()..group_key.bytes().len() - 2];
        expected.push(crate::EPOCH_KEY_PAIRS_LABEL)?;
        expected.push(group_json)?;
        expected.json(&epoch)?;
        expected.json(&self.own)?;
        expected.push(&[0, 1])?;
        Ok(expected)
    }
}

/// This is an operation-negative witness, not Source, governance, publication or
/// a successful Add observation. It is issued only while installing a FRESH
/// candidate provider before its first signer/group load. No post-Complete entry
/// exists on an ordinary initial/text-current restore.
#[must_use]
pub struct TextAddCandidateGuard {
    restore: Option<OwnerCheckpointRestoreGuard>,
    candidate: Arc<Candidate>,
    closed: bool,
}
impl TextAddCandidateGuard {
    pub fn complete_loading(&mut self, storage: &MemoryStorage) -> Result<(), MemoryStorageError> {
        self.candidate.active(LOADING)?;
        let restore = self.restore.take().ok_or(Error)?;
        {
            let a = restore.attempt.account.lock().map_err(|_| Error)?;
            let f = a.text_facts.ok_or(Error)?;
            if f.epoch != self.candidate.epoch || f.own_index != u64::from(self.candidate.own) {
                return Err(Error.into());
            }
        }
        restore.complete()?;
        let mut values = storage.values.write().map_err(|_| Error)?;
        let installed = storage.text_add.lock().map_err(|_| Error)?;
        if !installed
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(p, &self.candidate))
        {
            return Err(Error.into());
        }
        let mut a = self.candidate.account.lock().map_err(|_| Error)?;
        let count = values.len().checked_add(MAX_PROVIDER_WRITES).ok_or(Error)?;
        let maximum = account::backing(count).ok_or(Error)?;
        a.work(maximum)?;
        a.provider(maximum)?; // BEFORE the only possible table growth allocation.
        values.try_reserve(MAX_PROVIDER_WRITES).map_err(|_| Error)?;
        if values.allocation_size() > maximum {
            return Err(Error.into());
        }
        self.candidate.transition(LOADING, PREPARED)?;
        Ok(())
    }
    /// Mark BEFORE the first SDK build/key/RNG call. No error path changes this
    /// back to Prepared; the owning Cell may inspect/drop private scratch only.
    pub fn enter_add(&mut self) -> Result<(), MemoryStorageError> {
        self.candidate.transition(PREPARED, ADD).map_err(Into::into)
    }
    /// Call only AFTER all actual returned SDK message objects are cached.
    pub fn outputs_cached(&mut self) -> Result<(), MemoryStorageError> {
        self.candidate.transition(ADD, OUTPUTS).map_err(Into::into)
    }
    /// Serialization owns no provider writes. Only the SAME already-cached
    /// operation can progress to merge; ordinary wrappers remain closed.
    pub fn enter_merge(&mut self) -> Result<(), MemoryStorageError> {
        self.candidate
            .transition(OUTPUTS, MERGING)
            .map_err(Into::into)
    }
    pub fn close_merged(&mut self, storage: &MemoryStorage) -> Result<(), MemoryStorageError> {
        self.candidate.active(MERGING)?;
        let values = storage.values.read().map_err(|_| Error)?;
        let installed = storage.text_add.lock().map_err(|_| Error)?;
        if !installed
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(p, &self.candidate))
        {
            return Err(Error.into());
        }
        let mut account = self.candidate.account.lock().map_err(|_| Error)?;
        let mut facts = text_current::shape::Facts::default();
        for row in ROWS {
            let key = Key::new(row, &self.candidate.group, &self.candidate.signer)?;
            account
                .typed
                .lookup(values.allocation_size(), key.bytes().len())?;
            let bytes = match values.get(key.bytes()) {
                Some(value) => value.as_slice(),
                None if row.empty_list() => b"[]",
                None => return Err(Error.into()),
            };
            text_current::shape::check(
                row,
                bytes,
                &self.candidate.group,
                &self.candidate.signer,
                &mut facts,
                &mut account.typed,
            )?;
        }
        facts.finish()?;
        if facts.epoch != self.candidate.epoch.checked_add(1).ok_or(Error)?
            || facts.own_index != u64::from(self.candidate.own)
        {
            return Err(Error.into());
        }
        self.candidate.transition(MERGING, CLOSED)?;
        self.closed = true;
        Ok(())
    }
    pub fn fail(&mut self) {
        self.candidate.phase.store(FAILED, Ordering::Release);
    }
}
impl Drop for TextAddCandidateGuard {
    fn drop(&mut self) {
        if !self.closed {
            self.fail();
        }
    }
}

impl MemoryStorage {
    /// Install only on a fresh provider. The old restore policy stays Some for
    /// its entire lifetime, and ordinary require_unrestricted never succeeds.
    pub fn begin_text_add_candidate_restore(
        &self,
        group: &[u8],
        signer: &[u8],
        epoch: u64,
        own: u32,
        restore_limits: OwnerCheckpointRestoreLimits,
        operation_limits: TextAddOperationLimits,
    ) -> Result<TextAddCandidateGuard, MemoryStorageError> {
        epoch.checked_add(1).ok_or(Error)?;
        let group: [u8; 16] = group.try_into().map_err(|_| Error)?;
        let signer: [u8; 32] = signer.try_into().map_err(|_| Error)?;
        // Account body/counter allocation is charged before its Arc allocation.
        let account = OperationAccount::new(operation_limits)?;
        let restore = self.begin_closed_restore(
            &group,
            &signer,
            restore_limits,
            text_current::Family::TextCurrent,
        )?;
        let candidate = Arc::new(Candidate {
            phase: AtomicU8::new(LOADING),
            account: Mutex::new(account),
            group,
            signer,
            epoch,
            own,
        });
        let values = self.values.read().map_err(|_| Error)?;
        let mut installed = self.text_add.lock().map_err(|_| Error)?;
        if installed.is_some() {
            return Err(Error.into());
        }
        {
            // This extra row is NOT one of restore's thirteen typed getters.
            // Refuse malformed/unbounded old-epoch material before the first
            // signer/group typed SDK allocation. Merge later repeats this SAME
            // row's grammar and uses the counted decoder for the actual read.
            let mut account = candidate.account.lock().map_err(|_| Error)?;
            account.key_entry()?;
            let key = candidate.expected_epoch_key(epoch)?;
            account
                .typed
                .lookup(values.allocation_size(), key.bytes().len())?;
            epoch_keys::check(values.get(key.bytes()).ok_or(Error)?, &mut account.typed)?;
        }
        *installed = Some(candidate.clone());
        drop(installed);
        drop(values);
        Ok(TextAddCandidateGuard {
            restore: Some(restore),
            candidate,
            closed: false,
        })
    }

    pub(crate) fn text_add_write_group<K: Serialize + ?Sized, V: Serialize + ?Sized>(
        &self,
        label: &[u8],
        group: &K,
        value: &V,
    ) -> Result<bool, MemoryStorageError> {
        let mut values = self.values.write().map_err(|_| Error)?;
        let installed = self.text_add.lock().map_err(|_| Error)?;
        let Some(p) = installed.as_ref() else {
            return Ok(false);
        };
        let phase = p.phase.load(Ordering::Acquire);
        let row = Row::from_label(label)?;
        if !((phase == ADD && matches!(row, Row::State | Row::Messages))
            || (phase == MERGING
                && matches!(
                    row,
                    Row::Tree
                        | Row::Context
                        | Row::Interim
                        | Row::Confirmation
                        | Row::OwnIndex
                        | Row::Epoch
                        | Row::Messages
                        | Row::Resumption
                        | Row::State
                )))
        {
            return Err(Error.into());
        }
        let mut a = p.account.lock().map_err(|_| Error)?;
        a.write_entry(MAX_ROW)?;
        let key = p.group_key(row, group, &mut a)?;
        a.typed
            .lookup(values.allocation_size(), key.bytes().len())?;
        #[cfg(any(test, feature = "test-utils"))]
        if phase == ADD && matches!(row, Row::State) {
            test_support::pending_serialize(&mut a)?;
        }
        let value = a.encode(value, MAX_ROW)?;
        let key = a.owned_key(&key)?;
        // complete_loading reserved MAX_PROVIDER_WRITES insertions. Every actual
        // operation write spends one before entry; no implicit reserve occurs.
        values.insert(key, value);
        #[cfg(any(test, feature = "test-utils"))]
        {
            let event = match (phase, row) {
                (ADD, Row::State) => Some(test_support::TextAddTestEvent::PendingWritten),
                (ADD, Row::Messages) => Some(test_support::TextAddTestEvent::OldRatchetsWritten),
                (MERGING, Row::State) => Some(test_support::TextAddTestEvent::MergeStateWritten),
                (MERGING, Row::Tree) => Some(test_support::TextAddTestEvent::MergeTreeWritten),
                _ => None,
            };
            if let Some(event) = event {
                test_support::event(event)?;
            }
        }
        Ok(true)
    }
    pub(crate) fn text_add_read_epoch<
        G: Serialize + ?Sized,
        E: Serialize + ?Sized,
        V: DeserializeOwned,
    >(
        &self,
        group: &G,
        epoch: &E,
        own: u32,
    ) -> Result<Option<Vec<V>>, MemoryStorageError> {
        let values = self.values.read().map_err(|_| Error)?;
        let installed = self.text_add.lock().map_err(|_| Error)?;
        let Some(p) = installed.as_ref() else {
            return Ok(None);
        };
        p.active(MERGING)?;
        #[cfg(any(test, feature = "test-utils"))]
        test_support::event(test_support::TextAddTestEvent::MergeEpochRead)?;
        let mut a = p.account.lock().map_err(|_| Error)?;
        let key = p.epoch_key(group, epoch, own, p.epoch, &mut a)?;
        a.typed
            .lookup(values.allocation_size(), key.bytes().len())?;
        // A missing old-key row is corruption, never the legacy empty default.
        let bytes = values.get(key.bytes()).ok_or(Error)?;
        epoch_keys::check(bytes, &mut a.typed)?;
        let result = decode::from_slice(bytes, &mut a.typed)?;
        Ok(Some(result))
    }
    pub(crate) fn text_add_write_epoch<
        G: Serialize + ?Sized,
        E: Serialize + ?Sized,
        V: Serialize,
    >(
        &self,
        group: &G,
        epoch: &E,
        own: u32,
        pairs: &[V],
    ) -> Result<bool, MemoryStorageError> {
        let mut values = self.values.write().map_err(|_| Error)?;
        let installed = self.text_add.lock().map_err(|_| Error)?;
        let Some(p) = installed.as_ref() else {
            return Ok(false);
        };
        p.active(MERGING)?;
        if pairs.is_empty() || pairs.len() > epoch_keys::MAX_KEYS {
            return Err(Error.into());
        }
        let mut a = p.account.lock().map_err(|_| Error)?;
        a.write_entry(epoch_keys::MAX_FRAME)?;
        let key = p.epoch_key(
            group,
            epoch,
            own,
            p.epoch.checked_add(1).ok_or(Error)?,
            &mut a,
        )?;
        a.typed
            .lookup(values.allocation_size(), key.bytes().len())?;
        let value = a.encode(pairs, epoch_keys::MAX_FRAME)?;
        epoch_keys::check(&value, &mut a.typed)?;
        values.insert(a.owned_key(&key)?, value);
        #[cfg(any(test, feature = "test-utils"))]
        test_support::event(test_support::TextAddTestEvent::MergeEpochWritten)?;
        Ok(true)
    }
    pub(crate) fn text_add_delete_epoch<G: Serialize + ?Sized, E: Serialize + ?Sized>(
        &self,
        group: &G,
        epoch: &E,
        own: u32,
    ) -> Result<bool, MemoryStorageError> {
        let mut values = self.values.write().map_err(|_| Error)?;
        let installed = self.text_add.lock().map_err(|_| Error)?;
        let Some(p) = installed.as_ref() else {
            return Ok(false);
        };
        p.active(MERGING)?;
        let mut a = p.account.lock().map_err(|_| Error)?;
        a.write_entry(0)?;
        let key = p.epoch_key(group, epoch, own, p.epoch, &mut a)?;
        a.typed
            .lookup(values.allocation_size(), key.bytes().len())?;
        values.remove(key.bytes()).ok_or(Error)?;
        #[cfg(any(test, feature = "test-utils"))]
        test_support::event(test_support::TextAddTestEvent::MergeEpochDeleted)?;
        Ok(true)
    }
    pub(crate) fn text_add_delete_empty_group<K: Serialize + ?Sized>(
        &self,
        label: &[u8],
        group: &K,
    ) -> Result<bool, MemoryStorageError> {
        let mut values = self.values.write().map_err(|_| Error)?;
        let installed = self.text_add.lock().map_err(|_| Error)?;
        let Some(p) = installed.as_ref() else {
            return Ok(false);
        };
        p.active(MERGING)?;
        let row = Row::from_label(label)?;
        if !matches!(row, Row::Proposals | Row::OwnLeaves) {
            return Err(Error.into());
        }
        let mut a = p.account.lock().map_err(|_| Error)?;
        a.write_entry(0)?;
        let key = p.group_key(row, group, &mut a)?;
        a.typed
            .lookup(values.allocation_size(), key.bytes().len())?;
        if let Some(bytes) = values.get(key.bytes()) {
            let mut c = cursor::Cursor::new(bytes, &mut a.typed);
            c.take(b'[')?;
            c.take(b']')?;
            c.finish()?;
        }
        values.remove(key.bytes());
        Ok(true)
    }
}
