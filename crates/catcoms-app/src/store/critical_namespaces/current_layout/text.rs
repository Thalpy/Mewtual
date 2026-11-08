//! Read-only original namespace for the real operational text capture.
//!
//! The base guard acquires AD-borrowed pins -> canonical -> Root, checks the SAME installed
//! writer/source/layout/group/pin originals, and reads no old genesis head. The separate
//! affine promotion child requires fresh Current/lane/physical proof before staging or
//! selecting empty O_G. Neither path signs, creates a task, renews Scope or grants deletion.
//! Physical text capture independently authenticates its selected O/H/control/index closure.
use super::*;
pub(in crate::store) use create::{
    AdmittedV2TextCompletedWorkspaceRead, BoundV2TextCompletedCreatePredecessor,
    PreparedV2TextCompletedWorkspaceReadCapture, V2TextCompletedWorkspaceSourceSlot,
    V2_TEXT_COMPLETED_WORKSPACE_ADMISSION_LAYOUT_BYTES,
};

pub(in crate::store) use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextAcceptedQueryLease, V2TextAcceptedQueryLease,
};
pub(in crate::store) use create::{
    AdmittedV2TextAcceptedCreateCommit, AdmittedV2TextAcceptedCreateStage,
    SavedAcceptedV2TextAdvanceGuard, SavedAcceptedV2TextFinalGuard,
    V2_TEXT_ACCEPTED_ADVANCE_GUARD_LAYOUT_BYTES, V2_TEXT_ACCEPTED_CREATE_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use create::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
    V2_TEXT_RETIREMENT_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use create::{
    AdmittedV2TextIntentCommit, AdmittedV2TextIntentStage, V2_TEXT_INTENT_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use create::{
    AdmittedV2TextRawSignatureCommit, AdmittedV2TextRawSignatureStage,
    V2_TEXT_RAW_SIGNATURE_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use create::{
    AdmittedV2TextRetainRecoveryRead, AdmittedV2TextRetainRepair, AdmittedV2TextRetainScratch,
    PreparedV2TextRetainRecoveryCapture, PreparedV2TextRetainRepairLease,
    PreparedV2TextRetainScratchLease, V2TextRetainRecoverySourceSlot,
    V2_TEXT_RETAIN_RECOVERY_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use create::{
    SavedCompletedV2TextFinishGuard, V2_TEXT_COMPLETED_FINISH_LAYOUT_BYTES,
};
pub(in crate::store) use create::{
    SavedUnsignedV2TextSigningGuard, V2_TEXT_SIGNING_GUARD_LAYOUT_BYTES,
};
pub(in crate::store) use query::{
    AdmittedV2TextAcceptedQuery, PreparedV2TextAcceptedQueryCapture, V2TextAcceptedQuerySourceSlot,
    V2_TEXT_ACCEPTED_QUERY_ADMISSION_LAYOUT_BYTES,
};
mod create;
pub(in crate::store) use create::{
    AdmittedV2TextRetainCommit, AdmittedV2TextRetainStage, V2_TEXT_RETAIN_ADMISSION_LAYOUT_BYTES,
};
mod promotion;
pub(in crate::store) use create::{
    AdmittedV2TextWorkspaceRead, BoundV2TextCreatePredecessor, PreparedV2TextWorkspaceReadCapture,
    V2TextWorkspaceSourceSlot, V2_TEXT_WORKSPACE_ADMISSION_LAYOUT_BYTES,
};
mod query;
use crate::store::chat_admission::v2_current::{
    V2TextCurrentAdmission, V2_TEXT_SCOPE_METADATA_BYTES,
};
use crate::store::owner_evidence::canonical::current_vacancy::text::{
    V2TextOwnerNamespace, V2_TEXT_OWNER_NAMESPACE_BYTES,
};
pub(in crate::store) use crate::store::owner_evidence::canonical::text_lane::{
    PreparedV2TextQueryLease, V2TextQueryLease,
};
use crate::store::owner_evidence::canonical::v2_registration::V2WriterObservation;
use crate::store::persistence::v2_text::V2TextReadResources;
use crate::store::persistence::{GenesisSourceDataRef, V2TextOwnerRef};
pub(in crate::store) use promotion::{
    AdmittedV2TextPromotionCommit, AdmittedV2TextPromotionStage, PreparedV2TextPhysicalLease,
    V2_TEXT_PROMOTION_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use promotion::{
    AdmittedV2TextPromotionScratch, AdmittedV2TextPromotionSeedRepair,
    PreparedV2TextPromotionScratchCapture, PreparedV2TextPromotionSeedRepairCapture,
    V2TextPromotionScratchSourceSlot, V2TextPromotionSeedRepairSourceSlot,
    V2_TEXT_PROMOTION_REPAIR_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use query::{
    AdmittedV2TextTokenQuery, PreparedV2TextTokenQueryCapture, V2TextTokenQuerySourceSlot,
    V2_TEXT_TOKEN_QUERY_ADMISSION_LAYOUT_BYTES,
};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard};
use zeroize::Zeroizing;

type DirectoryData<'a> = (&'a Path, &'a File, catcoms_storage::kept::SessionId);
type PinData<'a> = (&'a File, catcoms_storage::kept::SessionId, &'a [u8]);

#[cfg(all(test, windows))]
std::thread_local! {
    // The real full source proof has completed. A replacement here must still be caught
    // by the following real ID3 proof, not an observation sampled before source IO.
    static AFTER_ORIGINAL_SOURCE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Checked in the actual physical read/capture invoice before reservation/allocation.
/// Original Source/SDK/task/Scope fees remain independently retained. Four bounded old
/// selectors, one closed pin path, native conversion scratch and two named File probes
/// coexist conservatively; the single pin read never grows its requested capacity.
pub(in crate::store) const V2_TEXT_CURRENT_GROUP_GUARD_BYTES: usize =
    2 * std::mem::size_of::<V2TextCurrentGroupGuard<'static, 'static>>()
        + V2_TEXT_OWNER_NAMESPACE_BYTES
        + V2_TEXT_SCOPE_METADATA_BYTES
        + 5 * (MAX_OPEN_BASE_PATH + 256)
        + 2 * std::mem::size_of::<String>()
        + 2 * std::mem::size_of::<File>()
        + 2 * (super::super::archive_current_codec::MAX_PIN_FRAME + 1)
        + 64 * 1024;

pub(in crate::store) struct V2TextCurrentGroupGuard<'guard, 'writer> {
    // Both loans refer to actual existing originals. They allocate/clone nothing and
    // cannot outlive the owner's source/resource loan. Root releases before canonical.
    directory: DirectoryData<'guard>,
    pin: PinData<'guard>,
    root: MutexGuard<'guard, Option<super::super::archive_bootstrap::MountedRoot>>,
    owner: V2TextOwnerNamespace<'guard, 'writer>,
    resources: &'guard V2TextReadResources,
}

fn original_directory_and_pin<'guard, 'writer>(
    admission: &'guard V2TextCurrentAdmission<'writer>,
) -> Result<(DirectoryData<'guard>, PinData<'guard>), AppError> {
    let original = admission.owner_ref().original_association();
    match (
        original.warm_head(),
        original.cold_head(),
        original.repaired_head(),
    ) {
        (Some(head), None, None) => Ok((head.group_directory_data(), head.text_pin_data()?)),
        (None, Some(head), None) => Ok((head.directory_data(), head.text_pin_data()?)),
        (None, None, Some(head)) => Ok((head.seed().directory_data(), head.text_pin_data()?)),
        _ => Err(refused()),
    }
}

fn pin_path(base: &Path) -> Result<PathBuf, AppError> {
    let capacity = base
        .as_os_str()
        .len()
        .checked_add(1 + "pin.bin".len())
        .filter(|n| *n <= MAX_OPEN_BASE_PATH + 256)
        .ok_or_else(refused)?;
    let mut path = PathBuf::with_capacity(capacity);
    path.push(base);
    path.push("pin.bin");
    if path.capacity() > capacity {
        return Err(refused());
    }
    Ok(path)
}

fn require_pin_name(path: &Path, pin: PinData<'_>) -> Result<(), AppError> {
    if catcoms_storage::kept::session_id(pin.0).map_err(|e| AppError::Io(e.to_string()))? != pin.1
        || !matches!(
            catcoms_storage::kept::session_at(path, pin.1),
            catcoms_storage::kept::SessionAt::Ours
        )
    {
        return Err(refused());
    }
    Ok(())
}

fn require_pin_bytes(path: &Path, pin: PinData<'_>) -> Result<(), AppError> {
    let expected = pin.2;
    if expected.is_empty() || expected.len() > super::super::archive_current_codec::MAX_PIN_FRAME {
        return Err(refused());
    }
    require_pin_name(path, pin)?;
    let mut file = pin.0.try_clone().map_err(|e| AppError::Io(e.to_string()))?;
    if file
        .metadata()
        .map_err(|e| AppError::Io(e.to_string()))?
        .len()
        != expected.len() as u64
    {
        return Err(refused());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|e| AppError::Io(e.to_string()))?;
    let capacity = expected.len().checked_add(1).ok_or_else(refused)?;
    let mut bytes = Zeroizing::new(vec![0; capacity]);
    let mut used = 0;
    while used < capacity {
        match file.read(&mut bytes[used..]) {
            Ok(0) => break,
            Ok(n) => used += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(AppError::Io(e.to_string())),
        }
    }
    if used != expected.len() || &bytes[..used] != expected {
        return Err(refused());
    }
    require_pin_name(path, pin)
}

impl<'guard, 'writer> V2TextCurrentGroupGuard<'guard, 'writer> {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn after_original_source_for_test(&self, hook: Box<dyn FnOnce()>) {
        AFTER_ORIGINAL_SOURCE.with(|slot| *slot.borrow_mut() = Some(hook));
    }

    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.scope_admission().store()
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'writer> {
        self.scope_admission().source()
    }
    pub(in crate::store) fn owner_ref(&self) -> &V2TextOwnerRef<'writer> {
        self.scope_admission().owner_ref()
    }
    pub(in crate::store) fn scope_admission(&self) -> &V2TextCurrentAdmission<'writer> {
        self.owner.scope_admission()
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.scope_admission().task()
    }
    pub(in crate::store) fn archive_root(&self) -> &super::super::archive_bootstrap::MountedRoot {
        self.root
            .as_ref()
            .expect("actual Root checked by private read constructor")
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.resources
    }
    pub(in crate::store) fn directory_data(&self) -> DirectoryData<'_> {
        self.directory
    }
    /// Actual original source target only; no caller path or raw source adopter.
    pub(in crate::store) fn target_directory_data(&self) -> DirectoryData<'_> {
        self.source().source().original_target_data()
    }
    /// Exposes only the retained original pin's bounded immutable ciphertext and object
    /// identity to the typed capture. It supplies no caller-selected frame/path or key.
    pub(in crate::store) fn original_pin_data(&self) -> PinData<'_> {
        self.pin
    }
    /// Stable labels borrowed from the actual original genesis receipt and held Scope.
    /// In particular the parent label hashes the OLD genesis CIPHERTEXT, not its public
    /// Header. Current O/H authentication is still the physical capture consumer's job.
    /// No old seed/head file is reopened, decoded or required to remain named here.
    pub(in crate::store) fn original_genesis_context_data(
        &self,
    ) -> Result<
        (
            &super::super::archive_current_codec::SavedSourceExpectationData,
            &[u8],
            [u8; 32],
        ),
        AppError,
    > {
        self.validate_source_only()?;
        let original = self.owner_ref().original_association();
        let (expectation, frame) = match (
            original.warm_head(),
            original.cold_head(),
            original.repaired_head(),
        ) {
            (Some(head), None, None) => head.text_genesis_context_data()?,
            (None, Some(head), None) => head.text_genesis_context_data()?,
            (None, None, Some(head)) => head.text_genesis_context_data()?,
            _ => return Err(refused()),
        };
        let nonce = self.scope_admission().scope_nonce_data()?;
        self.validate_source_only()?;
        Ok((expectation, frame, nonce))
    }
    pub(in crate::store) fn require_installed_writer_observation(
        &self,
        original: &V2WriterObservation,
    ) -> Result<(), AppError> {
        self.owner.require_installed_text_writer(original)
    }
    /// Cached negatives only: no account, filesystem, codec, SDK or lock acquisition.
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.owner.validate_source_only()?;
        self.resources
            .require_original_negative(self.store(), self.source())?;
        let root = self.archive_root();
        if root.negative.is_cancelled()
            || !Arc::ptr_eq(&root.mount, &self.store().registry_mount())
            || root.descriptor.vault != self.source().data().context.spec_data().vault
        {
            return Err(refused());
        }
        Ok(())
    }
    /// Repeat after the physical consumer's content/account work. No old activation.seed
    /// or genesis head decoder is called: operational O/H selection is that consumer's job.
    pub(in crate::store) fn validate_original_namespace(&self) -> Result<(), AppError> {
        self.resources
            .validate_original(self.store(), self.source())?;
        self.owner.validate_old_files()?;
        self.require_installed_writer_observation(self.scope_admission().writer_observation()?)?;
        let pin_path = pin_path(self.directory.0)?;
        require_pin_bytes(&pin_path, self.pin)?;
        self.source()
            .source()
            .validate_under_archive_root(self.store(), self.archive_root())?;
        #[cfg(all(test, windows))]
        if let Some(hook) = AFTER_ORIGINAL_SOURCE.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
        self.store()
            .validate_mounted_current_layout_under_archive_root(self.archive_root())?;
        // Only final held/named objects and cached negatives after the full S0/ID3 waits.
        require_pin_name(&pin_path, self.pin)?;
        let (target, target_file, target_id) = self.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target,
            target_file,
            target_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            self.directory.0,
            self.directory.1,
            self.directory.2,
        )?;
        self.validate_source_only()
    }
}

