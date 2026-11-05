//! Pre-cleaner critical-format catalog. EMPTY is the only supported namespace set today.
//!
//! This is deliberately not a join/archive implementation. A nonempty candidate, including an
//! otherwise well-framed future join entry, refuses BEFORE the older operation/pin cleaners.
//! The fixed current AND stage must both authenticate before either can be settled. No exposed
//! Store has a catalog writer in this slice; the physical session excludes another constructor.
//!
//! Fixed logical allocation inventory (excluding the separately existing vault KDF/inventories):
//! one requested 553-byte read buffer, <=528-byte unframed ciphertext and <=512-byte plaintext,
//! plus the writer's 512-byte encoder, <=528-byte sealed output and <=552-byte frame. Initial
//! write verification can overlap the first two sets. Including two existing file handles,
//! domain/key copies and fixed codec state stays within an 8KiB logical metadata allowance.
//! Inputs are checked before these allocations and no entry container is allocated. This is not
//! a global host pool debit, allocator/RSS measurement, SDK heap bound or new active-host gate;
//! a nonempty namespace must supply its actual larger invoice and consumer together.
use super::*;
use catcoms_rt::RequestCancellation;
use catcoms_storage::VaultFormat;
use std::sync::Arc;
pub(super) mod read_pool;
pub use read_pool::CriticalCatalogReadPool;
use read_pool::{CatalogReadReservation, EMPTY_READ_FEE};

const TAG: &str = "catcoms/critical-vault-namespaces/v1";
pub(super) const MAX_PLAIN: usize = 512;
pub(super) const MAX_FRAMED: usize = MAX_PLAIN + 40;
const MAX_ENTRIES: usize = 16;

/// Fixed, nonredirectable metadata destinations. Persistence derives paths from the original
/// Store and this closed enum; no caller may supply a catalog filename or directory.
#[derive(Clone, Copy)]
pub(super) enum Slot {
    Current,
    Stage,
}
impl Slot {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Current => "chat-critical-namespaces.bin",
            Self::Stage => "chat-critical-namespaces.replace.tmp",
        }
    }
}

pub(super) fn refused() -> AppError {
    AppError::Invalid("critical vault metadata is unavailable or unsupported".into())
}

/// Negative original-mount custody, with actual known files kept open for their real barriers.
/// Construction proves only supported Empty metadata, never membership, Current or a saved head.
pub(super) struct ValidatedEmptyCatalog {
    mount: Arc<()>,
    original: RequestCancellation,
    pub(super) current: Option<File>,
    pub(super) stage: Option<File>,
    // Files and every dependent copy drop BEFORE this original prepaid read allowance.
    _read: Option<CatalogReadReservation>,
}
impl ValidatedEmptyCatalog {
    pub(super) fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.mount, &store.registry_mount()) || self.original.is_cancelled() {
            return Err(refused());
        }
        Ok(())
    }
}

