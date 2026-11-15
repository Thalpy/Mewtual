//! Private pre-cleaner dispatch for version2. No runtime writer/SDK can exist
//! before this sequence and the ordinary JOIN opener have both completed.
use super::super::super::pending_join_codec as grammar;
use super::super::{capability::NativeCapabilityData, data as old, preparation as previous};
use super::*;
use crate::store::persistence::native_registry::operational as io;
use zeroize::Zeroizing;

impl Handler {
    pub(in crate::store) fn inspect_storage(
        &self,
        store: &crate::store::ServerStore,
    ) -> Result<(), AppError> {
        self.require()?;
        let inventory = io::UpgradeInventory::capture(store)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        if let Some(raw) = inventory.seed() {
            match upgrade::authenticate(&key, upgrade::SEED_KEY, raw, UPGRADE_SEED_FRAME) {
                Ok(plain) => {
                    // AEAD success, including empty/malformed/future plaintext,
                    // NEVER enters first-torn disposal. Decode before any repair.
                    let output = upgrade::UpgradeOutput::decode_seed(store, &key, raw, &plain)?;
                    let mut repair = io::UpgradeIo::recover(store, inventory, &output.bytes())?;
                    return repair.advance_upgrade(store, &output.bytes());
                }
                Err(_) => {
                    validate_original_v1(store, &key, &inventory)?;
                    return inventory.abandon_first_torn(store);
                }
            }
        }
        if inventory.has_upgrade_outputs() {
            return Err(refused());
        }
        let plain = upgrade::catalog_plain(&key, inventory.catalog())?;
        let cap = upgrade::capability_data(&plain, &store.chat_vault_domain()?)?;
        capability::Capability::decode_exact(
            &upgrade::authenticate(
                &key,
                upgrade::MARKER_KEY,
                inventory.marker(),
                previous::MAX_MARKER_FRAME,
            )?,
            cap,
        )?;
        let index = data::Index::decode_data(
            &upgrade::authenticate(&key, upgrade::INDEX_KEY, inventory.index(), data::MAX_FRAME)?,
            cap,
        )?;
        inventory.require(store)?;
        if let Some(raw) = inventory.index_stage() {
            // With the actual retained interlock, correct producers cannot
            // leave both WALs. Refuse before either old cleaner can run.
            if inventory.catalog_stage().is_some() {
                return Err(refused());
            }
            let expected = Zeroizing::new(raw.to_vec());
            let authenticated =
                upgrade::authenticate(&key, upgrade::INDEX_KEY, &expected, data::MAX_FRAME);
            match authenticated {
                Err(_) => {
                    let mut repair = io::IndexStageCleanup::prepare_unauthenticated(
                        inventory.into_index_snapshot(store)?,
                    )?;
                    repair.run(store)
                }
                Ok(plain) => {
                    let next = data::Index::decode_data(&plain, cap)?;
                    if expected.as_slice() == inventory.index() {
                        let mut repair = io::IndexStageCleanup::prepare_duplicate(
                            inventory.into_index_snapshot(store)?,
                        )?;
                        return repair.run(store);
                    }
                    index.successor_data(&next, *blake3::hash(inventory.index()).as_bytes())?;
                    let mut repair = io::IndexCommitIo::recover(
                        store,
                        inventory.into_index_snapshot(store)?,
                        &expected,
                    )?;
                    repair.commit_index(store, &expected)
                }
            }
        } else {
            // A catalog Stage ALONE is ordinary supported JOIN recovery. Check
            // its same immutable native descriptor, then leave its exact
            // successor/duplicate classification to the existing JOIN opener.
            if let Some(stage) = inventory.catalog_stage() {
                let next = upgrade::catalog_plain(&key, stage)?;
                if upgrade::capability_data(&next, &store.chat_vault_domain()?)? != cap {
                    return Err(refused());
                }
            }
            inventory.require(store)
        }
    }
}

fn validate_original_v1(
    store: &crate::store::ServerStore,
    key: &[u8; 32],
    inventory: &io::UpgradeInventory,
) -> Result<(), AppError> {
    let plain = upgrade::catalog_plain(key, inventory.catalog())?;
    let parsed =
        grammar::parse_native_registry_layout(&plain, &store.chat_vault_domain()?, true, false)?;
    if parsed.count != 0 || inventory.has_upgrade_outputs() || inventory.index_stage().is_some() {
        return Err(refused());
    }
    let cap = NativeCapabilityData::for_current_data(
        parsed.current_layout.ok_or_else(refused)?,
        parsed.current_payload.ok_or_else(refused)?,
    )?;
    NativeCapabilityData::decode_exact(parsed.native_registry_payload.ok_or_else(refused)?, cap)?;
    NativeCapabilityData::decode_exact(
        &previous::open_kind(
            key,
            previous::MARKER_KEY,
            inventory.marker(),
            previous::MAX_MARKER_PLAIN,
        )?,
        cap,
    )?;
    let old = old::IndexData::decode_data(
        &previous::open_kind(key, previous::INDEX_KEY, inventory.index(), old::MAX_PLAIN)?,
        cap,
    )?;
    data::Index::migrate_v1_data(old)?;
    inventory.require(store)
}