impl ServerStore {
    pub(in crate::store) fn lock_v2_text_current_group<'guard, 'writer>(
        &'guard self,
        admission: &'guard V2TextCurrentAdmission<'writer>,
        resources: &'guard V2TextReadResources,
    ) -> Result<V2TextCurrentGroupGuard<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) || self.dir.as_os_str().len() > MAX_OPEN_BASE_PATH
        {
            return Err(refused());
        }
        // SAME real metadata loan is checked BEFORE namespace selectors/lock waits. Its
        // owning body was allocated outside AD and cannot drain during a guard's drop.
        resources.validate_original(self, admission.source())?;
        admission.validate_source_only()?;
        // Bind only the SAME actual Current Scope borrower, before selector/Root waits
        // or positive physical capture. This is negative custody, not a Current permit:
        // even a later constructor failure must retain this original cancellation fence
        // so outside-writer nonce/seal retries cannot revive a revoked Scope.
        resources.bind_scope_negative(admission)?;
        let owner = self.lock_v2_text_owner_namespace(admission)?;
        let root = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let (directory, pin) = original_directory_and_pin(admission)?;
        let guard = V2TextCurrentGroupGuard {
            directory,
            pin,
            root,
            owner,
            resources,
        };
        guard.validate_original_namespace()?;
        Ok(guard)
    }
}

impl std::fmt::Debug for V2TextCurrentGroupGuard<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextCurrentGroupGuard")
            .finish_non_exhaustive()
    }
}
