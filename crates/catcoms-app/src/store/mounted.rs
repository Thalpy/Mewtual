//! Closed native mounted-vault composition. This is a runtime owner, never a
//! serialized backend tag or permission to restore a V2 row as a legacy actor.
use super::{
    NativeTextRemovedJoinedVault, NativeTextVault, NativeTextVaultAuthenticationCheck, ServerStore,
};
use tokio::sync::{OwnedMappedMutexGuard, OwnedMutexGuard};

/// The ONE mounted vault owned by native AppState. The Legacy arm is explicit;
/// Active TextV2 and retired TextV2 have separate affine owners. Neither
/// provides raw Store/Current access.
/// Neither this sum nor its native owner is Clone, Deref or serializable.
pub enum MountedVault {
    Legacy(Box<ServerStore>),
    TextV2(NativeTextVault),
    TextV2Retired(Box<NativeTextRemovedJoinedVault>),
}
impl MountedVault {
    /// Legacy-only access under the original native mutex guard. None means
    /// the consumer must use the native V2 path or refuse; never manufacture a
    /// ServerActor, sidecar Store or default legacy backend for TextV2.
    pub fn as_legacy(&self) -> Option<&ServerStore> {
        match self {
            Self::Legacy(store) => Some(store),
            Self::TextV2(_) | Self::TextV2Retired(_) => None,
        }
    }

    /// Borrow the affine native facade without extracting its physical Store.
    /// Do not move this borrow across native guards/awaits: capture the facade's
    /// independently paid opaque observation and recheck the original instead.
    pub fn as_text_v2(&self) -> Option<&NativeTextVault> {
        match self {
            Self::TextV2(vault) => Some(vault),
            Self::Legacy(_) | Self::TextV2Retired(_) => None,
        }
    }

    /// Compare the authenticated association without lending ordinary
    /// access to the retired physical owner.
    pub fn matches_text_v2_authentication(
        &self,
        target: &NativeTextVaultAuthenticationCheck,
    ) -> bool {
        match self {
            Self::TextV2(vault) => target.is_for_original_vault(vault),
            Self::TextV2Retired(vault) => vault.matches_native_authentication_cached(target),
            Self::Legacy(_) => false,
        }
    }
}
impl From<ServerStore> for MountedVault {
    /// Allocate the legacy wrapper outside native guard/admission scopes, at
    /// initial mounting. This does not convert a native V2 physical owner.
    fn from(store: ServerStore) -> Self {
        Self::Legacy(Box::new(store))
    }
}
impl std::fmt::Debug for MountedVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Legacy(_) => "MountedVault::Legacy { .. }",
            Self::TextV2(_) => "MountedVault::TextV2 { .. }",
            Self::TextV2Retired(_) => "MountedVault::TextV2Retired { .. }",
        })
    }
}

/// Concrete old/new lease custody. A mapped lease retains the SAME original
/// native mutex, not a copied policy or an independently opened legacy Store.
pub(crate) enum LegacyStoreCustody {
    Standalone(OwnedMutexGuard<Option<ServerStore>>),
    Mounted(OwnedMappedMutexGuard<Option<MountedVault>, ServerStore>),
}
impl LegacyStoreCustody {
    pub(crate) fn as_ref(&self) -> Option<&ServerStore> {
        match self {
            Self::Standalone(guard) => guard.as_ref(),
            Self::Mounted(guard) => Some(guard),
        }
    }
    pub(crate) fn as_mut(&mut self) -> Option<&mut ServerStore> {
        match self {
            Self::Standalone(guard) => guard.as_mut(),
            Self::Mounted(guard) => Some(guard),
        }
    }
    pub(crate) fn from_mounted(
        guard: OwnedMutexGuard<Option<MountedVault>>,
        ordering: Box<dyn Send>,
    ) -> Result<(Self, Box<dyn Send>), MountedLeaseRefusal> {
        let reason = if guard.is_none() {
            MountedLeaseRefusalKind::NotMounted
        } else {
            MountedLeaseRefusalKind::DifferentBackend
        };
        // The guard excludes replacement during this closed selector. Neither
        // an arbitrary callback nor a V2 physical Store can enter this mapping.
        let mapped = OwnedMutexGuard::try_map(guard, |mounted| match mounted {
            Some(MountedVault::Legacy(store)) => Some(store.as_mut()),
            _ => None,
        });
        match mapped {
            Ok(mapped) => Ok((Self::Mounted(mapped), ordering)),
            Err(guard) => Err(MountedLeaseRefusal {
                _guard: guard,
                _ordering: ordering,
                reason,
            }),
        }
    }
}

/// No write/key effect was admitted. Keep the ORIGINAL mutex and ordering
/// custody until the native caller releases its final scope outside Root work.
/// No native backend, Raw Store or retry/SDK grant is returned by this error.
pub struct MountedLeaseRefusal {
    _guard: OwnedMutexGuard<Option<MountedVault>>,
    _ordering: Box<dyn Send>,
    reason: MountedLeaseRefusalKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountedLeaseRefusalKind {
    NotMounted,
    DifferentBackend,
}
impl MountedLeaseRefusal {
    pub fn kind(&self) -> MountedLeaseRefusalKind {
        self.reason
    }
    /// Caller-facing text after releasing retained native custody. Backend
    /// mismatch must not pretend a mounted native V2 vault is locked.
    pub fn message(&self) -> &'static str {
        match self.reason {
            MountedLeaseRefusalKind::NotMounted => "The vault is not open.",
            MountedLeaseRefusalKind::DifferentBackend => "This chat changed. Please try again.",
        }
    }
}
impl std::fmt::Debug for MountedLeaseRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MountedLeaseRefusal")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}
