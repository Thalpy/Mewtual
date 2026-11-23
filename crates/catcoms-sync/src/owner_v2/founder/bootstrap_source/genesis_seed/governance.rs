//! Whole-node PRIVATE Add continuation. This is an owning library boundary,
//! not an issuer of Root Current, a covering receipt, or native credentials.
//!
//! The original genesis provenance never changes. Only an exact completed MLS
//! candidate may replace the SDK pair, and its actual installed fingerprint is
//! kept separately. In particular an epoch comparison cannot renew Fresh.
use super::*;
use crate::owner_v2::V2MemberSignRefusal;
use catcoms_mls::group::text_current_add::{
    CompletedTextCurrentAddData, PreparedTextCurrentAdd, TextCurrentAddRequestData,
};
use catcoms_mls::persist::text_current_add::TextCurrentAddAdmission;
use catcoms_replication::chat::owner_header::{OwnerHeaderId, OwnerStateCommitment};
use catcoms_replication::chat::owner_profile::{AdmissionOrigin, AdmissionSpec};
use catcoms_replication::chat::owner_state::transition::{
    ReplayMath, UnsignedAdmissionStepData, UNSIGNED_ADMISSION_DATA_INVOICE,
};
use catcoms_replication::chat::V2AuthenticatedAdmission;
use catcoms_rt::{
    NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatPreparedKeyCut, NativeChatSourceObservation,
};

mod request;
use request::FrozenAddRequest;
mod installation;
pub use installation::{PreparedV2FounderAddInstall, RetiredV2FounderAddInstall};
mod invite;
mod member_transport;
mod recover;
pub use recover::PreparedV2FounderRecoveredAdd;
mod welcome;
pub use request::{V2FounderAddRequestData, MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES};
use welcome::PreparedWelcome;
mod native_stage;
use native_stage::NativeStageAdmission;
pub use native_stage::{RetiredV2FounderAddNativeCustody, V2FounderAddNativeStage};

/// Unchanged original preparation inputs on refusal. The independently paid
/// owning Box is built outside writers; it confers no renewed admission and
/// cannot replace an operation that already entered this whole-node owner.
pub type V2FounderAddPreparationRefusal = Box<(
    NativeChatSourceObservation,
    TextCurrentAddAdmission,
    NativeChatPreparedKeyCut,
    NativeChatPreparedKeyCut,
    NativeChatPreparedKeyCut,
)>;

/// Immutable observation of THIS wrapper's actual installation, not a saved
/// head, selected Core prefix, portable proof, or positive Source capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct V2InstalledFounderAddData {
    pub epoch: u64,
    pub authenticator: [u8; 32],
    pub header: OwnerHeaderId,
    pub state: OwnerStateCommitment,
}

/// The actual completed SDK output and unsigned governing data from one Add.
/// The provider snapshot is secret and must be sealed by an independently
/// funded Root transaction. This loan cannot select, save or install anything.
pub struct V2FounderAddOutcomeData<'a> {
    result: CompletedTextCurrentAddData<'a>,
    admission: &'a AdmissionBody,
    unsigned: &'a UnsignedAdmissionStepData,
    request: &'a FrozenAddRequest,
}

impl<'a> V2FounderAddOutcomeData<'a> {
    pub fn result_data(&self) -> &CompletedTextCurrentAddData<'a> {
        &self.result
    }

    pub fn admission_data(&self) -> &'a AdmissionBody {
        self.admission
    }

    pub fn unsigned_data(&self) -> &'a UnsignedAdmissionStepData {
        self.unsigned
    }

    pub fn require_request_data(
        &self,
        request: V2FounderAddRequestData<'_>,
    ) -> Result<(), SyncError> {
        self.request.require_same(request)
    }
}

impl<T, R> UnboundV2GenesisNode<T, R> {
    /// Outside writers with separate package verification credit. This uses
    /// the original source clock and performs no private candidate work.
    pub fn preflight_first_add_package_data(
        &self,
        request: V2FounderAddRequestData<'_>,
        limits: catcoms_mls::persist::text_current_add::TextCurrentAddLimits,
    ) -> Result<(), SyncError> {
        ordinary_create::source(self).map_err(|_| unavailable())?;
        self.check_add_package(request, limits)?;
        ordinary_create::source(self).map_err(|_| unavailable())
    }

