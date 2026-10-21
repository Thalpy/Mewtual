//! Local private downloads share the retained engine without becoming shared file sources.
//! The native caller owns consent/session/entry and destination permission; this layer checks
//! live group/file-key authority at every acquisition/export effect, including local cache hits.
use super::*;
use catcoms_storage::kept::{
    DownloadExportLease, DownloadId, DownloadRun, DownloadStatus, DownloadStore, ExportHalt,
    ExportJournal, ExportPresence, ExportState, KeepNetworkWork, KeepPlan, KeepRetry,
    KeepRetryReceipt, KeepWork,
};

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Metadata-only event coalescing; Keep of the same CID cannot mask this owner.
    pub fn download_progress(&self, id: DownloadId) -> Option<catcoms_storage::kept::KeepProgress> {
        self.download_store().ok()?.download_progress(id).ok()
    }
    /// Post-publication cleanup cannot remove a later run of the same private record.
    pub fn finish_download_export(&mut self, lease: &DownloadExportLease) -> Result<(), SyncError> {
        Ok(self.download_store_mut()?.finish_download_export(lease)?)
    }
    fn download_authorized(&self) -> Result<(), SyncError> {
        if !self.has_file_key() {
            return Err(SyncError::FileKeyUnavailable);
        }
        if !self.group.is_active() || !self.group.contains_device(&self.device_id()) {
            return Err(catcoms_storage::StorageError::Malformed.into());
        }
        Ok(())
    }
    fn download_store(&self) -> Result<&dyn DownloadStore, SyncError> {
        self.blobs.private_downloads().ok_or_else(|| {
            catcoms_storage::StorageError::Io("Downloads require the encrypted disk store.".into())
                .into()
        })
    }
    fn download_store_mut(&mut self) -> Result<&mut dyn DownloadStore, SyncError> {
        self.blobs.private_downloads_mut().ok_or_else(|| {
            catcoms_storage::StorageError::Io("Downloads require the encrypted disk store.".into())
                .into()
        })
    }
    /// Native-only downloads; does not grant a renderer or external path capability.
    pub fn downloads(&self) -> Result<Vec<DownloadStatus>, SyncError> {
        Ok(self.download_store()?.downloads()?)
    }
    /// Native-only download saved plans; does not grant a renderer or external path capability.
    pub fn download_saved_plans(&self, id: DownloadId) -> Result<Vec<KeepPlan>, SyncError> {
        Ok(self.download_store()?.download_saved_plans(id)?)
    }
    /// Native-only begin download; does not grant a renderer or external path capability.
    pub fn begin_download(
        &mut self,
        plans: Vec<KeepPlan>,
        selected: usize,
        listing: [u8; 32],
    ) -> Result<DownloadRun, SyncError> {
        self.download_authorized()?;
        Ok(self
            .download_store_mut()?
            .begin_download(plans, selected, listing)?)
    }
    /// Native-only resume download; does not grant a renderer or external path capability.
    pub fn resume_download(&mut self, id: DownloadId) -> Result<DownloadRun, SyncError> {
        self.download_authorized()?;
        Ok(self.download_store_mut()?.resume_download(id)?)
    }
    /// Native-only put download; does not grant a renderer or external path capability.
    pub fn put_download(&mut self, run: DownloadRun, bytes: &[u8]) -> Result<(), SyncError> {
        self.download_authorized()?;
        Ok(self.download_store_mut()?.put_download(run, bytes)?)
    }
    /// Native-only get download; does not grant a renderer or external path capability.
    pub fn get_download(
        &self,
        run: DownloadRun,
        cid: &Cid,
        max: usize,
    ) -> Result<Option<Vec<u8>>, SyncError> {
        self.download_authorized()?;
        Ok(self.download_store()?.get_download(run, cid, max)?)
    }
    /// Native-only fallback download; does not grant a renderer or external path capability.
    pub fn fallback_download(
        &mut self,
        run: DownloadRun,
        expected: &[u8; 32],
        tried: &[[u8; 32]],
        bad: bool,
    ) -> Result<Option<(DownloadRun, KeepPlan)>, SyncError> {
        self.download_authorized()?;
        Ok(self
            .download_store_mut()?
            .fallback_download(run, expected, tried, bad)?)
    }
    /// Native-only pause download; does not grant a renderer or external path capability.
    pub fn pause_download(&mut self, run: DownloadRun) -> Result<(), SyncError> {
        Ok(self.download_store_mut()?.pause_download(run)?)
    }
    /// Native-only block download; does not grant a renderer or external path capability.
    pub fn block_download(&mut self, run: DownloadRun) -> Result<(), SyncError> {
        self.download_authorized()?;
        Ok(self.download_store_mut()?.block_download(run)?)
    }
    /// Native-only finish download; does not grant a renderer or external path capability.
    pub fn finish_download(&mut self, run: DownloadRun) -> Result<DownloadExportLease, SyncError> {
        self.download_authorized()?;
        Ok(self.download_store_mut()?.finish_download(run)?)
    }
    /// Native-only download export; does not grant a renderer or external path capability.
    pub fn download_export(
        &self,
        id: DownloadId,
    ) -> Result<(DownloadExportLease, KeepPlan), SyncError> {
        self.download_authorized()?;
        Ok(self.download_store()?.download_export(id)?)
    }
    /// Native-only read download export; does not grant a renderer or external path capability.
    pub fn read_download_export(
        &self,
        lease: &DownloadExportLease,
        cid: &Cid,
        max: usize,
    ) -> Result<Vec<u8>, SyncError> {
        self.download_authorized()?;
        Ok(self
            .download_store()?
            .read_download_export(lease, cid, max)?)
    }
    /// Native-only cancel download; does not grant a renderer or external path capability.
    /// `Some` means Deleting is durable and native still has to remove the proven external
    /// partial before [`Self::finish_download_cleanup`] releases the record.
    pub fn cancel_download(&mut self, id: DownloadId) -> Result<Option<ExportState>, SyncError> {
        Ok(self.download_store_mut()?.cancel_download(id)?)
    }
    /// Release a Deleting record whose external cleanup already ran or was declined.
    pub fn finish_download_cleanup(&mut self, id: DownloadId) -> Result<(), SyncError> {
        Ok(self.download_store_mut()?.finish_download_cleanup(id)?)
    }
    /// Native-only saved export journal for preflight, reopen and cleanup. It names an external
    /// path and must never be projected to the renderer.
    pub fn download_export_state(&self, id: DownloadId) -> Result<Option<ExportState>, SyncError> {
        Ok(self.download_store()?.download_export_state(id)?)
    }
    /// Journal transitions. Live group/file-key authority is rechecked here like every other
    /// private-download effect; the caller separately checks the original native invocation.
    pub fn begin_download_export(
        &mut self,
        lease: &DownloadExportLease,
        journal: &ExportJournal,
        presence: ExportPresence,
    ) -> Result<(), SyncError> {
        self.download_authorized()?;
        Ok(self
            .download_store_mut()?
            .begin_download_export(lease, journal, presence)?)
    }
    pub fn resume_download_export(
        &mut self,
        lease: &DownloadExportLease,
        journal: &ExportJournal,
        presence: ExportPresence,
    ) -> Result<(), SyncError> {
        self.download_authorized()?;
        Ok(self
            .download_store_mut()?
            .resume_download_export(lease, journal, presence)?)
    }
    pub fn checkpoint_download_export(
        &mut self,
        lease: &DownloadExportLease,
        expected: &ExportJournal,
        next: &ExportJournal,
    ) -> Result<(), SyncError> {
        self.download_authorized()?;
        Ok(self
            .download_store_mut()?
            .checkpoint_download_export(lease, expected, next)?)
    }
    pub fn publish_download_export(
        &mut self,
        lease: &DownloadExportLease,
        expected: &ExportJournal,
    ) -> Result<(), SyncError> {
        self.download_authorized()?;
        Ok(self
            .download_store_mut()?
            .publish_download_export(lease, expected)?)
    }
    pub fn recover_download_export(
        &mut self,
        id: DownloadId,
        expected: &ExportJournal,
    ) -> Result<(), SyncError> {
        self.download_authorized()?;
        Ok(self
            .download_store_mut()?
            .recover_download_export(id, expected)?)
    }
    /// A halt only stops work, so it needs no file-key authority; it still requires the exact
    /// saved journal.
    pub fn halt_download_export(
        &mut self,
        id: DownloadId,
        expected: &ExportJournal,
        halt: ExportHalt,
    ) -> Result<(), SyncError> {
        Ok(self
            .download_store_mut()?
            .halt_download_export(id, expected, halt)?)
    }
    pub fn abandon_download_export(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
    ) -> Result<u64, SyncError> {
        Ok(self
            .download_store_mut()?
            .abandon_download_export(id, expected_intent)?)
    }
    /// Native-only set download paused; does not grant a renderer or external path capability.
    pub fn set_download_paused(
        &mut self,
        id: DownloadId,
        expected_intent: u64,
        paused: bool,
    ) -> Result<(), SyncError> {
        Ok(self
            .download_store_mut()?
            .set_download_paused(id, expected_intent, paused)?)
    }
    /// Native-only retry download; does not grant a renderer or external path capability.
    pub fn retry_download(
        &mut self,
        id: DownloadId,
        request: KeepRetry,
    ) -> Result<KeepRetryReceipt, SyncError> {
        Ok(self.download_store_mut()?.retry_download(id, request)?)
    }
    /// Charge persistent work using this Root's deterministic clock, never an IPC-supplied time.
    pub fn download_work(
        &mut self,
        run: DownloadRun,
        network: Option<KeepNetworkWork>,
    ) -> Result<KeepWork, SyncError> {
        self.download_authorized()?;
        let mono = self.clock.monotonic_ms();
        Ok(self
            .download_store_mut()?
            .download_work(run, network, mono)?)
    }
    pub fn download_run_current(&self, run: DownloadRun, cid: &Cid, version: &[u8; 32]) -> bool {
        self.download_authorized().is_ok()
            && self
                .download_store()
                .is_ok_and(|s| s.download_run_current(run, cid, version))
    }
}
