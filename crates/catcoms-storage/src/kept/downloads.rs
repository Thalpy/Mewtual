//! Private, durable download ownership. Download and Keep may name the same plaintext CID but
//! never share a journal, ciphertext reservation, run or cleanup authority. The common engine
//! supplies verification, conservative quota and persistent spend; public blob serving excludes
//! every Download record, including completed ones.
use super::*;

/// Saved download selector, not a read/export/write capability. Its sealed record and current
/// native scope must be revalidated before a new runtime run or export can be issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DownloadId([u8; 32]);
impl DownloadId {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Internal ownership key used for every lookup, path, deletion and active-run projection.
/// A shared CID is not interchangeable ownership: cancelling a Download cannot retire a Keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Owner {
    Keep(Cid),
    Download(DownloadId),
}
impl Owner {
    pub(super) fn name(self) -> String {
        match self {
            Self::Keep(cid) => cid.to_hex(),
            Self::Download(id) => format!("d-{}", hex::encode(id.0)),
        }
    }
    pub(super) fn parse(name: &str) -> Option<Self> {
        let owner = if let Some(value) = name.strip_prefix("d-") {
            Self::Download(DownloadId(hex::decode(value).ok()?.try_into().ok()?))
        } else {
            Self::Keep(Cid::from_hex(name)?)
        };
        (owner.name() == name).then_some(owner)
    }
}

/// Opaque current acquisition run. Only the storage engine can unwrap its shared internal
/// machinery; callers cannot pass it to CID-addressed Keep operations or invent a fresh budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadRun {
    pub(super) id: DownloadId,
    pub(super) run: KeepRun,
}
impl DownloadRun {
    pub fn id(&self) -> DownloadId {
        self.id
    }
}

/// Native metadata only. The plan contains encrypted manifest details and must not be copied
/// into the renderer's status projection. It supplies no permission to export plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadStatus {
    pub id: DownloadId,
    pub state: KeptTransfer,
    pub checked: bool,
    /// Typed external export state. Carries no path, name or identity.
    pub export: super::ExportView,
    /// A "Save as" choice is remembered for the next save (no path or name is exposed).
    pub planned: bool,
}

/// Observing lease for one verified completed private download in this mounted store. It cannot
/// keep the store or transfer alive; cancellation, replacement or remount revokes it. A caller
/// must separately retain its original native/Root scope and authorize a fresh destination.
#[derive(Clone)]
pub struct DownloadExportLease {
    pub(super) id: DownloadId,
    pub(super) version: [u8; 32],
    pub(super) guard: crate::blob_read::ReadGuard,
    pub(super) mount: crate::blob_read::ReadGuard,
}
impl std::fmt::Debug for DownloadExportLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DownloadExportLease { .. }")
    }
}
impl DownloadExportLease {
    pub fn id(&self) -> DownloadId {
        self.id
    }
    /// Observes storage custody only; this never authorizes an external path or native session.
    pub fn is_current(&self) -> bool {
        self.guard.current() && self.mount.current()
    }
}

impl<R: CryptoRngCore> KeptBlobStore<R> {
    pub(super) fn download_index(&self, id: DownloadId) -> Result<usize, StorageError> {
        self.ready()?;
        if self.scope.is_none() {
            return Err(StorageError::Malformed);
        }
        self.transfers
            .iter()
            .position(|t| t.record.owner() == Owner::Download(id))
            .ok_or(StorageError::Malformed)
    }
    fn download_token(&self, run: DownloadRun) -> Result<KeepRun, StorageError> {
        self.active
            .as_ref()
            .filter(|active| active.token == run.run && active.identity == Owner::Download(run.id))
            .map(|active| active.token)
            .ok_or(StorageError::Malformed)
    }
    pub(super) fn download_run(&self, run: DownloadRun) -> Result<KeepRun, StorageError> {
        let index = self.download_index(run.id)?;
        if self.transfers[index].poisoned || self.active.as_ref().is_none_or(|a| !*a.owner.borrow())
        {
            return Err(StorageError::Malformed);
        }
        self.download_token(run)
    }

