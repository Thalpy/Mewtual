//! One worker path for public Keep and private Download custody. The enum preserves the
//! immutable storage purpose across every local/network/fallback/terminal effect; a matching
//! CID can never convert a Download run into a Keep capability.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SavedRun {
    Keep(catcoms_storage::kept::KeepRun),
    Download(catcoms_storage::kept::DownloadRun),
}
impl SavedRun {
    pub(super) fn pause<T: MeshTransport, R: CryptoRngCore>(
        self,
        server: &mut Server<T, R>,
    ) -> Result<(), catcoms_sync::SyncError> {
        match self {
            Self::Keep(run) => server.sync.pause_keep(run),
            Self::Download(run) => server.sync.pause_download(run),
        }
    }
    pub(super) fn block<T: MeshTransport, R: CryptoRngCore>(
        self,
        server: &mut Server<T, R>,
    ) -> Result<(), catcoms_sync::SyncError> {
        match self {
            Self::Keep(run) => server.sync.block_keep(run),
            Self::Download(run) => server.sync.block_download(run),
        }
    }
    pub(super) fn put<T: MeshTransport, R: CryptoRngCore>(
        self,
        server: &mut Server<T, R>,
        bytes: &[u8],
    ) -> Result<(), catcoms_sync::SyncError> {
        match self {
            Self::Keep(run) => server.sync.put_keep(run, bytes),
            Self::Download(run) => server.sync.put_download(run, bytes),
        }
    }
    pub(super) fn get<T: MeshTransport, R: CryptoRngCore>(
        self,
        server: &Server<T, R>,
        cid: &Cid,
        max: usize,
    ) -> Result<Option<Vec<u8>>, catcoms_sync::SyncError> {
        match self {
            Self::Keep(run) => server.sync.get_keep(run, cid, max),
            Self::Download(run) => server.sync.get_download(run, cid, max),
        }
    }
    pub(super) fn work<T: MeshTransport, R: CryptoRngCore>(
        self,
        server: &mut Server<T, R>,
        network: Option<catcoms_storage::kept::KeepNetworkWork>,
    ) -> Result<catcoms_storage::kept::KeepWork, catcoms_sync::SyncError> {
        match self {
            Self::Keep(run) => server.sync.keep_work(run, network),
            Self::Download(run) => server.sync.download_work(run, network),
        }
    }
    pub(super) fn current<T: MeshTransport, R: CryptoRngCore>(
        self,
        server: &Server<T, R>,
        cid: &Cid,
        version: &[u8; 32],
    ) -> bool {
        match self {
            Self::Keep(run) => server.sync.keep_run_current(run, cid, version),
            Self::Download(run) => server.sync.download_run_current(run, cid, version),
        }
    }
    pub(super) fn fallback<T: MeshTransport, R: CryptoRngCore>(
        self,
        server: &mut Server<T, R>,
        expected: &[u8; 32],
        tried: &[[u8; 32]],
        bad: bool,
    ) -> Result<Option<(Self, catcoms_storage::kept::KeepPlan)>, catcoms_sync::SyncError> {
        match self {
            Self::Keep(run) => Ok(server
                .sync
                .fallback_keep(run, expected, tried, bad)?
                .map(|(run, plan)| (Self::Keep(run), plan))),
            Self::Download(run) => Ok(server
                .sync
                .fallback_download(run, expected, tried, bad)?
                .map(|(run, plan)| (Self::Download(run), plan))),
        }
    }
    pub(super) fn finish<T: MeshTransport, R: CryptoRngCore>(
        self,
        server: &mut Server<T, R>,
    ) -> Result<Option<catcoms_storage::kept::DownloadExportLease>, catcoms_sync::SyncError> {
        match self {
            Self::Keep(run) => {
                server.sync.finish_keep(run)?;
                Ok(None)
            }
            Self::Download(run) => Ok(Some(server.sync.finish_download(run)?)),
        }
    }
}
