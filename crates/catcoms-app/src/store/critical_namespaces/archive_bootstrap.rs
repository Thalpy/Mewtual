//! The concrete, unexposed archive-layout declaration. This is separate from source admission:
//! no descriptor, root marker or catalogue parser grants Fresh, Current or a message issuer.
use super::*;
use std::sync::Mutex;

pub(super) const ARCHIVE_ID: u32 = 2;
pub(super) const ARCHIVE_VERSION: u32 = 1;
const DESCRIPTOR_TAG: &str = "catcoms/private-chat-bootstrap-layout/v1";
pub(super) const MAX_DESCRIPTOR_BYTES: usize = 4096;
pub(in crate::store) const ROOT_MARKER: &str = "layout.bin";
pub(in crate::store) const MAX_MARKER_FRAMED: usize = MAX_DESCRIPTOR_BYTES + 512;
pub(in crate::store) const MAX_SEED_FRAMED: usize =
    super::pending_join_codec::MAX_CATALOG_FRAMED + MAX_MARKER_FRAMED + 8192;
pub(in crate::store) const DECLARATION_FEE: usize = 12 * MAX_SEED_FRAMED + 64 * 1024;
pub(in crate::store) const ROOT_METADATA_FEE: usize = 64 * 1024;

/// Trusted, explicit PRIVATE archive-bootstrap configuration. Its actual consumer is a newly
/// opened, never-exposed Store; normal and Pending-only mounts remain unable to read ARCHIVE2.
/// Supply one shared pool for all read/preparation/retained objects. This is a logical bound,
/// not free-space reservation, typed SDK admission or a process-wide/native memory claim.
#[derive(Clone)]
pub struct ArchiveBootstrapCatalogPolicy {
    pub(in crate::store) pending: PendingJoinCatalogPolicy,
    persistent_capacity: usize,
}
impl std::fmt::Debug for ArchiveBootstrapCatalogPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveBootstrapCatalogPolicy")
            .finish_non_exhaustive()
    }
}
impl ArchiveBootstrapCatalogPolicy {
    /// Explicitly compose the concrete original JOIN execution consumer with this physical
    /// archive opener. This is trusted Rust configuration, not a parsed permission; it grants
    /// no units/task, and default/Pending/archive-only opening remains descriptor-v1 strict.
    pub fn with_checkpoint_join_execution(
        mut self,
        initial_limit: u32,
        timeout_ms: u64,
    ) -> Result<Self, AppError> {
        self.pending.execution = Some(super::pending_join::execution_policy::Settings::new(
            initial_limit,
            timeout_ms,
        )?);
        Ok(self)
    }

    pub fn new(
        pool: CriticalCatalogReadPool,
        persistent_capacity: usize,
    ) -> Result<Self, AppError> {
        let required =
            2 * super::pending_join_codec::MAX_CATALOG_FRAMED + MAX_SEED_FRAMED + MAX_MARKER_FRAMED;
        if persistent_capacity < required {
            return Err(refused());
        }
        Ok(Self {
            pending: PendingJoinCatalogPolicy::new(
                pool,
                2 * super::pending_join_codec::MAX_CATALOG_FRAMED,
            )?,
            persistent_capacity,
        })
    }
}

/// The complete exact original namespace transaction. Files, ciphertext and all metadata drop
/// before the separately prepaid wrapper allowance. Nothing here holds a transport or SDK.
pub(in crate::store) struct Declaration {
    pub(in crate::store) candidate: Zeroizing<Vec<u8>>,
    pub(in crate::store) marker: Zeroizing<Vec<u8>>,
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
    pub(in crate::store) expected: super::pending_join_codec::Summary,
    pub(in crate::store) next: super::pending_join_codec::Summary,
    pub(in crate::store) descriptor: Descriptor,
    pub(in crate::store) seed_file: Option<super::pending_join::CreatedStage>,
    pub(in crate::store) stage_file: Option<super::pending_join::CreatedStage>,
    pub(in crate::store) marker_file: Option<super::pending_join::CreatedStage>,
}

/// New physical-open ownership, not conversion from an exposed Store. Errors retain the same
/// declaration bytes, file objects, key/session owner and fee for exact synchronous retry.
/// No borrowed Store escapes before completion, so no JOIN/legacy writer can race declaration.
pub struct PreparedArchiveBootstrapStore {
    declaration: Option<Box<Declaration>>,
    store: Option<Box<ServerStore>>,
    complete: bool,
    _fee: CatalogReadReservation,
}
impl std::fmt::Debug for PreparedArchiveBootstrapStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedArchiveBootstrapStore")
            .finish_non_exhaustive()
    }
}