    /// Enumerate private native metadata under the same full bound scope and shared quota.
    /// Completed Download records never become Keeps or public blob sources.
    pub fn downloads(&self) -> Result<Vec<DownloadStatus>, StorageError> {
        self.ready()?;
        if self.scope.is_none() {
            return Err(StorageError::Malformed);
        }
        Ok(self
            .transfers
            .iter()
            .filter_map(|t| match t.record.owner() {
                Owner::Download(id) => Some(DownloadStatus {
                    id,
                    state: self.transfer_state(t),
                    checked: t.checked && !t.poisoned,
                    export: self.export_view(t),
                    planned: t.record.planned.is_some(),
                }),
                Owner::Keep(_) => None,
            })
            .collect())
    }
    /// Cheap owner-specific event projection; never clone saved manifests on every page.
    pub fn download_progress(&self, id: DownloadId) -> Result<KeepProgress, StorageError> {
        let index = self.download_index(id)?;
        let transfer = &self.transfers[index];
        Ok(KeepProgress {
            phase: transfer.record.phase,
            verified_chunks: transfer.record.verified.iter().filter(|bit| **bit).count(),
            running: self
                .active
                .as_ref()
                .is_some_and(|a| a.identity == Owner::Download(id)),
            user_paused: transfer.record.user_paused,
            blocked: transfer.record.blocked,
            exhausted: transfer.record.exhausted,
        })
    }

    pub fn download_saved_plans(&self, id: DownloadId) -> Result<Vec<KeepPlan>, StorageError> {
        let index = self.download_index(id)?;
        let transfer = &self.transfers[index];
        if transfer.poisoned || transfer.record.phase == KeepPhase::Deleting {
            return Err(StorageError::Malformed);
        }
        Ok(transfer.record.plans().cloned().collect())
    }

    /// Create one distinct Download owner, or resume that exact saved manifest set. A repeat
    /// Save click cannot manufacture a new allowance for an unfinished/exhausted same-CID read.
    pub fn begin_download(
        &mut self,
        plans: Vec<KeepPlan>,
        selected: usize,
        listing: [u8; 32],
    ) -> Result<DownloadRun, StorageError> {
        self.ready()?;
        let scope = self.scope.clone().ok_or(StorageError::Malformed)?;
        // Check borrowed shapes before cloning untrusted native manifest details. The common
        // codec repeats these checks before publication, independently of the caller's resolver.
        if !(1..=record::MAX_VARIANTS).contains(&plans.len()) || selected >= plans.len() {
            return Err(StorageError::BlobSizeLimit);
        }
        for plan in &plans {
            plan.charge()?;
        }
        let proposed = record::Record::initial_download(
            scope,
            plans.clone(),
            selected,
            listing,
            &mut self.rng,
        )?;
        if let Some(existing) = self
            .transfers
            .iter()
            .find(|t| !t.record.public_retained() && t.record.plan.cid == proposed.plan.cid)
        {
            if existing.record.listing != listing || existing.record.plans().ne(plans.iter()) {
                return Err(StorageError::Malformed);
            }
            let Owner::Download(id) = existing.record.owner() else {
                unreachable!()
            };
            return self.resume_download(id);
        }
        let Owner::Download(id) = proposed.owner() else {
            unreachable!()
        };
        let run = self.begin_owned(
            proposed.plan.clone(),
            listing,
            None,
            proposed.owner(),
            Some(proposed),
        )?;
        Ok(DownloadRun { id, run })
    }

    /// Fresh runtime ownership of saved encrypted work; no path, new manifest or allowance is
    /// accepted. Completed copies are rechecked from chunk zero before another export lease.
    pub fn resume_download(&mut self, id: DownloadId) -> Result<DownloadRun, StorageError> {
        let index = self.download_index(id)?;
        let record = &self.transfers[index].record;
        let run = self.begin_owned(
            record.plan.clone(),
            record.listing,
            None,
            record.owner(),
            None,
        )?;
        Ok(DownloadRun { id, run })
    }