    fn check_add_package(
        &self,
        request: V2FounderAddRequestData<'_>,
        limits: catcoms_mls::persist::text_current_add::TextCurrentAddLimits,
    ) -> Result<(), SyncError> {
        let seed = &self.seed.body;
        drop(request.verify_seed(seed)?);
        let now = self._parts.body._clock.now_ms();
        if request.invite.expired_at_data(now) {
            return Err(unavailable());
        }
        let signed = request.request.spec_data();
        let packet = TextCurrentAddRequestData::from_untrusted_data(
            request.key_package,
            signed.joiner,
            request.invite.spec_data().nonce,
            signed.key_package_reference,
            now,
        )
        .with_expected_credential_data(signed.credential);
        catcoms_mls::group::text_current_add::check_private_text_add_package(
            &seed.device,
            &seed.group,
            packet,
            limits,
        )
        .map_err(|_| unavailable())
    }
}

impl std::fmt::Debug for V2FounderAddOutcomeData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2FounderAddOutcomeData")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prepared,
    SdkEntered,
    Complete,
    HeaderEntered,
    Raw,
    Materialized,
}

/// Fields containing SDK/secret/output custody precede the admitted End batch.
/// Dispose the WHOLE pending operation outside all Root/native writers. A host
/// must first reconcile its real disk workers and durable journal: idle Rust
/// ownership alone does not prove that an admitted write has finished.
struct PendingAdd {
    request: FrozenAddRequest,
    admitted_at_ms: Option<u64>,
    cell: Option<PreparedTextCurrentAdd>,
    admission: Option<TextCurrentAddAdmission>,
    body: Option<AdmissionBody>,
    unsigned: Option<UnsignedAdmissionStepData>,
    header: Option<SignedOwnerHeader>,
    raw: Option<[u8; 64]>,
    welcome: Option<PreparedWelcome>,
    retired_core: Option<catcoms_replication::chat::V2GeneralRetiredData>,
    retired_sdk: Option<catcoms_mls::group::text_current_add::RetiredTextCurrentAdd>,
    phase: Phase,
    original: NativeChatSourceObservation,
    sdk_cut: Option<NativeChatPreparedKeyCut>,
    header_cut: Option<NativeChatPreparedKeyCut>,
    welcome_cut: Option<NativeChatPreparedKeyCut>,
    // Monotone after actual closed-Cell proof. A spent cut alone never proves
    // its admitted unit still has an End, so later SDK continuation refuses.
    sdk_continuation_retired: bool,
    sdk_native: NativeStageAdmission,
    header_native: NativeStageAdmission,
    welcome_native: NativeStageAdmission,
    sdk_may_reauthorize: bool,
    header_may_reauthorize: bool,
    welcome_may_reauthorize: bool,
    // LAST during admitted/partial work and terminal disposal. An explicitly
    // proved CLOSED Cell may retain historical DATA after native_stage moves
    // finite custody out; that move monotonically forbids SDK continuation.
    ends: NativeChatKeyEnds,
}

/// Disjoint loan of the original Add signing state and its retained native
/// custody. Root can hold the actual seed/Current proof while using this loan.
pub struct V2FounderAddSigning<'a> {
    seed: &'a UnboundV2GenesisSeed,
    negative: V2FounderGovernanceNegative<'a>,
    pending: &'a mut PendingAdd,
}

impl V2FounderAddSigning<'_> {
    pub fn sign_header_native(&mut self) -> Result<(), SyncError> {
        let p = &mut *self.pending;
        if p.raw.is_some() {
            return Ok(());
        }
        self.negative.require_live_data()?;
        let unsigned = p.unsigned.as_ref().ok_or_else(unavailable)?;
        if p.phase != Phase::Complete {
            return Err(unavailable());
        }
        let signer = self.seed.body.context.spec_data().signer;
        if !self.seed.body.device.retained_ed25519_key_matches(&signer) {
            return Err(unavailable());
        }
        p.header_native.enter(
            p.header_cut.as_mut().ok_or_else(unavailable)?,
            &p.original,
            NativeChatKeyPurpose::GoverningHeader,
            &mut p.ends,
        )?;
        p.phase = Phase::HeaderEntered;
        #[cfg(test)]
        test_cut(TestCut::HeaderEntered);
        let raw = self
            .seed
            .body
            .device
            .sign_retained_ed25519_bounded(
                &signer,
                unsigned.signing_bytes_data(),
                catcoms_replication::chat::owner_header::MAX_HEADER_SEMANTIC_BYTES,
            )
            .map_err(|_| unavailable())?;
        p.raw = Some(raw);
        p.phase = Phase::Raw;
        #[cfg(test)]
        test_cut(TestCut::RawCached);
        Ok(())
    }
}

