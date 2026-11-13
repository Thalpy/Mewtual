//! Closed proof for retiring only an attributable first-torn3 claim/fence.
//! Decoder failure alone is not a deletion grant. Every old2 role and the
//! independently held Current context must authenticate before this is issued.
use super::*;
use crate::store::persistence::native_registry::lifecycle::upgrade::UpgradeInventory;

/// Affine, fixed DATA proof consumed by the held physical cleanup owner. No
/// caller can create it from a ready bool, selected generation or error string.
pub(in crate::store) struct VerifiedFirstTorn {
    original:
        crate::store::persistence::native_registry::lifecycle::upgrade::FirstTornInventoryWitness,
    roles: [[u8; 32]; 4],
    stage: Option<[u8; 32]>,
    preserve_join: bool,
}
impl VerifiedFirstTorn {
    pub(in crate::store) fn matches(&self, inventory: &UpgradeInventory) -> bool {
        let Some(seed) = inventory.seed() else {
            return false;
        };
        let [catalog, index, marker] = [inventory.catalog(), inventory.index(), inventory.marker()];
        let stage = inventory.catalog_stage();
        inventory.matches_first_torn_witness(&self.original)
            && self.roles == [catalog, index, marker, seed].map(|raw| *blake3::hash(raw).as_bytes())
            && self.stage == stage.map(|raw| *blake3::hash(raw).as_bytes())
    }
    pub(in crate::store) fn preserves_join_stage(&self) -> bool {
        self.preserve_join
    }
}

/// Accept later valid old2 generations and historical JOIN rows. A permanent
/// G1/zero-JOIN restriction here would strand every established vault after a
/// crash in the claim window. This validates DATA only, not runtime membership.
pub(in crate::store::critical_namespaces::native_registry) fn verify(
    store: &crate::store::ServerStore,
    inventory: &UpgradeInventory,
) -> Result<VerifiedFirstTorn, AppError> {
    inventory.require(store)?;
    if inventory.has_upgrade_outputs() || inventory.index_stage().is_some() {
        return Err(refused());
    }
    let layout = inventory.authenticate_expected(store)?;
    let expected = NativeCapabilityData::for_current_data(layout, &layout.encode_data()?)?;
    let key = Zeroizing::new(store.keys.db_key()?);
    let plain = Zeroizing::new(unseal(&key, &unframe(inventory.catalog())?)?);
    let parsed = grammar::parse_operational_native_registry_layout(&plain, &expected.vault, true)?;
    let original = NativeCapabilityData::for_current_data(
        parsed.current_layout.ok_or_else(refused)?,
        parsed.current_payload.ok_or_else(refused)?,
    )?;
    if original != expected {
        return Err(refused());
    }
    let cap = old_capability::Capability { original };
    old_capability::Capability::decode_exact(
        parsed.native_registry_payload.ok_or_else(refused)?,
        cap,
    )?;
    old_capability::Capability::decode_exact(
        &open(
            &key,
            OLD_MARKER_KEY,
            inventory.marker(),
            v1::MAX_MARKER_FRAME,
        )?,
        cap,
    )?;
    old_data::Index::decode_data(
        &open(&key, OLD_INDEX_KEY, inventory.index(), old_data::MAX_FRAME)?,
        cap,
    )?;
    // A predecessor may save JOIN between the seed-only claim and the empty
    // fence. Authenticate its COMPLETE supported relation; leave that exact
    // nonempty stage untouched for the ordinary2 JOIN opener. Unknown stages
    // never gain cleanup authority from a superficially matching descriptor.
    let preserve_join = if let Some(stage) = inventory.catalog_stage().filter(|raw| !raw.is_empty())
    {
        let next_plain = Zeroizing::new(unseal(&key, &unframe(stage)?)?);
        let next =
            grammar::parse_operational_native_registry_layout(&next_plain, &expected.vault, true)?;
        old_capability::Capability::decode_exact(
            next.native_registry_payload.ok_or_else(refused)?,
            cap,
        )?;
        if next_plain.as_slice() != plain.as_slice() {
            grammar::successor(
                &parsed.summary(*blake3::hash(inventory.catalog()).as_bytes()),
                &next.summary(*blake3::hash(stage).as_bytes()),
            )?;
        }
        true
    } else {
        false
    };
    let seed = inventory.seed().ok_or_else(refused)?;
    if seed.len() >= 40
        && !matches!(
            Output::decode_seed(&key, expected, seed),
            Err(SeedReadError::Unauthenticated(_))
        )
    {
        // This includes authenticated empty/future/invalid intent, over-limit
        // or otherwise indeterminate framing, and a complete valid intent.
        return Err(refused());
    }
    // Short0..39 applies only to this exact fixed held claim, with mutually
    // valid old2 outputs and no conflicting work. It is not a decoder category.
    inventory.require(store)?;
    Ok(VerifiedFirstTorn {
        original: inventory.first_torn_witness(),
        roles: [
            inventory.catalog(),
            inventory.index(),
            inventory.marker(),
            seed,
        ]
        .map(|raw| *blake3::hash(raw).as_bytes()),
        stage: inventory
            .catalog_stage()
            .map(|raw| *blake3::hash(raw).as_bytes()),
        preserve_join,
    })
}

#[cfg(test)]
impl crate::store::ServerStore {
    /// Exercise the actual verifier and held cleanup on a real JOIN fixture.
    /// This is not a cold3 opener or a public production repair command.
    pub(in crate::store) fn settle_native_index3_claim_for_test(&self) -> Result<(), AppError> {
        let pool = CriticalCatalogReadPool::new(PREPARATION_FEE)?;
        let paid = pool.try_reserve(PREPARATION_FEE)?;
        let inventory = UpgradeInventory::capture(self, &paid, &pool)?;
        let proof = verify(self, &inventory)?;
        let mut cleanup =
            crate::store::persistence::native_registry::lifecycle::upgrade::FirstTornIo::prepare(
                self, inventory, proof,
            )?;
        let result = cleanup.run(self);
        drop(cleanup);
        drop(paid);
        result
    }
}
