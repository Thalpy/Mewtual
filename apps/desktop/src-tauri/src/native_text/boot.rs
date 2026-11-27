//! One owned warm startup chain and a joined-aware cold opener. A resumed
//! native join uses its exact covered request or its completed Welcome.
use super::*;
use catcoms_app::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    NativeIndexPolicy, NativeIndexStore, NativeIndexUpgrade, NativeRegistryInstallation,
    NativeRegistryPolicy, NativeRegistryStore, NativeTextGovernanceUpgrade,
    NativeTextJoinedStorage, NativeTextJoinedVaultUpgrade, NativeTextVaultUpgrade,
    V2CurrentLayoutPolicy, V2MemberOutboxPolicy,
};

pub(super) enum Bootstrap {
    Fresh,
    Existing,
    Installing(Box<NativeRegistryInstallation>),
    Registry(NativeRegistryStore),
    Upgrading2(Box<NativeIndexUpgrade>),
    Index2(NativeIndexStore),
    Upgrading3(Box<NativeTextVaultUpgrade>),
    Governing(Box<NativeTextGovernanceUpgrade>),
    Joining(Box<NativeTextJoinedVaultUpgrade>),
    Ready(super::sources::ReadyBundle),
    Mounted,
    /// This process lost its original warm continuation during preparation.
    Failed,
}

type Atoms = (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>);
pub(super) struct NativeRowBudget {
    budget: NativeUiInvocationBudget,
    instance: u64,
}
impl NativeRowBudget {
    pub(super) async fn capture(state: &AppState) -> Self {
        Self {
            budget: state.servers.lock().await.native_budget(),
            instance: state.next_server_instance.fetch_add(1, Ordering::Relaxed),
        }
    }
}
const FAILED: &str = "Vault startup did not complete. Its files have been kept.";

pub(super) fn current_policy() -> Result<V2CurrentLayoutPolicy, catcoms_app::AppError> {
    // Independent logical accounts, not allocated heaps or available disk.
    // Execution configuration is needed by the subsequent actual founder path;
    // DATA grammar alone must never be mistaken for permission to execute it.
    V2CurrentLayoutPolicy::new(
        ArchiveBootstrapCatalogPolicy::new(
            CriticalCatalogReadPool::new(1024 * 1024 * 1024)?,
            1024 * 1024 * 1024,
        )?
        .with_bootstrap_source_data_read()
        .with_checkpoint_join_execution(2, 10_000)?,
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1064)?,
        1024 * 1024 * 1024,
    )?
    .with_member_outbox(V2MemberOutboxPolicy::new(
        CriticalCatalogReadPool::new(64 * 1024 * 1024)?,
        256,
        256 * 1024 * 1024,
        1 << 20,
        super::super::server_table::NATIVE_ROWS,
        1024 * 1024 * 1024,
    )?)
}