    pub fn download_run_current(&self, run: DownloadRun, cid: &Cid, version: &[u8; 32]) -> bool {
        self.download_run(run).is_ok()
            && self.active.as_ref().is_some_and(|a| {
                a.plan.cid == *cid
                    && &a.plan.version == version
                    && a.transfer.is_some_and(|i| !self.transfers[i].poisoned)
            })
    }
    pub fn download_work(
        &mut self,
        run: DownloadRun,
        network: Option<KeepNetworkWork>,
        mono: u64,
    ) -> Result<KeepWork, StorageError> {
        let token = self.download_run(run)?;
        self.keep_work(token, network, mono)
    }
    pub fn put_download(&mut self, run: DownloadRun, bytes: &[u8]) -> Result<(), StorageError> {
        let token = self.download_run(run)?;
        self.put_keep(token, bytes)
    }
    pub fn get_download(
        &self,
        run: DownloadRun,
        cid: &Cid,
        max: usize,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let token = self.download_run(run)?;
        self.get_keep(token, cid, max)
    }
    pub fn fallback_download(
        &mut self,
        run: DownloadRun,
        expected: &[u8; 32],
        tried: &[[u8; 32]],
        bad: bool,
    ) -> Result<Option<(DownloadRun, KeepPlan)>, StorageError> {
        let token = self.download_run(run)?;
        Ok(self
            .fallback_keep(token, expected, tried, bad)?
            .map(|(token, plan)| {
                (
                    DownloadRun {
                        id: run.id,
                        run: token,
                    },
                    plan,
                )
            }))
    }
    pub fn pause_download(&mut self, run: DownloadRun) -> Result<(), StorageError> {
        // Retiring an exact old run is allowed after a storage fault; this supplies no bytes
        // or new work and must never accidentally pause an active Keep of the same CID.
        let token = self.download_token(run)?;
        self.pause_keep(token)
    }
    pub fn block_download(&mut self, run: DownloadRun) -> Result<(), StorageError> {
        let token = self.download_run(run)?;
        self.block_keep(token)
    }
    /// The trusted application must have authenticated every plaintext chunk and the complete
    /// file CID before calling this barrier, just as for `finish_keep`.
    pub fn finish_download(
        &mut self,
        run: DownloadRun,
    ) -> Result<DownloadExportLease, StorageError> {
        let token = self.download_run(run)?;
        self.finish_keep(token)?;
        self.download_export(run.id).map(|(lease, _)| lease)
    }

