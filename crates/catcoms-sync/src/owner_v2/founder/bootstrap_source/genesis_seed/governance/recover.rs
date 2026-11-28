//! A closed SDK candidate restored from the exact authenticated Add archive.
catcoms_rt::redacted_debug! {
    PreparedV2FounderRecoveredAdd;
}

use super::*;
use catcoms_crypto::DeviceId;
use catcoms_mls::persist::{restore_server_text_current_bounded, TextCurrentSdkRestoreAdmission};
use catcoms_replication::chat::{
    join_bindings::GenesisAnchorBindingData, owner_facts::OwnerFactId, owner_header::HeaderKind,
    owner_state::StateFrame,
};

mod add;
mod invite;
mod member_signing;
mod member_transport;
mod review;
pub use add::{PreparedV2RecoveredAddInstall, V2RecoveredFounderAddSigning};

/// Paid private graph. Source installation, Core replay and publication have
/// their own Host proofs and cannot be authorized by these recovered labels.
pub struct PreparedV2FounderRecoveredAdd {
    pending: Option<Box<PendingAdd>>,
    device: MlsDevice,
    group: ServerGroup,
    expected: V2InstalledFounderAddData,
    source: BootstrapSourceId,
    founder: OwnerFactId,
    signer: [u8; 32],
    member: Option<V2AuthenticatedAdmission>,
    native_origin: Option<catcoms_rt::NativeChatSourceObservation>,
    lifetime: Arc<()>,
}

struct GenesisRestoreScope<'a> {
    source: BootstrapSourceId,
    group: &'a [u8],
    signer: [u8; 32],
    profile: OwnerFactId,
    header: OwnerHeaderId,
    founder: OwnerFactId,
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
                source: original.source,
                group: context.group,
                signer: context.signer,
                profile: original.profile.id(),
                header: original.header.id(),
                founder: original.admission.admission_id(),
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
        let source = original.source_id_data();
        let original = original.genesis_public_data();
        let context = original.context_data().spec_data();
        Self::restore_checked(
            GenesisRestoreScope {
                source,
                group: context.group,
                signer: context.signer,
                profile: original.profile_data().id(),
                header: original.header_data().id(),
                founder: original.admission_data().admission_id(),
            },
            expected,
            header,
            image,
            snapshot,
            admission,
        )
    }

    /// Reopen a closed founder graph for a later selected Add. The complete
    /// prefix must match the exact original bootstrap genesis and terminate
    /// at the named signed Admission. The host owns the separate archive and
    /// selector proof and funds the full prefix replay independently.
    pub fn restore_from_governing_prefix(
        original: &OpenedBootstrapSourceData,
        expected: V2InstalledFounderAddData,
        selected_admission: OwnerFactId,
        proof: &crate::V2GoverningJoinPrefixData<'_>,
        snapshot: &[u8],
        admission: TextCurrentSdkRestoreAdmission,
    ) -> Result<Self, SyncError> {
        Self::restore_selected_membership(
            original,
            expected,
            Some(selected_admission),
            proof,
            snapshot,
            admission,
        )
    }

    pub fn restore_selected_removal(
        original: &OpenedBootstrapSourceData,
        expected: V2InstalledFounderAddData,
        proof: &crate::V2GoverningJoinPrefixData<'_>,
        snapshot: &[u8],
        admission: TextCurrentSdkRestoreAdmission,
    ) -> Result<Self, SyncError> {
        Self::restore_selected_membership(original, expected, None, proof, snapshot, admission)
    }

    fn restore_selected_membership(
        original: &OpenedBootstrapSourceData,
        expected: V2InstalledFounderAddData,
        selected_admission: Option<OwnerFactId>,
        proof: &crate::V2GoverningJoinPrefixData<'_>,
        snapshot: &[u8],
        admission: TextCurrentSdkRestoreAdmission,
    ) -> Result<Self, SyncError> {
        if expected.epoch < 2 {
            return Err(unavailable());
        }
        let genesis = original.genesis_public_data();
        let context = genesis.context_data().spec_data();
        let profile = genesis.profile_data();
        let founder = genesis.admission_data().admission_id();
        let genesis_header = genesis.header_data();
        let anchor = GenesisAnchorBindingData {
            group: context.group.to_vec(),
            founder: context.signer,
            profile: *profile.id().as_bytes(),
            admission: *founder.as_bytes(),
            tenure: *profile.tenure().as_bytes(),
            header: *genesis_header.id().as_bytes(),
            state: *genesis_header.spec().state.as_bytes(),
        };
        let replay = match selected_admission {
            Some(joined) => proof.verify_selected_add(&anchor, expected.header, joined)?,
            None => proof.verify_selected_removal(&anchor, expected.header)?,
        };
        if replay.header().spec().state != expected.state
            || replay.mls_epoch_data() != expected.epoch
        {
            return Err(unavailable());
        }
        let state = StateFrame::decode(replay.image()).map_err(|_| unavailable())?;
        state
            .verify_header_binding(replay.header())
            .map_err(|_| unavailable())?;
        Self::restore_graph(
            GenesisRestoreScope {
                source: original.source_id_data(),
                group: context.group,
                signer: context.signer,
                profile: profile.id(),
                header: genesis_header.id(),
                founder,
            },
            expected,
            &state,
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
        Self::restore_graph(context, expected, &state, snapshot, admission)
    }

    fn restore_graph(
        context: GenesisRestoreScope<'_>,
        expected: V2InstalledFounderAddData,
        state: &StateFrame<'_>,
        snapshot: &[u8],
        admission: TextCurrentSdkRestoreAdmission,
    ) -> Result<Self, SyncError> {
        if !state.slots().any(|slot| {
            slot.admission.as_bytes() == context.founder.as_bytes()
                && slot.subject == context.signer
        }) {
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
            pending: None,
            device,
            group,
            expected,
            source: context.source,
            founder: context.founder,
            signer: context.signer,
            member: None,
            native_origin: None,
            lifetime: Arc::new(()),
        })
    }

    pub fn fingerprint_data(&self) -> V2InstalledFounderAddData {
        self.expected
    }
}
