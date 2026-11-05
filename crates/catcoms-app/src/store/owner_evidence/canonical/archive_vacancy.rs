//! Negative shared old-owner namespace custody for the actual new archive founder. Holding
//! this guard never installs a v1 head/pin or grants Current. It prevents old head reservations
//! racing the original admission/target-marker transaction, without requiring checkpoint budget.
use super::*;
use crate::store::chat_admission::predecessor::archive_founder::ArchiveFounderAdmission;
use crate::store::critical_namespaces::archive_bootstrap::founder_codec as codec;
use crate::store::persistence::CapturedFounderInvocation;
use catcoms_rt::{CryptoRngCore, MeshTransport};
use std::sync::MutexGuard;

/// Private fields require BOTH real existing owner locks. No scalar absence/status constructor
/// exists. All callers already hold admission; order is admission -> pins -> canonical pool ->
/// archive root. Existing pin-only operations never wait for admission while holding pins.
pub(in crate::store) struct ArchiveVacantOwnerGuard<'writer, T: MeshTransport, R: CryptoRngCore> {
    // This actual borrow is essential: pins/pool can NEVER outlive the original held
    // admission writer. Taking only &ServerStore here would allow its callback to return the
    // subordinate guards, dropping admission first and inverting the next lock acquisition.
    admission: &'writer ArchiveFounderAdmission<'writer, T, R>,
    pins: MutexGuard<'writer, super::super::pins::Runtime>,
    pool: MutexGuard<'writer, Runtime>,
    identifier: [u8; 32],
}
impl<T: MeshTransport, R: CryptoRngCore> ArchiveVacantOwnerGuard<'_, T, R> {
    /// Original actual creator/source validation while the physical writer ALREADY owns Root.
    /// Never call validate_original there: it intentionally checks Root by taking that mutex.
    /// This cannot reacquire admission/pins/canonical; their actual borrows remain in self.
    pub(in crate::store) fn validate_source_only(
        &self,
        captured: &CapturedFounderInvocation,
    ) -> Result<(), AppError> {
        self.admission.validate_source_only(captured)?;
        self.validate_owned_namespace()
    }
    pub(in crate::store) fn plan_data(&self) -> &codec::FoundingData {
        self.admission.plan_data()
    }
    pub(in crate::store) fn debit_id(&self) -> codec::DebitId {
        self.admission.debit_id()
    }
    pub(in crate::store) fn validate_original(&self) -> Result<(), AppError> {
        let admission = self.admission;
        admission.validate_original()?;
        self.validate_owned_namespace()
    }
    /// Read the SAME already-owned namespace, without taking any further writer/Root mutex.
    /// Physical persistence repeats this after file waits while holding Root; its own final
    /// root/session checks bracket this filesystem observation. Presence never means absence.
    fn validate_owned_namespace(&self) -> Result<(), AppError> {
        let admission = self.admission;
        if self.identifier != key(admission.group(), admission.signer())
            || !self.pins.managed
            || self.pins.dirty
            || self
                .pins
                .table
                .as_ref()
                .ok_or_else(refused)?
                .contains_key(admission.group())
            || self.pool.heads.contains_key(&self.identifier)
        {
            return Err(refused());
        }
        // A missing configured checkpoint budget does NOT establish disk absence. The pool
        // can still be un-inventoried at this point; inspect BOTH exact old fixed destinations
        // under these original locks. Never call owner_canonical_directory here: it can mkdir.
        let parent = admission.store().dir.join("owner-state");
        match fs::symlink_metadata(&parent) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Ok(metadata) if real_directory(&metadata) => {}
            _ => return Err(refused()),
        }
        let base = hex::encode(self.identifier);
        for suffix in [".bin", ".replace.tmp"] {
            match fs::symlink_metadata(parent.join(format!("{base}{suffix}"))) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(refused()),
            }
        }
        Ok(())
    }
}
fn real_directory(metadata: &fs::Metadata) -> bool {
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        true
    }
}
impl ServerStore {
    pub(in crate::store) fn lock_archive_vacant_owner_namespace<
        'writer,
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        &'writer self,
        admission: &'writer ArchiveFounderAdmission<'writer, T, R>,
    ) -> Result<ArchiveVacantOwnerGuard<'writer, T, R>, AppError> {
        admission.validate_original()?;
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        let pins = self.owner_evidence.pins.lock().map_err(|_| refused())?;
        let pool = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        let guard = ArchiveVacantOwnerGuard {
            admission,
            pins,
            pool,
            identifier: key(admission.group(), admission.signer()),
        };
        guard.validate_original()?;
        Ok(guard)
    }
}

#[cfg(test)]
pub(in crate::store) fn old_key_for_test(group: &[u8], signer: &[u8; 32]) -> [u8; 32] {
    key(group, signer)
}
#[cfg(test)]
#[derive(Clone, Copy)]
pub(in crate::store) enum MetadataConflict {
    Head,
    Pin,
}
/// Test-only NEGATIVE corruption/reservation fixture. No method yields a positive saved head,
/// profile binding or Current; even unknown old metadata must deny new-factory vacancy.
#[cfg(test)]
pub(in crate::store) fn install_conflicting_metadata_for_test(
    store: &ServerStore,
    group: &[u8],
    signer: &[u8; 32],
    conflict: MetadataConflict,
) {
    match conflict {
        MetadataConflict::Head => {
            let identifier = key(group, signer);
            store
                .owner_evidence
                .canonical
                .lock()
                .unwrap()
                .heads
                .insert(identifier, Arc::new(super::head::HeadCell::new(identifier)));
        }
        MetadataConflict::Pin => {
            store
                .owner_evidence
                .pins
                .lock()
                .unwrap()
                .table
                .as_mut()
                .unwrap()
                .insert(
                    group.to_vec(),
                    super::super::pins::Pin {
                        founder: *signer,
                        profile: OwnerFactId::from_bytes([71; 32]),
                        origin: super::super::pins::Origin::TrustedHost,
                        source: [72; 32],
                    },
                );
        }
    }
}
