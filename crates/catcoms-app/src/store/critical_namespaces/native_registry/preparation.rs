//! Original non-writing migration preparation. The separate `installation` API owns
//! physical namespace7 declaration/recovery; this preparation alone has no write operation.
use super::super::{
    catalog_declaration, current_layout, read_pool::CatalogReadReservation, refused,
};
use super::{capability::*, data::*};
use crate::store::{
    frame, persistence::native_registry::NativeMigrationInputs, unframe, CriticalCatalogReadPool,
    PreparedV2CurrentLayoutStore, ServerRecord, V2CurrentLayoutPolicy, V2CurrentLayoutStore,
};
use crate::AppError;
use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};
use std::path::Path;
use std::sync::Arc;
use zeroize::Zeroizing;

pub(in crate::store) const MAX_BASE_PATH: usize = current_layout::MAX_OPEN_BASE_PATH;
pub(in crate::store) const SEED_NAME: &str = "chat-critical-namespaces.native-registry.seed";
pub(in crate::store) const MARKER_NAME: &str = "native-registry-layout.bin";
pub(in crate::store) const INDEX_NAME: &str = "native-registry.bin";
pub(in crate::store) const INDEX_STAGE_NAME: &str = "native-registry.stage";
pub(in crate::store) const MAX_CATALOG_FRAME: usize =
    super::super::pending_join_codec::MAX_CATALOG_FRAMED;
pub(in crate::store) const MAX_LEGACY_FRAME: usize = MAX_PLAIN + 40;
pub(in crate::store) const MAX_INDEX_FRAME: usize = MAX_FRAME;
pub(in crate::store) const MAX_MARKER_PLAIN: usize = 1024;
pub(in crate::store) const MAX_MARKER_FRAME: usize = MAX_MARKER_PLAIN + 40;
pub(in crate::store) const MAX_SEED_PLAIN: usize =
    2 * MAX_CATALOG_FRAME + MAX_FRAME + MAX_MARKER_FRAME + 1024;
pub(in crate::store) const MAX_SEED_FRAME: usize = MAX_SEED_PLAIN + 40;
const SEED_TAG: &[u8] = b"catcoms/private-native-backend-declaration/v1\0";
pub(in crate::store::critical_namespaces::native_registry) const INDEX_KEY: &str =
    "catcoms/private-native-backend-index-key/v1";
pub(in crate::store::critical_namespaces::native_registry) const MARKER_KEY: &str =
    "catcoms/private-native-backend-marker-key/v1";
const SEED_KEY: &str = "catcoms/private-native-backend-declaration-key/v1";
mod installation;
pub use installation::{NativeRegistryInstallation, NativeRegistryPolicy, NativeRegistryStore};

/// Private handler plus its original prepaid lifetime. No public Current policy can
/// construct this switch, and ordinary openers cannot opt in by decoding an ID7.
#[derive(Clone)]
pub(in crate::store) enum NativeRegistryHandler {
    Migration(NativeMigrationHandler),
    Operational(super::operational::Handler),
    Lifecycle(super::lifecycle::admission::Handler),
}
impl NativeRegistryHandler {
    pub(in crate::store) fn require(&self) -> Result<(), AppError> {
        match self {
            Self::Migration(handler) => handler.require(),
            Self::Operational(handler) => handler.require(),
            Self::Lifecycle(handler) => handler.require(),
        }
    }
    pub(in crate::store) fn namespace_version(&self) -> u32 {
        match self {
            Self::Migration(_) => 1,
            Self::Operational(_) => 2,
            Self::Lifecycle(_) => 3,
        }
    }
    pub(in crate::store::critical_namespaces) fn parse_layout<'a>(
        &self,
        plain: &'a [u8],
        domain: &[u8; 32],
        executing: bool,
    ) -> Result<super::super::pending_join_codec::Parsed<'a>, AppError> {
        match self {
            Self::Migration(_) => super::super::pending_join_codec::parse_native_registry_layout(
                plain, domain, executing, false,
            ),
            Self::Operational(_) => {
                super::super::pending_join_codec::parse_operational_native_registry_layout(
                    plain, domain, executing,
                )
            }
            Self::Lifecycle(_) => {
                super::super::pending_join_codec::parse_lifecycle_native_registry_layout(
                    plain, domain, executing,
                )
            }
        }
    }
}
#[derive(Clone)]
pub(in crate::store) struct NativeMigrationHandler {
    policy: NativeRegistryPreparationPolicy,
    fee: Arc<CatalogReadReservation>,
    physical_fee: Arc<CatalogReadReservation>,
    persistent_capacity: usize,
}

