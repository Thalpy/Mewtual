//! Initial physical declaration and the closed native-registry mount. This installs only
//! migrated legacy DATA, not a V2 runtime or command credential. Public Current constructors
//! still cannot enable ID7, and this owner exposes neither its Current wrapper nor Store.
use super::super::super::pending_join_codec as catalog;
#[cfg(test)]
use super::super::super::Slot;
use super::*;
use crate::store::persistence::native_registry::{
    dispose_native_first_torn, read_native_current, NativeDeclarationBytes,
    NativeDeclarationInventory, NativeDeclarationIo,
};

// Separate physical invoice, in addition to preparation's retained and transient copies.
// Covers actual and reopened prefix/verification reads, cold seed unseal + decoded copies,
// exact selected index/marker overlap, writable handles and directory/name metadata. The
// original PREPARATION_FEE stays held until all dependent ciphertext and mount owners drop.
const PHYSICAL_FEE: usize = 5 * MAX_SEED_FRAME
    + 6 * MAX_FRAME
    + 4 * MAX_CATALOG_FRAME
    + 4 * MAX_MARKER_FRAME
    + 4 * super::super::super::governance_layout::codec::MAX_SEED_FRAME
    + 256 * 1024;
const PERSISTENT_MINIMUM: usize =
    2 * MAX_CATALOG_FRAME + MAX_SEED_FRAME + MAX_FRAME + MAX_MARKER_FRAME;
#[cfg(test)]
thread_local! {
    static AFTER_CLASSIFICATION: std::cell::RefCell<Option<ClassificationHook>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
type ClassificationHook = Box<dyn FnOnce(&Path)>;

/// Trusted initial native-registry storage policy. Its independent account pays both phases;
/// fixed-file capacity is an admission bound, not evidence of filesystem free space.
#[derive(Clone, Debug)]
pub struct NativeRegistryPolicy {
    preparation: NativeRegistryPreparationPolicy,
    persistent_capacity: usize,
}
impl NativeRegistryPolicy {
    pub fn new(
        pool: CriticalCatalogReadPool,
        persistent_capacity: usize,
    ) -> Result<Self, AppError> {
        if pool.capacity() < Self::required_capacity() || persistent_capacity < PERSISTENT_MINIMUM {
            return Err(refused());
        }
        Ok(Self {
            preparation: NativeRegistryPreparationPolicy::new(pool)?,
            persistent_capacity,
        })
    }
    pub const fn required_capacity() -> usize {
        PREPARATION_FEE + PHYSICAL_FEE
    }
    pub const fn required_persistent_capacity() -> usize {
        PERSISTENT_MINIMUM
    }
    fn physical_payment(
        &self,
        current: &V2CurrentLayoutPolicy,
    ) -> Result<Arc<CatalogReadReservation>, AppError> {
        self.preparation.require_independent(current)?;
        if self.persistent_capacity < PERSISTENT_MINIMUM {
            return Err(refused());
        }
        Ok(Arc::new(self.preparation.pool.try_reserve(PHYSICAL_FEE)?))
    }
}
impl NativeMigrationHandler {
    pub(in crate::store) fn require(&self) -> Result<(), AppError> {
        self.fee.validate(&self.policy.pool, PREPARATION_FEE)?;
        self.physical_fee
            .validate(&self.policy.pool, PHYSICAL_FEE)?;
        if self.persistent_capacity < PERSISTENT_MINIMUM {
            return Err(refused());
        }
        Ok(())
    }
}

/// Startup-private original declaration. An error retains the exact prepared outputs and
/// every opened output handle; retry never draws another nonce or adopts an equal replacement.
pub struct NativeRegistryInstallation {
    // Owning refusals stay small without allocating after an error. Both boxes and the
    // maximum phase overlap are included in PHYSICAL_FEE's metadata allowance.
    io: Box<NativeDeclarationIo>,
    prepared: Box<PreparedNativeRegistryMigration>,
    handler: NativeMigrationHandler,
    complete: bool,
}
impl std::fmt::Debug for NativeRegistryInstallation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeRegistryInstallation")
            .field("complete", &self.complete)
            .finish_non_exhaustive()
    }
}
impl NativeRegistryInstallation {
    /// A distinct writing API; the existing PreparedNativeRegistryMigration stays non-writing.
    /// Both invoices are acquired before prerequisite opening may repair or declare anything.
    pub fn prepare(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        current: V2CurrentLayoutPolicy,
        policy: NativeRegistryPolicy,
    ) -> Result<Self, AppError> {
        let physical_fee = policy.physical_payment(&current)?;
        let prepared = PreparedNativeRegistryMigration::open(
            dir,
            passphrase,
            rng,
            current,
            policy.preparation.clone(),
        )?;
        Self::from_prepared(prepared, policy, physical_fee)
    }

