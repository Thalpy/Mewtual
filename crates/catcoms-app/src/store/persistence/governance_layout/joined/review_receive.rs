//! Original physical ownership of a recipient review journal. Its visible
//! selector and contents are checked only by a paid review operation.
use super::*;
use crate::store::chat_admission::PhysicalMountNegative;
use crate::store::persistence::v2_member_outbox::fs::{
    read_bounded_held, require_exact_held, HeldDirectory,
};

mod declaration;
mod header;
mod index;
mod initial;
mod journal;
mod lane_head;
mod manifest;
mod objects;
mod plan;
mod transaction_check;
mod wal;
mod write;
pub(super) use declaration::{recover, require_no_claim_stage};
#[cfg(test)]
pub(in crate::store) use fault::{with_receiver_fault, ReceiverFault};
pub use journal::NativeTextReviewHeaderStatus;
#[cfg(test)]
mod fault;

pub(super) const MANIFEST_NAME: &str = "manifest.current";

pub(super) const MARKER_NAME: &str = "chat-critical-namespaces.review-receiver.marker";
pub(super) const DIRECTORY_NAME: &str = "chat-v2-review-received";

pub(super) fn require_absent(store: &ServerStore) -> Result<(), AppError> {
    require_no_claim_stage(store)?;
    absent(&fixed(store, false, MARKER_NAME)?)?;
    absent(&fixed(store, false, DIRECTORY_NAME)?)
}

pub(in crate::store) struct SelectedReviewReceiver {
    marker: OpenedArchiveObject,
    directory: HeldDirectory,
    mount: PhysicalMountNegative,
    _fee: CatalogReadReservation,
}

struct HeldManifest {
    path: PathBuf,
    file: (File, SessionId),
    bytes: Zeroizing<Vec<u8>>,
    data: manifest::ReceiverManifest,
}

impl ServerStore {
    pub(in crate::store) fn select_received_review_header(
        &self,
        binding: &catcoms_replication::chat::join_bindings::GenesisAnchorBindingData,
        incoming: &catcoms_replication::chat::owner_bodies::ReviewStreamEntry,
        require: impl FnMut(
            Option<catcoms_replication::chat::owner_bodies::ReviewStreamPosition>,
        ) -> Result<(), AppError>,
        rng: &mut dyn catcoms_rt::CryptoRngCore,
    ) -> Result<NativeTextReviewHeaderStatus, AppError> {
        let mounted = self
            .review_receive_mount
            .try_lock()
            .map_err(|error| match error {
                std::sync::TryLockError::WouldBlock => AppError::ChatV2ArchiveBusy,
                std::sync::TryLockError::Poisoned(_) => denied(),
            })?;
        mounted
            .as_ref()
            .ok_or_else(denied)?
            .select_header(self, binding, incoming, require, rng)
    }
}

impl HeldManifest {
    fn require_current(&mut self, root: &HeldDirectory) -> Result<(), AppError> {
        root.require()?;
        require_exact_held(&self.path, &mut self.file, &self.bytes)?;
        root.require()
    }
}

impl SelectedReviewReceiver {
    pub(super) fn capture(
        store: &ServerStore,
        descriptor: &[u8],
        key: &[u8; 32],
    ) -> Result<Self, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy.current.archive.pending.pool.try_reserve(16 * 1024)?;
        let expected = critical_namespaces::review_receive_namespace::marker_data(key, descriptor)?;
        let marker =
            read(&fixed(store, false, MARKER_NAME)?, expected.len())?.ok_or_else(denied)?;
        if marker.1.as_slice() != expected {
            return Err(denied());
        }
        let directory = HeldDirectory::open(fixed(store, false, DIRECTORY_NAME)?)?;
        let captured = Self {
            marker,
            directory,
            mount: store.capture_physical_mount_negative(),
            _fee: fee,
        };
        captured.require(store)?;
        Ok(captured)
    }

    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_original_physical_mount(&self.mount)?;
        self.directory.require()?;
        let (marker, bytes) = &self.marker;
        exact_held(
            &fixed(store, false, MARKER_NAME)?,
            marker,
            session_id(marker).map_err(io)?,
            bytes,
        )?;
        self.directory.require()
    }

    /// The initial manifest was selected during declaration. Lost or
    /// damaged files cannot recreate the empty state on this path.
    fn read_manifest(
        &self,
        store: &ServerStore,
        key: &manifest::ManifestKey,
    ) -> Result<HeldManifest, AppError> {
        self.require(store)?;
        let path = self.directory.child(MANIFEST_NAME);
        let (file, identity, bytes) =
            read_bounded_held(&path, manifest::MAX_MANIFEST_BYTES, false)?;
        let data = key.open(&bytes)?;
        let mut manifest = HeldManifest {
            path,
            file: (file, identity),
            bytes,
            data,
        };
        manifest.require_current(&self.directory)?;
        self.require(store)?;
        Ok(manifest)
    }
}
