//! Optional authenticated audit evidence. A writer calls this only after
//! covering the selected body and its directory, with the exact held file.
use super::*;
use std::path::PathBuf;

pub(super) struct Witnesses {
    directory: HeldDirectory,
    marker: [u8; 32],
}

pub(super) struct WitnessRead {
    path: PathBuf,
    file: (File, SessionId),
    bytes: [u8; 32],
}

impl WitnessRead {
    pub(super) fn require(&mut self) -> Result<(), AppError> {
        require_exact_held(&self.path, &mut self.file, &self.bytes)
    }
}

impl Witnesses {
    pub(super) fn new(directory: HeldDirectory, marker: [u8; 32]) -> Self {
        Self { directory, marker }
    }

    pub(super) fn original_directory(&self) -> Result<HeldDirectory, AppError> {
        self.directory.try_clone()
    }

    pub(super) fn cover(
        &self,
        scope: &HoldScope,
        vault_key: &[u8; 32],
        case: ReviewCaseId,
        frame: &[u8],
    ) -> Result<(), AppError> {
        self.directory.require()?;
        let label = scope.file_label(vault_key, case);
        let path = self
            .directory
            .child(&format!("{}.receipt", hex::encode(label)));
        let bytes = self.content(scope, vault_key, label, frame);
        let mode = match observe(&path)? {
            Presence::Absent => WriteMode::Create,
            Presence::Present => WriteMode::Existing,
        };
        let mut witness = open_exact_write(&path, mode)?;
        // An exact prefix can be completed after a crash. An incompatible
        // witness is retained without rewriting or approving the case.
        finish_exact_write(&path, &mut witness, &bytes)?;
        self.directory.sync()?;
        require_exact_held(&path, &mut witness, &bytes)?;
        self.directory.require()
    }

    /// An observed complete witness proves the body barriers preceded its
    /// creation. The audit never repairs a missing or incomplete witness.
    pub(super) fn require_selected(
        &self,
        scope: &HoldScope,
        vault_key: &[u8; 32],
        label: [u8; 32],
        frame: &[u8],
    ) -> Result<WitnessRead, AppError> {
        self.directory.require()?;
        let path = self
            .directory
            .child(&format!("{}.receipt", hex::encode(label)));
        let expected = self.content(scope, vault_key, label, frame);
        let (file, native, bytes) = read_bounded_held(&path, expected.len(), false)?;
        if bytes.as_slice() != expected {
            return Err(refused());
        }
        let mut held = WitnessRead {
            path,
            file: (file, native),
            bytes: expected,
        };
        held.require()?;
        self.directory.require()?;
        Ok(held)
    }

    fn content(
        &self,
        scope: &HoldScope,
        vault_key: &[u8; 32],
        label: [u8; 32],
        frame: &[u8],
    ) -> [u8; 32] {
        let key = Zeroizing::new(blake3::derive_key(
            "catcoms/private-chat-v2-removed-create-selection-key/v1",
            &*scope.key(vault_key),
        ));
        let mut hash = blake3::Hasher::new_keyed(&key);
        hash.update(&self.marker);
        hash.update(&label);
        hash.update(blake3::hash(frame).as_bytes());
        *hash.finalize().as_bytes()
    }
}
