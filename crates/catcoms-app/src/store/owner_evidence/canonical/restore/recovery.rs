//! Original admission -> pool/head custody for source-absent negative recovery.
//!
//! No untrusted scalar can stand in for this check. The admission child calls this while its
//! actual row writer is held, then retains both writers through the real retirement save.
use super::*;
use crate::store::ChatAdmissionKey;

/// Only the admission child chooses the negative cleanup effect, after its exact row save.
/// This is not a Current/publication witness or a caller-selected storage authorization flag.
pub(in crate::store) enum UnexposedHeadAction<T> {
    Observe(T),
    Retired(T),
}

impl ServerStore {
    /// `None` means preserve a live/pending/current head or an existing canonical/staged source.
    /// The synchronous callback cannot run unless both fixed files are absent under the SAME
    /// original serialized namespace. No files are removed or bytes reinterpreted as authority.
    pub(in crate::store) fn with_unexposed_owner_head<T>(
        &self,
        identity: &ChatAdmissionKey,
        finish: impl FnOnce() -> Result<UnexposedHeadAction<T>, AppError>,
    ) -> Result<Option<T>, AppError> {
        // Do not call chat_vault_format here: that would recursively take the admission writer
        // already held by our sole trusted caller. It checks managed OwnerEvidence beforehand.
        let identifier = key(identity.group(), identity.signer());
        let mut pool = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        let _configured = pool.budget.ok_or_else(refused)?;
        let cell = pool.heads.get(&identifier).cloned();
        let head = cell.as_ref().map(|cell| cell.lock()).transpose()?;
        if head
            .as_ref()
            .is_some_and(|head| !head.is_unexposed_and_unowned())
        {
            return Ok(None);
        }
        // This settles the actual root entry AND leaf directory durability, including a prior
        // visible mkdir/unlink whose parent flush failed. Existence alone is not that barrier.
        let directory = self.owner_canonical_directory()?;
        for extension in ["bin", "replace.tmp"] {
            let path = directory.join(format!("{}.{extension}", hex::encode(identifier)));
            match fs::symlink_metadata(path) {
                Ok(_) => return Ok(None), // preserve even unsupported/malformed/link-shaped data
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::Io(error.to_string())),
            }
        }
        // The pool guard prevents first head registration while the original row save runs.
        // A negative mount revocation takes neither writer and therefore must be reread here.
        if self.chat_mount.observer().is_cancelled() {
            return Err(refused());
        }
        let result = finish()?;
        let (output, retired) = match result {
            UnexposedHeadAction::Observe(output) => (output, false),
            UnexposedHeadAction::Retired(output) => (output, true),
        };
        drop(head);
        if retired {
            // Only the empty no-file slot is released. The non-evicting admission identity and
            // its Retired nonce remain sealed; retained source/history is never a cleanup target.
            pool.heads.remove(&identifier);
        }
        Ok(Some(output))
    }
}
