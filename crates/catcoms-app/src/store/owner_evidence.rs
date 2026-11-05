//! Mounted immutable owner configuration and canonical owner-state storage.
//!
//! Pinning is inert configuration, not permission to author/publish. The actual private factory,
//! original admission permit and canonical head transaction are additionally required. Numeric
//! server deletion never removes this namespace. Production activation remains separately gated.
use super::*;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_rt::RequestCancellation;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub(crate) mod canonical;
mod pins;
mod profile_capture;
pub(crate) use profile_capture::CapturedOwnerProfilePin;

/// One serializer for the entire root registry, across all numeric aliases/groups. Canonical
/// head writers are separate; never wait for admission while holding either of these writers.
#[derive(Default)]
pub(super) struct StoreOwnerState {
    pins: Mutex<pins::Runtime>,
    canonical: Arc<Mutex<canonical::Runtime>>,
    #[cfg(test)]
    pub(super) pin_fault: std::sync::atomic::AtomicU8,
    #[cfg(test)]
    pub(super) canonical_fault: std::sync::atomic::AtomicU8,
    #[cfg(test)]
    pub(super) directory_sync_failure: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub(super) restore_sync_failure: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub(super) before_owner_initialization_pool: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl ServerStore {
    /// Critical-format activation consumes the still-private mounted Store. Any failure drops
    /// its session and revokes original mount observers; no half-activated usable Store escapes.
    /// Call only in gated setup/unlock BEFORE exposing actors, with the existing passphrase in
    /// scope. This stores no passphrase and changes no existing admission nonce/phase/revision.
    /// It does NOT create a profile, grant, canonical head, admission or publication capability.
    pub fn into_owner_evidence_store(
        self,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        self.enable_chat_scope_management(passphrase, rng)?;
        {
            let format = self.chat_vault_format()?;
            let original_mount = self.chat_mount.observer();
            let mut roots = self
                .owner_evidence
                .pins
                .lock()
                .map_err(|_| pins::refused())?;
            if roots.managed != (format == catcoms_storage::VaultFormat::OwnerEvidence) {
                return Err(pins::refused());
            }
            self.flush_owner_pins(&mut roots, &original_mount, rng)?;
        }
        map_vault_passphrase_change(catcoms_storage::vault::activate_owner_evidence_vault(
            &self.dir, passphrase, rng,
        ))?;
        // Re-authenticate/update the SAME mounted scope runtime; this preserves v4 and every
        // existing Current/Retired/Initializing row. Never replace its table with an empty one.
        self.enable_chat_scope_management(passphrase, rng)?;
        self.owner_evidence
            .pins
            .lock()
            .map_err(|_| pins::refused())?
            .managed = true;
        Ok(self)
    }
}

impl ServerStore {
    /// A numeric mirror may never become the source or replace the canonical head of a known
    /// pinned group. Parse only borrowed outer MLS labels (no provider/Automerge expansion).
    /// Original bytes remain retained. A future separate bounded privileged reader may expose
    /// old numeric-prefix history; this slice does not implement that reader. The ordinary
    /// positive load/save path must refuse and use the canonical source adapter.
    pub(super) fn refuse_pinned_numeric_snapshot(&self, snapshot: &[u8]) -> Result<(), AppError> {
        let roots = self
            .owner_evidence
            .pins
            .lock()
            .map_err(|_| pins::refused())?;
        if roots.dirty {
            return Err(pins::refused());
        }
        let table = roots.table.as_ref().ok_or_else(pins::refused)?;
        if table.is_empty() {
            return Ok(());
        }
        let mut outer = Decoder::new(snapshot);
        let mut mls = Decoder::new(outer.get_bytes().map_err(|_| pins::refused())?);
        let _provider = mls.get_bytes().map_err(|_| pins::refused())?;
        let signer = mls.get_bytes().map_err(|_| pins::refused())?;
        let group = mls.get_bytes().map_err(|_| pins::refused())?;
        mls.finish().map_err(|_| pins::refused())?;
        if signer.len() != 32 || group.is_empty() || group.len() > 256 || table.contains_key(group)
        {
            return Err(pins::refused());
        }
        Ok(())
    }
}
