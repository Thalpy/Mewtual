//! Authenticated bounded ORIGINAL canonical input, still unbound until restored-head custody.
//!
//! This object is not a fresh proof or a durability receipt. Visible authenticated bytes after
//! restart still require exact file+child-parent re-sync under admission -> head before a writer
//! can start. The restore factory additionally validates actual signer/Active scope and original
//! facts against this independently captured immutable pin, never the tail's self-selected root.
use super::super::CapturedOwnerProfilePin;
use super::*;
use crate::store::{ChatAdmissionKey, ChatAdmissionResolution};
use std::ops::Range;

/// Non-Clone/nonserialized canonical source. Holds only original weak lifetime observations,
/// charged plaintext and the exact external scope classification; numeric mirrors/raw bytes
/// cannot construct it. Retired classification creates an OfflineRetired capsule BEFORE T.
pub(crate) struct ValidatedOwnerSnapshot {
    pub(super) pin: CapturedOwnerProfilePin,
    pub(super) cell: Arc<head::HeadCell>,
    pub(super) admission: ChatAdmissionResolution,
    pub(super) labels: Labels,
    pub(super) meta: head::Meta,
    pub(super) plain: Zeroizing<Vec<u8>>,
    snapshot: Range<usize>,
    pub(super) file_cap: usize,
    _fee: ReadFee,
}
impl std::fmt::Debug for ValidatedOwnerSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedOwnerSnapshot")
            .finish_non_exhaustive()
    }
}
pub(super) struct Labels {
    pub(super) signer: [u8; 32],
    pub(super) domain: [u8; 32],
    pub(super) nonce: [u8; 32],
    pub(super) revision: u64,
}
impl ValidatedOwnerSnapshot {
    pub(crate) fn group(&self) -> &[u8] {
        self.pin.group()
    }
    pub(crate) fn signer(&self) -> &[u8; 32] {
        &self.labels.signer
    }
    pub(crate) fn profile(&self) -> OwnerFactId {
        self.pin.profile()
    }
    pub(crate) fn founder(&self) -> &[u8; 32] {
        self.pin.founder()
    }
    pub(crate) fn domain(&self) -> [u8; 32] {
        self.labels.domain
    }
    pub(crate) fn nonce(&self) -> [u8; 32] {
        self.labels.nonce
    }
    pub(crate) fn revision(&self) -> u64 {
        self.labels.revision
    }
    pub(crate) fn admission(&self) -> &ChatAdmissionResolution {
        &self.admission
    }
    pub(crate) fn snapshot(&self) -> &[u8] {
        &self.plain[self.snapshot.clone()]
    }
    pub(crate) fn snapshot_digest(&self) -> [u8; 32] {
        self.meta.snapshot
    }
    pub(crate) fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.pin.validate_original(store, self.group())?;
        if self.labels.domain != store.chat_vault_domain()? {
            return Err(refused());
        }
        Ok(())
    }
}
struct ReadFee {
    pool: Weak<Mutex<Runtime>>,
    bytes: usize,
}
impl Drop for ReadFee {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            if let Ok(mut pool) = pool.lock() {
                if let Some(bytes) = pool.hot_bytes.checked_sub(self.bytes) {
                    pool.hot_bytes = bytes;
                }
            }
        }
    }
}
impl ServerStore {
    /// Prepare the exact canonical source under a previously captured mounted immutable pin.
    /// No transport is constructed here. This does NOT complete restart durability or return
    /// publication authority: final restored-head validation must still cross the real barrier.
    pub(crate) fn prepare_owner_canonical_restore(
        &self,
        pin: CapturedOwnerProfilePin,
        signer: [u8; 32],
    ) -> Result<ValidatedOwnerSnapshot, AppError> {
        pin.validate_original(self, pin.group())?;
        if self.chat_vault_format()? != catcoms_storage::VaultFormat::OwnerEvidence {
            return Err(refused());
        }
        let identifier = key(pin.group(), &signer);
        let (cell, cap, fee) = {
            let mut pool = self
                .owner_evidence
                .canonical
                .lock()
                .map_err(|_| refused())?;
            let budget = pool.budget.ok_or_else(refused)?;
            let cell = pool.heads.get(&identifier).cloned().ok_or_else(refused)?;
            let bytes = budget.buffer_fee()?;
            let hot = pool
                .hot_bytes
                .checked_add(bytes)
                .filter(|hot| *hot <= budget.hot_bytes)
                .ok_or_else(refused)?;
            pool.hot_bytes = hot;
            (
                cell,
                budget.snapshot + WRAPPER_FEE + SEALED_FEE,
                ReadFee {
                    pool: Arc::downgrade(&self.owner_evidence.canonical),
                    bytes,
                },
            )
        };
        let original = cell.read_stamp()?;
        let plain = {
            let (_file, raw) = self.read_owner_canonical_record(&identifier, cap)?;
            Zeroizing::new(unseal(&self.keys.db_key()?, &crate::store::unframe(&raw)?)?)
        };
        let (labels, meta, snapshot) = decode(&plain, &pin, &signer)?;
        if original.as_ref().is_some_and(|known| known != &meta) {
            return Err(refused());
        }
        let identity = ChatAdmissionKey::new(pin.group(), signer)?;
        let admission = self.resolve_chat_admission(&identity, labels.nonce)?;
        let source = ValidatedOwnerSnapshot {
            pin,
            cell,
            admission,
            labels,
            meta,
            plain,
            snapshot,
            file_cap: cap,
            _fee: fee,
        };
        source.validate_original(self)?;
        Ok(source)
    }
}
fn decode(
    bytes: &[u8],
    pin: &CapturedOwnerProfilePin,
    signer: &[u8; 32],
) -> Result<(Labels, head::Meta, Range<usize>), AppError> {
    let mut decoder = Decoder::new(bytes);
    if decoder.get_str().map_err(|_| refused())? != "catcoms/owner-canonical-snapshot/v1"
        || decoder.get_bytes().map_err(|_| refused())? != pin.group()
        || decoder.get_bytes().map_err(|_| refused())? != signer
        || decoder.get_bytes().map_err(|_| refused())? != pin.profile().as_bytes()
    {
        return Err(refused());
    }
    let domain = decoder
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let nonce = decoder
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let revision = decoder.get_u64().map_err(|_| refused())?;
    let generation = decoder.get_u64().map_err(|_| refused())?;
    let parent = decoder.get_bytes().map_err(|_| refused())?;
    // First initialization is the ONLY implemented writer. General successor grammar/head CAS
    // must be added with its actual covering producer, not accepted here speculatively.
    if generation != 1 || !parent.is_empty() {
        return Err(refused());
    }
    let attempt = decoder
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let snapshot = decoder.get_bytes().map_err(|_| refused())?;
    decoder.finish().map_err(|_| refused())?;
    let start = snapshot.as_ptr() as usize - bytes.as_ptr() as usize;
    let range = start..start.checked_add(snapshot.len()).ok_or_else(refused)?;
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/owner-head-id/v1\0");
    hash.update(bytes);
    let meta = head::Meta {
        generation,
        head: *hash.finalize().as_bytes(),
        snapshot: *blake3::hash(snapshot).as_bytes(),
        attempt,
        profile: pin.profile(),
    };
    Ok((
        Labels {
            signer: *signer,
            domain,
            nonce,
            revision,
        },
        meta,
        range,
    ))
}