    /// Read custody for an already whole-file-verified private copy. The native caller must
    /// capture its ORIGINAL Root/session/entry permission separately before using this lease.
    pub fn download_export(
        &self,
        id: DownloadId,
    ) -> Result<(DownloadExportLease, KeepPlan), StorageError> {
        let index = self.download_index(id)?;
        let transfer = &self.transfers[index];
        if transfer.poisoned
            || !transfer.checked
            || transfer.record.user_paused
            || transfer.record.blocked
        {
            return Err(StorageError::Malformed);
        }
        let plan = transfer
            .record
            .committed_plan()
            .ok_or(StorageError::Malformed)?;
        Ok((
            DownloadExportLease {
                id,
                version: plan.version,
                guard: transfer.read_epoch.guard(),
                mount: self.read_epoch.guard(),
            },
            plan.clone(),
        ))
    }
    pub fn read_download_export(
        &self,
        lease: &DownloadExportLease,
        cid: &Cid,
        max: usize,
    ) -> Result<Vec<u8>, StorageError> {
        if !lease.is_current() || !lease.mount.issued_by(&self.read_epoch) {
            return Err(StorageError::Malformed);
        }
        let index = self.download_index(lease.id)?;
        let transfer = &self.transfers[index];
        if !lease.guard.issued_by(&transfer.read_epoch) {
            return Err(StorageError::Malformed);
        }
        let plan = transfer
            .record
            .committed_plan()
            .filter(|p| p.version == lease.version)
            .ok_or(StorageError::Malformed)?;
        if transfer.poisoned
            || transfer.record.user_paused
            || transfer.record.blocked
            || !transfer.checked
        {
            return Err(StorageError::Malformed);
        }
        let bound = plan
            .chunks
            .iter()
            .find(|(address, _)| address == cid)
            .map(|(_, bound)| *bound)
            .ok_or(StorageError::Malformed)?;
        let bound = usize::try_from(bound).map_err(|_| StorageError::BlobSizeLimit)?;
        let bytes = transfer
            .store
            .get_bounded(cid, max.min(bound))?
            .ok_or(StorageError::Malformed)?;
        if !lease.is_current() {
            return Err(StorageError::Malformed);
        }
        Ok(bytes)
    }
    /// Remove/Clear. Without an export journal this releases the record at once, as before.
    /// With one, it durably publishes Deleting (keeping the journal so cleanup can still prove
    /// which external file is ours), revokes every export observer, and returns the journal:
    /// native then removes the proven partial and calls [`Self::finish_download_cleanup`].
    /// Repeating it on a record already in that state returns the journal again.
    pub fn cancel_download(&mut self, id: DownloadId) -> Result<Option<ExportState>, StorageError> {
        let index = self.download_index(id)?;
        if self.transfers[index].record.export.journal().is_none() {
            self.release_transfer(index)?;
            return Ok(None);
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.identity != Owner::Download(id))
        {
            return Err(StorageError::Io("pause the active kept copy first".into()));
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.identity == Owner::Download(id))
        {
            self.active = None;
        }
        let transfer = &mut self.transfers[index];
        transfer.read_epoch.retire();
        transfer.checked = false;
        transfer.export_presence = None;
        if transfer.record.phase != KeepPhase::Deleting {
            let mut next = transfer.record.successor()?;
            next.phase = KeepPhase::Deleting;
            transfer.publish(next, &self.metadata_key, &mut self.rng)?;
        }
        Ok(transfer.record.export.state())
    }
    /// Cleanup after committed external publication may remove only the exact exported
    /// generation. A second Save/reverification of the same record invalidates this cleanup.
    /// A record with a Writing journal cannot be released here: that would let a non-resumable
    /// fallback export complete a journaled save and orphan its tracked partial. A record that is
    /// already gone (for example a Remove that won the race) makes this a no-op.
    pub fn finish_download_export(
        &mut self,
        lease: &DownloadExportLease,
    ) -> Result<(), StorageError> {
        self.ready()?;
        if self.scope.is_none() {
            return Err(StorageError::Malformed);
        }
        let Some(index) = self
            .transfers
            .iter()
            .position(|t| t.record.owner() == Owner::Download(lease.id))
        else {
            return Ok(());
        };
        let transfer = &self.transfers[index];
        if !lease.is_current()
            || !lease.mount.issued_by(&self.read_epoch)
            || !lease.guard.issued_by(&transfer.read_epoch)
            || transfer
                .record
                .committed_plan()
                .is_none_or(|p| p.version != lease.version)
            || matches!(transfer.record.export, super::export::Export::Writing(_))
            || self.active.is_some()
        {
            return Err(StorageError::Malformed);
        }
        self.release_transfer(index)
    }
    pub fn set_download_paused(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
        paused: bool,
    ) -> Result<(), StorageError> {
        self.download_index(id)?;
        self.set_owned_paused(Owner::Download(id), *id.as_bytes(), expected_intent, paused)
    }
    /// Explicit bounded grant; repeating the exact request recovers its latest receipt and does
    /// not clear Pause, bad variants or the saved manifest set. It never starts acquisition.
    pub fn retry_download(
        &mut self,
        id: DownloadId,
        request: KeepRetry,
    ) -> Result<KeepRetryReceipt, StorageError> {
        self.download_index(id)?;
        self.retry_owned(Owner::Download(id), request)
    }
}