    /// Create one new empty vault and retain the SAME session/keys/lineage
    /// through critical activation and every private namespace declaration.
    /// Existing vaults/artifacts refuse; this is no general cold selector.
    /// Initial preparation errors preserve files and require cold recovery.
    pub fn prepare_fresh(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        current: V2CurrentLayoutPolicy,
        policy: NativeRegistryPolicy,
    ) -> Result<Self, AppError> {
        let physical_fee = policy.physical_payment(&current)?;
        let fee = Arc::new(policy.preparation.pool.try_reserve(PREPARATION_FEE)?);
        let opening = PreparedV2CurrentLayoutStore::create_for_native_registry(
            dir.as_ref(),
            passphrase,
            rng,
            current,
        )?;
        let prepared = PreparedNativeRegistryMigration::from_opening(
            opening,
            rng,
            policy.preparation.clone(),
            fee,
        )?;
        #[cfg(test)]
        super::super::super::current_layout::native_fresh_test_edge(
            super::super::super::current_layout::NativeFreshEdge::NativePrepared,
            prepared.current.cached_store()?,
        );
        Self::from_prepared(prepared, policy, physical_fee)
    }

    fn from_prepared(
        prepared: PreparedNativeRegistryMigration,
        policy: NativeRegistryPolicy,
        physical_fee: Arc<CatalogReadReservation>,
    ) -> Result<Self, AppError> {
        let handler = NativeMigrationHandler {
            policy: policy.preparation,
            fee: prepared.fee.clone(),
            physical_fee,
            persistent_capacity: policy.persistent_capacity,
        };
        handler.require()?;
        Ok(Self {
            io: Box::default(),
            prepared: Box::new(prepared),
            handler,
            complete: false,
        })
    }
    /// Complete the cached declaration. No SDK, key-signing, native session or RNG is used.
    /// A failed or uncertain attempt owns this same request until retry or cold classification.
    pub fn retry(&mut self) -> Result<(), AppError> {
        self.handler.require()?;
        let store = self.prepared.current.checked_store()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        self.prepared.output.require(&key, &self.prepared.inputs)?;
        self.io.write(
            store,
            &self.prepared.inputs,
            &self.prepared.output.bytes(self.prepared.inputs.catalog()),
        )?;
        self.prepared.current.cached_store()?;
        self.complete = true;
        Ok(())
    }
    /// Owning refusal preserves the original transaction. No raw Store/Current can escape.
    pub fn into_native_registry(mut self) -> Result<NativeRegistryStore, Self> {
        if !self.complete || self.handler.require().is_err() {
            return Err(self);
        }
        if self
            .prepared
            .current
            .install_native_registry_handler(NativeRegistryHandler::Migration(self.handler.clone()))
            .is_err()
        {
            return Err(self);
        }
        if self
            .prepared
            .current
            .checked_store()
            .and_then(require_settled)
            .is_err()
        {
            return Err(self);
        }
        let Self {
            io,
            prepared,
            handler,
            complete: _,
        } = self;
        let PreparedNativeRegistryMigration {
            read: _,
            inputs,
            output,
            current,
            policy,
            fee,
        } = *prepared;
        drop(io);
        drop(inputs);
        drop(output);
        drop(policy);
        drop(fee);
        Ok(NativeRegistryStore {
            read: Default::default(),
            #[cfg(test)]
            read_hook: Default::default(),
            current,
            handler,
        })
    }
}

