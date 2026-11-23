//! A read-only cold founder role for the original selected first Add.
use super::*;
use crate::store::critical_namespaces::{
    archive_current_codec as genesis, read_pool::CatalogReadReservation,
};
use crate::store::persistence::v2_genesis_activation::require_directory_name;
use crate::store::persistence::v2_group_directory::original_group_path;
use crate::store::persistence::v2_text::{
    capture::original_context_from_genesis,
    control::governed_add::held::require_exact_held,
    io::{
        archive_child, fixed_child, read_control, read_control_into, OpenedTextControl,
        MAX_TEXT_PATH,
    },
    promotion::seed::parse_data,
};
use crate::store::persistence::{open_archive_directory, OriginalBootstrapReadObservation};
use crate::store::{RetainedBootstrapSourceData, ServerStore};
use catcoms_replication::chat::{
    owner_header::OwnerHeaderId, owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE,
};
use catcoms_storage::kept::{session_id, SessionId};
use catcoms_storage::VaultPhysicalWorkLease;
use catcoms_sync::owner_v2::{PreparedV2FounderRecoveredAdd, V2InstalledFounderAddData};
use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};

const ABSENT: [&str; 4] = [
    "head.stage",
    "text.seed.stage.v1",
    "text.wal",
    selected_head::NAME,
];
const SEED_FRAME_LIMIT: usize = 32768 + 40;

pub(in crate::store) struct HeldColdFounderAdd {
    group: PathBuf,
    directory: File,
    directory_id: SessionId,
    pin: OpenedTextControl,
    seed: OpenedTextControl,
    head: OpenedTextControl,
    archive_path: PathBuf,
    archive: OpenedTextControl,
    selected: selected_head::GovernedHeadData,
    recovered: RecoveredGovernedAdd,
    original: OriginalBootstrapReadObservation,
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl HeldColdFounderAdd {
    pub(in crate::store) fn capture(
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        pin_id: [u8; 32],
    ) -> Result<Self, AppError> {
        const BYTES: usize = 6 * (MAX_BYTES + 1)
            + 12 * MAX_TEXT_PATH
            + 4 * SEED_FRAME_LIMIT
            + 256 * 1024
            + REPLAY_MATH_PRIVATE_INVOICE
            + 4 * std::mem::size_of::<HeldColdFounderAdd>();
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(BYTES)?;
        let lease = store._session.retain_for_physical_work();
        store.validate_mounted_current_layout()?;
        source.validate_original(store)?;
        let expected = source.saved_expectation_data(store)?;
        let data = source.data().genesis_public_data();
        let public = GenesisPublicData {
            profile: data.profile_data(),
            admission: data.admission_data(),
            header: data.header_data(),
            image: data.image_data(),
        };
        let (target, _, _) = source.original_target_data();
        require_stages_absent(target)?;
        let group = original_group_path(
            store,
            expected.layout_data(),
            expected.context.spec_data().group,
        )?;
        let directory = open_archive_directory(&group)?;
        let directory_id =
            session_id(&directory).map_err(|error| AppError::Io(error.to_string()))?;
        require_directory_name(&group, &directory, directory_id)?;
        let pin = read_control(&group.join("pin.bin"), genesis::MAX_PIN_FRAME)?;
        let actual_pin = genesis::V2GroupPinId::of(&pin.frame);
        if actual_pin.as_bytes() != &pin_id {
            return Err(refused());
        }

        let key = Zeroizing::new(store.keys.db_key()?);
        let seed = read_control(&fixed_child(target, "text.seed")?, SEED_FRAME_LIMIT)?;
        let plain = open_control_data(
            &key,
            TextControlKindData::Seed.id_data(&seed.frame),
            &seed.frame,
        )?;
        let promotion = parse_data(&plain)?;
        let context = original_context_from_genesis(
            &expected,
            promotion.original_head,
            &pin.frame,
            &public,
            &key,
        )?;
        let pin_data = genesis::open_pin_data(
            &key,
            expected.layout_data(),
            expected.context.spec_data().group,
            &pin.frame,
        )?;
        genesis::open_head_data(
            &key,
            &expected,
            actual_pin,
            &pin_data,
            promotion.original_head,
        )?
        .require_original_public_data(&pin_data, &public)?;

        let head = read_control(&fixed_child(target, "head.bin")?, selected_head::FRAME_LEN)?;
        let selected = selected_head::GovernedHeadData::open_data(&key, &head.frame)?;
        if promotion.context != context
            || selected.transition.context != context.id_data()?
            || selected.transition.before_head != TextHeadIdData::of_data(promotion.head_frame)
            || selected.transition.before_archive.as_bytes()
                != TextControlKindData::Archive
                    .id_data(promotion.archive_frame)
                    .as_bytes()
        {
            return Err(refused());
        }
        let archive_path = archive_child(target, selected.archive.as_bytes())?;
        let archive = read_control_into(
            &archive_path,
            MAX_BYTES,
            &mut Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1))),
        )?;
        let recovered =
            authenticate_selected(&key, &selected, &archive.frame, context.id_data()?, &public)?;
        let mut result = Self {
            group,
            directory,
            directory_id,
            pin,
            seed,
            head,
            archive_path,
            archive,
            selected,
            recovered,
            original: source.observe_original(),
            _lease: lease,
            _fee: fee,
        };
        result.require_current(store, source)?;
        Ok(result)
    }

    pub(in crate::store) fn require_current(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
    ) -> Result<(OwnerHeaderId, u64), AppError> {
        self.original.require_same_original(source)?;
        source.validate_original(store)?;
        store.validate_mounted_current_layout()?;
        let (target, directory, id) = source.original_target_data();
        require_directory_name(target, directory, id)?;
        require_directory_name(&self.group, &self.directory, self.directory_id)?;
        require_exact_held(&self.group.join("pin.bin"), &mut self.pin)?;
        require_exact_held(&fixed_child(target, "text.seed")?, &mut self.seed)?;
        require_exact_held(&self.archive_path, &mut self.archive)?;
        require_exact_held(&fixed_child(target, "head.bin")?, &mut self.head)?;
        require_stages_absent(target)?;
        require_directory_name(target, directory, id)?;
        require_directory_name(&self.group, &self.directory, self.directory_id)?;
        source.require_original_negative(store)?;
        Ok((
            self.selected.transition.header,
            self.selected.transition.epoch,
        ))
    }

    pub(in crate::store) fn restore_sdk(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
    ) -> Result<PreparedV2FounderRecoveredAdd, AppError> {
        self.require_current(store, source)?;
        let head = self.selected.transition;
        let restored = PreparedV2FounderRecoveredAdd::restore_from_bootstrap(
            source.data(),
            V2InstalledFounderAddData {
                epoch: head.epoch,
                authenticator: head.authenticator,
                header: head.header,
                state: head.state,
            },
            &self.recovered.header,
            self.recovered.image_data()?,
            self.recovered.snapshot_data()?,
            super::held::restore_admission(store)?,
        )?;
        self.require_current(store, source)?;
        Ok(restored)
    }
}

fn require_stages_absent(target: &Path) -> Result<(), AppError> {
    for name in ABSENT {
        if !matches!(fs::symlink_metadata(fixed_child(target, name)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        {
            return Err(refused());
        }
    }
    Ok(())
}