#[cfg(test)]
mod tests;

/// Native-only private acquisition and export interface. Every operation addresses one sealed
/// Download owner under the mounted group/device scope; none is a public blob-serving API.
/// Implementations must retain the common quota, saved manifest set and persistent execution
/// budget across remounts. Runtime tokens and export leases are never renderer capabilities.
pub trait DownloadStore {
    /// Small coalescing projection for one private owner, independent of same-CID Keep state.
    fn download_progress(&self, id: DownloadId) -> Result<KeepProgress, StorageError>;
    /// Retire only the exact private export generation after successful external publication.
    fn finish_download_export(&mut self, lease: &DownloadExportLease) -> Result<(), StorageError>;
    fn downloads(&self) -> Result<Vec<DownloadStatus>, StorageError>;
    fn download_saved_plans(&self, id: DownloadId) -> Result<Vec<KeepPlan>, StorageError>;
    fn begin_download(
        &mut self,
        plans: Vec<KeepPlan>,
        selected: usize,
        listing: [u8; 32],
    ) -> Result<DownloadRun, StorageError>;
    fn resume_download(&mut self, id: DownloadId) -> Result<DownloadRun, StorageError>;
    fn download_work(
        &mut self,
        run: DownloadRun,
        network: Option<KeepNetworkWork>,
        mono: u64,
    ) -> Result<KeepWork, StorageError>;
    fn put_download(&mut self, run: DownloadRun, bytes: &[u8]) -> Result<(), StorageError>;
    fn get_download(
        &self,
        run: DownloadRun,
        cid: &Cid,
        max: usize,
    ) -> Result<Option<Vec<u8>>, StorageError>;
    fn fallback_download(
        &mut self,
        run: DownloadRun,
        expected: &[u8; 32],
        tried: &[[u8; 32]],
        bad: bool,
    ) -> Result<Option<(DownloadRun, KeepPlan)>, StorageError>;
    fn pause_download(&mut self, run: DownloadRun) -> Result<(), StorageError>;
    fn block_download(&mut self, run: DownloadRun) -> Result<(), StorageError>;
    fn finish_download(&mut self, run: DownloadRun) -> Result<DownloadExportLease, StorageError>;
    fn download_export(
        &self,
        id: DownloadId,
    ) -> Result<(DownloadExportLease, KeepPlan), StorageError>;
    fn read_download_export(
        &self,
        lease: &DownloadExportLease,
        cid: &Cid,
        max: usize,
    ) -> Result<Vec<u8>, StorageError>;
    /// `Some` means Deleting is durable and native must remove the proven partial, then call
    /// [`DownloadStore::finish_download_cleanup`]; `None` means the record is already released.
    fn cancel_download(&mut self, id: DownloadId) -> Result<Option<ExportState>, StorageError>;
    fn set_download_paused(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
        paused: bool,
    ) -> Result<(), StorageError>;
    fn retry_download(
        &mut self,
        id: DownloadId,
        request: KeepRetry,
    ) -> Result<KeepRetryReceipt, StorageError>;
    /// Cheap original-run fence; false after replacement, cancellation or remount.
    fn download_run_current(&self, run: DownloadRun, cid: &Cid, version: &[u8; 32]) -> bool;