/// Actual opaque ID7 mount. Its public surface is deliberately DATA-only until native row
/// admission is connected; opening/decoding this registry grants no V2 signing or send right.
pub struct NativeRegistryStore {
    read: std::sync::Mutex<()>,
    #[cfg(test)]
    read_hook: std::sync::Mutex<Option<ReadHook>>,
    current: V2CurrentLayoutStore,
    handler: NativeMigrationHandler,
}
#[cfg(test)]
type ReadHook = Box<dyn FnOnce() + Send>;
impl std::fmt::Debug for NativeRegistryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeRegistryStore")
            .finish_non_exhaustive()
    }
}
impl NativeRegistryStore {
    /// Sole consuming internal transfer into the distinct version2 upgrade.
    /// No public Store/Current getter or handler switch is introduced.
    pub(in crate::store::critical_namespaces::native_registry) fn into_index_upgrade_current(
        self,
    ) -> Result<V2CurrentLayoutStore, AppError> {
        self.handler.require()?;
        require_settled(self.current.checked_store()?)?;
        Ok(self.current)
    }
    /// Regression access to the actual mounted owner for the real JOIN writer. This is
    /// absent from production and confers no native command/signing authorization.
    #[cfg(test)]
    pub(in crate::store) fn actual_store_for_test(
        &self,
    ) -> Result<&crate::store::ServerStore, AppError> {
        self.current.checked_store()
    }
    /// Reopen an already-declared registry, repairing only its exact initial declaration.
    /// This private policy cannot be recovered from DATA or passed to a public Current opener.
    pub fn open(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        mut current: V2CurrentLayoutPolicy,
        policy: NativeRegistryPolicy,
    ) -> Result<Self, AppError> {
        let physical_fee = policy.physical_payment(&current)?;
        let fee = Arc::new(policy.preparation.pool.try_reserve(PREPARATION_FEE)?);
        let handler = NativeMigrationHandler {
            policy: policy.preparation,
            fee,
            physical_fee,
            persistent_capacity: policy.persistent_capacity,
        };
        handler.require()?;
        current.native_registry = Some(NativeRegistryHandler::Migration(handler.clone()));
        let mut opening = PreparedV2CurrentLayoutStore::open(dir, passphrase, rng, current)?;
        opening.retry(rng)?;
        let current = opening.into_current_layout_store().map_err(|_| refused())?;
        require_settled(current.checked_store()?)?;
        Ok(Self {
            read: Default::default(),
            #[cfg(test)]
            read_hook: Default::default(),
            current,
            handler,
        })
    }
    /// Current verified row count, never a cached success after mount revocation or file change.
    pub fn row_count_data(&self) -> Result<usize, AppError> {
        let _read = self.read.try_lock().map_err(|_| refused())?;
        #[cfg(test)]
        let hook = self.read_hook.lock().map_err(|_| refused())?.take();
        #[cfg(test)]
        if let Some(hook) = hook {
            hook();
        }
        self.handler.require()?;
        require_settled(self.current.checked_store()?)
    }
}

impl MigrationOutput {
    fn bytes<'a>(&'a self, original: &'a [u8]) -> NativeDeclarationBytes<'a> {
        NativeDeclarationBytes {
            original,
            candidate: &self.candidate,
            seed: &self.seed,
            marker: &self.marker,
            index: &self.index,
        }
    }
}
fn parse(
    store: &crate::store::ServerStore,
    key: &[u8; 32],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = Zeroizing::new(unseal(key, &unframe(raw)?)?);
    catalog::parse_native_registry_layout(&plain, &store.chat_vault_domain()?, true, false)?;
    Ok(plain)
}
fn capability(parsed: &catalog::Parsed<'_>) -> Result<NativeCapabilityData, AppError> {
    let cap = NativeCapabilityData::for_current_data(
        parsed.current_layout.ok_or_else(refused)?,
        parsed.current_payload.ok_or_else(refused)?,
    )?;
    NativeCapabilityData::decode_exact(parsed.native_registry_payload.ok_or_else(refused)?, cap)
}
fn require_settled(store: &crate::store::ServerStore) -> Result<usize, AppError> {
    let inputs = NativeMigrationInputs::capture_recovery(store)?;
    let inventory = NativeDeclarationInventory::capture(store)?;
    if inventory.seed.is_some() || inventory.stage.is_some() {
        return Err(refused());
    }
    let key = Zeroizing::new(store.keys.db_key()?);
    let plain = parse(store, &key, inputs.catalog())?;
    let parsed =
        catalog::parse_native_registry_layout(&plain, &store.chat_vault_domain()?, true, false)?;
    let cap = capability(&parsed)?;
    let marker = inventory.marker.as_ref().ok_or_else(refused)?;
    NativeCapabilityData::decode_exact(
        &open_kind(&key, MARKER_KEY, marker.raw(), MAX_MARKER_PLAIN)?,
        cap,
    )?;
    let index = inventory.index.as_ref().ok_or_else(refused)?;
    let index = IndexData::decode_data(&open_kind(&key, INDEX_KEY, index.raw(), MAX_PLAIN)?, cap)?;
    inventory.require(store)?;
    inputs.require_selected(store)?;
    Ok(index.rows.len())
}

