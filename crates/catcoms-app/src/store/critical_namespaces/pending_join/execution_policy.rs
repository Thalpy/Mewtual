//! Explicit trusted PRIVATE checkpoint-network configuration. Not a wire/IPC permission.
use super::*;

#[derive(Clone, Copy)]
pub(in crate::store::critical_namespaces) struct Settings {
    pub(in crate::store::critical_namespaces) initial_limit: u32,
    pub(in crate::store::critical_namespaces) timeout_ms: u64,
}
impl Settings {
    pub(in crate::store::critical_namespaces) fn new(
        initial_limit: u32,
        timeout_ms: u64,
    ) -> Result<Self, AppError> {
        if initial_limit == 0
            || initial_limit > super::execution_state::MAX_GRANT
            || timeout_ms == 0
            || timeout_ms > 60_000
        {
            return Err(refused());
        }
        Ok(Self {
            initial_limit,
            timeout_ms,
        })
    }
}

/// Explicit physical-open policy for the original finite checkpoint network consumer. It
/// shares the SAME caller pool with read/write/source/lower work. Saved grant/counter labels
/// and cloned policy handles are not resource reservations or live source/task permission.
#[derive(Clone)]
pub struct CheckpointJoinExecutionPolicy {
    pub(in crate::store) pending: PendingJoinCatalogPolicy,
}
impl std::fmt::Debug for CheckpointJoinExecutionPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CheckpointJoinExecutionPolicy")
            .finish_non_exhaustive()
    }
}
impl CheckpointJoinExecutionPolicy {
    pub fn new(
        pool: CriticalCatalogReadPool,
        persistent_capacity: usize,
        initial_limit: u32,
        timeout_ms: u64,
    ) -> Result<Self, AppError> {
        let settings = Settings::new(initial_limit, timeout_ms)?;
        let mut pending = PendingJoinCatalogPolicy::new(pool, persistent_capacity)?;
        pending.execution = Some(settings);
        Ok(Self { pending })
    }
}

impl ServerStore {
    /// Open an ALREADY activated v6 vault with explicit original checkpoint execution support.
    /// It starts no worker, grants/resets no attempts and never exposes a ready owner. Both
    /// current/stage use the actual policy BEFORE old cleaners. Default/Pending stays strict.
    pub fn open_with_checkpoint_join_execution(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        policy: CheckpointJoinExecutionPolicy,
    ) -> Result<Self, AppError> {
        let pending = policy.pending;
        Self::open_with_catalog_policy(
            dir,
            passphrase,
            rng,
            Some(pending.pool.clone()),
            Some(pending),
            None,
        )
    }
}