    // Journaled external export. These write only the sealed record; every external filesystem
    // effect happens in native through `ExportFile`, never on the caller's actor thread.
    fn download_export_state(&self, id: DownloadId) -> Result<Option<ExportState>, StorageError>;
    /// Remember a "Save as" choice for the exact current run; see
    /// [`KeptBlobStore::plan_download_destination`].
    fn plan_download_destination(
        &mut self,
        run: DownloadRun,
        choice: &super::ChosenDestination,
    ) -> Result<(), StorageError>;
    /// Where a fresh export goes; see [`KeptBlobStore::download_planned_destination`].
    #[allow(clippy::type_complexity)]
    fn download_planned_destination(
        &self,
        id: DownloadId,
    ) -> Result<Option<Result<super::ChosenDestination, super::ChoiceRefusal>>, StorageError>;
    fn begin_download_export(
        &mut self,
        lease: &DownloadExportLease,
        journal: &ExportJournal,
        presence: ExportPresence,
    ) -> Result<(), StorageError>;
    fn resume_download_export(
        &mut self,
        lease: &DownloadExportLease,
        journal: &ExportJournal,
        presence: ExportPresence,
    ) -> Result<(), StorageError>;
    fn checkpoint_download_export(
        &mut self,
        lease: &DownloadExportLease,
        expected: &ExportJournal,
        next: &ExportJournal,
    ) -> Result<(), StorageError>;
    fn publish_download_export(
        &mut self,
        lease: &DownloadExportLease,
        expected: &ExportJournal,
    ) -> Result<(), StorageError>;
    fn recover_download_export(
        &mut self,
        id: DownloadId,
        expected: &ExportJournal,
    ) -> Result<(), StorageError>;
    fn halt_download_export(
        &mut self,
        id: DownloadId,
        expected: &ExportJournal,
        halt: ExportHalt,
    ) -> Result<(), StorageError>;
    /// Save new copy, first step: retire observers under the intent CAS, keeping the journal.
    fn retire_download_export(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
    ) -> Result<u64, StorageError>;
    fn abandon_download_export(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
    ) -> Result<u64, StorageError>;
    fn finish_download_cleanup(&mut self, id: DownloadId) -> Result<(), StorageError>;
}

