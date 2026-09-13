//! Volatile tail preparation retains the original mount/server scope across network and cold
//! parsing. This adapter has no store writes, ordinary receiver permit or canonical source.
use super::*;
use catcoms_sync::registry_seed::{
    CompletedProvisionalStudioTail, PendingProvisionalStudioTail, ProvisionalStudioTailPreparation,
};

pub struct ProvisionalStudioTailAttempt<T: MeshTransport> {
    inner: PendingProvisionalStudioTail<T>,
    scope: seed::Scope,
}
pub struct ProvisionalStudioTailCompletion {
    inner: CompletedProvisionalStudioTail,
    scope: seed::Scope,
}
pub struct ServerProvisionalStudioTailPreparation {
    inner: ProvisionalStudioTailPreparation,
    scope: seed::Scope,
}
impl<T: MeshTransport> ProvisionalStudioTailAttempt<T> {
    pub async fn fetch(self) -> ProvisionalStudioTailCompletion {
        ProvisionalStudioTailCompletion {
            inner: self.inner.fetch().await,
            scope: self.scope,
        }
    }
}
impl ServerProvisionalStudioTailPreparation {
    /// The caller must own a bounded process worker permit until this synchronous call exits.
    /// A completed result owns its seed reservation, independently of that parser permit.
    pub fn prepare(self) -> Result<ServerPreparedProvisionalStudioSeed, AppError> {
        Ok(ServerPreparedProvisionalStudioSeed {
            inner: self.inner.prepare()?,
            scope: self.scope,
        })
    }
}
impl<T: MeshTransport> std::fmt::Debug for ProvisionalStudioTailAttempt<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProvisionalStudioTailAttempt { .. }")
    }
}
impl std::fmt::Debug for ProvisionalStudioTailCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProvisionalStudioTailCompletion { .. }")
    }
}
impl std::fmt::Debug for ServerProvisionalStudioTailPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerProvisionalStudioTailPreparation { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    pub fn prepare_provisional_studio_tail(
        &mut self,
        store: &ServerStore,
        server: u64,
        seed: ServerPreparedProvisionalStudioSeed,
    ) -> Result<ProvisionalStudioTailAttempt<T>, AppError> {
        self.check_provisional_seed_scope(store, server, &seed.scope)?;
        Ok(ProvisionalStudioTailAttempt {
            inner: self.sync.prepare_provisional_studio_tail(seed.inner)?,
            scope: seed.scope,
        })
    }
    pub fn complete_provisional_studio_tail(
        &mut self,
        store: &ServerStore,
        server: u64,
        completed: ProvisionalStudioTailCompletion,
    ) -> Result<Option<ServerProvisionalStudioTailPreparation>, AppError> {
        self.check_provisional_seed_scope(store, server, &completed.scope)?;
        Ok(self
            .sync
            .complete_provisional_studio_tail(completed.inner)?
            .map(|inner| ServerProvisionalStudioTailPreparation {
                inner,
                scope: completed.scope,
            }))
    }
}
