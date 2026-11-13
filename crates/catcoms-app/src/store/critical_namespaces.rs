//! Pre-cleaner critical-format catalog. The DEFAULT reader supports only EMPTY; the explicit
//! private Pending-only policy below has its own bounded parser, source producer and invoice.
//!
//! This default path is not a join/archive implementation. A nonempty candidate refuses BEFORE
//! the older operation/pin cleaners unless the concrete configured Pending handler admits it.
//! The fixed current AND stage must both authenticate before either can be settled. No exposed
//! Store has a writer in the Empty prerequisite; the configured extension owns a separate exact
//! original catalog serializer. The physical session excludes another constructor in either.
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
pub(in crate::store) mod archive_bootstrap;
// The concrete physical Current handler consumes this closed DATA codec. Other constructors
// retain their own strict parsers and refuse current/stage/seed/marker3 before older cleaners.
pub(in crate::store) mod archive_current_namespace_codec;
// Closed metadata DATA used by the real seed-present Current2 storage/owning consumer.
// Decoding alone still issues no source/fee/writer or operational protocol authority.
pub(in crate::store) mod archive_current_codec;
mod catalog_envelope;
// Shared exact sorted-insert declaration DATA for independently declared namespaces (4, 5).
mod catalog_declaration;
pub(in crate::store) mod current_group;
pub(in crate::store) mod current_layout;
pub(in crate::store) mod governance_layout;
pub(in crate::store) mod governance_namespace_codec;
// Namespace5: the ordinary-member outbox layout. Its descriptor codec is DATA only; the
// configured handler in `member_outbox_layout` is the only physical interpreter.
pub(in crate::store) mod member_outbox_layout;
pub(in crate::store) mod member_outbox_namespace_codec;
// Independent native registry: non-writing preparation plus its private physical handler.
#[cfg(test)]
mod native2_predecessor_tests;
#[cfg(test)]
mod native7_predecessor_tests;
pub(in crate::store) mod native_registry;
pub use archive_bootstrap::{
    ArchiveBootstrapCatalogPolicy, ArchiveFounderBegin, ArchiveFounderCreation,
    ArchiveFounderExisting, ArchiveFounderInputs, ArchiveFounderPlanning,
    ArchiveFounderPlanningProgress, ArchiveFounderPolicy, ArchiveFounderProgress,
    ArchiveFounderSourcePreparation, ArchiveFounderSourceProgress, FreshV2TextFounderBegin,
    FreshV2TextFounderBinding, FreshV2TextFounderPolicy, PreparedArchiveBootstrapStore,
    PreparedFreshV2TextFounder, ReturnedArchiveFounderCreation, ReturnedArchiveFounderInputs,
    ReturnedArchiveFounderPlanning,
};
pub use current_layout::{
    CanonicalFilenameInventoryPolicy, PreparedV2CurrentLayoutStore, V2CurrentLayoutPolicy,
    V2CurrentLayoutStore,
};
pub use governance_layout::{
    PreparedV2GovernanceLayoutStore, V2GovernanceLayoutPolicy, V2GovernanceLayoutStore,
};
pub use member_outbox_layout::V2MemberOutboxPolicy;
pub use native_registry::{
    NativeCreationStateData, NativeCreationStatusData, NativeIndexPolicy, NativeIndexStore,
    NativeIndexUpgrade, NativeRegistryInstallation, NativeRegistryPolicy,
    NativeRegistryPreparationPolicy, NativeRegistryStore, NativeTextUiCandidate,
    NativeTextUiGarbage, NativeTextUiObservation, NativeTextUiPreparation, NativeTextUiRetirement,
    NativeTextUiRetirementBatch, NativeTextUiRetirementState, NativeTextVault,
    NativeUiInvocationBudget, NativeUiInvocationMetadata, PreparedNativeRegistryMigration,
};
pub(super) mod read_pool;
pub use read_pool::CriticalCatalogReadPool;
use read_pool::{CatalogReadReservation, EMPTY_READ_FEE};
pub(in crate::store) mod pending_join;
pub(in crate::store) mod pending_join_codec;
pub use pending_join::{
    ActivatedCheckpointJoin, CheckpointJoinExecutionPolicy, CheckpointJoinNetwork,
    CheckpointJoinSubmissionStatus, JournalledCheckpointJoinPreparation, PendingJoinCatalogPolicy,
    ReturnedCheckpointJoinActivation,
};

