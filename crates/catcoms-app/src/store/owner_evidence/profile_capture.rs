//! Inert immutable expected-root custody captured BEFORE an actual join starts.
//!
//! A download, current roster, snapshot or self-signed Profile cannot mint this object. The
//! trusted-host path deliberately names its trust decision: it consumes metadata of an opaque
//! ACTUAL founder preparation supplied by the host. Production automatic invite/root transfer
//! remains a separate protocol gate. Neither capture method installs a pin or authorizes effects.
use super::*;
use catcoms_sync::owner_evidence::PreparedOwnerEvidence;

/// Original destination mount plus expected root, not a runtime authoring capability.
/// Non-Clone/nonserialized: the fixed join attempt owns it before its first await and retains
/// that same object through initialization. No post-join setter or scalar constructor exists.
pub(crate) struct CapturedOwnerProfilePin {
    mount: Arc<()>,
    negative: RequestCancellation,
    group: Vec<u8>,
    profile: OwnerFactId,
    founder: [u8; 32],
}
impl std::fmt::Debug for CapturedOwnerProfilePin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturedOwnerProfilePin")
            .finish_non_exhaustive()
    }
}
impl CapturedOwnerProfilePin {
    pub(crate) fn group(&self) -> &[u8] {
        &self.group
    }
    pub(crate) fn profile(&self) -> OwnerFactId {
        self.profile
    }
    pub(crate) fn founder(&self) -> &[u8; 32] {
        &self.founder
    }
    pub(crate) fn is_revoked(&self) -> bool {
        self.negative.is_cancelled()
    }

    /// Recheck ORIGINAL physical destination custody; never select a replacement mount/pin.
    /// No admission lock is acquired, so this can also run inside an already-held permit.
    /// Missing installed configuration is allowed only because this object is inert. The
    /// canonical saver must install and durably sync the matching immutable pin before saving
    /// or releasing a positive initialized owner. Conflicting/uncertain configuration refuses.
    pub(crate) fn validate_original(
        &self,
        store: &ServerStore,
        group: &[u8],
    ) -> Result<(), AppError> {
        if group != self.group()
            || !Arc::ptr_eq(&self.mount, &store.registry_mount())
            || self.negative.is_cancelled()
        {
            return Err(pins::refused());
        }
        let roots = store
            .owner_evidence
            .pins
            .lock()
            .map_err(|_| pins::refused())?;
        if roots.dirty {
            return Err(pins::refused());
        }
        if roots
            .table
            .as_ref()
            .ok_or_else(pins::refused)?
            .get(group)
            .is_some_and(|pin| pin.profile != self.profile || pin.founder != self.founder)
        {
            return Err(pins::refused());
        }
        Ok(())
    }
}
impl ServerStore {
    /// Explicit trusted actual-founder handoff, captured on the destination BEFORE join.
    /// The preparation is unforgeable by decoding, but the host still chooses whom it trusts;
    /// this method does not authenticate transport distribution or silently trust an invite's
    /// non-founder inviter. Managed3 may capture inert expectations during setup; positive
    /// canonical activation requires the separately authenticated critical OwnerEvidence mode.
    pub(crate) fn capture_trusted_owner_profile(
        &self,
        prepared: &PreparedOwnerEvidence,
    ) -> Result<CapturedOwnerProfilePin, AppError> {
        if self.chat_vault_format()? == catcoms_storage::VaultFormat::Legacy {
            return Err(pins::refused());
        }
        if prepared.group().is_empty() || prepared.group().len() > 256 {
            return Err(pins::refused());
        }
        let capture = CapturedOwnerProfilePin {
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            group: prepared.group().to_vec(),
            profile: prepared.profile_id(),
            founder: *prepared.founder(),
        };
        capture.validate_original(self, capture.group())?;
        Ok(capture)
    }

    /// Capture only an already installed immutable mounted root, never source-provided labels.
    pub(crate) fn capture_mounted_owner_profile(
        &self,
        group: &[u8],
    ) -> Result<CapturedOwnerProfilePin, AppError> {
        self.chat_vault_format()?;
        let (profile, founder) = {
            let roots = self
                .owner_evidence
                .pins
                .lock()
                .map_err(|_| pins::refused())?;
            if !roots.managed || roots.dirty {
                return Err(pins::refused());
            }
            let pin = roots
                .table
                .as_ref()
                .ok_or_else(pins::refused)?
                .get(group)
                .ok_or_else(pins::refused)?;
            (pin.profile, pin.founder)
        };
        let capture = CapturedOwnerProfilePin {
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            group: group.to_vec(),
            profile,
            founder,
        };
        capture.validate_original(self, group)?;
        Ok(capture)
    }
}

