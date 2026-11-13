//! Independent native backend registry, critical namespace7/version1.
//!
//! The DATA codecs below confer no row, Source, runtime or native authority.
//! Physical declaration/selection and the checked native producer are separate
//! consumers. No default opener may infer support from decoded descriptor bytes.
mod capability;
mod data;
pub(in crate::store) mod preparation;

pub use preparation::{NativeRegistryPreparationPolicy, PreparedNativeRegistryMigration};