// Named conservative LOGICAL inventory, not RSS. All maxima are independent of
// untrusted lengths. Actual buffers use their bounded input lengths, not these maxima.
// The final retained object holds two catalog frames, legacy input, initial index,
// marker and the complete seed. During construction/revalidation we additionally
// cover seed encoder/AEAD/frame/unframe/plain copies, complete catalog pair opens,
// legacy unframing/plain + decoded/cloned metadata, index encode/open/clone and
// two sets of row/string metadata and uniqueness scratch. No fee comes from ID5.
const RETAINED_BYTES: usize =
    2 * MAX_CATALOG_FRAME + MAX_LEGACY_FRAME + MAX_FRAME + MAX_MARKER_FRAME + MAX_SEED_FRAME;
const TRANSIENT_BYTES: usize = 6 * MAX_SEED_FRAME
    + 8 * MAX_CATALOG_FRAME
    + 8 * MAX_FRAME
    + 6 * MAX_LEGACY_FRAME
    + 8 * MAX_MARKER_FRAME;
const METADATA_BYTES: usize =
    4 * MAX_ROWS * (std::mem::size_of::<RowData>() + std::mem::size_of::<ServerRecord>() + 32 + 8)
        + 12 * (MAX_BASE_PATH + 256)
        + 256 * 1024;
pub(in crate::store) const PREPARATION_FEE: usize =
    RETAINED_BYTES + TRANSIENT_BYTES + METADATA_BYTES;

