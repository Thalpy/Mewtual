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

/// Fixed ORIGINAL names only. InitialStage remains exact-v4 S1; OperationStage has its own
/// exact-parent/current/full-tail proof and can never fall through the S1 algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum OwnerSourceLocation {
    Canonical,
    InitialStage,
    OperationStage,
}

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
    pub(super) location: OwnerSourceLocation,
    pub(super) operation_parent: Option<OperationParent>,
    pub(super) operation_canonical: Option<head::Meta>,
    // Captured with the SAME read-fee reservation, never selected by a later caller or reread
    // from a replacement mount. These are negative resource limits, not runtime authority.
    restore_limits: catcoms_sync::OwnerInitializationRestoreLimits,
    _fee: ReadFee,
}
pub(super) struct OperationParent {
    pub(super) plain: Zeroizing<Vec<u8>>,
    snapshot: Range<usize>,
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
    pub(crate) fn restore_limits(&self) -> catcoms_sync::OwnerInitializationRestoreLimits {
        self.restore_limits
    }
    pub(crate) fn is_operation_checkpoint(&self) -> bool {
        self.meta.generation == 2
    }
    pub(crate) fn operation_parent_snapshot(&self) -> Option<&[u8]> {
        self.operation_parent
            .as_ref()
            .map(|parent| &parent.plain[parent.snapshot.clone()])
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
impl ReadFee {
    /// Charge the SAME original pool before SDK/result decode. Base four buffers alone do
    /// not cover retained parent + raw/cipher/plain rereads alongside the decoded outcome.
    fn reserve_operation_overlap(&mut self) -> Result<(), AppError> {
        let pool = self.pool.upgrade().ok_or_else(refused)?;
        let mut runtime = pool.lock().map_err(|_| refused())?;
        let extra = catcoms_sync::owner_evidence::OWNER_FIRST_ADD_EXTRA_PRIVATE_BYTES;
        let budget = runtime.budget.ok_or_else(refused)?;
        let bytes = self.bytes.checked_add(extra).ok_or_else(refused)?;
        let hot = runtime
            .hot_bytes
            .checked_add(extra)
            .filter(|n| *n <= budget.hot_bytes)
            .ok_or_else(refused)?;
        runtime.hot_bytes = hot;
        self.bytes = bytes;
        Ok(())
    }
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
        let format = self.chat_vault_format()?;
        if !format.requires_owner_evidence() {
            return Err(refused());
        }
        let identifier = key(pin.group(), &signer);
        let (cell, cap, restore_limits, mut fee) = {
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
                catcoms_sync::OwnerInitializationRestoreLimits {
                    max_snapshot_bytes: budget.snapshot,
                    max_mls_bytes: budget.mls,
                    max_mls_backing_bytes: budget.mls_backing,
                },
                ReadFee {
                    pool: Arc::downgrade(&self.owner_evidence.canonical),
                    bytes,
                },
            )
        };
        let operation_stage = format == catcoms_storage::VaultFormat::OwnerOperations
            && self.owner_canonical_stage_exists(&identifier)?;
        let (original, pending) = if format == catcoms_storage::VaultFormat::OwnerOperations {
            cell.read_operation_stamp(operation_stage)?
        } else {
            (cell.read_stamp()?, None)
        };
        // A present canonical source always wins, including when it is malformed. Falling
        // back after a failed canonical parse could reinstall a stale first-stage candidate.
        let mut location = if self.owner_canonical_record_exists(&identifier)? {
            OwnerSourceLocation::Canonical
        } else {
            // Keep this exact v4 check even when the positive canonical format guard above
            // grows to support v5. A v5 stage can cover operation/debt state, not this S1 path.
            if format != catcoms_storage::VaultFormat::OwnerEvidence {
                return Err(refused());
            }
            OwnerSourceLocation::InitialStage
        };
        let mut plain = {
            let (_file, raw) = self.read_owner_restore_record(&identifier, cap, location)?;
            Zeroizing::new(unseal(&self.keys.db_key()?, &crate::store::unframe(&raw)?)?)
        };
        let mut decoded = decode(&plain, &pin, &signer, format)?;
        let mut operation_parent = None;
        let mut operation_canonical = None;
        if operation_stage {
            // Missing canonical, corrupt stage, foreign/unknown parent and differing gen2
            // candidates are all preservation/refusal. No old-source fallback occurs.
            if location != OwnerSourceLocation::Canonical {
                return Err(refused());
            }
            let stage_plain = {
                let (_file, raw) = self.read_owner_restore_record(
                    &identifier,
                    cap,
                    OwnerSourceLocation::OperationStage,
                )?;
                Zeroizing::new(unseal(&self.keys.db_key()?, &crate::store::unframe(&raw)?)?)
            };
            let stage = decode(&stage_plain, &pin, &signer, format)?;
            let parent = stage.parent.ok_or_else(refused)?;
            if decoded.labels.domain != stage.labels.domain
                || decoded.labels.nonce != stage.labels.nonce
                || decoded.labels.revision != stage.labels.revision
            {
                return Err(refused());
            }
            if decoded.meta.generation == 1 {
                if parent.head != decoded.meta.head
                    || parent.snapshot != decoded.meta.snapshot
                    || parent.generation != decoded.meta.generation
                    || plain[decoded.snapshot.clone()].len()
                        > restore_limits
                            .max_mls_bytes
                            .checked_add(64 * 1024)
                            .ok_or_else(refused)?
                {
                    return Err(refused());
                }
                operation_parent = Some(OperationParent {
                    plain,
                    snapshot: decoded.snapshot.clone(),
                });
            } else if decoded.meta != stage.meta || plain.as_slice() != stage_plain.as_slice() {
                return Err(refused());
            } // Duplicate layout drops canonical plaintext here; never retain two full gen2.
            let original_matches_parent = |known: &head::Meta| {
                known.generation == parent.generation
                    && known.head == parent.head
                    && known.snapshot == parent.snapshot
                    && known.profile == pin.profile()
            };
            if original.as_ref().is_some_and(|known| {
                known != &decoded.meta && known != &stage.meta && !original_matches_parent(known)
            }) || pending.as_ref().is_some_and(|known| known != &stage.meta)
            {
                return Err(refused());
            }
            // After a rename/failed parent sync the live head may still name the original
            // parent even though BOTH fixed names contain the candidate. Preserve that exact
            // original stamp; final CAS must not invent or silently discard pending state.
            operation_canonical = Some(original.clone().unwrap_or_else(|| decoded.meta.clone()));
            plain = stage_plain;
            decoded = stage;
            location = OwnerSourceLocation::OperationStage;
        }
        if !operation_stage && pending.is_some() {
            // An exact earlier rename may already have consumed the stage, while its writer
            // dropped before a successful directory barrier/install. Only the actual gen2
            // candidate and its exact original pending stamp may settle this debt.
            let predecessor = decoded.parent.ok_or_else(refused)?;
            if pending.as_ref() != Some(&decoded.meta)
                || original.as_ref().is_some_and(|known| {
                    known != &decoded.meta
                        && (known.generation != predecessor.generation
                            || known.head != predecessor.head
                            || known.snapshot != predecessor.snapshot
                            || known.profile != pin.profile())
                })
            {
                return Err(refused());
            }
            operation_canonical = Some(original.clone().unwrap_or_else(|| decoded.meta.clone()));
            location = OwnerSourceLocation::OperationStage;
        }
        let DecodedOwnerFrame {
            labels,
            meta,
            snapshot,
            parent,
        } = decoded;
        if let Some(predecessor) = parent {
            if location == OwnerSourceLocation::InitialStage {
                // S1 only repairs actual v4 generation1. A possible successor stage must not
                // be promoted by that first-initialization algorithm.
                return Err(refused());
            }
            catcoms_sync::owner_evidence::validate_owner_operation_snapshot(
                &plain[snapshot.clone()],
                restore_limits,
                catcoms_sync::owner_evidence::OwnerOperationSourceExpectation {
                    group: pin.group(),
                    signer: &signer,
                    profile: pin.profile(),
                    predecessor,
                    request: meta.attempt,
                },
            )
            .map_err(|_| refused())?;
            fee.reserve_operation_overlap()?;
        }
        if location != OwnerSourceLocation::OperationStage
            && original.as_ref().is_some_and(|known| known != &meta)
        {
            return Err(refused());
        }
        let identity = ChatAdmissionKey::new(pin.group(), signer)?;
        let admission = self.resolve_chat_admission(&identity, labels.nonce)?;
        if location == OwnerSourceLocation::OperationStage
            && matches!(&admission, ChatAdmissionResolution::Initializing(_))
        {
            return Err(refused());
        }
        if location == OwnerSourceLocation::InitialStage {
            // Known Retired bytes can remain an offline capsule. Current plus a missing .bin
            // is never permission to reinstall a possibly stale stage. v5 is excluded by the
            // actual mounted-format gate above, independently of any saved source labels.
            match &admission {
                ChatAdmissionResolution::Initializing(preparation) => {
                    preparation.validate_initialization_attempt(&meta.attempt)?;
                }
                ChatAdmissionResolution::Retired => {}
                ChatAdmissionResolution::Current(_) => return Err(refused()),
            }
        }
        let source = ValidatedOwnerSnapshot {
            pin,
            cell,
            admission,
            labels,
            meta,
            plain,
            snapshot,
            file_cap: cap,
            location,
            operation_parent,
            operation_canonical,
            restore_limits,
            _fee: fee,
        };
        source.validate_original(self)?;
        Ok(source)
    }
}
struct DecodedOwnerFrame {
    labels: Labels,
    meta: head::Meta,
    snapshot: Range<usize>,
    parent: Option<catcoms_sync::owner_evidence::OwnerOperationPredecessor>,
}
fn decode(
    bytes: &[u8],
    pin: &CapturedOwnerProfilePin,
    signer: &[u8; 32],
    format: catcoms_storage::VaultFormat,
) -> Result<DecodedOwnerFrame, AppError> {
    let mut decoder = Decoder::new(bytes);
    let is_operation = match decoder.get_str().map_err(|_| refused())? {
        "catcoms/owner-canonical-snapshot/v1" => false,
        "catcoms/owner-canonical-snapshot/v2"
            if format == catcoms_storage::VaultFormat::OwnerOperations =>
        {
            true
        }
        _ => return Err(refused()),
    };
    if decoder.get_bytes().map_err(|_| refused())? != pin.group()
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
    let parent_head = decoder.get_bytes().map_err(|_| refused())?;
    let parent = if is_operation {
        if generation != 2 {
            return Err(refused());
        }
        let head = parent_head.try_into().map_err(|_| refused())?;
        let parent_generation = decoder.get_u64().map_err(|_| refused())?;
        if parent_generation != 1 {
            return Err(refused());
        }
        let snapshot = decoder
            .get_bytes()
            .map_err(|_| refused())?
            .try_into()
            .map_err(|_| refused())?;
        Some(catcoms_sync::owner_evidence::OwnerOperationPredecessor {
            domain,
            nonce,
            head,
            generation: parent_generation,
            snapshot,
        })
    } else {
        if generation != 1 || !parent_head.is_empty() {
            return Err(refused());
        }
        None
    };
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
    Ok(DecodedOwnerFrame {
        labels: Labels {
            signer: *signer,
            domain,
            nonce,
            revision,
        },
        meta,
        snapshot: range,
        parent,
    })
}
