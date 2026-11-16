//! Saved files propagate (owner decision 2 of cache housekeeping,
//! `docs/design-cache-housekeeping.md`): a private Download's chunks reach the shared cache, from
//! which this device serves members, only once a save of that Download has been **published**
//! outside the app. An in-progress, paused, failed, cancelled or removed Download never makes this
//! device a source.
//!
//! - Acquisition copies nothing. The private store is never served.
//! - Each export read (`ReadDownloadChunk`) stages its chunk in the primary cache's staging area,
//!   unless the cache already holds it. Staged blobs are invisible to `has`/`get`, to serving and
//!   to `file_available`, and are not counted in cache usage.
//! - `FinishDownloadExport`, which native calls only after the save published, releases the
//!   private record and then promotes the staged chunks (one rename each, no re-encryption), but
//!   only if the export lease was still current and the release succeeded. A lease retired
//!   meanwhile (a Remove that won the race) or a refused release drops them instead.
//! - Cancel/Remove drops them. Anything a failed export leaves staged is never served and goes at
//!   the next attach (`Server::clear_staged_uploads`), or when the Download is saved or removed.
//!
//! Staging happens during the export rather than all at once at its end so that the end costs only
//! renames: a whole 1 GiB file re-sealed in one actor turn would stall every other command of the
//! server for seconds. Every failure here is logged and never fails the read, save or removal.
//!
//! Limitation: a journaled save whose temporary second name could not be removed keeps its record
//! (native skips `FinishDownloadExport`), so that save does not propagate; Clear later drops what
//! it staged.
use super::*;

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Stage one export chunk (already read from the private store, whose read re-verified its
    /// ciphertext CID) for propagation once the save publishes. A chunk the cache holds already is
    /// skipped; a staged blob that does not hash to `cid` is dropped again.
    pub(crate) fn stage_export_chunk(&mut self, cid: &Cid, ciphertext: &[u8]) {
        if self.sync.has_blob(cid) {
            return;
        }
        match self.sync.put_staged_blob(ciphertext) {
            Ok(staged) if staged == *cid => {}
            Ok(staged) => {
                let _ = self.sync.drop_staged_blob(&staged);
                tracing::warn!("an export chunk did not match its address and was not staged");
            }
            Err(error) => {
                tracing::warn!(%error, "an export chunk could not be staged for the shared cache");
            }
        }
    }

    /// The `FinishDownloadExport` step, called only after native published the save: release the
    /// private record, then promote what the export staged into the shared cache, or drop it if
    /// the lease had been retired (the Download was removed meanwhile) or the release was refused.
    pub(crate) fn finish_download_export_and_propagate(
        &mut self,
        ready: &DownloadReady,
    ) -> Result<(), catcoms_sync::SyncError> {
        // Sampled before the release, which itself retires the lease.
        let current = ready.is_current();
        let finished = self.sync.finish_download_export(&ready.lease);
        let publish = current && finished.is_ok();
        for chunk in &ready.manifest.chunks {
            let cid = chunk.ciphertext_cid;
            let outcome = if publish {
                self.sync.promote_staged_blob(&cid)
            } else {
                self.sync.drop_staged_blob(&cid)
            };
            if let Err(error) = outcome {
                tracing::warn!(%error, publish, "a saved chunk could not be moved into the shared cache");
            }
        }
        finished
    }

    /// The `CancelDownload` step (Remove/Clear): cancel the private record, then drop anything its
    /// exports staged, so a removed Download never reaches the shared cache. The saved plans are
    /// read first because a cancel without an export journal releases the record at once.
    pub(crate) fn cancel_download_and_unstage(
        &mut self,
        id: catcoms_storage::kept::DownloadId,
    ) -> Result<Option<catcoms_storage::kept::ExportState>, catcoms_sync::SyncError> {
        let staged: Vec<Cid> = self
            .sync
            .download_saved_plans(id)
            .map(|plans| {
                plans
                    .iter()
                    .flat_map(|plan| plan.chunks.iter().map(|(cid, _)| *cid))
                    .collect()
            })
            .unwrap_or_default();
        let cancelled = self.sync.cancel_download(id)?;
        for cid in &staged {
            // Only staged blobs: held cache content is never touched by this.
            let _ = self.sync.drop_staged_blob(cid);
        }
        Ok(cancelled)
    }
}
