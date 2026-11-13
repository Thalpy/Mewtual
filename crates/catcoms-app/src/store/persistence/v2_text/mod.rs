//! Operational text persistence DATA and immutable candidates.
//!
//! Declared with the paired original Current, resource and first-promotion work consumers.
//! A decoder or sealed candidate here supplies no selection, scratch disposal, deletion,
//! signing or cold-resume permission. Ordinary posting and later-head recovery are separate.

mod accepted_query;
mod archive;
mod artifacts;
mod capture;
mod common;
mod context;
mod control;
mod head;
mod index;
mod io;
mod ordinary_data;
mod private_index;
mod promotion;
mod query;
mod records;
mod request;
mod resources;
mod retain;
mod retirement;
mod transaction;
mod wal;
mod workspace;

pub(in crate::store) use accepted_query::{
    CapturedV2TextAcceptedQuery, OriginalV2TextAcceptedQueryObservation,
    PreparedV2TextAcceptedQuery, V2TextAcceptedQueryCaptureClaim, V2TextAcceptedQueryReaderClaim,
    V2TextAcceptedQueryResources, V2TextAcceptedQueryResultData,
};
#[cfg(all(test, windows))]
pub(in crate::store) use archive::TextArchiveData;
#[cfg(all(test, windows))]
pub(in crate::store) use artifacts::TextArtifactRoleData;
pub(in crate::store) use capture::{PreparedV2TextParentCapture, SelectedV2TextHead};
pub(in crate::store) use capture::{V2TextFounderAnchorData, V2_TEXT_FOUNDER_ANCHOR_LAYOUT_BYTES};
pub(in crate::store) use common::{
    TextContextIdData, TextGenesisParentIdData, TextHeadIdData, TextRetirementPlanIdData,
    TextTransactionIdData,
};
pub(in crate::store) use context::{TextContextData, TextContextSpecData, MAX_CONTEXT_PLAIN};
pub(in crate::store) use head::{TextPendingData, TextRootData};
pub(in crate::store) use ordinary_data::{
    encode_unsigned_intent_data, OriginalV2RetainObservation, PreparedV2RetainData,
    PreparedV2RetainOriginalCustody, TextUnsignedIntentBindingsData, TextUnsignedIntentFrameData,
    V2TextRetainWorkspaceClaim, V2TextTransactionMetadataResources, TEXT_TRANSACTION_METADATA_FEE,
};
#[cfg(all(test, any(windows, target_os = "linux")))]
pub(in crate::store) use promotion::V2TextPromotionTestCut;
pub(in crate::store) use promotion::{
    CapturedV2TextPromotionScratch, CapturedV2TextPromotionSeedRepair,
    OriginalV2TextPromotionObservation, PreparedV2TextPromotion, PreparedV2TextPromotionCapture,
    PreparedV2TextPromotionScratch, PreparedV2TextPromotionSeedRepair,
    ReadyV2TextPromotionSelection, SavedV2TextPromotion, V2TextPromotionRecoveryClassData,
    V2TextPromotionRecoveryResources, V2TextQuotaPolicy,
};
pub(in crate::store) use query::{
    CapturedV2TextTokenQuery, OriginalV2TextTokenQueryObservation, PreparedV2TextTokenQuery,
    V2TextTokenQueryCaptureClaim, V2TextTokenQueryReaderClaim, V2TextTokenQueryResources,
};
pub(in crate::store) use request::TextRequestDigestData;
#[cfg(all(test, windows))]
pub(in crate::store) use request::TextTokenKeyData;
pub(in crate::store) use resources::V2TextReadResources;
#[cfg(all(test, windows))]
pub(in crate::store) use transaction::V2TextRetainTestCut;
pub(in crate::store) use transaction::V2_TEXT_RETAIN_ROOT_PHASE_BYTES;
pub(in crate::store) use transaction::{
    OriginalV2TextRetainTransactionObservation, PreparedV2TextRetainTransaction,
    ReadyV2TextRetainSelection, SavedV2RetainedAction,
};
pub(in crate::store) use workspace::{
    CapturedV2TextWorkspace, PreparedV2TextWorkspaceCapture, ReadyV2TextWorkspaceClear,
    V2TextWorkspaceCaptureClaim,
};

#[cfg(all(test, windows))]
pub(in crate::store) use retirement::{
    RetirementCarryData, RetirementEntryData, RetirementKindData,
};
#[cfg(all(test, windows))]
pub(in crate::store) use wal::{TextWalFrameData, TextWorkWalCleanupData};

#[cfg(all(test, windows))]
pub(in crate::store) use transaction::V2TextRetainRecoveryTestCut;
pub(in crate::store) use transaction::{
    CapturedV2TextRetainRecovery, OriginalV2TextRetainRecoveryObservation,
    PreparedV2TextRetainRecovery, ReadyV2TextRetainRepair, ReadyV2TextRetainScratch,
    V2TextRetainRecoveryCaptureClaim, V2TextRetainStageFence, V2TextRetainWarmRecoveryClassData,
};

pub(in crate::store) use transaction::RetainedAQuerySourceData;

#[cfg(all(test, windows))]
pub(in crate::store) use transaction::V2TextIntentTestCut;
pub(in crate::store) use transaction::{
    OriginalV2TextUnsignedIntentObservation, PreparedV2TextUnsignedIntent,
    ReadyV2TextUnsignedIntentSelection, SavedV2UnsignedAppendIntent,
};

#[cfg(all(test, windows))]
pub(in crate::store) use transaction::V2TextRawSignatureTestCut;
pub(in crate::store) use transaction::{
    OriginalV2TextRawSignatureObservation, PreparedV2TextRawSignature,
    ReadyV2TextRawSignatureSelection, SavedV2RawAppendSignature,
};

#[cfg(all(test, windows))]
pub(in crate::store) use transaction::V2TextAcceptedCreateTestCut;
pub(in crate::store) use transaction::{
    OriginalV2TextAcceptedCreateObservation, PreparedV2TextAcceptedCreate,
    ReadyV2TextAcceptedCreateSelection, SavedV2AcceptedCreate,
};

pub(in crate::store) use transaction::V2TextRetirementRootsClaim;

#[cfg(all(test, windows))]
pub(in crate::store) use transaction::V2TextRetirementTestCut;
pub(in crate::store) use transaction::{
    ReadyV2TextRetirementCompletion, SavedV2TextRetirementCompletion, SavedV2TextRetirementDeletion,
};

pub(in crate::store) use transaction::CompletedV2TextRetirementSourceData;

mod completed_workspace;
pub(in crate::store) use completed_workspace::{
    CapturedV2TextCompletedWorkspace, CompletedV2TextParentData,
    OriginalV2TextCompletedWorkspaceObservation, PreparedV2TextCompletedWorkspaceCapture,
    ReadyV2TextCompletedWorkspace, V2TextCompletedWorkspaceCaptureClaim,
};
pub(in crate::store) use transaction::NextRetainedAResidentSourceData;