impl ServerStore {
    /// Inert root first, before the covering private snapshot. A post-rename uncertain registry
    /// rewrites its SAME entire table before returning success; no other group/alias is dropped.
    pub(super) fn install_private_founder_pin<T: catcoms_rt::MeshTransport, R: CryptoRngCore>(
        &self,
        private: &crate::store::chat_admission::fresh::PrivateManagedServer<T, R>,
        prepared: &PreparedOwnerEvidence,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if !self.chat_vault_format()?.requires_owner_evidence()
            || !prepared.matches_original_mls_state(&private.server()?.sync)
        {
            return Err(pins::refused());
        }
        let original = private.preparation()?;
        use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
        if original.group() != prepared.group()
            || original.signer() != prepared.founder()
            || original.reservation_observer().is_cancelled()
        {
            return Err(pins::refused());
        }
        let negative = self.chat_mount.observer();
        let mut roots = self
            .owner_evidence
            .pins
            .lock()
            .map_err(|_| pins::refused())?;
        if !roots.managed {
            return Err(pins::refused());
        }
        let table = roots.table.as_mut().ok_or_else(pins::refused)?;
        if let Some(pin) = table.get(prepared.group()) {
            if pin.profile != prepared.profile_id() || &pin.founder != prepared.founder() {
                return Err(pins::refused());
            }
        } else {
            if !pins::fits_rows(table.len().checked_add(1).ok_or_else(pins::refused)?) {
                return Err(pins::refused());
            }
            let mut source = blake3::Hasher::new();
            source.update(b"catcoms/private-founder-pin-source/v1\0");
            source.update(prepared.profile_id().as_bytes());
            table.insert(
                prepared.group().to_vec(),
                pins::Pin {
                    founder: *prepared.founder(),
                    profile: prepared.profile_id(),
                    origin: pins::Origin::PrivateFounder,
                    source: *source.finalize().as_bytes(),
                },
            );
            roots.dirty = true;
        }
        if roots.dirty {
            self.flush_owner_pins(&mut roots, &negative, rng)?;
        }
        if negative.is_cancelled() || original.reservation_observer().is_cancelled() {
            return Err(pins::refused());
        }
        Ok(())
    }
}

impl ServerStore {
    /// CHECKPOINT trusted-process handoff from an actual Saved/Current/writer owner, not raw
    /// public labels or a prepare-only Profile. Production remote acquisition uses the separately
    /// signed complete invite + real public-header floor; this is never that wire adapter.
    pub(crate) fn capture_ready_owner_profile<T: catcoms_rt::MeshTransport, R: CryptoRngCore>(
        &self,
        source_store: &ServerStore,
        source: &canonical::OwnedOwnerServer<T, R>,
    ) -> Result<CapturedOwnerProfilePin, AppError> {
        let (group, profile, founder, original) =
            source.capture_ready_profile_origin(source_store)?;
        // SOURCE Current/head guards have returned. Never hold them while entering DESTINATION
        // admission/pins; two crossed trusted-process handoffs cannot create a lock cycle.
        if self.chat_vault_format()? == catcoms_storage::VaultFormat::Legacy
            || original.is_cancelled()
        {
            return Err(pins::refused());
        }
        let capture = CapturedOwnerProfilePin {
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            group,
            profile,
            founder,
        };
        capture.validate_original(self, capture.group())?;
        if original.is_cancelled() {
            return Err(pins::refused());
        }
        // Root authentication is now an inert immutable trust decision. Do not retain source's
        // ephemeral publisher: owner restart/offline must not invalidate already captured root.
        Ok(capture)
    }
}

impl ServerStore {
    pub(super) fn install_joined_owner_pin<T: catcoms_rt::MeshTransport, R: CryptoRngCore>(
        &self,
        private: &crate::store::chat_admission::fresh::PrivateManagedServer<T, R>,
        verified: &catcoms_sync::owner_evidence::VerifiedJoinedOwnerEvidence,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if !self.chat_vault_format()?.requires_owner_evidence()
            || !verified.matches_original(&private.server()?.sync)
        {
            return Err(pins::refused());
        }
        let captured = private.captured_profile().ok_or_else(pins::refused)?;
        captured.validate_original(self, verified.group())?;
        if captured.profile() != verified.profile_id() {
            return Err(pins::refused());
        }
        let original = private.preparation()?;
        use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;
        if original.group() != verified.group()
            || original.signer() != verified.subject()
            || original.reservation_observer().is_cancelled()
        {
            return Err(pins::refused());
        }
        let negative = self.chat_mount.observer();
        let mut roots = self
            .owner_evidence
            .pins
            .lock()
            .map_err(|_| pins::refused())?;
        if !roots.managed {
            return Err(pins::refused());
        }
        let table = roots.table.as_mut().ok_or_else(pins::refused)?;
        if let Some(pin) = table.get(verified.group()) {
            if pin.profile != captured.profile() || &pin.founder != captured.founder() {
                return Err(pins::refused());
            }
        } else {
            if !pins::fits_rows(table.len().checked_add(1).ok_or_else(pins::refused)?) {
                return Err(pins::refused());
            }
            let mut source = blake3::Hasher::new();
            source.update(b"catcoms/trusted-owner-handoff-source/v1\0");
            source.update(captured.profile().as_bytes());
            table.insert(
                verified.group().to_vec(),
                pins::Pin {
                    profile: captured.profile(),
                    founder: *captured.founder(),
                    origin: pins::Origin::TrustedHost,
                    source: *source.finalize().as_bytes(),
                },
            );
            roots.dirty = true;
        }
        if roots.dirty {
            self.flush_owner_pins(&mut roots, &negative, rng)?;
        }
        if negative.is_cancelled() || original.reservation_observer().is_cancelled() {
            return Err(pins::refused());
        }
        Ok(())
    }
}