/// Same actual directory and immutable marker for this physical mount. Session IDs are never
/// serialized; holding the objects prevents in-session number reuse. They are not hostile-
/// filesystem/exclusive-link guarantees. The live private-vault-directory assumption applies.
pub(in crate::store) struct MountedRoot {
    pub(in crate::store) descriptor: Descriptor,
    pub(in crate::store) directory: File,
    pub(in crate::store) directory_id: catcoms_storage::kept::SessionId,
    pub(in crate::store) marker: File,
    pub(in crate::store) marker_id: catcoms_storage::kept::SessionId,
    pub(in crate::store) marker_digest: [u8; 32],
    pub(in crate::store) mount: Arc<()>,
    pub(in crate::store) negative: RequestCancellation,
    pub(in crate::store) _fee: CatalogReadReservation,
}
pub(in crate::store) type SharedRoot = Mutex<Option<MountedRoot>>;

mod seed;
use seed::{encode_marker, encode_seed, open_marker_data, open_seed_data, seal_marker, seal_seed};
pub(in crate::store) mod founder;
pub(in crate::store) mod founder_codec;
pub use founder::{
    ArchiveFounderBegin, ArchiveFounderCreation, ArchiveFounderExisting, ArchiveFounderInputs,
    ArchiveFounderPlanning, ArchiveFounderPlanningProgress, ArchiveFounderPolicy,
    ArchiveFounderProgress, ReturnedArchiveFounderCreation, ReturnedArchiveFounderInputs,
};

impl ServerStore {
    /// Select only the actual physical opener's concrete grammar. A caller-supplied payload
    /// cannot opt into a namespace/consumer. Strict Pending-only parsing remains unchanged.
    pub(in crate::store) fn parse_configured_join_catalog<'a>(
        &self,
        plain: &'a [u8],
    ) -> Result<super::pending_join_codec::Parsed<'a>, AppError> {
        let domain = self.chat_vault_domain()?;
        let execution = self
            .pending_join_policy
            .as_ref()
            .and_then(|policy| policy.execution.as_ref());
        match (self.archive_bootstrap_policy.as_ref(), execution) {
            (None, None) => super::pending_join_codec::parse(plain, &domain),
            (Some(_), None) => super::pending_join_codec::parse_bootstrap(plain, &domain),
            (None, Some(_)) => super::pending_join_codec::parse_executing(plain, &domain),
            (Some(_), Some(_)) => {
                super::pending_join_codec::parse_bootstrap_executing(plain, &domain)
            }
        }
    }

    fn prepare_archive_namespace_declaration(
        &self,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Declaration, AppError> {
        let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
        let read = policy
            .pending
            .pool
            .try_reserve(super::pending_join::READ_FEE)?;
        let (_file, raw) = self
            .open_pending_join_catalog_slot(Slot::Current, &read)?
            .ok_or_else(refused)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
        let current = self.parse_configured_join_catalog(&plain)?;
        if current.archive.is_some() || self.chat_mount.observer().is_cancelled() {
            return Err(refused());
        }
        let expected = current.summary(*blake3::hash(&raw).as_bytes());
        {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_ref().ok_or_else(refused)?;
            if mounted.pending.is_some() || mounted.current.framed != expected.framed {
                return Err(refused());
            }
        }
        let vault = self.chat_vault_domain()?;
        let mut root = [0; 32];
        rng.fill_bytes(&mut root);
        let descriptor = Descriptor { vault, root };
        let encoded = descriptor.encode()?;
        let envelope = super::catalog_envelope::parse(
            &plain,
            &vault,
            super::pending_join_codec::MAX_CATALOG_PLAIN,
        )?;
        let mut output = Encoder::with_capacity(super::pending_join_codec::MAX_CATALOG_PLAIN);
        output
            .put_str(super::catalog_envelope::TAG_V2)
            .map_err(|_| refused())?;
        output.put_bytes(&vault).map_err(|_| refused())?;
        output.put_u64(current.generation.checked_add(1).ok_or_else(refused)?);
        output.put_bytes(&current.digest).map_err(|_| refused())?;
        output.put_u32(envelope.count as u32 + 1);
        // This factory adds exactly one closed descriptor. Preserve the COMPLETE existing
        // JOIN payload/version rather than iterating known fields and dropping future data.
        for index in 0..envelope.count {
            let entry = envelope.entry(index)?;
            if entry.id != super::pending_join_codec::JOIN_ID {
                return Err(refused());
            }
            output.put_u32(entry.id);
            output.put_u32(entry.version);
            output.put_bytes(entry.payload).map_err(|_| refused())?;
        }
        output.put_u32(ARCHIVE_ID);
        output.put_u32(ARCHIVE_VERSION);
        output.put_bytes(&encoded).map_err(|_| refused())?;
        let candidate_plain = Zeroizing::new(output.finish());
        let next_data = self.parse_configured_join_catalog(&candidate_plain)?;
        let mut next = next_data.summary([0; 32]);
        super::pending_join_codec::successor(&expected, &next)?;
        if next.archive.as_ref() != Some(&descriptor) {
            return Err(refused());
        }
        let candidate = Zeroizing::new(frame(&seal(&key, &candidate_plain, rng)?));
        next.framed = *blake3::hash(&candidate).as_bytes();
        let marker_plain = encode_marker(&descriptor)?;
        let marker = seal_marker(&key, &marker_plain, rng)?;
        let seed_plain = encode_seed(&descriptor, &expected, &candidate, &marker)?;
        let seed = seal_seed(&key, &seed_plain, rng)?;
        Ok(Declaration {
            candidate,
            marker,
            seed,
            expected,
            next,
            descriptor,
            seed_file: None,
            stage_file: None,
            marker_file: None,
        })
    }
}