/// SAME complete original node and private candidate slot. There is no SDK,
/// runtime, transport, mutable seed getter, or conversion back to genesis.
/// The real host independently retains all source/metadata fees and must own
/// its original journal, Core verifier, Current and native final admission.
pub struct PreparedV2FounderGovernanceNode<T, R> {
    pending: Option<Box<PendingAdd>>,
    installed: Option<V2InstalledFounderAddData>,
    installed_invites: [Option<[u8; 16]>; crate::MAX_V2_GOVERNING_JOIN_STEPS],
    member: Option<V2AuthenticatedAdmission>,
    node: UnboundV2GenesisNode<T, R>,
}

/// Independent requested metadata/output inventory, in addition to the whole
/// node, the actual full MLS Add reservation, retained native metadata, and the
/// host's Core replay/verifier payment. Reserve BEFORE constructing the wrapper
/// or beginning an operation. It is neither an allocation/RSS cap nor a fee.
pub const FOUNDER_GOVERNANCE_DATA_BYTES: usize = UNSIGNED_ADMISSION_DATA_INVOICE
    + 8 * MAX_HEADER_BYTES
    + 4 * MAX_ADMISSION_BYTES
    + 16 * MAX_GROUP_ID_BYTES
    + 4 * MAX_FOUNDER_ADD_KEY_PACKAGE_BYTES
    + 16 * catcoms_replication::chat::join_bindings::MAX_JOIN_BINDING_BYTES
    + 2 * catcoms_mls::group::text_current_add::MAX_TEXT_ADD_ROUTING_PLAIN_BYTES
    + 4 * std::mem::size_of::<PendingAdd>()
    + 4 * std::mem::size_of::<V2InstalledFounderAddData>()
    + 4 * std::mem::size_of::<V2FounderAddOutcomeData<'static>>()
    + 4 * std::mem::size_of::<V2FounderAddSigning<'static>>()
    + 4 * std::mem::size_of::<PreparedV2FounderAddInstall<'static>>()
    + 4 * std::mem::size_of::<RetiredV2FounderAddNativeCustody>()
    + 4 * std::mem::size_of::<PreparedV2FounderGovernanceNode<(), ()>>()
    + 4 * std::mem::size_of::<(
        NativeChatSourceObservation,
        TextCurrentAddAdmission,
        NativeChatPreparedKeyCut,
        NativeChatPreparedKeyCut,
        NativeChatPreparedKeyCut,
    )>()
    + 16 * 1024;

impl<T, R> UnboundV2GenesisNode<T, R> {
    /// Consume an ORIGINAL warm founder with its original attached scope. Cold
    /// decoded genesis DATA cannot become a live governor through this method.
    /// Refusal returns the unchanged whole node; success only moves fields.
    pub fn prepare_governance_data(self) -> Result<PreparedV2FounderGovernanceNode<T, R>, Self> {
        if !self.is_governance_ready_data() {
            return Err(self);
        }
        Ok(PreparedV2FounderGovernanceNode {
            pending: None,
            installed: None,
            installed_invites: [None; crate::MAX_V2_GOVERNING_JOIN_STEPS],
            member: None,
            node: self,
        })
    }

    /// Future Root can supply its actual paid owner slots. Prove the original
    /// warm node and the empty destination before taking anything, then cache
    /// its governed owner with only infallible moves. Both slots survive a
    /// refusal unchanged, including when the destination is already occupied.
    pub fn prepare_governance_into_data(
        original: &mut Option<Self>,
        destination: &mut Option<PreparedV2FounderGovernanceNode<T, R>>,
    ) -> Result<(), SyncError> {
        if destination.is_some()
            || !original
                .as_ref()
                .is_some_and(Self::is_governance_ready_data)
        {
            return Err(unavailable());
        }
        let node = original.take().expect("validated original warm node");
        *destination = Some(PreparedV2FounderGovernanceNode {
            pending: None,
            installed: None,
            installed_invites: [None; crate::MAX_V2_GOVERNING_JOIN_STEPS],
            member: None,
            node,
        });
        Ok(())
    }