/// Inspect fixed metadata BEFORE lower vault opening could allocate a new identity. In
/// particular a dangling vault link is present/unsupported, not permission to create a DEK.
pub(super) fn refuse_orphan_before_vault_creation(dir: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(dir.join("vault.bin")) {
        Ok(metadata) if metadata.file_type().is_file() => return Ok(()),
        Ok(_) => return Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(refused()),
    }
    for slot in [Slot::Current, Slot::Stage] {
        match fs::symlink_metadata(dir.join(slot.name())) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    Ok(())
}

/// Complete borrowed framing validation precedes the supported-shape decision. This codec
/// reserves ID/version/payload framing for future reviewed consumers but supports NONE of them.
/// A valid nonempty or different-generation record stays unsupported; it never becomes Empty.
fn validate_empty(bytes: &[u8], domain: &[u8; 32]) -> Result<(), AppError> {
    if bytes.len() > MAX_PLAIN {
        return Err(refused());
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.get_str().map_err(|_| refused())? != TAG
        || decoder.get_bytes().map_err(|_| refused())? != domain
    {
        return Err(refused());
    }
    let generation = decoder.get_u64().map_err(|_| refused())?;
    let count = decoder.get_u32().map_err(|_| refused())? as usize;
    if count > MAX_ENTRIES || count > decoder.remaining() / 12 {
        return Err(refused());
    }
    let mut previous = 0;
    for _ in 0..count {
        let id = decoder.get_u32().map_err(|_| refused())?;
        let version = decoder.get_u32().map_err(|_| refused())?;
        let _payload = decoder.get_bytes().map_err(|_| refused())?;
        if id <= previous || version == 0 {
            return Err(refused());
        }
        previous = id;
    }
    decoder.finish().map_err(|_| refused())?;
    if generation != 1 || count != 0 {
        return Err(refused());
    }
    Ok(())
}

fn encode_empty(domain: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut encoder = Encoder::with_capacity(MAX_PLAIN);
    encoder.put_str(TAG).map_err(|_| refused())?;
    encoder.put_bytes(domain).map_err(|_| refused())?;
    encoder.put_u64(1);
    encoder.put_u32(0);
    let bytes = Zeroizing::new(encoder.finish());
    validate_empty(&bytes, domain)?;
    Ok(bytes)
}

impl ServerStore {
    /// Actual constructor entry. Inspect BOTH catalog sources before calling any old gate,
    /// including when the authenticated wrapper is old. A v6 wrapper cannot ignore a missing
    /// catalog; an old wrapper cannot ignore pending newer-format intent. No empty old lineage
    /// is manufactured: operations recovery below requires every existing mandatory inventory.
    pub(super) fn open_critical_namespace_gate(
        &self,
        actual_format: VaultFormat,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let Some(catalog) = self.inspect_empty_critical_catalog()? else {
            if actual_format == VaultFormat::CriticalNamespaces {
                return Err(refused());
            }
            return self.open_owner_operations_gate(actual_format, passphrase, rng);
        };
        // Catalog activation ALWAYS followed a supported v5 transaction. Its intent must be
        // present even if an authentic older wrapper was replayed; otherwise the old gate could
        // initialize legacy absence before discovering the new format. This check is read-only.
        if !self.has_owner_operations_intent()? {
            return Err(refused());
        }
        self.open_owner_operations_gate(actual_format, passphrase, rng)?;
        self.settle_empty_critical_catalog(&catalog)?;
        if self.chat_vault_format()? != VaultFormat::CriticalNamespaces {
            self.activate_critical_namespace_wrapper(passphrase, rng, &catalog)?;
        }
        catalog.validate_original(self)
    }

    /// Consume a still-private Store. Empty catalog preparation and SAME-DEK v6 confirmation
    /// precede exposure; every error drops/revokes the original mount. No join/archive writer or
    /// supported namespace is created. Never call this on a running actor/native Store.
    pub fn into_critical_namespace_store(
        self,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        let store = self.into_owner_operations_store(passphrase, rng)?;
        let catalog = match store.inspect_empty_critical_catalog()? {
            Some(catalog) => catalog,
            None => {
                if store.chat_vault_format()? == VaultFormat::CriticalNamespaces {
                    return Err(refused());
                }
                let plain = encode_empty(&store.chat_vault_domain()?)?;
                let sealed = seal(&store.keys.db_key()?, &plain, rng)?;
                store.write_initial_critical_catalog(&frame(&sealed))?;
                store
                    .inspect_empty_critical_catalog()?
                    .ok_or_else(refused)?
            }
        };
        store.settle_empty_critical_catalog(&catalog)?;
        store.activate_critical_namespace_wrapper(passphrase, rng, &catalog)?;
        Ok(store)
    }

    fn activate_critical_namespace_wrapper(
        &self,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        catalog: &ValidatedEmptyCatalog,
    ) -> Result<(), AppError> {
        catalog.validate_original(self)?;
        map_vault_passphrase_change(catcoms_storage::vault::activate_critical_namespace_vault(
            &self.dir, passphrase, rng,
        ))?;
        let opened = open_or_create_vault_classified(&self.dir, passphrase, rng)?;
        self.confirm_critical_namespace_format(opened, catalog)
    }

    fn inspect_empty_critical_catalog(&self) -> Result<Option<ValidatedEmptyCatalog>, AppError> {
        let read = if let Some(pool) = &self.catalog_read_pool {
            let mut possible_source = false;
            for slot in [Slot::Current, Slot::Stage] {
                match fs::symlink_metadata(self.dir.join(slot.name())) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    _ => possible_source = true,
                }
            }
            if possible_source {
                Some(pool.try_reserve(EMPTY_READ_FEE)?)
            } else {
                None
            }
        } else {
            None
        };
        // Read/authenticate sequentially under exclusive physical custody. Each bounded raw
        // frame/plaintext is dropped before the next, while actual file handles stay open. Both
        // valid Empty parses imply IDENTICAL canonical plaintext; ciphertext nonces may differ.
        let current = self.read_empty_critical_catalog_slot(Slot::Current, read.as_ref())?;
        let stage = self.read_empty_critical_catalog_slot(Slot::Stage, read.as_ref())?;
        if current.is_none() && stage.is_none() {
            return Ok(None);
        }
        Ok(Some(ValidatedEmptyCatalog {
            mount: self.registry_mount(),
            original: self.chat_mount.observer(),
            current,
            stage,
            _read: read,
        }))
    }

    fn read_empty_critical_catalog_slot(
        &self,
        slot: Slot,
        read: Option<&CatalogReadReservation>,
    ) -> Result<Option<File>, AppError> {
        let Some((file, raw)) = self.open_critical_catalog_slot(slot, read)? else {
            return Ok(None);
        };
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&raw)?)?);
        validate_empty(&plain, &self.chat_vault_domain()?)?;
        Ok(Some(file))
    }
}

#[cfg(test)]
mod tests;
