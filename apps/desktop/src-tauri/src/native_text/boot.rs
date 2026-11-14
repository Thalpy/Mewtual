//! One owned warm startup chain. This deliberately has no cold-format selector:
//! an existing directory is refused without trying any legacy/native opener.
//! Once a transaction exists, only that exact transaction may finish it.
use super::*;
use catcoms_app::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    NativeIndexPolicy, NativeIndexStore, NativeIndexUpgrade, NativeRegistryInstallation,
    NativeRegistryPolicy, NativeRegistryStore, NativeTextVault, NativeTextVaultUpgrade,
    V2CurrentLayoutPolicy, V2MemberOutboxPolicy,
};

pub(super) enum Bootstrap {
    Fresh,
    Installing(Box<NativeRegistryInstallation>),
    Registry(NativeRegistryStore),
    Upgrading2(Box<NativeIndexUpgrade>),
    Index2(NativeIndexStore),
    Upgrading3(Box<NativeTextVaultUpgrade>),
    Ready(NativeTextVault),
    Mounted,
    /// Initial preparation/unwind cannot safely reconstruct warm custody. Keep
    /// every file and refuse; the eventual closed cold selector must handle it.
    Failed,
}

type Atoms = (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>);
const FAILED: &str = "Vault startup did not complete. Its files have been kept.";

fn current_policy() -> Result<V2CurrentLayoutPolicy, catcoms_app::AppError> {
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
        1,
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
                    // This is a negative fresh-install restriction, not format
                    // inference or native authority. Never inspect an existing
                    // vault and then choose a different grammar from its files.
                    if let Some(parent) = directory.parent() {
                        std::fs::create_dir_all(parent).map_err(|_| FAILED)?;
                    }
                    std::fs::create_dir(directory).map_err(|_| {
                        "Existing vaults are not supported by this Chat V2 checkpoint."
                    })?;
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
                    match (*original).into_vault() {
                        Ok(owner) => *self = Self::Ready(owner),
                        Err(original) => {
                            *self = Self::Upgrading3(Box::new(original));
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

    pub(super) fn take_ready(&mut self) -> Option<NativeTextVault> {
        match std::mem::replace(self, Self::Mounted) {
            Self::Ready(owner) => Some(owner),
            other => {
                *self = other;
                None
            }
        }
    }
}
