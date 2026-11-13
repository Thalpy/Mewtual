//! Owning v1-G1 -> v2 physical upgrade. This consumes the opaque old DATA owner,
//! not a caller-asserted capability, Store reference, ready flag or library Host.
use super::super::super::pending_join_codec as grammar;
use super::super::{capability::NativeCapabilityData, data as old, preparation as previous};
use super::*;
use crate::store::persistence::native_registry::operational as io;
use crate::store::{
    frame, unframe, NativeRegistryStore, V2CurrentLayoutPolicy, V2CurrentLayoutStore,
};
use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(super) const INDEX_KEY: &str = "catcoms/private-native-backend-index-key/v2";
pub(super) const MARKER_KEY: &str = "catcoms/private-native-backend-marker-key/v2";
pub(super) const SEED_KEY: &str = "catcoms/private-native-backend-upgrade-key/v2";
const SEED_TAG: &[u8] = b"catcoms/private-native-backend-upgrade/v2\0";
const SEED_PLAIN: usize = UPGRADE_SEED_FRAME - 40;
const PERSISTENT_MINIMUM: usize =
    2 * CATALOG_FRAME + 2 * data::MAX_FRAME + 2 * previous::MAX_MARKER_FRAME + UPGRADE_SEED_FRAME;
const _: () = assert!(UPGRADE_SEED_FRAME == io::MAX_UPGRADE_SEED_FRAME);

/// Independent logical quota for the complete upgrade/index overlap. Persistent
/// capacity is an admission ceiling, not a promise of filesystem free space.
#[derive(Clone, Debug)]
pub struct NativeIndexPolicy {
    pub(super) pool: CriticalCatalogReadPool,
    persistent_capacity: usize,
}
impl NativeIndexPolicy {
    pub fn new(
        pool: CriticalCatalogReadPool,
        persistent_capacity: usize,
    ) -> Result<Self, AppError> {
        if pool.capacity() < OPERATIONAL_FEE || persistent_capacity < PERSISTENT_MINIMUM {
            return Err(refused());
        }
        Ok(Self {
            pool,
            persistent_capacity,
        })
    }
    pub const fn required_capacity() -> usize {
        OPERATIONAL_FEE
    }
    pub const fn required_persistent_capacity() -> usize {
        PERSISTENT_MINIMUM
    }
    pub(super) fn handler(&self, current: &V2CurrentLayoutPolicy) -> Result<Handler, AppError> {
        if self.persistent_capacity < PERSISTENT_MINIMUM
            || current
                .member_outbox
                .as_ref()
                .is_none_or(|member| self.pool.same_account(&member.pool))
            || self.pool.same_account(&current.archive.pending.pool)
        {
            return Err(refused());
        }
        let fee = Arc::new(payment::IndexPayment::new(
            self.pool.try_reserve(OPERATIONAL_FEE)?,
        ));
        Ok(Handler {
            pool: self.pool.clone(),
            gate: Arc::new(interlock::IndexInterlock::new(fee.clone())),
            fee,
        })
    }
}

pub(super) fn authenticate(
    key: &[u8; 32],
    domain: &str,
    raw: &[u8],
    max_frame: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(40..=max_frame).contains(&raw.len()) {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    // Successful empty plaintext is STILL authentication. The caller must keep
    // it distinct from failure before deciding whether scratch is disposable.
    Ok(Zeroizing::new(unseal(&derived, &unframe(raw)?)?))
}
fn sealed(
    key: &[u8; 32],
    domain: &str,
    plain: &[u8],
    max_frame: usize,
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if plain.is_empty()
        || plain
            .len()
            .checked_add(40)
            .is_none_or(|len| len > max_frame)
    {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&derived, plain, rng)?)))
}
pub(super) fn catalog_plain(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() > CATALOG_FRAME {
        return Err(refused());
    }
    let plain = Zeroizing::new(unseal(key, &unframe(raw)?)?);
    // The actual caller chooses v1 or v2 grammar after unsealing; an authenticated
    // future grammar is preserved/refused rather than classified as torn scratch.
    Ok(plain)
}
pub(super) fn capability_data(
    plain: &[u8],
    domain: &[u8; 32],
) -> Result<capability::Capability, AppError> {
    let parsed = grammar::parse_operational_native_registry_layout(plain, domain, true)?;
    let cap = capability::Capability::for_current_data(
        parsed.current_layout.ok_or_else(refused)?,
        parsed.current_payload.ok_or_else(refused)?,
    )?;
    capability::Capability::decode_exact(parsed.native_registry_payload.ok_or_else(refused)?, cap)
}

