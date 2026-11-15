//! Store-private per-command key custody for the owning native founder driver.
//!
//! Construct this before any Root/operation writer, from the actual slot's
//! source owner and the native factory's original command observation. A caller
//! must retain the whole value outside those guards through the complete call.
//! Neither weak source labels nor cached SDK/signature DATA can construct it.
use super::*;
use catcoms_rt::{
    NativeChatKeyEnds, NativeChatKeyPurpose as Purpose, NativeChatPreparedKeyCut,
    NativeChatSdkRestoreEnds, NativeChatSourceObservation, NativeChatSourceOwner,
    NativeChatUiCommand, NativeChatWarmSdkReopenCut,
};

/// Closed Store-private routing; it is never a decoded permission or callback.
/// Every public library wrapper supplies Library unconditionally. Only the
/// actual owning native operation may borrow its pre-captured command keys.
pub(in crate::store) enum FounderAccess<'a> {
    Library,
    Native(&'a mut NativeFounderKeys),
}
impl FounderAccess<'_> {
    pub(in crate::store) fn require(&self, origin: &FounderOrigin) -> Result<(), AppError> {
        match self {
            Self::Library => origin.require_library(),
            Self::Native(keys) => keys.preflight(origin),
        }
    }
    pub(in crate::store) fn reborrow(&mut self) -> FounderAccess<'_> {
        match self {
            Self::Library => FounderAccess::Library,
            Self::Native(keys) => FounderAccess::Native(keys),
        }
    }
    pub(in crate::store) fn prepare_bootstrap<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        node: &catcoms_sync::ChannelSync<T, R>,
        signing: &mut catcoms_sync::owner_v2::RetainedFounderSigning,
    ) -> Result<catcoms_sync::owner_v2::PreparedFounder, AppError> {
        match self {
            Self::Library => node
                .prepare_owner_v2_founder_retained(signing)
                .map_err(AppError::from),
            Self::Native(keys) => keys.prepare_bootstrap(node, signing),
        }
    }
}

pub(in crate::store) struct NativeFounderKeys {
    pub(in crate::store) original: NativeChatSourceObservation,
    device: NativeChatPreparedKeyCut,
    sdk: NativeChatPreparedKeyCut,
    profile: NativeChatPreparedKeyCut,
    header: NativeChatPreparedKeyCut,
    member: NativeChatPreparedKeyCut,
    warm_sdk: NativeChatWarmSdkReopenCut,
    warm_ends: NativeChatSdkRestoreEnds,
    // LAST: primitive End accounting outlives each prepared metadata pin. The
    // concrete native worker must drop this entire object outside its guards.
    ends: NativeChatKeyEnds,
}
impl NativeFounderKeys {
    pub(in crate::store) fn prepare(
        source: &NativeChatSourceOwner,
        command: &NativeChatUiCommand,
    ) -> Result<Self, AppError> {
        Ok(Self {
            original: source.observe(),
            device: source
                .prepare_cut(command, Purpose::Device)
                .map_err(|_| refused())?,
            sdk: source
                .prepare_cut(command, Purpose::FounderSdk)
                .map_err(|_| refused())?,
            profile: source
                .prepare_cut(command, Purpose::BootstrapProfile)
                .map_err(|_| refused())?,
            header: source
                .prepare_cut(command, Purpose::BootstrapHeader)
                .map_err(|_| refused())?,
            member: source
                .prepare_cut(command, Purpose::MemberAction)
                .map_err(|_| refused())?,
            warm_sdk: source
                .prepare_warm_sdk_reopen(command)
                .map_err(|_| refused())?,
            warm_ends: NativeChatSdkRestoreEnds::new(),
            ends: NativeChatKeyEnds::new(),
        })
    }

    pub(in crate::store) fn preflight(&self, origin: &FounderOrigin) -> Result<(), AppError> {
        match origin {
            FounderOrigin::NativeRequired(original) if original.same_original(&self.original) => {}
            _ => return Err(refused()),
        }
        self.sdk
            .preflight(&self.original, Purpose::FounderSdk)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn sdk_entry(&mut self) -> NativeFounderSdkEntry<'_> {
        NativeFounderSdkEntry {
            original: &self.original,
            cut: &mut self.sdk,
            ends: &mut self.ends,
        }
    }

    pub(in crate::store) fn prepare_bootstrap<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        node: &catcoms_sync::ChannelSync<T, R>,
        signing: &mut catcoms_sync::owner_v2::RetainedFounderSigning,
    ) -> Result<catcoms_sync::owner_v2::PreparedFounder, AppError> {
        node.prepare_owner_v2_founder_native_retained(
            signing,
            &self.original,
            &mut self.profile,
            &mut self.header,
            &mut self.ends,
        )
        .map_err(AppError::from)
    }

    pub(in crate::store) fn enter_device(&mut self) -> Result<(), AppError> {
        self.device
            .enter(&self.original, Purpose::Device, &mut self.ends)
            .map_err(|_| refused())
    }

    /// Cached Device progress is still bound to its original source. A row
    /// can retire independently of its Host/UI/physical mount; cached success
    /// must not silently outlive that revocation or admit another primitive.
    pub(in crate::store) fn require_device(&self) -> Result<(), AppError> {
        self.device
            .preflight(&self.original, Purpose::Device)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn member_entry(
        &mut self,
    ) -> (
        &NativeChatSourceObservation,
        &mut NativeChatPreparedKeyCut,
        &mut NativeChatKeyEnds,
    ) {
        (&self.original, &mut self.member, &mut self.ends)
    }

    /// Separate from fresh SDK creation. The concrete warm caller additionally
    /// supplies its real completed handoff/read evidence to the Store decoder.
    pub(in crate::store) fn warm_sdk_entry(
        &mut self,
    ) -> (
        &NativeChatSourceObservation,
        &mut NativeChatWarmSdkReopenCut,
        &mut NativeChatSdkRestoreEnds,
    ) {
        (&self.original, &mut self.warm_sdk, &mut self.warm_ends)
    }
}
