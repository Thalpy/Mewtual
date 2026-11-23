//! A closed SDK candidate restored from the exact authenticated Add archive.
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_mls::persist::{restore_server_text_current_bounded, TextCurrentSdkRestoreAdmission};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId, owner_header::HeaderKind, owner_state::StateFrame,
};

/// Paid private graph. Source installation, Core replay and publication have
/// their own Host proofs and cannot be authorized by these recovered labels.
pub struct PreparedV2FounderRecoveredAdd {
    _device: MlsDevice,
    _group: ServerGroup,
    expected: V2InstalledFounderAddData,
}

struct GenesisRestoreScope<'a> {
    group: &'a [u8],
    signer: [u8; 32],
    profile: OwnerFactId,
    header: OwnerHeaderId,
}

impl PreparedV2FounderRecoveredAdd {
    pub fn restore(
        original: V2GenesisSourceFields<'_>,
        expected: V2InstalledFounderAddData,
        header: &SignedOwnerHeader,
        image: &[u8],
        snapshot: &[u8],
        admission: TextCurrentSdkRestoreAdmission,
    ) -> Result<Self, SyncError> {
        let original = original.data();
        let context = original.context.spec_data();
        Self::restore_checked(
            GenesisRestoreScope {
                group: context.group,
                signer: context.signer,
                profile: original.profile.id(),
                header: original.header.id(),
            },
            expected,
            header,
            image,
            snapshot,
            admission,
        )
    }

    /// Restore the selected successor SDK from the authenticated original
    /// bootstrap source while retaining a separate closed graph. No genesis
    /// SDK entry, live projection, signing permission or runtime follows.
    pub fn restore_from_bootstrap(
        original: &OpenedBootstrapSourceData,
        expected: V2InstalledFounderAddData,
        header: &SignedOwnerHeader,
        image: &[u8],
        snapshot: &[u8],
        admission: TextCurrentSdkRestoreAdmission,
    ) -> Result<Self, SyncError> {
        let original = original.genesis_public_data();
        let context = original.context_data().spec_data();
        Self::restore_checked(
            GenesisRestoreScope {
                group: context.group,
                signer: context.signer,
                profile: original.profile_data().id(),
                header: original.header_data().id(),
            },
            expected,
            header,
            image,
            snapshot,
            admission,
        )
    }

    fn restore_checked(
        context: GenesisRestoreScope<'_>,
        expected: V2InstalledFounderAddData,
        header: &SignedOwnerHeader,
        image: &[u8],
        snapshot: &[u8],
        admission: TextCurrentSdkRestoreAdmission,
    ) -> Result<Self, SyncError> {
        let state = StateFrame::decode(image).map_err(|_| unavailable())?;
        state
            .verify_header_binding(header)
            .map_err(|_| unavailable())?;
        if header.id() != expected.header
            || header.spec().state != expected.state
            || header.spec().parent != context.header
            || header.spec().kind != HeaderKind::Admission
            || state.meta().mls_epoch != expected.epoch
            || expected.epoch != 1
            || state.meta().group != context.group
            || state.meta().issuer != context.signer
            || state.meta().profile != context.profile
            || state.slots().len() != 2
        {
            return Err(unavailable());
        }
        let limits =
            catcoms_mls::persist::text_current_add::TextCurrentAddLimits::governed_text_v2();
        let (device, group) = restore_server_text_current_bounded(
            snapshot,
            limits.max_snapshot_bytes,
            limits.max_provider_backing_bytes,
            admission,
        )?;
        if !group.is_active()
            || group.group_id_ref() != context.group
            || !device.retained_ed25519_key_matches(&context.signer)
            || !group.has_local_signer(&context.signer)
            || !group.is_designated_committer(&device)
            || group.epoch() != expected.epoch
            || group.epoch_authenticator_id() != expected.authenticator
        {
            return Err(unavailable());
        }
        let roster = group.member_device_ids();
        if roster.len() != state.slots().len()
            || !state
                .slots()
                .all(|slot| roster.contains(&DeviceId::from_public_key_bytes(&slot.subject)))
        {
            return Err(unavailable());
        }
        Ok(Self {
            _device: device,
            _group: group,
            expected,
        })
    }

    pub fn fingerprint_data(&self) -> V2InstalledFounderAddData {
        self.expected
    }
}
