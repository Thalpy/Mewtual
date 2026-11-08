//! Narrow explicit timestamp for an actual pending-commit merge. This does not
//! override join/receive/default clocks or validate a Source/governance permit.
use super::errors::MergePendingCommitError;
use super::*;
#[cfg(not(target_arch = "wasm32"))]
use std::time::SystemTime;
#[cfg(target_arch = "wasm32")]
use web_time::SystemTime;

/// Private choice shared by the SAME merge algorithm. Legacy calls the existing
/// replacement helper, so its SystemTime::now remains at the original statement.
#[derive(Clone, Copy)]
pub(super) enum MergeTimestamp {
    Legacy,
    Explicit(SystemTime),
}

impl MlsGroup {
    /// Merge the already-pending commit, supplying only the new epoch message
    /// secrets' persisted `added_at` timestamp. ALL other merge checks, effects,
    /// error ordering, key handling and cleanup use the ordinary implementation.
    ///
    /// The caller caches this time with its original operation before SDK entry;
    /// it must not retry an uncertain partial merge by selecting a new timestamp.
    /// Operational/no-pending remains the existing no-op. No device/group/key is
    /// returned, and this is not membership, Source, save or publication authority.
    /// This does not change lifetime validation or receive/join/default clocks.
    pub fn merge_pending_commit_with_time<Provider: OpenMlsProvider>(
        &mut self,
        provider: &Provider,
        now: SystemTime,
    ) -> Result<(), MergePendingCommitError<Provider::StorageError>> {
        self.merge_pending_commit_at(provider, MergeTimestamp::Explicit(now))
    }

    /// Numerical inline DATA only. Transitive graphs, SDK locals, copies and
    /// operation buffers are additional; this reserves or permits nothing.
    pub const fn text_add_merge_time_layout_bytes() -> usize {
        core::mem::size_of::<MergeTimestamp>()
    }
}