const TAG: &str = "catcoms/critical-vault-namespaces/v1";
pub(super) const MAX_PLAIN: usize = 512;
pub(super) const MAX_FRAMED: usize = MAX_PLAIN + 40;
const MAX_ENTRIES: usize = 16;

// Reserved physical destinations for the reviewed bootstrap transaction. Neither current
// reader implements ARCHIVE2 yet. Checking these names is therefore a NEGATIVE fence only:
// an absent/old/Empty catalog cannot hide a possibly newer private source from old cleaners.
// The eventual concrete archive opener must classify the seed AND both catalog candidates
// before replacing this refusal; it must not simply skip the check when a flag is enabled.
pub(in crate::store) const ARCHIVE_BOOTSTRAP_ROOT: &str = "chat-owner-v2";
pub(in crate::store) const ARCHIVE_DECLARATION_SEED: &str = "chat-critical-namespaces.archive.seed";

/// No currently supported handler can interpret these private namespace objects. Preserve
/// every kind of presence, including an empty directory, a dangling link or an unreadable
/// object. In particular, `Path::exists` would turn a dangling link into false absence.
fn refuse_unimplemented_archive_storage(dir: &Path) -> Result<(), AppError> {
    for name in [
        ARCHIVE_BOOTSTRAP_ROOT,
        ARCHIVE_DECLARATION_SEED,
        current_group::DECLARATION_SEED,
    ] {
        match fs::symlink_metadata(dir.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    Ok(())
}

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
pub(super) fn refuse_orphan_before_vault_creation(
    dir: &Path,
    native_handler: bool,
) -> Result<(), AppError> {
    // Only the private native opener has already paid for its handler. It may defer
    // classification on an EXISTING real vault; absence never permits adopting artifacts.
    if !native_handler {
        super::persistence::native_registry::refuse_unsupported_storage(dir)?;
    }
    match fs::symlink_metadata(dir.join("vault.bin")) {
        Ok(metadata) if metadata.file_type().is_file() => return Ok(()),
        Ok(_) => return Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(refused()),
    }
    super::persistence::native_registry::refuse_unsupported_storage(dir)?;
    for slot in [Slot::Current, Slot::Stage] {
        match fs::symlink_metadata(dir.join(slot.name())) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    governance_layout::refuse_unsupported_storage(dir)?;
    // A vault that does not exist yet can never own namespace5 artifacts.
    member_outbox_layout::refuse_unsupported_storage(dir)?;
    refuse_unimplemented_archive_storage(dir)?;
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
        // Only the private prepaid native opener can classify/repair namespace7. All
        // ordinary openers keep the early fixed-artifact refusal before older cleaners.
        if self
            .current_layout_policy
            .as_ref()
            .is_some_and(|policy| policy.native_registry.is_some())
        {
            if actual_format != VaultFormat::CriticalNamespaces {
                return Err(refused());
            }
            self.inspect_and_repair_native_registry_declaration()?;
        } else {
            super::persistence::native_registry::refuse_unsupported_storage(&self.dir)?;
        }
        // Every opener WITHOUT the concrete namespace5 handler refuses its fixed artifacts
        // before any gate below can settle, clean or reinterpret them. The configured handler
        // instead inspects them first inside the Current3 (and governance) gate.
        if self.member_outbox_policy().is_none() {
            member_outbox_layout::refuse_unsupported_storage(&self.dir)?;
        }
        if self.governance_layout_policy.is_some() {
            return self.open_governance_layout_catalog_gate(actual_format, passphrase, rng);
        }
        governance_layout::refuse_unsupported_storage(&self.dir)?;
        if self.current_layout_policy.is_some() {
            return self.open_current_layout_catalog_gate(actual_format, passphrase, rng);
        }
        if self.archive_bootstrap_policy.is_some() {
            return self.open_archive_bootstrap_catalog_gate(actual_format, passphrase, rng);
        }
        // Both the default and explicit Pending-only constructor still reject ARCHIVE2.
        // Refuse its physical artifacts even if a replayed Empty/old catalog omits ID2;
        // no lower-format gate may clean or infer absence first. This is not an archive
        // inspector, supported declaration, repair action or permission to create its root.
        refuse_unimplemented_archive_storage(&self.dir)?;
        if self.pending_join_policy_enabled() {
            return self.open_pending_join_catalog_gate(actual_format, passphrase, rng);
        }
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