/// Decode a complete seed before any repair write, then validate its deterministic migration
/// mapping against the ACTUAL retained legacy input. Authentication alone does not prove G1.
fn recovered_output(
    key: &[u8; 32],
    seed: Zeroizing<Vec<u8>>,
    plain: &[u8],
    inputs: &NativeMigrationInputs,
    store: &crate::store::ServerStore,
) -> Result<(Zeroizing<Vec<u8>>, MigrationOutput), AppError> {
    let mut input = Decoder::new(plain);
    if input.get_bytes().map_err(|_| refused())? != SEED_TAG {
        return Err(refused());
    }
    let descriptor = input.get_bytes().map_err(|_| refused())?;
    let original = input.get_bytes().map_err(|_| refused())?;
    let candidate = input.get_bytes().map_err(|_| refused())?;
    let index = input.get_bytes().map_err(|_| refused())?;
    let marker = input.get_bytes().map_err(|_| refused())?;
    if original.len() > MAX_CATALOG_FRAME
        || candidate.len() > MAX_CATALOG_FRAME
        || index.len() > MAX_FRAME
        || marker.len() > MAX_MARKER_FRAME
    {
        return Err(refused());
    }
    match input.get_u8().map_err(|_| refused())? {
        0 if inputs.legacy().is_none() => {}
        1 => {
            let digest = input.get_bytes().map_err(|_| refused())?;
            if Some(digest)
                != inputs
                    .legacy()
                    .map(blake3::hash)
                    .as_ref()
                    .map(|value| value.as_bytes().as_slice())
            {
                return Err(refused());
            }
        }
        _ => return Err(refused()),
    }
    input.finish().map_err(|_| refused())?;
    let candidate_plain = parse(store, key, candidate)?;
    let parsed = catalog::parse_native_registry_layout(
        &candidate_plain,
        &store.chat_vault_domain()?,
        true,
        false,
    )?;
    let cap = capability(&parsed)?;
    NativeCapabilityData::decode_exact(descriptor, cap)?;
    let index_plain = open_kind(key, INDEX_KEY, index, MAX_PLAIN)?;
    let count = IndexData::decode_data(&index_plain, cap)?.rows.len();
    let output = MigrationOutput {
        capability: cap,
        candidate: Zeroizing::new(candidate.to_vec()),
        index: Zeroizing::new(index.to_vec()),
        marker: Zeroizing::new(marker.to_vec()),
        seed,
        count,
    };
    output.require_data(key, original, inputs.legacy())?;
    Ok((Zeroizing::new(original.to_vec()), output))
}

