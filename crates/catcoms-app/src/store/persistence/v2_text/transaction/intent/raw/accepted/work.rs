//! Finite SAME-store first-B work. The consumed marker owns the actual OS lease
//! and canonical writer End until ALL helper Files and cleanup have drained.
//! W_B and parent barriers precede artifacts; all original bytes precede selector CAS.
use super::super::super::super::work::io as native;
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextAcceptedCreateCommit, AdmittedV2TextAcceptedCreateStage,
};
use catcoms_storage::kept::{session_at, SessionAt};
use std::fs;
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum V2TextAcceptedCreateTestCut {
    WalCreated,
    WalStaged,
    WalReplaced,
    WalSaved,
    ArtifactCreated,
    ArchiveCreated,
    PublicationCreated,
    ArtifactsSaved,
    HeadCreated,
    HeadStaged,
    ReadyCached,
    HeadRenamed,
    SavedCached,
}
#[cfg(test)]
#[derive(Default)]
pub(super) struct AcceptedTestHooks {
    fault: Option<V2TextAcceptedCreateTestCut>,
    partial: Option<(V2TextAcceptedCreateTestCut, usize)>,
    pause: Option<(V2TextAcceptedCreateTestCut, Box<dyn FnOnce() + Send>)>,
}
#[cfg(test)]
impl AcceptedTestHooks {
    fn hit_created(
        &mut self,
        cut: V2TextAcceptedCreateTestCut,
        path: &Path,
        expected: &[u8],
    ) -> Result<(), AppError> {
        self.hit(cut)?;
        if self.partial.as_ref().is_some_and(|(c, _)| *c == cut) {
            use std::io::Write;
            let (_, len) = self.partial.take().expect("real B partial creation cut");
            if len >= expected.len() {
                return Err(refused());
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(failed)?;
            file.write_all(&expected[..len]).map_err(failed)?;
            file.sync_all().map_err(failed)?;
            return Err(failed("injected original B partial write"));
        }
        Ok(())
    }

    fn hit(&mut self, cut: V2TextAcceptedCreateTestCut) -> Result<(), AppError> {
        if self.pause.as_ref().is_some_and(|(c, _)| *c == cut) {
            let (_, run) = self.pause.take().expect("real B worker cut");
            run();
        }
        if self.fault == Some(cut) {
            self.fault = None;
            return Err(failed("injected B IO failure"));
        }
        Ok(())
    }
}
pub(super) struct AcceptedSlots {
    wal: Option<SessionId>,
    head: Option<SessionId>,
    artifacts: [Option<SessionId>; MAX_ACCEPTED_ARTIFACTS],
}
impl AcceptedSlots {
    pub(super) fn empty() -> Self {
        Self {
            wal: None,
            head: None,
            artifacts: [None; MAX_ACCEPTED_ARTIFACTS],
        }
    }
}
impl PreparedV2TextAcceptedCreate {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_partial_write_fault_for_test(
        &mut self,
        cut: V2TextAcceptedCreateTestCut,
        len: usize,
    ) {
        self.hooks.partial = Some((cut, len));
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn original_output_for_test(
        &self,
        cut: V2TextAcceptedCreateTestCut,
    ) -> Result<(PathBuf, Zeroizing<Vec<u8>>), AppError> {
        let base = &self.parent.signature.parent.base;
        match cut {
            V2TextAcceptedCreateTestCut::WalCreated => Ok((
                owned_path(&base.wal_stage_path)?,
                Zeroizing::new(self.wal.as_ref().ok_or_else(refused)?.frame_data().to_vec()),
            )),
            V2TextAcceptedCreateTestCut::HeadCreated => Ok((
                fixed_child(&base.target, "head.stage")?,
                Zeroizing::new(
                    self.candidate
                        .as_ref()
                        .ok_or_else(refused)?
                        .frame_data()
                        .to_vec(),
                ),
            )),
            V2TextAcceptedCreateTestCut::ArtifactCreated
            | V2TextAcceptedCreateTestCut::ArchiveCreated
            | V2TextAcceptedCreateTestCut::PublicationCreated => {
                let role = match cut {
                    V2TextAcceptedCreateTestCut::ArtifactCreated => TextArtifactRoleData::Manifest,
                    V2TextAcceptedCreateTestCut::ArchiveCreated => TextArtifactRoleData::Archive,
                    _ => TextArtifactRoleData::Publication,
                };
                let artifacts = self.artifact_frames_data()?;
                let a = artifacts
                    .iter()
                    .find(|a| a.role_data() == role)
                    .ok_or_else(refused)?;
                Ok((
                    artifact_child(base.pages()?, a.role_data(), &a.id_data())?,
                    Zeroizing::new(a.frame_data().to_vec()),
                ))
            }
            _ => Err(refused()),
        }
    }

    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_fault_for_test(&mut self, cut: V2TextAcceptedCreateTestCut) {
        self.hooks.fault = Some(cut);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_pause_for_test(
        &mut self,
        cut: V2TextAcceptedCreateTestCut,
        run: Box<dyn FnOnce() + Send>,
    ) {
        self.hooks.pause = Some((cut, run));
    }
    fn validate_work_accounts(&self) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        self.signature
            .intent
            .retained
            .custody
            .work_resources()
            .validate_prepaid_account()?;
        self.signature.intent.unsigned.validate_accounts_only()?;
        self.signature.require_original_raw_data()
    }
    pub(in crate::store) fn run_stage(
        &mut self,
        admitted: AdmittedV2TextAcceptedCreateStage,
    ) -> Result<(), AppError> {
        admitted.require_same_append(&self.observe_original())?;
        self.validate_work_accounts()?;
        if self.ready.is_some() {
            return Ok(());
        }
        let mut slots = std::mem::replace(&mut self.slots, AcceptedSlots::empty());
        #[cfg(test)]
        let mut hooks = std::mem::take(&mut self.hooks);
        let result = self.stage_inner(
            &mut slots,
            #[cfg(test)]
            &mut hooks,
        );
        self.slots = slots;
        #[cfg(test)]
        {
            self.hooks = hooks;
        }
        let facts = result?;
        #[cfg(all(test, windows))]
        let facts = {
            let mut facts = facts;
            facts.drop_notice.notice = self.saved_drop_notice.take();
            facts
        };
        self.ready = Some(ReadyV2TextAcceptedCreateSelection {
            original: self.observe_original(),
            facts: Arc::new(facts),
            parent: Arc::clone(&self.parent),
            wal_native: self.slots.wal.ok_or_else(refused)?,
            wal_len: self.wal.as_ref().ok_or_else(refused)?.frame_data().len(),
            work: Arc::clone(self.signature.intent.retained.custody.work_resources()),
            metadata: self.metadata.clone(),
        });
        #[cfg(test)]
        self.hooks.hit(V2TextAcceptedCreateTestCut::ReadyCached)?;
        drop(admitted);
        Ok(())
    }
    fn stage_inner(
        &self,
        slots: &mut AcceptedSlots,
        #[cfg(test)] hooks: &mut AcceptedTestHooks,
    ) -> Result<source::AcceptedSelectedFacts, AppError> {
        let base = &self.parent.signature.parent.base;
        let target = native::require_directory(&base.target, base.target_id)?;
        let pages = native::require_directory(base.pages()?, base.pages_id)?;
        let old = native::require_exact(&base.head_path, None, &self.parent.head_frame)?;
        // The real prior WS (or exact SAME WB uncertain replacement) is fully compared
        // before any B output. Dirty/partial journals never become absent/permission.
        self.ensure_wal(
            slots,
            #[cfg(test)]
            hooks,
        )?;
        #[cfg(test)]
        hooks.hit(V2TextAcceptedCreateTestCut::WalSaved)?;
        let artifacts = self.artifact_frames_data()?;
        let mut facts = Vec::with_capacity(artifacts.len());
        for (i, a) in artifacts.iter().enumerate() {
            let path = artifact_child(base.pages()?, a.role_data(), &a.id_data())?;
            #[cfg(test)]
            let cut = match a.role_data() {
                TextArtifactRoleData::Archive => V2TextAcceptedCreateTestCut::ArchiveCreated,
                TextArtifactRoleData::Publication => {
                    V2TextAcceptedCreateTestCut::PublicationCreated
                }
                _ => V2TextAcceptedCreateTestCut::ArtifactCreated,
            };
            super::warm::write_original(&path, a.frame_data(), &mut slots.artifacts[i], || {
                #[cfg(test)]
                hooks.hit_created(cut, &path, a.frame_data())?;
                Ok(())
            })?;
            facts.push(source::AcceptedArtifactFact {
                path,
                native: slots.artifacts[i].ok_or_else(refused)?,
                frame: Zeroizing::new(a.frame_data().to_vec()),
            });
        }
        native::sync_parent(base.pages()?, base.pages_id)?;
        #[cfg(test)]
        hooks.hit(V2TextAcceptedCreateTestCut::ArtifactsSaved)?;
        let expected = self.candidate.as_ref().ok_or_else(refused)?.frame_data();
        let stage = fixed_child(&base.target, "head.stage")?;
        super::warm::write_original(&stage, expected, &mut slots.head, || {
            #[cfg(test)]
            hooks.hit_created(V2TextAcceptedCreateTestCut::HeadCreated, &stage, expected)?;
            Ok(())
        })?;
        native::sync_parent(&base.target, base.target_id)?;
        #[cfg(test)]
        hooks.hit(V2TextAcceptedCreateTestCut::HeadStaged)?;
        require_named(&base.head_path, &old)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            base.pages()?,
            &pages,
            base.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &base.target,
            &target,
            base.target_id,
        )?;
        Ok(source::AcceptedSelectedFacts {
            target: owned_path(&base.target)?,
            target_id: base.target_id,
            pages: owned_path(base.pages()?)?,
            pages_id: base.pages_id,
            head_path: owned_path(&base.head_path)?,
            head_native: slots.head.ok_or_else(refused)?,
            selected: Zeroizing::new(expected.to_vec()),
            artifacts: facts,
            context: self.parent.head.context.clone(),
            header: self.parent.header.clone(),
            #[cfg(all(test, windows))]
            drop_notice: source::AcceptedFactsDropNotice::default(),
            resources: self.resources().clone(),
            metadata: self.metadata.clone(),
        })
    }
    fn ensure_wal(
        &self,
        slots: &mut AcceptedSlots,
        #[cfg(test)] hooks: &mut AcceptedTestHooks,
    ) -> Result<(), AppError> {
        let base = &self.parent.signature.parent.base;
        let expected = self.wal.as_ref().ok_or_else(refused)?.frame_data();
        if slots
            .wal
            .is_some_and(|id| matches!(session_at(&base.wal_path, id), SessionAt::Ours))
        {
            native::require_absent(&base.wal_stage_path)?;
            let main = native::sync_exact(&base.wal_path, slots.wal, expected)?;
            native::sync_parent(&base.target, base.target_id)?;
            return require_named(&base.wal_path, &main);
        }
        if slots.head.is_some() || slots.artifacts.iter().any(Option::is_some) {
            return Err(refused());
        }
        native::require_absent(&fixed_child(&base.target, "head.stage")?)?;
        native::require_absent(&fixed_child(&base.target, "text.seed.stage.v1")?)?;
        for a in self.artifact_frames_data()? {
            native::require_absent(&artifact_child(base.pages()?, a.role_data(), &a.id_data())?)?;
        }
        let previous = native::sync_exact(
            &base.wal_path,
            None,
            self.signature
                .wal
                .as_ref()
                .ok_or_else(refused)?
                .frame_data(),
        )?;
        super::warm::write_original(&base.wal_stage_path, expected, &mut slots.wal, || {
            #[cfg(test)]
            hooks.hit_created(
                V2TextAcceptedCreateTestCut::WalCreated,
                &base.wal_stage_path,
                expected,
            )?;
            Ok(())
        })?;
        #[cfg(test)]
        hooks.hit(V2TextAcceptedCreateTestCut::WalStaged)?;
        native::sync_parent(&base.target, base.target_id)?;
        let stage = native::require_exact(&base.wal_stage_path, slots.wal, expected)?;
        require_named(&base.wal_path, &previous)?;
        require_named(&base.wal_stage_path, &stage)?;
        fs::rename(&base.wal_stage_path, &base.wal_path).map_err(failed)?;
        #[cfg(test)]
        hooks.hit(V2TextAcceptedCreateTestCut::WalReplaced)?;
        native::sync_parent(&base.target, base.target_id)?;
        require_named(&base.wal_path, &stage)?;
        native::require_absent(&base.wal_stage_path)
    }
    pub(in crate::store) fn run_commit(
        &mut self,
        admitted: AdmittedV2TextAcceptedCreateCommit,
    ) -> Result<(), AppError> {
        admitted.require_same_append(&self.observe_original())?;
        self.validate_work_accounts()?;
        if self.saved.is_some() {
            return Ok(());
        }
        let ready = self.ready.as_ref().ok_or_else(refused)?;
        ready
            .original
            .require_same_original(&self.observe_original())?;
        // The complete flat facts are prepared before selection and already belong
        // to the new B payment, not the old S source chain. Construct the tiny Saved
        // wrapper only AFTER actual barriers succeed; allocator abort is not a
        // recoverable IO result or a fabricated pre-CAS receipt.
        let facts = Arc::clone(&ready.facts);
        #[cfg(test)]
        let mut hooks = std::mem::take(&mut self.hooks);
        let result = self.commit_inner(
            #[cfg(test)]
            &mut hooks,
        );
        #[cfg(test)]
        {
            self.hooks = hooks;
        }
        result?;
        self.saved = Some(Arc::new(SavedV2AcceptedCreate {
            head: TextHeadIdData::of_data(&facts.selected),
            facts,
            original: self.observe_original(),
            metadata: self.metadata.clone(),
        }));
        #[cfg(test)]
        self.hooks.hit(V2TextAcceptedCreateTestCut::SavedCached)?;
        drop(admitted);
        Ok(())
    }
    fn commit_inner(&self, #[cfg(test)] hooks: &mut AcceptedTestHooks) -> Result<(), AppError> {
        let ready = self.ready.as_ref().ok_or_else(refused)?;
        let base = &self.parent.signature.parent.base;
        let target = native::require_directory(&base.target, base.target_id)?;
        let pages = native::require_directory(base.pages()?, base.pages_id)?;
        let wal = native::sync_exact(
            &base.wal_path,
            Some(ready.wal_native),
            self.wal.as_ref().ok_or_else(refused)?.frame_data(),
        )?;
        native::require_absent(&base.wal_stage_path)?;
        // Keep every one of the bounded exact artifact Files alive through CAS.
        // This is a complete proof, never a metadata-only subset or selectedO-only claim.
        let mut held = Vec::with_capacity(ready.facts.artifacts.len());
        for a in &ready.facts.artifacts {
            held.push(native::sync_exact(&a.path, Some(a.native), &a.frame)?);
        }
        native::sync_parent(base.pages()?, base.pages_id)?;
        let current = read_control(&base.head_path, 1725)?;
        let expected = self.candidate.as_ref().ok_or_else(refused)?.frame_data();
        let stage_path = fixed_child(&base.target, "head.stage")?;
        if current.frame == self.parent.head_frame {
            let stage = native::sync_exact(&stage_path, Some(ready.facts.head_native), expected)?;
            native::sync_parent(&base.target, base.target_id)?;
            require_named(&base.head_path, &current)?;
            require_named(&stage_path, &stage)?;
            require_named(&base.wal_path, &wal)?;
            for (a, file) in ready.facts.artifacts.iter().zip(&held) {
                require_named(&a.path, file)?;
            }
            fs::rename(&stage_path, &base.head_path).map_err(failed)?;
            #[cfg(test)]
            hooks.hit(V2TextAcceptedCreateTestCut::HeadRenamed)?;
            native::sync_parent(&base.target, base.target_id)?;
            require_named(&base.head_path, &stage)?;
        } else if current.frame.as_slice() == expected {
            native::require_absent(&stage_path)?;
            native::sync_exact(&base.head_path, Some(ready.facts.head_native), expected)?;
            native::sync_parent(&base.target, base.target_id)?;
        } else {
            return Err(refused());
        }
        native::require_absent(&stage_path)?;
        require_named(&base.wal_path, &wal)?;
        for (a, file) in ready.facts.artifacts.iter().zip(&held) {
            require_named(&a.path, file)?;
        }
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            base.pages()?,
            &pages,
            base.pages_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &base.target,
            &target,
            base.target_id,
        )
    }
}
/// Closed first-B immutable namespace only. Publication controls use `u-`, never an
/// alias of Retain/Intent/Signature or a generic caller-supplied filename.
pub(super) fn artifact_child(
    base: &Path,
    role: TextArtifactRoleData,
    id: &[u8; 32],
) -> Result<PathBuf, AppError> {
    let prefix = match role {
        TextArtifactRoleData::Manifest
        | TextArtifactRoleData::AcceptedIndex
        | TextArtifactRoleData::TokenIndex
        | TextArtifactRoleData::PublicationIndex
        | TextArtifactRoleData::AcceptanceRecords
        | TextArtifactRoleData::AcceptanceNodes
        | TextArtifactRoleData::HeaderBody => "p-",
        TextArtifactRoleData::Archive => "h-",
        TextArtifactRoleData::Publication => "u-",
        _ => return Err(refused()),
    };
    let cap = base
        .as_os_str()
        .len()
        .checked_add(71)
        .filter(|n| *n <= MAX_PATH)
        .ok_or_else(refused)?;
    let mut name = String::with_capacity(70);
    name.push_str(prefix);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in id {
        name.push(HEX[(b >> 4) as usize] as char);
        name.push(HEX[(b & 15) as usize] as char);
    }
    name.push_str(".bin");
    let mut out = PathBuf::with_capacity(cap);
    out.push(base);
    out.push(name);
    if out.capacity() > MAX_PATH {
        return Err(refused());
    }
    Ok(out)
}
fn failed(e: impl std::fmt::Display) -> AppError {
    AppError::Io(e.to_string())
}
