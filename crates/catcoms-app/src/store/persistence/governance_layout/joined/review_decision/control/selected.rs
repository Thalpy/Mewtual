//! Exact outcome recovery and retirement after an indexed head is selected.
use super::*;
use crate::store::persistence::v2_member_outbox::fs::remove_listed;
use crate::store::NativeTextInitialReviewRequest;

impl ServerStore {
    pub(in crate::store) fn finish_selected_initial_review(
        &self,
        pin: &FounderGenesisAnchorData,
        request: &NativeTextInitialReviewRequest,
        mut recheck: impl FnMut(ReviewStreamPosition) -> Result<(), AppError>,
    ) -> Result<Option<ReviewStreamEntry>, AppError> {
        let original = self
            .review_decision_mount
            .try_lock()
            .map_err(|_| denied())?;
        let mount = original.as_ref().ok_or_else(denied)?;
        mount.require(self)?;
        let root = &mount.directory;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _fee = policy.current.archive.pending.pool.try_reserve(WORK)?;
        let _lease = self._session.retain_for_physical_work();
        let key = Zeroizing::new(self.keys.db_key()?);
        let marker = mount.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let key = JournalKey::new(&key, marker);
        require_control_names(root)?;
        let Some(mut selected) = disk::FirstDecisionFiles::read(root, &key)? else {
            return Ok(None);
        };
        if mount.warm.is_some() {
            return Err(denied());
        }
        let (entry, request_digest) = selected.with_record(&key, pin, |record| {
            let held = record.record.held_data();
            let decision = record.record.decision_data();
            let spec = decision.decision().spec();
            if held.target_data().spec().channel != request.channel
                || request.case.id != decision.case_id()
                || held.target_data().spec().action != request.case.action
                || held.original_digest_data() != request.case.original
                || held.removal_data() != request.case.removal
                || spec.intent != request.intent
                || spec.decision != request.decision
                || spec.previous.is_some()
                || record.request_digest != *decision.id().as_bytes()
            {
                return Err(denied());
            }
            decision
                .verify_new_transition_from(None)
                .map_err(|_| denied())?;
            Ok((record.record.entry_data().clone(), record.request_digest))
        })?;
        let position = entry.position_data();
        recheck(position)?;
        mount.require(self)?;
        selected.cover(root)?;
        let wal_path = root.child(disk::SELECTION_WAL);
        if let Some(mut wal) = disk::HeldDecisionFile::read(wal_path.clone(), MAX_SELECTION_BYTES)?
        {
            let plain = DecisionSelection::open(&key, &wal.bytes)?;
            let selection = DecisionSelection::decode(&plain)?;
            selected.require_wal_outputs(&selection)?;
            let expected = selection.verify_data(&key, pin, (&entry, request_digest))?;
            if expected.roots.is_none() {
                return Err(denied());
            }
            let mut controls = [None, None, None];
            for role in ControlRole::ALL {
                controls[role as usize] =
                    disk::HeldDecisionFile::read(root.child(role.filename()), role.cap())?;
            }
            let frames = controls
                .each_ref()
                .map(|frame| frame.as_ref().map(|file| file.bytes.as_slice()));
            selection.require_remaining_controls(frames)?;
            wal.sync_complete()?;
            root.sync()?;
            for role in ControlRole::ALL.into_iter().rev() {
                let Some(original) = controls[role as usize].take() else {
                    absent(&root.child(role.filename()))?;
                    continue;
                };
                recheck(position)?;
                mount.require(self)?;
                selected.require_current(root)?;
                wal.require()?;
                require_control_names(root)?;
                retire_file(root, original)?;
                #[cfg(test)]
                if take_fault(match role {
                    ControlRole::Intent => ReviewControlFault::AfterRetiringIntent,
                    ControlRole::Attempt => ReviewControlFault::AfterRetiringAttempt,
                    ControlRole::Raw => ReviewControlFault::AfterRetiringRaw,
                }) {
                    return Err(AppError::Io(
                        "injected stop after retiring review control".into(),
                    ));
                }
            }
            retire_wal(root, wal, || {
                recheck(position)?;
                mount.require(self)?;
                selected.require_current(root)
            })?;
        }
        recheck(position)?;
        mount.require(self)?;
        selected.require_current(root)?;
        require_control_names(root)?;
        require_controls_absent(root)?;
        absent(&wal_path)?;
        Ok(Some(entry))
    }
}

fn retire_wal(
    root: &HeldDirectory,
    wal: disk::HeldDecisionFile,
    require_selected: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    require_selected()?;
    require_control_names(root)?;
    require_controls_absent(root)?;
    #[cfg(test)]
    if take_fault(ReviewControlFault::BeforeRetiringWal) {
        return Err(AppError::Io(
            "injected stop before retiring review WAL".into(),
        ));
    }
    retire_file(root, wal)
}

fn require_controls_absent(root: &HeldDirectory) -> Result<(), AppError> {
    for role in ControlRole::ALL {
        absent(&root.child(role.filename()))?;
    }
    Ok(())
}

fn retire_file(root: &HeldDirectory, mut original: disk::HeldDecisionFile) -> Result<(), AppError> {
    let path = original.path.clone();
    root.require()?;
    original.require()?;
    remove_listed(&path)?;
    // Windows can retain a delete-pending name while this handle is open.
    drop(original);
    absent(&path)?;
    root.sync()?;
    absent(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_selection_refuses_to_retire_wal_when_an_input_returns_at_the_last_check() {
        let temp = tempfile::tempdir().unwrap();
        let root = HeldDirectory::open(temp.path().to_path_buf()).unwrap();
        let pending = root.child(disk::SELECTION_WAL);
        let raw = root.child(ControlRole::Raw.filename());
        std::fs::write(&pending, b"authenticated transaction").unwrap();
        let wal = disk::HeldDecisionFile::read(pending.clone(), MAX_SELECTION_BYTES)
            .unwrap()
            .unwrap();

        assert!(retire_wal(&root, wal, || {
            std::fs::write(&raw, b"original retired bytes").unwrap();
            Ok(())
        })
        .is_err());
        assert_eq!(
            std::fs::read(pending).unwrap(),
            b"authenticated transaction"
        );
        assert_eq!(std::fs::read(raw).unwrap(), b"original retired bytes");
    }
}