impl PreparedArchiveBootstrapStore {
    /// Open an ALREADY activated v6 vault privately. Namespace bootstrap never creates an SDK
    /// group/device, performs networking, or converts an already exposed mount. No Store
    /// reference escapes before the declaration and its actual root barriers complete.
    pub fn open(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        policy: ArchiveBootstrapCatalogPolicy,
    ) -> Result<Self, AppError> {
        // Pay BEFORE constructor reads, boxes, candidate codecs, file handles or RNG. Existing
        // Pending reads retain their own debit; this separately covers the compound seed and
        // all raw/cipher/plain/root-marker overlaps, not SDK/KDF/native allocations.
        let fee = policy.pending.pool.try_reserve(DECLARATION_FEE)?;
        // Bound the native path copies in this private handler's logical metadata invoice.
        // Other constructors keep their existing path policy. No path is read from a peer.
        if dir.as_ref().as_os_str().len() > 4096 {
            return Err(refused());
        }
        if !ServerStore::exists(dir.as_ref()) {
            return Err(refused());
        }
        if policy.persistent_capacity
            < 2 * super::pending_join_codec::MAX_CATALOG_FRAMED
                + MAX_SEED_FRAMED
                + MAX_MARKER_FRAMED
        {
            return Err(refused());
        }
        let store = Box::new(ServerStore::open_with_catalog_policy(
            dir,
            passphrase,
            rng,
            Some(policy.pending.pool.clone()),
            Some(policy.pending.clone()),
            Some(policy),
        )?);
        let complete = store
            .archive_bootstrap_root
            .lock()
            .map_err(|_| refused())?
            .is_some();
        Ok(Self {
            declaration: None,
            store: Some(store),
            complete,
            _fee: fee,
        })
    }

    /// Retry the SAME original namespace candidate after a short write or lost barrier.
    /// Once prepared, no retry draws randomness or re-encodes/reseals any output.
    pub fn retry(&mut self, rng: &mut impl CryptoRngCore) -> Result<(), AppError> {
        let store = self.store.as_ref().ok_or_else(refused)?;
        if store.chat_mount.observer().is_cancelled() {
            return Err(refused());
        }
        if self.complete {
            return Ok(());
        }
        if self.declaration.is_none() {
            self.declaration = Some(Box::new(store.prepare_archive_namespace_declaration(rng)?));
        }
        let declaration = self.declaration.as_mut().ok_or_else(refused)?;
        // The candidate is retained in self BEFORE the first fallible I/O. This caller is the
        // actual sole new-physical-open owner; a pointer/label cannot construct its write debt.
        store.write_archive_namespace_declaration(declaration, &self._fee)?;
        store.install_declared_archive_root(&declaration.descriptor)?;
        {
            let mut mounted = store.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_mut().ok_or_else(refused)?;
            if mounted.pending.is_some()
                || (mounted.current.framed != declaration.expected.framed
                    && mounted.current.framed != declaration.next.framed)
            {
                return Err(refused());
            }
            mounted.current = declaration.next.clone();
        }
        self.complete = true;
        Ok(())
    }