pub(super) struct UpgradeOutput {
    pub(super) capability: capability::Capability,
    original: Zeroizing<Vec<u8>>,
    catalog: Zeroizing<Vec<u8>>,
    old_index: Zeroizing<Vec<u8>>,
    index: Zeroizing<Vec<u8>>,
    old_marker: Zeroizing<Vec<u8>>,
    marker: Zeroizing<Vec<u8>>,
    seed: Zeroizing<Vec<u8>>,
    // MUST remain last: the observation follows real cached buffer destruction.
    #[cfg(test)]
    retirement: Option<io::RetirementObservation>,
}
impl UpgradeOutput {
    pub(super) fn bytes(&self) -> io::UpgradeBytes<'_> {
        io::UpgradeBytes {
            original: &self.original,
            catalog: &self.catalog,
            old_index: &self.old_index,
            index: &self.index,
            old_marker: &self.old_marker,
            marker: &self.marker,
            seed: &self.seed,
        }
    }
    fn seed_plain(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let bytes = self.bytes();
        let descriptor = self.capability.encode_data()?;
        let fields = [
            SEED_TAG,
            descriptor.as_slice(),
            bytes.original,
            bytes.catalog,
            bytes.old_index,
            bytes.index,
            bytes.old_marker,
            bytes.marker,
        ];
        let length = fields
            .iter()
            .try_fold(0_usize, |length, field| {
                length
                    .checked_add(4)
                    .and_then(|n| n.checked_add(field.len()))
            })
            .ok_or_else(refused)?;
        if length > SEED_PLAIN {
            return Err(refused());
        }
        let mut out = Encoder::with_capacity(length);
        for field in fields {
            out.put_bytes(field).map_err(|_| refused())?;
        }
        Ok(Zeroizing::new(out.finish()))
    }
    pub(super) fn decode_seed(
        store: &crate::store::ServerStore,
        key: &[u8; 32],
        raw: &[u8],
        plain: &[u8],
    ) -> Result<Self, AppError> {
        if plain.len() > SEED_PLAIN {
            return Err(refused());
        }
        let mut input = Decoder::new(plain);
        if input.get_bytes().map_err(|_| refused())? != SEED_TAG {
            return Err(refused());
        }
        let descriptor = input.get_bytes().map_err(|_| refused())?;
        if descriptor.len() > previous::MAX_MARKER_PLAIN {
            return Err(refused());
        }
        // All borrowed fields are bounded before allocating any owned frame copy.
        let mut fields = [&[][..]; 6];
        for (field, limit) in fields.iter_mut().zip([
            CATALOG_FRAME,
            CATALOG_FRAME,
            data::MAX_FRAME,
            data::MAX_FRAME,
            previous::MAX_MARKER_FRAME,
            previous::MAX_MARKER_FRAME,
        ]) {
            *field = input.get_bytes().map_err(|_| refused())?;
            if !(40..=limit).contains(&field.len()) {
                return Err(refused());
            }
        }
        input.finish().map_err(|_| refused())?;
        let old_plain = catalog_plain(key, fields[0])?;
        let old = grammar::parse_native_registry_layout(
            &old_plain,
            &store.chat_vault_domain()?,
            true,
            false,
        )?;
        if old.count != 0 {
            return Err(refused());
        }
        let original = NativeCapabilityData::for_current_data(
            old.current_layout.ok_or_else(refused)?,
            old.current_payload.ok_or_else(refused)?,
        )?;
        NativeCapabilityData::decode_exact(
            old.native_registry_payload.ok_or_else(refused)?,
            original,
        )?;
        let capability = capability::Capability { original };
        capability::Capability::decode_exact(descriptor, capability)?;
        let value = Self {
            capability,
            original: Zeroizing::new(fields[0].to_vec()),
            catalog: Zeroizing::new(fields[1].to_vec()),
            old_index: Zeroizing::new(fields[2].to_vec()),
            index: Zeroizing::new(fields[3].to_vec()),
            old_marker: Zeroizing::new(fields[4].to_vec()),
            marker: Zeroizing::new(fields[5].to_vec()),
            seed: Zeroizing::new(raw.to_vec()),
            #[cfg(test)]
            retirement: None,
        };
        value.validate(store, key)?;
        Ok(value)
    }
    pub(super) fn validate(
        &self,
        store: &crate::store::ServerStore,
        key: &[u8; 32],
    ) -> Result<(), AppError> {
        let old_plain = catalog_plain(key, &self.original)?;
        let next_plain = catalog_plain(key, &self.catalog)?;
        if catalog::upgrade_data(
            &old_plain,
            &store.chat_vault_domain()?,
            self.capability.original,
        )? != next_plain
            || capability_data(&next_plain, &store.chat_vault_domain()?)? != self.capability
        {
            return Err(refused());
        }
        let marker = previous::open_kind(
            key,
            previous::MARKER_KEY,
            &self.old_marker,
            previous::MAX_MARKER_PLAIN,
        )?;
        NativeCapabilityData::decode_exact(&marker, self.capability.original)?;
        let old_index = old::IndexData::decode_data(
            &previous::open_kind(key, previous::INDEX_KEY, &self.old_index, old::MAX_PLAIN)?,
            self.capability.original,
        )?;
        let expected = data::Index::migrate_v1_data(old_index)?.encode_data()?;
        let index = authenticate(key, INDEX_KEY, &self.index, data::MAX_FRAME)?;
        if expected != index {
            return Err(refused());
        }
        data::Index::decode_data(&index, self.capability)?;
        capability::Capability::decode_exact(
            &authenticate(key, MARKER_KEY, &self.marker, previous::MAX_MARKER_FRAME)?,
            self.capability,
        )?;
        Ok(())
    }
}

