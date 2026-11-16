//! Seed-absent genesis selection is a separate physical read role. It authenticates the
//! actual pin/head and preserves unknown work; it never handles an error from a present seed.
use super::super::genesis_inventory::{require_fixed_genesis_inventory, GenesisSeedInventoryRole};
use super::*;

pub(super) fn require_absent_inventory(closure: &CompleteColdClosure) -> Result<(), AppError> {
    if !matches!(closure.seed, ColdSeedSelection::Absent) {
        return Err(critical_namespaces::refused());
    }
    // This bounded read-only walk rejects unknown operation/private objects and head.stage.
    // It has no delete/repair authority and proves both original held directory names.
    require_fixed_genesis_inventory(
        (
            &closure.group_path,
            &closure.group_directory,
            closure.group_id,
        ),
        (
            closure
                .head_path
                .parent()
                .ok_or_else(critical_namespaces::refused)?,
            &closure.target_directory,
            closure.target_id,
        ),
        GenesisSeedInventoryRole::Absent,
    )
}

pub(super) fn require_original_genesis_content(
    closure: &CompleteColdClosure,
    store: &ServerStore,
    source: &GenesisSourceDataRef<'_>,
) -> Result<(), AppError> {
    require_absent_inventory(closure)?;
    let key = Zeroizing::new(store.keys.db_key()?);
    let expected = &closure.resources.body.expected;
    let public = source.data();
    let pin = codec::open_pin_data(
        &key,
        expected.layout_data(),
        expected.context.spec_data().group,
        &closure.pin.frame,
    )?;
    pin.require_source_data(expected)?;
    let head = codec::open_head_data(
        &key,
        expected,
        V2GroupPinId::of(&closure.pin.frame),
        &pin,
        &closure.head.frame,
    )?;
    // The existing strict genesis decoder requires header1/no parent and the exact original
    // source context. Compare complete public representatives as well as their signed IDs:
    // a later head or an alternative image cannot silently become bootstrap state.
    head.require_original_public_data(
        &pin,
        &codec::GenesisPublicData {
            profile: public.profile,
            admission: public.admission,
            header: public.header,
            image: public.state,
        },
    )
}
