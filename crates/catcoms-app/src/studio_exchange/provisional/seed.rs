use super::*;
use catcoms_sync::registry_seed::{
    CompletedProvisionalStudioSeed, PendingProvisionalStudioSeed, PreparedProvisionalStudioSeed,
    ProvisionalStudioSeedPreparation, ProvisionalStudioSeedUse,
};

struct Scope {
    mount: Arc<()>,
    server: u64,
    target: StudioTarget,
}
pub struct ProvisionalStudioSeedAttempt<T: MeshTransport> {
    inner: PendingProvisionalStudioSeed<T>,
    scope: Scope,
}
pub struct ProvisionalStudioSeedCompletion {
    inner: CompletedProvisionalStudioSeed,
    scope: Scope,
}
pub struct ServerProvisionalStudioSeedPreparation {
    inner: ProvisionalStudioSeedPreparation,
    scope: Scope,
}
pub struct ServerPreparedProvisionalStudioSeed {
    inner: PreparedProvisionalStudioSeed,
    scope: Scope,
}
impl<T: MeshTransport> ProvisionalStudioSeedAttempt<T> {
    pub async fn fetch(self) -> ProvisionalStudioSeedCompletion {
        ProvisionalStudioSeedCompletion {
            inner: self.inner.fetch().await,
            scope: self.scope,
        }
    }
}
impl ServerProvisionalStudioSeedPreparation {
    /// Cold work owns its capacity without a Server/store borrow. The actor must separately
    /// retain a worker permit until this call exits, even when its join handle is cancelled.
    pub fn prepare(self) -> Result<ServerPreparedProvisionalStudioSeed, AppError> {
        Ok(ServerPreparedProvisionalStudioSeed {
            inner: self.inner.prepare()?,
            scope: self.scope,
        })
    }
}
impl<T: MeshTransport> std::fmt::Debug for ProvisionalStudioSeedAttempt<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProvisionalStudioSeedAttempt { .. }")
    }
}
macro_rules! redacted_debug {
    ($($name:ident),+) => {$(impl std::fmt::Debug for $name {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(concat!(stringify!($name), " { .. }"))
        }
    })+};
}
redacted_debug!(
    ProvisionalStudioSeedCompletion,
    ServerProvisionalStudioSeedPreparation,
    ServerPreparedProvisionalStudioSeed
);
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    fn check_provisional_seed_scope(
        &self,
        store: &ServerStore,
        server: u64,
        scope: &Scope,
    ) -> Result<(), AppError> {
        if scope.server != server || !Arc::ptr_eq(&scope.mount, &store.registry_mount()) {
            return Err(invalid(
                "provisional seed belongs to a replaced mount/server",
            ));
        }
        self.check_studio_channel(scope.target)
    }
    pub fn prepare_provisional_studio_seed(
        &mut self,
        store: &ServerStore,
        server: u64,
        hint: ServerProvisionalStudioHint,
    ) -> Result<ProvisionalStudioSeedAttempt<T>, AppError> {
        let scope = Scope {
            mount: hint.mount,
            server: hint.server,
            target: hint.target,
        };
        self.check_provisional_seed_scope(store, server, &scope)?;
        Ok(ProvisionalStudioSeedAttempt {
            inner: self.sync.prepare_provisional_studio_seed(hint.inner)?,
            scope,
        })
    }
    pub fn complete_provisional_studio_seed(
        &mut self,
        store: &ServerStore,
        server: u64,
        completed: ProvisionalStudioSeedCompletion,
    ) -> Result<Option<ServerProvisionalStudioSeedPreparation>, AppError> {
        self.check_provisional_seed_scope(store, server, &completed.scope)?;
        Ok(self
            .sync
            .complete_provisional_studio_seed(completed.inner)?
            .map(|inner| ServerProvisionalStudioSeedPreparation {
                inner,
                scope: completed.scope,
            }))
    }
    /// Recheck lifecycle after detached parsing and at each use. The callback receives only an
    /// unconfirmed projection; no source, pointer, journal or canonical Read is changed here.
    pub fn with_provisional_studio_seed<O>(
        &self,
        store: &ServerStore,
        server: u64,
        prepared: &ServerPreparedProvisionalStudioSeed,
        inspect: impl FnOnce(ProvisionalStudioSeedUse<'_>) -> O,
    ) -> Result<O, AppError> {
        self.check_provisional_seed_scope(store, server, &prepared.scope)?;
        Ok(self
            .sync
            .with_provisional_studio_seed(&prepared.inner, inspect)?)
    }
}