#[cfg(test)]
thread_local! {
    // A one-shot cut against the ACTUAL newly opened mount after all sealing
    // and temporary destruction. It manufactures no positive owner or proof.
    static CANCEL_BEFORE_RETURN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Independent trusted account for the non-writing migration preparation only.
/// This is NOT a namespace7 handler switch or permission to publish native rows.
#[derive(Clone, Debug)]
pub struct NativeRegistryPreparationPolicy {
    pool: CriticalCatalogReadPool,
}
impl NativeRegistryPreparationPolicy {
    pub fn new(pool: CriticalCatalogReadPool) -> Result<Self, AppError> {
        if pool.capacity() < PREPARATION_FEE {
            return Err(refused());
        }
        Ok(Self { pool })
    }
    /// Required logical capacity, excluding the existing Current/JOIN open phase,
    /// SDK, KDF and any future physical writer or operational native row owner.
    pub const fn required_capacity() -> usize {
        PREPARATION_FEE
    }
    fn require_independent(&self, current: &V2CurrentLayoutPolicy) -> Result<(), AppError> {
        let member = current.member_outbox.as_ref().ok_or_else(refused)?;
        if self.pool.capacity() < PREPARATION_FEE
            || self.pool.same_account(&current.archive.pending.pool)
            || self.pool.same_account(&member.pool)
        {
            return Err(refused());
        }
        Ok(())
    }
}

/// One actual newly opened, private mount and its exact cached migration outputs.
/// No Store/Host/key/row getter, writer, generic callback or decoding-based authority.
/// Files and ciphertexts drain before the original mount and its final payment.
pub struct PreparedNativeRegistryMigration {
    read: std::sync::Mutex<()>,
    inputs: NativeMigrationInputs,
    output: MigrationOutput,
    current: V2CurrentLayoutStore,
    policy: NativeRegistryPreparationPolicy,
    fee: Arc<CatalogReadReservation>,
}
impl std::fmt::Debug for PreparedNativeRegistryMigration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedNativeRegistryMigration")
            .finish_non_exhaustive()
    }
}
struct MigrationOutput {
    capability: NativeCapabilityData,
    candidate: Zeroizing<Vec<u8>>,
    index: Zeroizing<Vec<u8>>,
    marker: Zeroizing<Vec<u8>>,
    seed: Zeroizing<Vec<u8>>,
    count: usize,
}
impl PreparedNativeRegistryMigration {
    /// Open a NEW private Current3/member5 owner, finish its existing declaration
    /// protocol, then prepare native registry bytes without writing any of them.
    /// Existing Current/JOIN openers may repair/declare their own formats. Native
    /// preparation fails without a mounted member outbox or while ANY JOIN row
    /// (including paused work) exists. Nothing may use this as a freeze grant.
    pub fn open(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        current_policy: V2CurrentLayoutPolicy,
        policy: NativeRegistryPreparationPolicy,
    ) -> Result<Self, AppError> {
        policy.require_independent(&current_policy)?;
        // Failure here must precede even prerequisite opening, KDF and RNG.
        let fee = Arc::new(policy.pool.try_reserve(PREPARATION_FEE)?);
        crate::store::persistence::native_registry::refuse_unsupported_storage(dir.as_ref())?;
        let mut opening = PreparedV2CurrentLayoutStore::open(dir, passphrase, rng, current_policy)?;
        opening.retry(rng)?;
        let current = opening.into_current_layout_store().map_err(|_| refused())?;
        let store = current.checked_store()?;
        store.validate_mounted_member_outbox_layout()?;
        let inputs = NativeMigrationInputs::capture(store, &fee, &policy.pool)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let old = Zeroizing::new(unseal(&key, &unframe(inputs.catalog())?)?);
        let parsed = store.parse_configured_join_catalog(&old)?;
        if parsed.count != 0 || parsed.member_outbox.is_none() {
            return Err(refused());
        }
        {
            let catalog = store.pending_join_catalog.lock().map_err(|_| refused())?;
            let catalog = catalog.as_ref().ok_or_else(refused)?;
            if catalog.pending.is_some()
                || catalog.current.framed != *blake3::hash(inputs.catalog()).as_bytes()
            {
                return Err(refused());
            }
        }
        let layout = parsed.current_layout.ok_or_else(refused)?;
        let capability = NativeCapabilityData::for_current_data(
            layout,
            parsed.current_payload.ok_or_else(refused)?,
        )?;
        let records = match inputs.legacy() {
            None => Vec::new(),
            Some(raw) => {
                let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
                decode_bounded_legacy(&plain)?
            }
        };
        let index_data = IndexData::migrate_legacy_data(
            capability,
            inputs.legacy().map(|raw| *blake3::hash(raw).as_bytes()),
            &records,
        )?;
        let count = index_data.rows.len();
        let index = seal_kind(&key, INDEX_KEY, &index_data.encode_data()?, MAX_PLAIN, rng)?;
        let descriptor = capability.encode_data()?;
        let next = catalog_declaration::insert_entry_data(
            &old,
            &layout.vault,
            NAMESPACE_ID,
            NAMESPACE_VERSION,
            &descriptor,
        )?;
        let candidate = Zeroizing::new(frame(&seal(&key, &next, rng)?));
        let marker = seal_kind(&key, MARKER_KEY, &descriptor, MAX_MARKER_PLAIN, rng)?;
        let seed_plain = seed_plain(
            capability,
            inputs.catalog(),
            &candidate,
            &index,
            &marker,
            inputs.legacy(),
        )?;
        let seed = seal_kind(&key, SEED_KEY, &seed_plain, MAX_SEED_PLAIN, rng)?;
        let output = MigrationOutput {
            capability,
            candidate,
            index,
            marker,
            seed,
            count,
        };
        output.require(&key, &inputs)?;
        inputs.require(store, &fee, &policy.pool)?;
        store.validate_mounted_member_outbox_layout()?;
        current.checked_store()?;
        // All temporary records/parsed plaintext/key buffers drop before final
        // negative. No caller can access this private Store while constructing.
        drop(records);
        drop(index_data);
        drop(seed_plain);
        drop(next);
        drop(descriptor);
        drop(old);
        drop(key);
        #[cfg(test)]
        if CANCEL_BEFORE_RETURN.with(|cut| cut.replace(false)) {
            current.cached_store()?.chat_mount.revoke();
        }
        current.cached_store()?;
        Ok(Self {
            read: Default::default(),
            inputs,
            output,
            current,
            policy,
            fee,
        })
    }

