//! Flat selected B facts, funded by NEW Bmetadata, independent of retired predecessors.
//! All paths/Files/body comparisons are prepared before final original Source/ID3;
//! afterward only SAME-held content/name checks and cached negatives remain.
use super::named::AcceptedNamedFiles;
use super::*;
pub(super) struct AcceptedArtifactFact {
    pub(super) path: PathBuf,
    pub(super) native: SessionId,
    pub(super) frame: Zeroizing<Vec<u8>>,
}
pub(super) struct AcceptedSelectedFacts {
    pub(super) target: PathBuf,
    pub(super) target_id: SessionId,
    pub(super) pages: PathBuf,
    pub(super) pages_id: SessionId,
    pub(super) head_path: PathBuf,
    pub(super) head_native: SessionId,
    pub(super) selected: Zeroizing<Vec<u8>>,
    pub(super) artifacts: Vec<AcceptedArtifactFact>,
    pub(super) context: TextContextData,
    pub(super) header: SignedOwnerHeader,
    #[cfg(all(test, windows))]
    pub(super) drop_notice: AcceptedFactsDropNotice,
    pub(super) resources: V2TextReadResources,
    pub(super) metadata: AcceptedMetadataResources,
}
impl AcceptedSelectedFacts {
    fn capture_artifacts<'g>(
        &'g self,
        guard: &'g V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<AcceptedNamedFiles<'g>, AppError> {
        let mut held = AcceptedNamedFiles::capture(
            guard,
            &self.target,
            self.target_id,
            &self.pages,
            self.pages_id,
        )?;
        for a in &self.artifacts {
            held.add_exact_file(owned_path(&a.path)?, a.native, &a.frame)?;
        }
        Ok(held)
    }
    pub(super) fn capture_ready<'g>(
        &'g self,
        guard: &'g V2TextCurrentGroupGuard<'_, '_>,
        parent: &'g [u8],
    ) -> Result<AcceptedNamedFiles<'g>, AppError> {
        let mut held = self.capture_artifacts(guard)?;
        // Only genuine NotFound chooses the exact own lost-rename branch; all other
        // stage errors remain errors. This checks content as well as native identity.
        let stage = fixed_child(&self.target, "head.stage")?;
        match std::fs::symlink_metadata(&stage) {
            Ok(_) => held.add_exact_file(stage, self.head_native, &self.selected)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                held.add_absence(stage)?;
                held.add_exact_file(
                    owned_path(&self.head_path)?,
                    self.head_native,
                    &self.selected,
                )?;
            }
            Err(e) => return Err(failed(e)),
        }
        let current = read_control(&self.head_path, 1725)?;
        if current.frame.as_slice() != parent && current.frame != self.selected {
            return Err(refused());
        }
        let id = catcoms_storage::kept::session_id(&current.file).map_err(failed)?;
        held.add_exact_file(
            owned_path(&self.head_path)?,
            id,
            if current.frame.as_slice() == parent {
                parent
            } else {
                &self.selected
            },
        )?;
        Ok(held)
    }
    pub(super) fn capture_selected<'g>(
        &'g self,
        guard: &'g V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<AcceptedNamedFiles<'g>, AppError> {
        let mut held = self.capture_artifacts(guard)?;
        held.add_exact_file(
            owned_path(&self.head_path)?,
            self.head_native,
            &self.selected,
        )?;
        Ok(held)
    }
    pub(super) fn validate_original(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.metadata.validate_prepaid_account()?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        let (target, _, id) = guard.target_directory_data();
        if target != self.target || id != self.target_id {
            return Err(refused());
        }
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.context {
            return Err(refused());
        }
        guard.validate_original_namespace()
    }
}
fn failed(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

#[cfg(all(test, windows))]
#[derive(Default)]
pub(super) struct AcceptedFactsDropNotice {
    pub(super) notice: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
}
#[cfg(all(test, windows))]
impl Drop for AcceptedFactsDropNotice {
    fn drop(&mut self) {
        if let Some((gone, refunded)) = &self.notice {
            assert!(
                !refunded.load(std::sync::atomic::Ordering::SeqCst),
                "B flat facts outlived actual NEW B metadata fee"
            );
            gone.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}
