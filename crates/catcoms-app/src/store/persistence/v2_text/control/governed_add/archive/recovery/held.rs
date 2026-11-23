//! Independent paid Head2 and archive reader.
use super::*;
use crate::store::critical_namespaces::{
    current_layout::V2TextCurrentGroupGuard, read_pool::CatalogReadReservation,
};
use crate::store::persistence::v2_text::{
    capture::original_context,
    control::governed_add::held::require_exact_held,
    io::{
        archive_child, fixed_child, read_control, read_control_into, OpenedTextControl,
        MAX_TEXT_PATH,
    },
    V2TextReadResources,
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use catcoms_mls::persist::{
    text_current_add::TextCurrentAddLimits, TextCurrentSdkGraphReservation,
    TextCurrentSdkRestoreAdmission, TextCurrentSdkRestoreLimits,
};
use catcoms_replication::chat::owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE;
use catcoms_storage::{kept::SessionId, VaultPhysicalWorkLease};
use catcoms_sync::owner_v2::{PreparedV2FounderRecoveredAdd, V2InstalledFounderAddData};
use std::path::PathBuf;

pub(in crate::store) struct HeldGovernedArchiveRead {
    head: Option<ObservedHead>,
    archive: Option<OpenedTextControl>,
    recovered: Option<RecoveredGovernedAdd>,
    archive_buffer: Option<Zeroizing<Vec<u8>>>,
    resources: V2TextReadResources,
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

struct ObservedHead {
    path: PathBuf,
    directory: SessionId,
    raw: OpenedTextControl,
    context: TextContextIdData,
    archive: Option<(selected_head::GovernedHeadData, PathBuf)>,
}

struct RestoredSdkPayment {
    limits: TextCurrentSdkRestoreLimits,
    _fee: CatalogReadReservation,
}

impl TextCurrentSdkGraphReservation for RestoredSdkPayment {
    fn limits(&self) -> TextCurrentSdkRestoreLimits {
        self.limits
    }
}

impl HeldGovernedArchiveRead {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<Self, AppError> {
        const BYTES: usize = 6 * (MAX_BYTES + 1)
            + 2 * MAX_TEXT_PATH
            + 256 * 1024
            + REPLAY_MATH_PRIVATE_INVOICE
            + 4 * std::mem::size_of::<HeldGovernedArchiveRead>();
        let resources = V2TextReadResources::prepare_paid(store, source)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(BYTES)?;
        fee.validate(pool, BYTES)?;
        let archive_buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
        let lease = store._session.retain_for_physical_work();
        resources.require_original_negative(store, source)?;
        Ok(Self {
            head: None,
            archive: None,
            recovered: None,
            archive_buffer,
            resources,
            _lease: lease,
            _fee: fee,
        })
    }

    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.resources
    }

    pub(in crate::store) fn capture_head(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        if self.head.is_some() {
            return Err(refused());
        }
        guard.require_governed_native_invite_parent()?;
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .require_bound_original_negative(guard.store(), guard.source())?;
        let (target, directory, id) = guard.target_directory_data();
        let path = fixed_child(target, "head.bin")?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        let context = original_context(guard, &key)?.id_data()?;
        let raw = read_control(&path, selected_head::FRAME_LEN)?;
        self.head = Some(ObservedHead {
            path,
            directory: id,
            raw,
            context,
            archive: None,
        });
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }

    pub(in crate::store) fn authenticate_head(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let head = self.head.as_mut().ok_or_else(refused)?;
        if head.archive.is_none() {
            let key = Zeroizing::new(store.keys.db_key()?);
            let decoded = selected_head::GovernedHeadData::open_data(&key, &head.raw.frame)?;
            if decoded.transition.context != head.context {
                return Err(refused());
            }
            let path = archive_child(
                head.path.parent().ok_or_else(refused)?,
                decoded.archive.as_bytes(),
            )?;
            head.archive = Some((decoded, path));
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn capture_archive(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        if self.archive.is_some() {
            return Err(refused());
        }
        self.require_head_current(guard)?;
        let (_, path) = self
            .head
            .as_ref()
            .and_then(|head| head.archive.as_ref())
            .ok_or_else(refused)?;
        self.archive = Some(read_control_into(
            path,
            MAX_BYTES,
            &mut self.archive_buffer,
        )?);
        self.require_head_current(guard)
    }

    pub(in crate::store) fn authenticate_archive(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        if self.recovered.is_none() {
            let head = self.head.as_ref().ok_or_else(refused)?;
            head.archive.as_ref().ok_or_else(refused)?;
            let archive = self.archive.as_ref().ok_or_else(refused)?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let original = source.data();
            self.recovered = Some(authenticate_selected(
                &key,
                &head.raw.frame,
                &archive.frame,
                head.context,
                &GenesisPublicData {
                    profile: original.profile,
                    admission: original.admission,
                    header: original.header,
                    image: original.state,
                },
            )?);
        }
        self.resources
            .require_bound_original_negative(store, source)
    }

    pub(in crate::store) fn require_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<&RecoveredGovernedAdd, AppError> {
        self.require_head_current(guard)?;
        let (_, path) = self
            .head
            .as_ref()
            .and_then(|head| head.archive.as_ref())
            .ok_or_else(refused)?;
        require_exact_held(path, self.archive.as_mut().ok_or_else(refused)?)?;
        self.require_head_current(guard)?;
        self.recovered.as_ref().ok_or_else(refused)
    }

    /// Restore the closed SDK graph outside writers, under an independent
    /// typed and provider invoice that survives the reader itself.
    pub(in crate::store) fn prepare_restored_sdk(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<PreparedV2FounderRecoveredAdd, AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let graph = self.recovered.as_ref().ok_or_else(refused)?;
        let head = self
            .head
            .as_ref()
            .and_then(|head| head.archive.as_ref())
            .ok_or_else(refused)?
            .0
            .transition;
        let admission = restore_admission(store)?;
        let restored = PreparedV2FounderRecoveredAdd::restore(
            source.seed(),
            V2InstalledFounderAddData {
                epoch: head.epoch,
                authenticator: head.authenticator,
                header: head.header,
                state: head.state,
            },
            &graph.header,
            graph.image_data()?,
            graph.snapshot_data()?,
            admission,
        )?;
        self.resources
            .require_bound_original_negative(store, source)?;
        Ok(restored)
    }

    fn require_head_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_governed_native_invite_parent()?;
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .require_bound_original_negative(guard.store(), guard.source())?;
        let head = self
            .head
            .as_mut()
            .filter(|head| head.archive.is_some())
            .ok_or_else(refused)?;
        let (target, directory, id) = guard.target_directory_data();
        if head.path.parent() != Some(target) || head.directory != id {
            return Err(refused());
        }
        require_exact_held(&head.path, &mut head.raw)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }
}

pub(super) fn restore_admission(
    store: &ServerStore,
) -> Result<TextCurrentSdkRestoreAdmission, AppError> {
    let limits = TextCurrentAddLimits::governed_text_v2();
    let max_typed_bytes = limits.restore_typed_bytes + 128 * 1024;
    let bytes = max_typed_bytes
        + limits.max_provider_backing_bytes
        + limits.max_provider_payload_bytes
        + 2 * limits.max_snapshot_bytes;
    let pool = &store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?
        .pending
        .pool;
    let fee = pool.try_reserve(bytes)?;
    fee.validate(pool, bytes)?;
    Ok(TextCurrentSdkRestoreAdmission::new(RestoredSdkPayment {
        limits: TextCurrentSdkRestoreLimits {
            max_typed_bytes,
            max_work: limits.restore_work,
        },
        _fee: fee,
    })?)
}
