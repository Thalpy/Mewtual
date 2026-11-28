//! Independent native backend registry, critical namespace7/version1.
//!
//! The DATA codecs below confer no row, Source, runtime or native authority.
//! Physical declaration/selection and the checked native producer are separate
//! consumers. No default opener may infer support from decoded descriptor bytes.
pub(in crate::store::critical_namespaces) mod capability;
mod data;
// The private owning3 upgrade/strict opener selects its concrete handler;
// ordinary openers and the closed v1/v2 dispatch gain no implicit support.
pub(in crate::store::critical_namespaces) mod lifecycle;
pub(in crate::store) use lifecycle::preparation::first_torn::VerifiedFirstTorn as VerifiedNativeIndex3FirstTorn;
pub(in crate::store) use lifecycle::preparation::VerifiedColdCatalog as VerifiedNativeIndex3ColdCatalog;
pub(in crate::store) use lifecycle::wal::{
    Decision as NativeIndex3WalDecision, VerifiedWal as VerifiedNativeIndex3Wal,
};
pub(in crate::store) use lifecycle::{
    verify_joined_pin_parent, verify_shared_history_parent, PreparedNativeRejoinIndex,
    VerifiedFounderReviewParent, VerifiedGovernanceNative,
};
// Named full invoice shared with the closed physical reader. Publishing its
// size grants no policy, reserve, handler or physical/native authority.
pub(in crate::store) const NATIVE_INDEX3_FULL_FEE: usize = lifecycle::preparation::PREPARATION_FEE;
pub(in crate::store::critical_namespaces) mod operational;
pub(in crate::store) mod preparation;

pub use lifecycle::joined::NativeTextJoinedRegistryRowData;
pub use operational::NativeTextFounderDependencyProvider;
pub use operational::NativeTextInvitePublication;
pub use operational::NativeTextJoinedRequestPreparation;
pub use operational::NativeTextJoinedRowSelection;
pub use operational::NativeTextJoinedVaultUpgrade;
pub use operational::{
    NativeCreationStateData, NativeCreationStatusData, NativeIndexPolicy, NativeIndexStore,
    NativeIndexUpgrade, NativeTextCreationBudget, NativeTextCreationInputs, NativeTextCreationJob,
    NativeTextCreationPreparation, NativeTextCreationProgress, NativeTextCreationRefusal,
    NativeTextCreationRequest, NativeTextCreationSettings,
    NativeTextFounderMemberIngressPreparation, NativeTextFounderMemberPublication,
    NativeTextFounderMemberPublicationReceipt, NativeTextFounderMemberReceiver,
    NativeTextGovernanceUpgrade, NativeTextGovernedStorage, NativeTextHistoryResponseBudget,
    NativeTextHistoryResponseCredit, NativeTextJoinedStorage, NativeTextMemberAdmissionFailure,
    NativeTextMemberBudget, NativeTextMemberFinalPreparation, NativeTextMemberHistoryItem,
    NativeTextMemberHistoryPage, NativeTextMemberHistoryStatus, NativeTextMemberJob,
    NativeTextMemberPreparation, NativeTextMemberSaveJob, NativeTextRuntimeSelection,
    NativeTextSourceBudget, NativeTextSourceCommand, NativeTextSourcePolicy,
    NativeTextSourcePreparation, NativeTextSourceRetirement, NativeTextSourceRetirementPreparation,
    NativeTextSourceRetirementRefusal, NativeTextSourceRetirementStatus, NativeTextSources,
    NativeTextUiAddRetirement, NativeTextUiCandidate, NativeTextUiGarbage, NativeTextUiObservation,
    NativeTextUiPreparation, NativeTextUiRetirement, NativeTextUiRetirementBatch,
    NativeTextUiRetirementDrain, NativeTextUiRetirementState, NativeTextVault,
    NativeTextVaultAuthentication, NativeTextVaultAuthenticationCheck, NativeTextVaultUpgrade,
    NativeTextVaultUpgradeRefusal, NativeTextWorkRetirement, NativeTextWorkRetirementPreparation,
    NativeUiInvocationBudget, NativeUiInvocationMetadata, NativeUiStateBudget, NativeUiStateCredit,
    NativeUiStatePreparation, NativeUiStateRead, NativeUiStateSaved,
};
pub use operational::{
    NativeTextColdFounderGoverningRequest, NativeTextColdFounderGoverningRequests,
    NativeTextColdFounderJoinRequest, NativeTextColdFounderJoinRequests,
    NativeTextColdFounderMemberIngress, NativeTextColdFounderMemberReceiver,
    NativeTextColdFounderPublication, NativeTextColdFounderPublicationReceipt,
    NativeTextColdFounderSource, NativeTextFounderReviewExchange, NativeTextFounderReviewPage,
    NativeTextFounderReviewReply, NativeTextFounderReviewResult, NativeTextFounderReviewSweep,
};
pub use operational::{
    NativeTextJoinedArchiveDependency, NativeTextJoinedArchiveReplay,
    NativeTextJoinedArchivedActions, NativeTextJoinedDependencyAttachRefusal,
    NativeTextJoinedDependencyExchange, NativeTextJoinedDependencyProvider,
    NativeTextJoinedDependencyResponse, NativeTextJoinedGoverningExchange,
    NativeTextJoinedGoverningIncoming, NativeTextJoinedGoverningProvider,
    NativeTextJoinedGoverningResponse, NativeTextJoinedHistoryExchange,
    NativeTextJoinedHistoryPage, NativeTextJoinedHistoryResponse,
    NativeTextJoinedMemberIngressPreparation, NativeTextJoinedMemberLedger,
    NativeTextJoinedMemberProjection, NativeTextJoinedMemberPublication,
    NativeTextJoinedMemberPublicationReceipt, NativeTextJoinedMemberReceiver,
    NativeTextJoinedReceivedOriginal, NativeTextJoinedReviewExchange,
    NativeTextJoinedReviewResponse, NativeTextJoinedSelectionPreparation,
};
pub use operational::{
    NativeTextJoinedContactApprovalProof, NativeTextJoinedContactConsent,
    NativeTextJoinedReconnect, NativeTextJoinedSavedContactApprovals,
};
pub use operational::{
    NativeTextJoinedDelivery, NativeTextJoinedExchange, NativeTextJoinedInstallation,
};
pub use operational::{NativeTextJoinedMemberFinalPreparation, NativeTextJoinedMemberSaveJob};
pub use operational::{
    NativeTextJoinedRecovery, NativeTextJoinedRuntime, NativeTextRejoinDelivery,
    NativeTextRejoinExchange, NativeTextRemovedJoinedVault,
};
pub use preparation::{
    NativeRegistryInstallation, NativeRegistryPolicy, NativeRegistryPreparationPolicy,
    NativeRegistryStore, PreparedNativeRegistryMigration,
};
