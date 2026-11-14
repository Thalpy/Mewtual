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
// Named full invoice shared with the closed physical reader. Publishing its
// size grants no policy, reserve, handler or physical/native authority.
pub(in crate::store) const NATIVE_INDEX3_FULL_FEE: usize = lifecycle::preparation::PREPARATION_FEE;
pub(in crate::store::critical_namespaces) mod operational;
pub(in crate::store) mod preparation;

pub use operational::{
    NativeCreationStateData, NativeCreationStatusData, NativeIndexPolicy, NativeIndexStore,
    NativeIndexUpgrade, NativeTextUiCandidate, NativeTextUiGarbage, NativeTextUiObservation,
    NativeTextUiPreparation, NativeTextUiRetirement, NativeTextUiRetirementBatch,
    NativeTextUiRetirementState, NativeTextVault, NativeTextVaultAuthentication,
    NativeTextVaultUpgrade, NativeTextVaultUpgradeRefusal, NativeUiInvocationBudget,
    NativeUiInvocationMetadata,
};
pub use preparation::{
    NativeRegistryInstallation, NativeRegistryPolicy, NativeRegistryPreparationPolicy,
    NativeRegistryStore, PreparedNativeRegistryMigration,
};