    fn is_governance_ready_data(&self) -> bool {
        self._parts.body.warm.is_some()
            && self._parts.body.fresh.is_some()
            && ordinary_create::source(self).is_ok()
    }
}

impl<T, R> PreparedV2FounderGovernanceNode<T, R> {
    pub fn preflight_first_add_package_data(
        &self,
        request: V2FounderAddRequestData<'_>,
        limits: catcoms_mls::persist::text_current_add::TextCurrentAddLimits,
    ) -> Result<(), SyncError> {
        if self.installed.is_some() {
            return Err(unavailable());
        }
        self.node.preflight_first_add_package_data(request, limits)
    }

    /// Verify a subsequent package against this installed MLS parent and the
    /// matching replay before a host reserves the next governing operation.
    pub fn preflight_subsequent_add_package_data(
        &self,
        replay: &ReplayMath,
        request: V2FounderAddRequestData<'_>,
        limits: catcoms_mls::persist::text_current_add::TextCurrentAddLimits,
    ) -> Result<(), SyncError> {
        self.require_sdk_current_data()?;
        let installed = self.installed.ok_or_else(unavailable)?;
        if self.pending.is_some()
            || self
                .available_invite_slot(request.invite.spec_data().nonce)
                .is_none()
            || replay.profile() != &self.node.seed.body.genesis.profile
            || replay.header().id() != installed.header
            || replay.header().spec().state != installed.state
            || replay.mls_epoch_data() != installed.epoch
        {
            return Err(unavailable());
        }
        self.node.check_add_package(request, limits)?;
        self.require_sdk_current_data()
    }

    fn available_invite_slot(&self, nonce: [u8; 16]) -> Option<usize> {
        if self.installed_invites.contains(&Some(nonce)) {
            return None;
        }
        self.installed_invites.iter().position(Option::is_none)
    }