impl Bootstrap {
    pub(super) fn advance(
        &mut self,
        directory: &Path,
        secret: &[u8],
        atoms: Atoms,
        budget: NativeUiInvocationBudget,
        source_policy: catcoms_app::store::NativeTextSourcePolicy,
        row: NativeRowBudget,
    ) -> Result<(), String> {
        if secret.is_empty() || secret.len() > MAX_SECRET {
            return Err("Enter a valid vault secret.".into());
        }
        let mut rng = OsCryptoRng;
        loop {
            // Before any potentially unwinding work, the slot is terminal. No
            // error/panic may turn a partially written vault back into Fresh.
            let phase = std::mem::replace(self, Self::Failed);
            match phase {
                Self::Fresh => {
                    if let Some(parent) = directory.parent() {
                        std::fs::create_dir_all(parent).map_err(|_| FAILED)?;
                    }
                    match std::fs::create_dir(directory) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            *self = Self::Existing;
                            continue;
                        }
                        Err(_) => return Err(FAILED.into()),
                    }
                    let current = current_policy().map_err(|_| FAILED)?;
                    let policy = NativeRegistryPolicy::new(
                        CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity())
                            .map_err(|_| FAILED)?,
                        NativeRegistryPolicy::required_persistent_capacity(),
                    )
                    .map_err(|_| FAILED)?;
                    // The Root constructor retains the same physical session,
                    // keys and lower lineage through all namespace declarations.
                    // A failure before its owning plan exists remains terminal.
                    let original = NativeRegistryInstallation::prepare_fresh(
                        directory, secret, &mut rng, current, policy,
                    )
                    .map_err(|_| FAILED)?;
                    *self = Self::Installing(Box::new(original));
                }
                Self::Existing => {
                    let result = (|| {
                        let current = current_policy().map_err(|_| FAILED)?;
                        const EXISTING: &str = "Could not unlock the existing Chat V2 vault.";
                        let storage = NativeTextJoinedStorage::open_for_native_startup(
                            directory, secret, &mut rng, current,
                        )
                        .map_err(|_| EXISTING)?;
                        match storage.row_count_data().map_err(|_| EXISTING)? {
                            0 => {
                                let vault = storage
                                    .into_native_vault_without_registered_row(atoms, budget)
                                    .map_err(|_| EXISTING)?;
                                super::sources::ReadyBundle::new(vault, &source_policy)
                            }
                            1 => {
                                let (_owner, signal) = tokio::sync::watch::channel(false);
                                let negative = catcoms_rt::RequestCancellation::new(signal, None);
                                if storage
                                    .recovered_join_registration_data()
                                    .map_err(|_| EXISTING)?
                                    .is_none()
                                {
                                    let founder = storage
                                        .prepare_recovered_founder()
                                        .map_err(|_| EXISTING)?;
                                    let name = founder.registration_data().1.to_owned();
                                    let display =
                                        founder.display_name_data().map_err(|_| EXISTING)?;
                                    let (vault, sources, selection, authentication) = founder
                                        .into_recovered_native_sources(
                                            atoms,
                                            budget,
                                            &source_policy,
                                            secret,
                                            catcoms_app::store::NativeTextCreationInputs::new(
                                                rng,
                                                OsCryptoRng,
                                                Box::new(SystemClock),
                                            ),
                                            &negative,
                                        )
                                        .map_err(|_| EXISTING)?;
                                    return super::sources::ReadyBundle::recovered_founder(
                                        vault,
                                        sources,
                                        selection,
                                        authentication,
                                        &name,
                                        &display,
                                        &row.budget,
                                        row.instance,
                                    );
                                }
                                let runtime = storage
                                    .bind_recovered_native_runtime(
                                        rng,
                                        Box::new(SystemClock),
                                        &negative,
                                    )
                                    .map_err(|_| EXISTING)?;
                                let (vault, sources) = runtime
                                    .into_recovered_native_sources(
                                        atoms,
                                        budget.clone(),
                                        &source_policy,
                                        &negative,
                                    )
                                    .map_err(|error| match error {
                                        catcoms_app::AppError::ChatV2MembershipRemoved => {
                                            "This device was removed from this Chat V2 server."
                                        }
                                        _ => EXISTING,
                                    })?;
                                super::sources::ReadyBundle::recovered(
                                    vault,
                                    sources,
                                    secret,
                                    &budget,
                                    &row.budget,
                                    row.instance,
                                )
                            }
                            _ => Err(EXISTING.into()),
                        }
                    })();
                    return match result {
                        Ok(ready) => {
                            *self = Self::Ready(ready);
                            Ok(())
                        }
                        Err(error) => {
                            *self = Self::Existing;
                            Err(error)
                        }
                    };
                }
                Self::Installing(mut original) => {
                    if original.retry().is_err() {
                        *self = Self::Installing(original);
                        return Err(FAILED.into());
                    }
                    match (*original).into_native_registry() {
                        Ok(owner) => *self = Self::Registry(owner),
                        Err(original) => {
                            *self = Self::Installing(Box::new(original));
                            return Err(FAILED.into());
                        }
                    }
                }
                Self::Registry(owner) => {
                    let policy = NativeIndexPolicy::new(
                        CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity())
                            .map_err(|_| FAILED)?,
                        NativeIndexPolicy::required_persistent_capacity(),
                    )
                    .map_err(|_| FAILED)?;
                    // This old preparation consumes its private owner on
                    // refusal. Failed remains terminal; never silently reopen.
                    *self = Self::Upgrading2(Box::new(
                        NativeIndexUpgrade::prepare(owner, policy, &mut rng).map_err(|_| FAILED)?,
                    ));
                }
                Self::Upgrading2(mut original) => {
                    if original.retry().is_err() {
                        *self = Self::Upgrading2(original);
                        return Err(FAILED.into());
                    }
                    match (*original).into_index_store() {
                        Ok(owner) => *self = Self::Index2(owner),
                        Err(original) => {
                            *self = Self::Upgrading2(Box::new(original));
                            return Err(FAILED.into());
                        }
                    }
                }
                Self::Index2(owner) => {
                    match NativeTextVaultUpgrade::prepare(
                        owner,
                        atoms.0.clone(),
                        atoms.1.clone(),
                        atoms.2.clone(),
                        budget.clone(),
                    ) {
                        Ok(original) => *self = Self::Upgrading3(Box::new(original)),
                        Err(refusal) => {
                            let (owner, _) = refusal.into_parts();
                            *self = Self::Index2(owner);
                            return Err(FAILED.into());
                        }
                    }
                }
                Self::Upgrading3(mut original) => {
                    if original.retry(&mut rng).is_err() {
                        *self = Self::Upgrading3(original);
                        return Err(FAILED.into());
                    }
                    match (*original).prepare_governance() {
                        Ok(original) => *self = Self::Governing(Box::new(original)),
                        Err(original) => {
                            *self = Self::Upgrading3(Box::new(original));
                            return Err(FAILED.into());
                        }
                    }
                }
                Self::Governing(mut original) => {
                    if let Err(_error) = original.retry(&mut rng) {
                        #[cfg(test)]
                        eprintln!("native startup refused during governed selection: {_error}");
                        *self = Self::Governing(original);
                        return Err(FAILED.into());
                    }
                    match (*original).prepare_joined_vault() {
                        Ok(original) => *self = Self::Joining(Box::new(original)),
                        Err(original) => {
                            #[cfg(test)]
                            eprintln!("native startup refused the original governed handoff");
                            *self = Self::Governing(Box::new(original));
                            return Err(FAILED.into());
                        }
                    }
                }
                Self::Joining(mut original) => {
                    if let Err(_error) = original.retry_with_shared_history(&mut rng) {
                        #[cfg(test)]
                        eprintln!("native startup refused during joined selection: {_error}");
                        *self = Self::Joining(original);
                        return Err(FAILED.into());
                    }
                    match (*original).into_vault() {
                        Ok(owner) => {
                            *self = Self::Ready(super::sources::ReadyBundle::new(
                                owner,
                                &source_policy,
                            )?)
                        }
                        Err(original) => {
                            #[cfg(test)]
                            eprintln!("native startup refused the original joined handoff");
                            *self = Self::Joining(Box::new(original));
                            return Err(FAILED.into());
                        }
                    }
                }
                Self::Ready(owner) => {
                    *self = Self::Ready(owner);
                    return Ok(());
                }
                Self::Mounted => {
                    *self = Self::Mounted;
                    return Ok(());
                }
                Self::Failed => return Err(FAILED.into()),
            }
        }
    }

    pub(super) fn take_ready(&mut self) -> Option<super::sources::ReadyBundle> {
        match std::mem::replace(self, Self::Mounted) {
            Self::Ready(owner) => Some(owner),
            other => {
                *self = other;
                None
            }
        }
    }
}