impl crate::store::ServerStore {
    /// Pure known-DATA classification for retiring only our unauthenticated claim beside
    /// predecessor governance progress. No native/governance mounted owner is issued.
    pub(in crate::store) fn native_governance_abandonment_current_data(
        &self,
        current: &[u8],
    ) -> Result<
        Option<
            super::super::super::governance_namespace_codec::capability::GovernanceCapabilityData,
        >,
        AppError,
    > {
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(current)?)?);
        let parsed =
            catalog::parse_native_registry_layout(&plain, &self.chat_vault_domain()?, true, true)?;
        if parsed.native_registry_payload.is_some() {
            return Err(refused());
        }
        Ok(parsed.governance)
    }
    /// First configured pre-cleaner gate. No unconfigured opener can enter this method.
    pub(in crate::store) fn inspect_and_repair_native_registry_declaration(
        &self,
    ) -> Result<(), AppError> {
        let handler = self
            .current_layout_policy
            .as_ref()
            .and_then(|value| value.native_registry.as_ref())
            .ok_or_else(refused)?;
        handler.require()?;
        // This presence probe reads no content through a possibly substituted root. The
        // actual root/directory custody is captured before opening any root artifact.
        if let NativeRegistryHandler::Operational(handler) = handler {
            return handler.inspect_storage(self);
        }
        if let NativeRegistryHandler::Lifecycle(handler) = handler {
            return handler.inspect_storage(self);
        }
        crate::store::persistence::native_registry::operational::refuse_upgrade_debt(self)?;
        let present = crate::store::persistence::native_registry::native_artifacts_present(self)?;
        let raw = read_native_current(self)?.ok_or_else(refused)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
        // Known4 is DATA-only here, for retiring OUR first-torn claim after predecessor
        // progress. Complete ID7 recovery below still refuses governance entirely.
        let before =
            catalog::parse_native_registry_layout(&plain, &self.chat_vault_domain()?, true, true)?;
        if !present && before.native_registry_payload.is_none() {
            return Ok(());
        }
        let governance_compat = before.governance.is_some()
            || crate::store::persistence::native_registry::native_governance_seed_present(self)?;
        let inputs = if governance_compat {
            NativeMigrationInputs::capture_governance_abandonment(self)?
        } else {
            NativeMigrationInputs::capture_recovery(self)?
        };
        if inputs.catalog() != raw.as_slice() {
            return Err(refused());
        }
        let inventory = NativeDeclarationInventory::capture(self)?;
        if governance_compat {
            let seed = inventory.seed.as_ref().ok_or_else(refused)?;
            if before.native_registry_payload.is_some()
                || inventory.index.is_some()
                || inventory.marker.is_some()
                || authenticate_seed(&key, seed.raw()).is_ok()
            {
                return Err(refused());
            }
            // Stage may belong to governance/JOIN, including partial bytes. Its ownership
            // is explicitly Untouched in this negative compatibility role.
            return dispose_native_first_torn(self, &inputs, inventory);
        }
        let Some(seed) = inventory.seed.as_ref() else {
            // Before ordinary JOIN settlement both slots must retain the same descriptor.
            let cap = capability(&before)?;
            if let Some(stage) = &inventory.stage {
                let next_plain = parse(self, &key, stage.raw())?;
                let next = catalog::parse_native_registry_layout(
                    &next_plain,
                    &self.chat_vault_domain()?,
                    true,
                    false,
                )?;
                if capability(&next)? != cap {
                    return Err(refused());
                }
            }
            let marker = inventory.marker.as_ref().ok_or_else(refused)?;
            NativeCapabilityData::decode_exact(
                &open_kind(&key, MARKER_KEY, marker.raw(), MAX_MARKER_PLAIN)?,
                cap,
            )?;
            let index = inventory.index.as_ref().ok_or_else(refused)?;
            IndexData::decode_data(&open_kind(&key, INDEX_KEY, index.raw(), MAX_PLAIN)?, cap)?;
            inventory.require(self)?;
            return inputs.require_selected(self);
        };
        // A seed that authenticates but fails the closed grammar is NEVER disposable.
        // Only unauthenticated first-seed scratch with no ID7/output has abandonment custody.
        let seed_plain = match authenticate_seed(&key, seed.raw()) {
            Ok(plain) => plain,
            Err(_) => {
                if before.native_registry_payload.is_some() {
                    return Err(refused());
                }
                if let Some(stage) = &inventory.stage {
                    if !stage.raw().is_empty() {
                        let next_plain = parse(self, &key, stage.raw())?;
                        let next = catalog::parse_native_registry_layout(
                            &next_plain,
                            &self.chat_vault_domain()?,
                            true,
                            false,
                        )?;
                        catalog::successor(
                            &before.summary(*blake3::hash(&raw).as_bytes()),
                            &next.summary(*blake3::hash(stage.raw()).as_bytes()),
                        )?;
                    }
                }
                #[cfg(test)]
                if let Some(hook) = AFTER_CLASSIFICATION.with(|hook| hook.borrow_mut().take()) {
                    hook(&self.dir);
                }
                return dispose_native_first_torn(self, &inputs, inventory);
            }
        };
        let (original, output) = recovered_output(
            &key,
            Zeroizing::new(seed.raw().to_vec()),
            &seed_plain,
            &inputs,
            self,
        )?;
        let bytes = output.bytes(&original);
        let mut io =
            NativeDeclarationIo::capture_recovery(self, &bytes, inputs.catalog(), &inventory)?;
        // No classification handle may keep Windows cleanup delete-pending.
        drop(inventory);
        io.write(self, &inputs, &bytes)
    }
}

/// AEAD success and grammar success are separate facts. In particular, authenticated EMPTY
/// plaintext is malformed/future DATA to preserve, never an unauthenticated first-torn claim.
fn authenticate_seed(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() > MAX_SEED_FRAME {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
    Ok(Zeroizing::new(unseal(&derived, &unframe(raw)?)?))
}

#[cfg(test)]
mod tests;