    /// Original immutable provenance, always the initial pin/source context.
    /// These historical labels do not describe the installed live SDK epoch.
    pub fn genesis_data(&self) -> V2GenesisSeedData<'_> {
        self.node.data()
    }

    /// Borrow the same retained original for Root's existing source check.
    /// The caller still proves the covering parent and live Current separately.
    pub fn borrow_genesis_seed_data(&self) -> &UnboundV2GenesisSeed {
        self.node.borrow_genesis_seed_data()
    }

    pub fn installed_add_data(&self) -> Option<V2InstalledFounderAddData> {
        self.installed
    }

    /// Full SDK source checks, OUTSIDE Core/final writers. The installed state
    /// exists only after this wrapper's exclusive exact-pair swap. Root must
    /// separately match its genuine covering receipt and selected Core prefix.
    pub fn require_sdk_current_data(&self) -> Result<(), SyncError> {
        self.stable_negative().require_live_data()?;
        let s = &self.node.seed.body;
        let spec = s.context.spec_data();
        let Some(installed) = self.installed else {
            return ordinary_create::source(&self.node).map_err(|_| unavailable());
        };
        if !s.group.is_active()
            || s.group.group_id_ref() != spec.group
            || !s.device.retained_ed25519_key_matches(&spec.signer)
            || !s.group.has_local_signer(&spec.signer)
            || !s.group.is_designated_committer(&s.device)
            || s.group.epoch() != installed.epoch
            || s.group.epoch_authenticator_id() != installed.authenticator
        {
            return Err(unavailable());
        }
        Ok(())
    }

    fn stable_negative(&self) -> V2FounderGovernanceNegative<'_> {
        V2FounderGovernanceNegative {
            source: self.node.seed.body.source,
            context: &self.node.seed.body.context,
            scope: &self.node.seed.body.metadata.scope,
            parts: StableParts {
                warm: self.node._parts.body.warm.as_ref(),
                fresh: self.node._parts.body.fresh.as_ref(),
            },
        }
    }

    /// Bind all THREE originally prepared native cuts to ONE original native source,
    /// retaining that observation with the independent payment and End batch.
    /// Install the exact originally prepared native cuts and independent
    /// MLS payment BEFORE admission. Root calls only after covering its exact
    /// request/parent reservation and reconciling ordinary-message custody.
    /// A live pending operation is never replaced by a new request or cuts.
    /// This allocates its named Box; call outside writers under a separate fee.
    pub fn prepare_add_data(
        &mut self,
        request: V2FounderAddRequestData<'_>,
        original: NativeChatSourceObservation,
        admission: TextCurrentAddAdmission,
        sdk_cut: NativeChatPreparedKeyCut,
        header_cut: NativeChatPreparedKeyCut,
        welcome_cut: NativeChatPreparedKeyCut,
    ) -> Result<(), V2FounderAddPreparationRefusal> {
        if self.pending.is_some()
            || self
                .available_invite_slot(request.invite.spec_data().nonce)
                .is_none()
            || !admission.supports_routing_data()
            || sdk_cut
                .require_never_entered_data(&original, NativeChatKeyPurpose::AddSdk)
                .is_err()
            || header_cut
                .require_never_entered_data(&original, NativeChatKeyPurpose::GoverningHeader)
                .is_err()
            || welcome_cut
                .require_never_entered_data(&original, NativeChatKeyPurpose::JoinWelcome)
                .is_err()
            || sdk_cut
                .preflight(&original, NativeChatKeyPurpose::AddSdk)
                .is_err()
            || header_cut
                .preflight(&original, NativeChatKeyPurpose::GoverningHeader)
                .is_err()
            || welcome_cut
                .preflight(&original, NativeChatKeyPurpose::JoinWelcome)
                .is_err()
            || self.require_sdk_current_data().is_err()
        {
            // Owning refusal is boxed outside writers under the same caller's
            // metadata allowance. Every original input is returned unchanged.
            return Err(Box::new((
                original,
                admission,
                sdk_cut,
                header_cut,
                welcome_cut,
            )));
        }
        let Ok(request) = FrozenAddRequest::prepare(&self.node.seed.body, self.installed, request)
        else {
            return Err(Box::new((
                original,
                admission,
                sdk_cut,
                header_cut,
                welcome_cut,
            )));
        };
        self.pending = Some(Box::new(PendingAdd {
            request,
            admitted_at_ms: None,
            cell: None,
            admission: Some(admission),
            body: None,
            unsigned: None,
            header: None,
            raw: None,
            welcome: None,
            retired_core: None,
            retired_sdk: None,
            phase: Phase::Prepared,
            original,
            sdk_cut: Some(sdk_cut),
            header_cut: Some(header_cut),
            welcome_cut: Some(welcome_cut),
            sdk_continuation_retired: false,
            sdk_native: NativeStageAdmission::new(),
            header_native: NativeStageAdmission::new(),
            welcome_native: NativeStageAdmission::new(),
            sdk_may_reauthorize: false,
            header_may_reauthorize: false,
            welcome_may_reauthorize: false,
            ends: NativeChatKeyEnds::new(),
        }));
        Ok(())
    }

    /// One finite secret snapshot/restore and genuine Add on the SAME private
    /// candidate. Native entry covers preparation too, not only SDK dispatch.
    /// An entered construction failure is sticky; it cannot reinstantiate the
    /// admission's cumulative counters. Returned Cell outputs are retained
    /// before late source/native checks. Never call while holding writers.
    pub fn execute_add_data_native(
        &mut self,
        request: V2FounderAddRequestData<'_>,
    ) -> Result<(), SyncError>
    where
        R: CryptoRngCore,
    {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.phase == Phase::Prepared)
        {
            self.require_sdk_current_data()?;
        }
        let p = self.pending.as_mut().ok_or_else(unavailable)?;
        p.request.require_same(request)?;
        if p.sdk_continuation_retired {
            return Err(unavailable()); // use the explicit cached DATA loan
        }
        if p.phase == Phase::Prepared {
            if !p.sdk_native.may_enter() {
                return Err(unavailable());
            }
            // Sample the ORIGINAL retained node clock, immediately before the
            // first native SDK entry. Prepared time and caller time cannot
            // extend the signed invitation or the actual package lifetime.
            let now_ms = self.node._parts.body._clock.now_ms();
            p.request.require_unexpired(now_ms)?;
            p.sdk_native.enter(
                p.sdk_cut.as_mut().ok_or_else(unavailable)?,
                &p.original,
                NativeChatKeyPurpose::AddSdk,
                &mut p.ends,
            )?;
            p.admitted_at_ms = Some(now_ms);
            p.phase = Phase::SdkEntered;
            #[cfg(test)]
            test_cut(TestCut::SdkEntered);
            PreparedTextCurrentAdd::prepare_with_routing_into(
                &self.node.seed.body.device,
                &self.node.seed.body.group,
                p.request.sdk_request(now_ms),
                &p.request.routing_plain,
                &mut p.admission,
                &mut p.cell,
            )
            .map_err(|_| unavailable())?;
        } else {
            // Constructor retries with an existing Cell compare EVERY request
            // field without decoding, restoring or drawing again. No fresh cut
            // or owning admission may be substituted after native entry.
            if p.cell.is_none() {
                return Err(unavailable());
            }
            PreparedTextCurrentAdd::prepare_with_routing_into(
                &self.node.seed.body.device,
                &self.node.seed.body.group,
                p.request
                    .sdk_request(p.admitted_at_ms.ok_or_else(unavailable)?),
                &p.request.routing_plain,
                &mut p.admission,
                &mut p.cell,
            )
            .map_err(|_| unavailable())?;
        }
        if p.phase == Phase::SdkEntered {
            #[cfg(test)]
            test_cut(TestCut::BeforeSdkExecution);
            self.finish_admitted_add_data_native()?;
        }
        // No late negative intervenes between real SDK outputs and their
        // owning Cell/phase above. Exact borrowed DATA remains recoverable.
        let p = self.pending.as_ref().ok_or_else(unavailable)?;
        p.sdk_cut
            .as_ref()
            .ok_or_else(unavailable)?
            .preflight(&p.original, NativeChatKeyPurpose::AddSdk)
            .map_err(|_| unavailable())?;
        self.require_sdk_current_data()
    }

    /// Historical completed Cell loan, including after native/source closure.
    /// No source permission, fresh SDK operation or owning secret copy escapes.
    pub fn completed_add_data(&self) -> Result<CompletedTextCurrentAddData<'_>, SyncError> {
        self.pending
            .as_ref()
            .and_then(|p| p.cell.as_ref())
            .ok_or_else(unavailable)?
            .completed_result_data()
            .map_err(|_| unavailable())
    }

    /// Exact unsigned Admission from the opaque REAL Add observation and the
    /// host's SAME selected verifier replay. This does not select that replay;
    /// Root supplies its current guard independently. No SDK re-snapshot occurs.
    pub fn prepare_admission_data(&mut self, replay: &ReplayMath) -> Result<(), SyncError> {
        self.require_sdk_current_data()?;
        let p = self.pending.as_mut().ok_or_else(unavailable)?;
        let result = p
            .cell
            .as_ref()
            .ok_or_else(unavailable)?
            .completed_result_data()
            .map_err(|_| unavailable())?;
        let change = result.membership_change_data();
        let spec = self.node.seed.body.context.spec_data();
        if replay.profile() != &self.node.seed.body.genesis.profile
            || replay.mls_epoch_data() != change.before_epoch()
            || replay.header().id()
                != self
                    .installed
                    .map_or(self.node.seed.body.genesis.header.id(), |s| s.header)
            || change.group() != spec.group
            || change.author() != &spec.signer
            || change.subject() != &p.request.spec.joiner
            || change.key_package_reference() != Some(p.request.spec.key_package_reference)
            || change.credential_digest() != Some(p.request.spec.credential)
            || change.before_epoch() != p.request.before_epoch
            || change.after_epoch() != p.request.after_epoch
            || replay.header().id() != p.request.parent
        {
            return Err(unavailable());
        }
        if let Some(unsigned) = &p.unsigned {
            // An exact retry never rebuilds the preimage. A changed selected
            // parent cannot reuse the original owner's signing permission.
            return if unsigned.header_spec_data().parent == replay.header().id() {
                Ok(())
            } else {
                Err(unavailable())
            };
        }
        if p.phase != Phase::Complete {
            return Err(unavailable());
        }
        let body = AdmissionBody::new(AdmissionSpec {
            group: spec.group.to_vec(),
            profile: replay.profile().id(),
            subject: *change.subject(),
            origin: AdmissionOrigin::Add {
                commit: change.commit_digest(),
                before_epoch: change.before_epoch(),
                after_epoch: change.after_epoch(),
                key_package: change.key_package_reference().ok_or_else(unavailable)?,
                credential: change.credential_digest().ok_or_else(unavailable)?,
            },
        })
        .map_err(|_| unavailable())?;
        let unsigned = replay
            .prepare_admission_data(&body)
            .map_err(|_| unavailable())?;
        p.body = Some(body);
        p.unsigned = Some(unsigned);
        Ok(())
    }

    /// Borrow immutable preimage/image/body. Root must durably cover these and
    /// the SAME completed SDK result BEFORE invoking the governing key edge.
    pub fn unsigned_admission_data(
        &self,
    ) -> Result<(&AdmissionBody, &UnsignedAdmissionStepData), SyncError> {
        let p = self.pending.as_ref().ok_or_else(unavailable)?;
        Ok((
            p.body.as_ref().ok_or_else(unavailable)?,
            p.unsigned.as_ref().ok_or_else(unavailable)?,
        ))
    }

    /// Borrow the exact jointly prepared outcome for the durable record.
    /// The signed Header can only follow that record, so this works before
    /// signing and continues to lend the same original bytes afterwards.
    pub fn outcome_data(&self) -> Result<V2FounderAddOutcomeData<'_>, SyncError> {
        let result = self.completed_add_data()?;
        let (admission, unsigned) = self.unsigned_admission_data()?;
        let request = &self.pending.as_ref().ok_or_else(unavailable)?.request;
        Ok(V2FounderAddOutcomeData {
            result,
            admission,
            unsigned,
            request,
        })
    }

    /// Final primitive only, after Root's genuine saved-outcome/current/Core
    /// proofs. Cache raw64 IMMEDIATELY; an unwind after entry with no cached raw
    /// stays terminal. A cached signature is historical DATA, not permission.
    pub fn sign_admission_native(&mut self) -> Result<(), SyncError> {
        self.signing_parts_data()?.1.sign_header_native()
    }

    /// Root can use the seed loan for its original Current while the second
    /// loan retains only this operation's fixed native signing state.
    pub fn signing_parts_data(
        &mut self,
    ) -> Result<(V2GenesisSourceFields<'_>, V2FounderAddSigning<'_>), SyncError> {
        let seed = &self.node.seed;
        let negative = V2FounderGovernanceNegative {
            source: seed.body.source,
            context: &seed.body.context,
            scope: &seed.body.metadata.scope,
            parts: StableParts {
                warm: self.node._parts.body.warm.as_ref(),
                fresh: self.node._parts.body.fresh.as_ref(),
            },
        };
        let pending = self.pending.as_deref_mut().ok_or_else(unavailable)?;
        Ok((
            seed.source_fields(),
            V2FounderAddSigning {
                seed,
                negative,
                pending,
            },
        ))
    }

    /// Borrow the exact cached key result for Root's RawHeader record. This
    /// remains available after a lost reply, before or after materialization.
    pub fn raw_admission_signature_data(&self) -> Result<&[u8; 64], SyncError> {
        self.pending
            .as_ref()
            .and_then(|p| p.raw.as_ref())
            .ok_or_else(unavailable)
    }

    /// Deterministic materialization outside writers. It never calls the key,
    /// including after a lost signing reply or a late cancellation.
    pub fn materialize_admission_data(&mut self) -> Result<(), SyncError> {
        let p = self.pending.as_mut().ok_or_else(unavailable)?;
        if p.header.is_some() {
            return Ok(());
        }
        let unsigned = p.unsigned.as_ref().ok_or_else(unavailable)?;
        let raw = p.raw.ok_or_else(unavailable)?;
        let header = SignedOwnerHeader::sign_using(unsigned.header_spec_data().clone(), |bytes| {
            if bytes == unsigned.signing_bytes_data() {
                Ok(raw)
            } else {
                Err(catcoms_replication::chat::ChatError::Dependency)
            }
        })
        .map_err(|_| unavailable())?;
        p.header = Some(header);
        p.phase = Phase::Materialized;
        Ok(())
    }

    pub fn signed_admission_data(&self) -> Result<&SignedOwnerHeader, SyncError> {
        self.pending
            .as_ref()
            .and_then(|p| p.header.as_ref())
            .ok_or_else(unavailable)
    }

    /// Ordinary-member source preflight outside Core/final writers. A pending
    /// Add cannot race a new message, but original member results owned by Root
    /// remain untouched. Root must reconcile them BEFORE preparing the Add.
    pub fn ordinary_member_signer_data(&self) -> Result<[u8; 32], V2MemberSignRefusal> {
        if self.pending.is_some() {
            return Err(V2MemberSignRefusal::Source);
        }
        self.require_sdk_current_data()
            .map_err(|_| V2MemberSignRefusal::Source)?;
        Ok(self.node.seed.body.context.spec_data().signer)
    }

    /// Allocation-free stable negatives under the held Core reader, followed
    /// by the shared bounded member-key tail. No SDK pair lookup occurs here.
    pub fn sign_ordinary_admitted_native<'a>(
        &self,
        inputs: &crate::owner_v2::V2MemberDraftInputs,
        guard: catcoms_replication::chat::V2OrdinarySigningGuard<'a>,
        original: &NativeChatSourceObservation,
        cut: &mut NativeChatPreparedKeyCut,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<catcoms_replication::chat::V2OrdinarySigningGuard<'a>, V2MemberSignRefusal> {
        if self.pending.is_some()
            || self.stable_negative().require_live_data().is_err()
            || guard.signer_data() != &self.node.seed.body.context.spec_data().signer
        {
            return Err(V2MemberSignRefusal::Source);
        }
        inputs.require_founder_origin(&self.node.seed)?;
        crate::owner_v2::member::sign_admitted_with_device_native(
            &self.node.seed.body.device,
            inputs,
            guard,
            crate::owner_v2::member::NativeMemberSigning {
                original,
                cut,
                ends,
            },
        )
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2FounderGovernanceNode<T, R> {
    /// Draw once through the SAME original runtime Clock/RNG and seed identity.
    /// A saved Add never changes the founder's ordinary Admission or pin.
    pub fn draw_ordinary_draft_inputs(
        &mut self,
        slot: &mut crate::owner_v2::V2MemberDraftInputs,
    ) -> Result<(u64, [u8; 16]), V2MemberSignRefusal> {
        self.ordinary_member_signer_data()?;
        slot.bind_founder_origin(&self.node.seed)?;
        if slot.enter_clock()? {
            slot.cache_time(self.node._parts.body._clock.now_ms());
        }
        self.ordinary_member_signer_data()?;
        if slot.enter_nonce()? {
            let mut nonce = [0; 16];
            self.node._parts.body._rng.fill_bytes(&mut nonce);
            slot.cache_nonce(nonce);
        }
        self.ordinary_member_signer_data()?;
        slot.drawn()
    }
}