    /// Historical DATA only. This count does not mean rows were installed.
    pub fn legacy_row_count_data(&self) -> usize {
        self.output.count
    }

    /// Re-prove the SAME original file identities/bytes and live mount without
    /// drawing randomness, rebinding a replacement or changing the candidate.
    /// A later writer must additionally own its real exclusion/commit protocol.
    pub fn revalidate_inputs(&self) -> Result<(), AppError> {
        // The retained invoice covers ONE complete validation inventory. Shared callers
        // cannot allocate another copy while an earlier read is still using that payment.
        let _read = self.read.try_lock().map_err(|_| refused())?;
        self.fee.validate(&self.policy.pool, PREPARATION_FEE)?;
        let store = self.current.checked_store()?;
        store.validate_mounted_member_outbox_layout()?;
        self.inputs.require(store, &self.fee, &self.policy.pool)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        self.output.require(&key, &self.inputs)?;
        drop(key);
        self.inputs.require(store, &self.fee, &self.policy.pool)?;
        self.current.cached_store()?;
        Ok(())
    }
}

/// Strict bounded compatibility decoder, BEFORE metadata copies. The legacy
/// v1/v2 codec accepted every nonzero DM flag; preserve that historical meaning.
/// Do not change the legacy reader or infer a v3 from a trailing byte block.
fn decode_bounded_legacy(raw: &[u8]) -> Result<Vec<ServerRecord>, AppError> {
    if raw.len() > MAX_PLAIN {
        return Err(refused());
    }
    let mut input = Decoder::new(raw);
    let count = input.get_u32().map_err(|_| refused())? as usize;
    if count > MAX_ROWS || count > input.remaining() / 16 {
        return Err(refused());
    }
    let mut ids = Vec::with_capacity(count);
    let mut native_length = GENERATION1_HEADER_BYTES;
    for _ in 0..count {
        ids.push(input.get_u64().map_err(|_| refused())?);
        let name = input.get_str().map_err(|_| refused())?;
        let invite = input.get_str().map_err(|_| refused())?;
        if name.len() > MAX_NAME || invite.len() > MAX_INVITE {
            return Err(refused());
        }
        native_length = native_length
            .checked_add(1 + 8 + 36 + 4 + name.len() + 1 + 4)
            .and_then(|n| n.checked_add(invite.len()))
            .filter(|n| *n <= MAX_PLAIN)
            .ok_or_else(refused)?;
    }
    if input.remaining() != 0 && input.remaining() != count {
        return Err(refused());
    }
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(refused());
    }
    drop(ids);
    // The existing decoder only copies after the complete bounded validation.
    super::super::super::decode_registry(raw)
}