/// Startup-private physical migration; retry owns exactly the same cached
/// ciphertexts and continuously held files. No RNG/SDK is used by retry.
pub struct NativeIndexUpgrade {
    admission: interlock::AdmissionOwner,
    io: Box<io::UpgradeIo>,
    output: Box<UpgradeOutput>,
    current: Box<V2CurrentLayoutStore>,
    // Field order is custody: all files/frames drain before the mount, then
    // the v2 invoice. No v2 handler exists in the old Store before installation.
    pub(super) handler: Handler,
    complete: bool,
}
impl std::fmt::Debug for NativeIndexUpgrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeIndexUpgrade")
            .field("complete", &self.complete)
            .finish_non_exhaustive()
    }
}
impl NativeIndexUpgrade {
    /// Consume a genuine opaque v1 owner. Preparation changes no files; an
    /// unsuccessful preparation drops that private mount and can be reopened.
    /// This is storage migration, never a creator/SDK/native-session grant.
    pub fn prepare(
        previous: NativeRegistryStore,
        policy: NativeIndexPolicy,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        let current = previous.into_index_upgrade_current()?;
        let store = current.checked_store()?;
        let handler = policy.handler(store.current_layout_policy.as_ref().ok_or_else(refused)?)?;
        let inventory = io::UpgradeInventory::capture(store)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let original_plain = catalog_plain(&key, inventory.catalog())?;
        let parsed = grammar::parse_native_registry_layout(
            &original_plain,
            &store.chat_vault_domain()?,
            true,
            false,
        )?;
        if parsed.count != 0 {
            return Err(refused());
        }
        let original = NativeCapabilityData::for_current_data(
            parsed.current_layout.ok_or_else(refused)?,
            parsed.current_payload.ok_or_else(refused)?,
        )?;
        NativeCapabilityData::decode_exact(
            parsed.native_registry_payload.ok_or_else(refused)?,
            original,
        )?;
        NativeCapabilityData::decode_exact(
            &previous::open_kind(
                &key,
                previous::MARKER_KEY,
                inventory.marker(),
                previous::MAX_MARKER_PLAIN,
            )?,
            original,
        )?;
        let old = old::IndexData::decode_data(
            &previous::open_kind(&key, previous::INDEX_KEY, inventory.index(), old::MAX_PLAIN)?,
            original,
        )?;
        let index = data::Index::migrate_v1_data(old)?;
        let capability = capability::Capability { original };
        let candidate =
            catalog::upgrade_data(&original_plain, &store.chat_vault_domain()?, original)?;
        let mut output = UpgradeOutput {
            capability,
            original: Zeroizing::new(inventory.catalog().to_vec()),
            catalog: Zeroizing::new(frame(&seal(&key, &candidate, rng)?)),
            old_index: Zeroizing::new(inventory.index().to_vec()),
            index: sealed(&key, INDEX_KEY, &index.encode_data()?, data::MAX_FRAME, rng)?,
            old_marker: Zeroizing::new(inventory.marker().to_vec()),
            marker: sealed(
                &key,
                MARKER_KEY,
                &capability.encode_data()?,
                previous::MAX_MARKER_FRAME,
                rng,
            )?,
            seed: Zeroizing::new(Vec::new()),
            #[cfg(test)]
            retirement: None,
        };
        output.seed = sealed(
            &key,
            SEED_KEY,
            &output.seed_plain()?,
            UPGRADE_SEED_FRAME,
            rng,
        )?;
        output.validate(store, &key)?;
        let io = io::UpgradeIo::prepare(store, inventory, &output.bytes())?;
        current.checked_store()?;
        Ok(Self {
            admission: interlock::AdmissionOwner::new(handler.gate.clone()),
            current: Box::new(current),
            handler,
            output: Box::new(output),
            io: Box::new(io),
            complete: false,
        })
    }
    pub fn retry(&mut self) -> Result<(), AppError> {
        self.handler.require()?;
        let store = self.current.checked_store()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        self.output.validate(store, &key)?;
        self.io.advance_upgrade(store, &self.output.bytes())?;
        self.complete = true;
        Ok(())
    }
    /// Expose only the opaque v2 DATA owner after all original file/parent
    /// barriers and cleanup. No raw Store/Current/backend getter is supplied.
    pub fn into_index_store(mut self) -> Result<NativeIndexStore, Self> {
        if !self.complete || self.retry().is_err() {
            return Err(self);
        }
        if self
            .current
            .install_native_registry_handler(previous::NativeRegistryHandler::Operational(
                self.handler.clone(),
            ))
            .is_err()
        {
            return Err(self);
        }
        // Installing the private grammar rereads the catalog. Retain the old
        // exact File custody through a final full tuple proof; equal replacement
        // bytes during that read must not be adopted by this original owner.
        if self.retry().is_err() {
            return Err(self);
        }
        let Self {
            admission,
            current,
            handler,
            output,
            io,
            complete: _,
        } = self;
        drop(io);
        drop(output);
        Ok(NativeIndexStore {
            _admission: admission,
            physical: Arc::new(super::mount::NativeIndexPhysical {
                current: *current,
                handler,
                read: std::sync::Mutex::new(()),
            }),
        })
    }
}

#[cfg(test)]
mod tests;