    /// Move the actual Store only after its declaration/root/parent barriers succeeded. A
    /// refusal returns the same private owner, including all exact bytes, handles and fee.
    pub fn into_store(mut self) -> Result<ServerStore, Self> {
        if !self.complete
            || self
                .store
                .as_ref()
                .is_none_or(|store| store.chat_mount.observer().is_cancelled())
        {
            return Err(self);
        }
        match self.store.take() {
            Some(store) => Ok(*store),
            None => Err(self),
        }
    }
}

impl ServerStore {
    /// Concrete new-physical-open inspector. Every complete current/stage/seed source is
    /// authenticated or retained as an explicit refusal before any old-format gate runs.
    /// Source directories remain negative inventory; this handler grants no SDK/source owner.
    pub(in crate::store) fn open_archive_bootstrap_catalog_gate(
        &self,
        actual_format: VaultFormat,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if actual_format != VaultFormat::CriticalNamespaces {
            return Err(refused());
        }
        let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
        let fee = policy.pending.pool.try_reserve(DECLARATION_FEE)?;
        let read = policy
            .pending
            .pool
            .try_reserve(super::pending_join::READ_FEE)?;
        let (_current_file, current_raw) = self
            .open_pending_join_catalog_slot(Slot::Current, &read)?
            .ok_or_else(refused)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let current_plain = Zeroizing::new(unseal(&key, &unframe(&current_raw)?)?);
        let current = self.parse_configured_join_catalog(&current_plain)?;
        let current = current.summary(*blake3::hash(&current_raw).as_bytes());

        if let Some((seed_file, raw)) = self.read_archive_declaration_seed(&fee)? {
            let plain = open_seed_data(&key, &raw)?;
            let saved = seed::parse_seed_data(&plain, &self.chat_vault_domain()?)?;
            let candidate_plain = Zeroizing::new(unseal(&key, &unframe(saved.candidate)?)?);
            let candidate = self.parse_configured_join_catalog(&candidate_plain)?;
            let next = candidate.summary(*blake3::hash(saved.candidate).as_bytes());
            if next.archive.as_ref() != Some(&saved.descriptor)
                || next.parent != Some(saved.expected_plain)
            {
                return Err(refused());
            }
            open_marker_data(&key, &saved.descriptor, saved.marker)?;
            let expected = if current.framed == saved.expected_frame {
                if current.digest != saved.expected_plain || current.archive.is_some() {
                    return Err(refused());
                }
                super::pending_join_codec::successor(&current, &next)?;
                current.clone()
            } else if current.framed == next.framed && current.digest == next.digest {
                // Already promoted: these are inert retry labels, NOT an original source or
                // reconstructed durability token. The actual Current and strict SAME root
                // are re-synced below; a missing root MUST NOT be recreated on this branch.
                let mut expected = next.clone();
                expected.generation = next.generation.checked_sub(1).ok_or_else(refused)?;
                expected.parent = None;
                expected.digest = saved.expected_plain;
                expected.framed = saved.expected_frame;
                expected.archive = None;
                expected
            } else {
                return Err(refused());
            };
            let identity = catcoms_storage::kept::session_id(&seed_file)
                .map_err(|error| AppError::Io(error.to_string()))?;
            let mut declaration = Declaration {
                candidate: Zeroizing::new(saved.candidate.to_vec()),
                marker: Zeroizing::new(saved.marker.to_vec()),
                seed: Zeroizing::new(raw.to_vec()),
                expected,
                next,
                descriptor: saved.descriptor,
                seed_file: Some(super::pending_join::CreatedStage {
                    file: seed_file,
                    identity: Some(identity),
                }),
                stage_file: None,
                marker_file: None,
            };
            // This actual constructor owns the physical session and exposes no Store reference.
            // Cold prefix repair is possible ONLY from this complete independently authenticated
            // seed and its actual selected parent/next, never from a partial/unknown seed.
            self.write_archive_namespace_declaration(&mut declaration, &fee)?;
        } else {
            let opened = self.inspect_pending_join_catalog()?;
            match (&current.archive, &opened.selected.archive) {
                (None, None) => super::refuse_unimplemented_archive_storage(&self.dir)?,
                (Some(old), Some(next)) if old == next => {
                    // Valid current declaration, including an ordinary retained JOIN successor.
                    // A missing/replaced/invalid root refuses; it is NEVER initialized as empty.
                    self.install_declared_archive_root(next)?;
                }
                // An ID2 stage with no exact independently retained seed is not a first-create
                // grant, even when it decrypts and a directory happens to be present.
                _ => return Err(refused()),
            }
        }

        // All archive sources have now been classified and required physical retry completed.
        // Missing old mandatory inventories still refuse; no fallthrough manufactures lineage.
        self.open_pending_join_catalog_gate(actual_format, passphrase, rng)?;
        let descriptor = {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            mounted
                .as_ref()
                .ok_or_else(refused)?
                .current
                .archive
                .clone()
        };
        if let Some(descriptor) = descriptor {
            self.install_declared_archive_root(&descriptor)?;
        }
        Ok(())
    }

