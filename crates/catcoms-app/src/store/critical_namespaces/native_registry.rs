//! Independent native backend registry, critical namespace7/version1.
//!
//! The DATA codecs below confer no row, Source, runtime or native authority.
//! Physical declaration/selection and the checked native producer are separate
//! consumers. No default opener may infer support from decoded descriptor bytes.
pub(in crate::store::critical_namespaces) mod capability;
mod data;
// Private index3 contract under construction. No physical opener supports it
// yet; registering this module must never widen the closed v1/v2 dispatch.
mod lifecycle;
pub(in crate::store::critical_namespaces) mod operational;
pub(in crate::store) mod preparation;

pub use operational::{
    NativeCreationStateData, NativeCreationStatusData, NativeIndexPolicy, NativeIndexStore,
    NativeIndexUpgrade, NativeTextUiCandidate, NativeTextUiGarbage, NativeTextUiObservation,
    NativeTextUiPreparation, NativeTextUiRetirement, NativeTextUiRetirementBatch,
    NativeTextUiRetirementState, NativeTextVault, NativeUiInvocationBudget,
    NativeUiInvocationMetadata,
};
pub use preparation::{
    NativeRegistryInstallation, NativeRegistryPolicy, NativeRegistryPreparationPolicy,
    NativeRegistryStore, PreparedNativeRegistryMigration,
};