struct StableParts<'a> {
    warm: Option<&'a WarmOrigin>,
    fresh: Option<&'a VerifiedFreshChatAdmission>,
}

/// Stable-field loan that can coexist with the EXCLUSIVE SDK installation.
/// It never reborrows the SDK pair, hashes, allocates or invokes a callback.
/// Root still requires its genuine saved/current/native proofs at final entry.
pub struct V2FounderGovernanceNegative<'a> {
    source: BootstrapSourceId,
    context: &'a BootstrapSourceContextData,
    scope: &'a LocalChatScope,
    parts: StableParts<'a>,
}
impl V2FounderGovernanceNegative<'_> {
    pub fn require_live_data(&self) -> Result<(), SyncError> {
        let (Some(warm), Some(fresh)) = (self.parts.warm, self.parts.fresh) else {
            return Err(unavailable());
        };
        let spec = self.context.spec_data();
        if warm.source != self.source
            || &warm.context != self.context
            || !self.scope.genesis_attachment_live()
            || !warm.scope.matches_genesis_attachment(self.scope)
            || !warm
                .fresh
                .matches_governed_origin_life(fresh, spec.group, &spec.signer)
        {
            return Err(unavailable());
        }
        Ok(())
    }
}

impl std::fmt::Debug for V2FounderGovernanceNegative<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2FounderGovernanceNegative")
            .finish_non_exhaustive()
    }
}

impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for PreparedV2FounderGovernanceNode<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2FounderGovernanceNode")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;

// Synchronous, thread-local cuts only. Detach before invocation so a hook may
// close the original lifecycle without re-entering a held hook RefCell.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestCut {
    NativeAdmissionAttempting,
    SdkEntered,
    BeforeSdkExecution,
    SdkComplete,
    HeaderEntered,
    RawCached,
    WelcomeEntered,
    WelcomeRawCached,
}
#[cfg(test)]
type TestHook = (TestCut, Box<dyn FnOnce()>);
#[cfg(test)]
std::thread_local! {
    static TEST_CUT: std::cell::RefCell<Option<TestHook>> =
        const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn test_cut(cut: TestCut) {
    let hook = TEST_CUT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|(wanted, _)| *wanted == cut) {
            slot.take()
        } else {
            None
        }
    });
    if let Some((_, hook)) = hook {
        hook();
    }
}