    fn install_declared_archive_root(&self, descriptor: &Descriptor) -> Result<(), AppError> {
        let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
        let fee = policy.pending.pool.try_reserve(ROOT_METADATA_FEE)?;
        let root = self.open_declared_archive_root(descriptor, fee)?;
        let retired = {
            let mut mounted = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            mounted.replace(root)
        };
        // Actual file/metadata/fee owners drop outside the root guard.
        drop(retired);
        Ok(())
    }

    /// Negative-only old-protocol fence under admission, before issued retries, RNG or a
    /// source-absence decision. Unknown/partial/nonregular target presence is not absence.
    /// The concrete V2 source saver needs its own original intent path; this has no bypass.
    pub(in crate::store) fn require_no_archive_bootstrap_target(
        &self,
        group: &[u8],
        signer: &[u8; 32],
    ) -> Result<(), AppError> {
        if self.archive_bootstrap_policy.is_none() {
            return Ok(());
        }
        let mounted = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let mounted = mounted.as_ref().ok_or_else(refused)?;
        if self.archive_target_absent_under_original_root(mounted, group, signer)? {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

pub(in crate::store) fn verify_marker_frame(
    key: &[u8; 32],
    descriptor: &Descriptor,
    raw: &[u8],
) -> Result<(), AppError> {
    open_marker_data(key, descriptor, raw)
}

/// Fixed name only. Frame the group length and full signer; no numeric alias, truncated
/// device ID, peer endpoint or caller filename can select another logical target.
pub(in crate::store) fn target_name(
    vault: &[u8; 32],
    group: &[u8],
    signer: &[u8; 32],
) -> Result<[u8; 64], AppError> {
    if group.is_empty() || group.len() > 256 {
        return Err(refused());
    }
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/private-chat-bootstrap-target/v1\0");
    hash.update(vault);
    hash.update(&(group.len() as u32).to_be_bytes());
    hash.update(group);
    hash.update(signer);
    let mut name = [0; 64];
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in hash.finalize().as_bytes().iter().enumerate() {
        name[index * 2] = HEX[(byte >> 4) as usize];
        name[index * 2 + 1] = HEX[(byte & 15) as usize];
    }
    Ok(name)
}

#[cfg(test)]
mod tests;

/// Immutable closed-layout DATA, never a caller path. The independently generated root identity
/// binds the actual directory marker to this catalogue declaration across physical remounts.
/// A missing marker/root after a current declaration is UNKNOWN, never a new empty archive.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct Descriptor {
    pub(in crate::store) vault: [u8; 32],
    pub(in crate::store) root: [u8; 32],
}
impl std::fmt::Debug for Descriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateChatBootstrapLayout")
            .finish_non_exhaustive()
    }
}
impl Descriptor {
    pub(super) fn parse(bytes: &[u8], vault: &[u8; 32]) -> Result<Self, AppError> {
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err(refused());
        }
        let mut input = Decoder::new(bytes);
        if input.get_str().map_err(|_| refused())? != DESCRIPTOR_TAG
            || input.get_bytes().map_err(|_| refused())? != vault
            || input.get_u16().map_err(|_| refused())? != 1 // closed physical layout
            || input.get_u8().map_err(|_| refused())? != 1
        // unexposed bootstrap consumer only
        {
            return Err(refused());
        }
        let root = input
            .get_bytes()
            .map_err(|_| refused())?
            .try_into()
            .map_err(|_| refused())?;
        input.finish().map_err(|_| refused())?;
        Ok(Self {
            vault: *vault,
            root,
        })
    }

    pub(super) fn encode(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        // The schema is fixed; the 4KiB ceiling is not an invitation to retain arbitrary
        // extensions. New layouts/consumers require a new explicit supported discriminator.
        let mut output = Encoder::with_capacity(MAX_DESCRIPTOR_BYTES);
        output.put_str(DESCRIPTOR_TAG).map_err(|_| refused())?;
        output.put_bytes(&self.vault).map_err(|_| refused())?;
        output.put_u16(1);
        output.put_u8(1);
        output.put_bytes(&self.root).map_err(|_| refused())?;
        Ok(Zeroizing::new(output.finish()))
    }
}
