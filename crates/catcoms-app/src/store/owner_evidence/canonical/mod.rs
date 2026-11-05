//! The single canonical private owner head, independent of numeric server aliases.
//!
//! All mutation uses admission -> head -> synchronous original Store I/O. A current scope alone
//! cannot overwrite this source or turn an old private candidate into the current owner. The
//! CHECKPOINT-1 budget below remains explicitly configured and nonactivatable for production.
use super::*;
use crate::store::chat_admission::{ChatAdmissionStorageMode, ChatAdmissionStoragePermit};
use crate::store::{ChatAdmissionBinding, ChatAdmissionPreparation};
use catcoms_sync::local_chat_scope::{ManagedChatScopeBinding, ManagedChatScopeReservation};
use std::sync::{Arc, MutexGuard, Weak};
use tokio::sync::watch;

mod budget;
mod head;
mod initialize;
mod inventory;
mod joined;
pub use joined::{
    OwnerCheckpointJoinAttempt, OwnerDirectCheckpointJoinAttempt, OwnerJoinedInitialization,
};
mod restore;
mod source;
pub use budget::OwnerEvidenceBudget;
pub(crate) use budget::{
    OwnerInitializationReservation, OwnerInitializationSnapshotLimits, OwnerOperationReservation,
    OwnerPendingCreation,
};
pub(crate) use head::{
    OwnerHeadWriteGuard, OwnerWriterRegistration, SavedOwnerInitialization, SavedOwnerRestore,
    SavedOwnerSuccessor,
};
pub use initialize::{OwnedOwnerServer, OwnerFounderInitialization};
pub(in crate::store) use restore::UnexposedHeadAction;
pub use restore::{
    OfflineOwnerEvidence, OwnerRestoreClassification, OwnerRestoreInitialization,
    OwnerRestorePreparation, OwnerUnexposedInitialization, OwnerUnexposedRetirement,
};
pub(in crate::store) use source::OwnerSourceLocation;
pub(crate) use source::ValidatedOwnerSnapshot;

const MAX_SNAPSHOT_BYTES: usize = 144 * 1024 * 1024;
const MAX_MLS_BYTES: usize = 8 * 1024 * 1024;
const MAX_MLS_BACKING_BYTES: usize = 16 * 1024 * 1024;
const WRAPPER_FEE: usize = 1024;
const SEALED_FEE: usize = 40;
const MAX_HEADS: usize = 65_536;

#[derive(Default)]
pub(super) struct Runtime {
    budget: Option<OwnerEvidenceBudget>,
    heads: BTreeMap<[u8; 32], Arc<head::HeadCell>>,
    // Includes the ONE retained frozen candidate and every transient encoding/sealing buffer.
    // A reservation owns this fee until successful transfer or explicit attempt drop.
    hot_bytes: usize,
    pending_creations: usize,
}

fn refused() -> AppError {
    AppError::Invalid("canonical owner state is unavailable or unsupported".into())
}

fn key(group: &[u8], signer: &[u8; 32]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/owner-canonical-key/v1\0");
    hash.update(&(group.len() as u32).to_be_bytes());
    hash.update(group);
    hash.update(signer);
    *hash.finalize().as_bytes()
}