impl<R: CryptoRngCore> DownloadStore for KeptBlobStore<R> {
    fn download_progress(&self, id: DownloadId) -> Result<KeepProgress, StorageError> {
        KeptBlobStore::download_progress(self, id)
    }
    fn finish_download_export(&mut self, lease: &DownloadExportLease) -> Result<(), StorageError> {
        KeptBlobStore::finish_download_export(self, lease)
    }
    fn downloads(&self) -> Result<Vec<DownloadStatus>, StorageError> {
        KeptBlobStore::downloads(self)
    }
    fn download_saved_plans(&self, id: DownloadId) -> Result<Vec<KeepPlan>, StorageError> {
        KeptBlobStore::download_saved_plans(self, id)
    }
    fn begin_download(
        &mut self,
        plans: Vec<KeepPlan>,
        selected: usize,
        listing: [u8; 32],
    ) -> Result<DownloadRun, StorageError> {
        KeptBlobStore::begin_download(self, plans, selected, listing)
    }
    fn resume_download(&mut self, id: DownloadId) -> Result<DownloadRun, StorageError> {
        KeptBlobStore::resume_download(self, id)
    }
    fn download_work(
        &mut self,
        run: DownloadRun,
        network: Option<KeepNetworkWork>,
        mono: u64,
    ) -> Result<KeepWork, StorageError> {
        KeptBlobStore::download_work(self, run, network, mono)
    }
    fn put_download(&mut self, run: DownloadRun, bytes: &[u8]) -> Result<(), StorageError> {
        KeptBlobStore::put_download(self, run, bytes)
    }
    fn get_download(
        &self,
        run: DownloadRun,
        cid: &Cid,
        max: usize,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        KeptBlobStore::get_download(self, run, cid, max)
    }
    fn fallback_download(
        &mut self,
        run: DownloadRun,
        expected: &[u8; 32],
        tried: &[[u8; 32]],
        bad: bool,
    ) -> Result<Option<(DownloadRun, KeepPlan)>, StorageError> {
        KeptBlobStore::fallback_download(self, run, expected, tried, bad)
    }
    fn pause_download(&mut self, run: DownloadRun) -> Result<(), StorageError> {
        KeptBlobStore::pause_download(self, run)
    }
    fn block_download(&mut self, run: DownloadRun) -> Result<(), StorageError> {
        KeptBlobStore::block_download(self, run)
    }
    fn finish_download(&mut self, run: DownloadRun) -> Result<DownloadExportLease, StorageError> {
        KeptBlobStore::finish_download(self, run)
    }
    fn download_export(
        &self,
        id: DownloadId,
    ) -> Result<(DownloadExportLease, KeepPlan), StorageError> {
        KeptBlobStore::download_export(self, id)
    }
    fn read_download_export(
        &self,
        lease: &DownloadExportLease,
        cid: &Cid,
        max: usize,
    ) -> Result<Vec<u8>, StorageError> {
        KeptBlobStore::read_download_export(self, lease, cid, max)
    }
    fn cancel_download(&mut self, id: DownloadId) -> Result<Option<ExportState>, StorageError> {
        KeptBlobStore::cancel_download(self, id)
    }
    fn set_download_paused(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
        paused: bool,
    ) -> Result<(), StorageError> {
        KeptBlobStore::set_download_paused(self, id, expected_intent, paused)
    }
    fn retry_download(
        &mut self,
        id: DownloadId,
        request: KeepRetry,
    ) -> Result<KeepRetryReceipt, StorageError> {
        KeptBlobStore::retry_download(self, id, request)
    }
    fn download_run_current(&self, run: DownloadRun, cid: &Cid, version: &[u8; 32]) -> bool {
        KeptBlobStore::download_run_current(self, run, cid, version)
    }
    fn download_export_state(&self, id: DownloadId) -> Result<Option<ExportState>, StorageError> {
        KeptBlobStore::download_export_state(self, id)
    }
    fn plan_download_destination(
        &mut self,
        run: DownloadRun,
        choice: &super::ChosenDestination,
    ) -> Result<(), StorageError> {
        KeptBlobStore::plan_download_destination(self, run, choice)
    }
    fn download_planned_destination(
        &self,
        id: DownloadId,
    ) -> Result<Option<Result<super::ChosenDestination, super::ChoiceRefusal>>, StorageError> {
        KeptBlobStore::download_planned_destination(self, id)
    }
    fn begin_download_export(
        &mut self,
        lease: &DownloadExportLease,
        journal: &ExportJournal,
        presence: ExportPresence,
    ) -> Result<(), StorageError> {
        KeptBlobStore::begin_download_export(self, lease, journal, presence)
    }
    fn resume_download_export(
        &mut self,
        lease: &DownloadExportLease,
        journal: &ExportJournal,
        presence: ExportPresence,
    ) -> Result<(), StorageError> {
        KeptBlobStore::resume_download_export(self, lease, journal, presence)
    }
    fn checkpoint_download_export(
        &mut self,
        lease: &DownloadExportLease,
        expected: &ExportJournal,
        next: &ExportJournal,
    ) -> Result<(), StorageError> {
        KeptBlobStore::checkpoint_download_export(self, lease, expected, next)
    }
    fn publish_download_export(
        &mut self,
        lease: &DownloadExportLease,
        expected: &ExportJournal,
    ) -> Result<(), StorageError> {
        KeptBlobStore::publish_download_export(self, lease, expected)
    }
    fn recover_download_export(
        &mut self,
        id: DownloadId,
        expected: &ExportJournal,
    ) -> Result<(), StorageError> {
        KeptBlobStore::recover_download_export(self, id, expected)
    }
    fn halt_download_export(
        &mut self,
        id: DownloadId,
        expected: &ExportJournal,
        halt: ExportHalt,
    ) -> Result<(), StorageError> {
        KeptBlobStore::halt_download_export(self, id, expected, halt)
    }
    fn retire_download_export(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
    ) -> Result<u64, StorageError> {
        KeptBlobStore::retire_download_export(self, id, expected_intent)
    }
    fn abandon_download_export(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
    ) -> Result<u64, StorageError> {
        KeptBlobStore::abandon_download_export(self, id, expected_intent)
    }
    fn finish_download_cleanup(&mut self, id: DownloadId) -> Result<(), StorageError> {
        KeptBlobStore::finish_download_cleanup(self, id)
    }
}