fn seal_kind(
    key: &[u8; 32],
    domain: &str,
    plain: &[u8],
    cap: usize,
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if plain.is_empty() || plain.len() > cap {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&derived, plain, rng)?)))
}
pub(in crate::store::critical_namespaces::native_registry) fn open_kind(
    key: &[u8; 32],
    domain: &str,
    raw: &[u8],
    cap: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() <= 40 || raw.len() > cap + 40 {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    let result = Zeroizing::new(unseal(&derived, &unframe(raw)?)?);
    if result.is_empty() || result.len() > cap {
        return Err(refused());
    }
    Ok(result)
}
fn seed_plain(
    cap: NativeCapabilityData,
    original: &[u8],
    candidate: &[u8],
    index: &[u8],
    marker: &[u8],
    legacy: Option<&[u8]>,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut out =
        Encoder::with_capacity(512 + original.len() + candidate.len() + index.len() + marker.len());
    for field in [
        SEED_TAG,
        &cap.encode_data()?,
        original,
        candidate,
        index,
        marker,
    ] {
        out.put_bytes(field).map_err(|_| refused())?;
    }
    out.put_u8(u8::from(legacy.is_some()));
    if let Some(legacy) = legacy {
        out.put_bytes(blake3::hash(legacy).as_bytes())
            .map_err(|_| refused())?;
    }
    if out.len() > MAX_SEED_PLAIN {
        return Err(refused());
    }
    Ok(Zeroizing::new(out.finish()))
}
impl MigrationOutput {
    fn require(&self, key: &[u8; 32], input: &NativeMigrationInputs) -> Result<(), AppError> {
        self.require_data(key, input.catalog(), input.legacy())
    }
    fn require_data(
        &self,
        key: &[u8; 32],
        original_raw: &[u8],
        legacy_raw: Option<&[u8]>,
    ) -> Result<(), AppError> {
        let marker = open_kind(key, MARKER_KEY, &self.marker, MAX_MARKER_PLAIN)?;
        NativeCapabilityData::decode_exact(&marker, self.capability)?;
        let index = open_kind(key, INDEX_KEY, &self.index, MAX_PLAIN)?;
        let index = IndexData::decode_data(&index, self.capability)?;
        if index.generation != 1
            || index.parent.is_some()
            || index.rows.len() != self.count
            || index
                .rows
                .iter()
                .any(|row| !matches!(row.backend, BackendData::Legacy { .. }))
        {
            return Err(refused());
        }
        // Authentication of both files does not prove the initial rows came
        // from THIS exact migration input. Recompute its deterministic DATA
        // mapping so an authenticated inconsistent seed cannot install G1.
        let records = match legacy_raw {
            None => Vec::new(),
            Some(raw) => decode_bounded_legacy(&Zeroizing::new(unseal(key, &unframe(raw)?)?))?,
        };
        let expected_index = IndexData::migrate_legacy_data(
            self.capability,
            legacy_raw.map(|raw| *blake3::hash(raw).as_bytes()),
            &records,
        )?;
        if index != expected_index {
            return Err(refused());
        }
        let original = Zeroizing::new(unseal(key, &unframe(original_raw)?)?);
        let parent = super::super::pending_join_codec::parse_configured_current_layout(
            &original,
            &self.capability.vault,
            true,
            true,
        )?;
        if parent.count != 0 || parent.member_outbox.is_none() {
            return Err(refused());
        }
        let candidate = Zeroizing::new(unseal(key, &unframe(&self.candidate)?)?);
        let descriptor = catalog_declaration::validate_insert_pair_data(
            &original,
            &candidate,
            &self.capability.vault,
            NAMESPACE_ID,
            NAMESPACE_VERSION,
        )?;
        NativeCapabilityData::decode_exact(descriptor, self.capability)?;
        let actual = open_kind(key, SEED_KEY, &self.seed, MAX_SEED_PLAIN)?;
        let expected = seed_plain(
            self.capability,
            original_raw,
            &self.candidate,
            &self.index,
            &self.marker,
            legacy_raw,
        )?;
        if actual != expected {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
pub(in crate::store::critical_namespaces::native_registry) mod tests;
