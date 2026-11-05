//! Local admission lineage survives replacement/deletion of individual server snapshots.
//!
//! The sealed inventory is not historical MLS authority. It fences local authoring lifetimes.
//! Only the mounted store owns revoking senders; decoded labels and retained observers cannot
//! keep a retired mount/admission alive. All inventory and final storage admissions share one
//! synchronous owner, so an older positive lease cannot write across a completed retirement.
use super::*;
use catcoms_rt::{RequestAuthority, RequestCancellation};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
mod custody;
pub(crate) mod fresh;
pub(in crate::store) mod group_lineage;
pub(in crate::store) mod journal_basis;
mod old_owner_group;
mod owner_writer;
pub(in crate::store) mod predecessor;
mod recovery;
pub use recovery::{OwnerUnexposedInitialization, OwnerUnexposedRetirement};
pub(crate) mod restore;
pub(crate) mod retired;
mod revocation;
mod storage;
pub(crate) use storage::ChatAdmissionStoragePermit;

const DOMAIN: &str = "catcoms/chat-admission-inventory/v1";
const FILE: &str = "chat-admissions.bin";
/// Non-evicting logical identities, including retired identities and failed issued reservations.
pub const MAX_CHAT_ADMISSION_IDENTITIES: usize = 65_536;
const MAX_BYTES: usize = 32 * 1024 * 1024;
// Reserve maximum row framing at first issuance. Changing Current to Retired adds no bytes.
const RESERVED_ROW_BYTES: usize = 384;
const MIN_ROW_BYTES: usize = 123;

fn refused() -> AppError {
    AppError::Invalid(
        "chat admission inventory is unavailable or does not authorize this scope".into(),
    )
}

/// Exact logical identity; numeric server aliases cannot evade retirement of the same admission.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChatAdmissionKey {
    group: Vec<u8>,
    signer: [u8; 32],
}
impl ChatAdmissionKey {
    /// Metadata only. This key cannot mint a live mount/admission observer.
    pub fn new(group: &[u8], signer: [u8; 32]) -> Result<Self, AppError> {
        if group.is_empty() || group.len() > 256 {
            return Err(refused());
        }
        Ok(Self {
            group: group.to_vec(),
            signer,
        })
    }
    pub fn group(&self) -> &[u8] {
        &self.group
    }
    pub fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Initializing,
    Current,
    Retired,
}
/// Closed storage phases: initialization is a private covering save, never a publication grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChatAdmissionStorageMode {
    Initializing,
    Current,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Legacy,
    Fresh,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    nonce: [u8; 32],
    revision: u64,
    phase: Phase,
    origin: Origin,
    provenance: [u8; 32],
}

/// Original pre-await lineage capture. A later completion cannot borrow a newer mount or row.
pub struct ChatAdmissionCapture {
    key: ChatAdmissionKey,
    original: Option<Record>,
    mount_identity: Arc<()>,
    mount: RequestCancellation,
    admission: Option<RequestCancellation>,
    // A valid issuance revokes its previous owner before I/O. Preserve its OWN exact candidate
    // even on pre-rename failure, rather than treating that self-revocation as a new admission.
    issued: Mutex<Option<ChatAdmissionPreparation>>,
}
impl std::fmt::Debug for ChatAdmissionCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatAdmissionCapture")
            .finish_non_exhaustive()
    }
}

/// An exact initialization reservation, never a publication/storage capability.
#[derive(Clone)]
pub struct ChatAdmissionPreparation {
    key: ChatAdmissionKey,
    record: Record,
    domain: [u8; 32],
    mount_identity: Arc<()>,
    mount: RequestCancellation,
    admission: RequestCancellation,
    owner_identity: Arc<()>,
    revoker: ChatAdmissionRevoker,
}
impl std::fmt::Debug for ChatAdmissionPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatAdmissionPreparation")
            .finish_non_exhaustive()
    }
}

/// Restore classification. Initializing resumes the same recorded nonce; Retired permits only
/// retained reads/original-store negative repair, never automatic bootstrap or publication.
#[derive(Debug)]
pub enum ChatAdmissionResolution {
    Current(ChatAdmissionBinding),
    Initializing(ChatAdmissionPreparation),
    Retired,
}
/// Saved covers the supplied snapshot callback; AlreadyCurrent is only an exact scope replay
/// and cannot acknowledge a newer dirty snapshot ticket or reinstall an old candidate graph.
#[derive(Debug)]
pub enum ChatAdmissionFinish {
    Saved(ChatAdmissionBinding),
    AlreadyCurrent(ChatAdmissionBinding),
}
impl ChatAdmissionFinish {
    /// Extract runtime custody without changing whether this call covered a server snapshot.
    pub fn into_binding(self) -> ChatAdmissionBinding {
        match self {
            Self::Saved(binding) | Self::AlreadyCurrent(binding) => binding,
        }
    }
}
impl ChatAdmissionPreparation {
    /// Negative-only check for the original first-initialization source. A decoded attempt
    /// digest cannot create fresh custody: this preparation already came from the mounted
    /// independent lineage table, and final storage still requires its actual held permit.
    pub(crate) fn validate_initialization_attempt(
        &self,
        attempt: &[u8; 32],
    ) -> Result<(), AppError> {
        if self.record.phase != Phase::Initializing
            || self.record.origin != Origin::Fresh
            || &self.record.provenance != attempt
            || self.mount.is_cancelled()
            || self.admission.is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }

    pub fn nonce(&self) -> [u8; 32] {
        self.record.nonce
    }
    pub fn domain(&self) -> [u8; 32] {
        self.domain
    }
    pub fn revoker(&self) -> ChatAdmissionRevoker {
        self.revoker.clone()
    }
}

/// Runtime custody returned only after the actual mounted store verifies a Current record.
/// No decoder, public field constructor or saved boolean can mint this binding.
#[derive(Clone)]
pub struct ChatAdmissionBinding {
    key: ChatAdmissionKey,
    nonce: [u8; 32],
    revision: u64,
    domain: [u8; 32],
    mount_identity: Arc<()>,
    mount: RequestCancellation,
    publication: RequestCancellation,
    owner_identity: Arc<()>,
    revoker: ChatAdmissionRevoker,
    owner_writer_identity: Option<Arc<()>>,
}
impl std::fmt::Debug for ChatAdmissionBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Opaque runtime custody: never print group/full key, nonce/domain, revision or mount.
        f.debug_struct("ChatAdmissionBinding")
            .finish_non_exhaustive()
    }
}
impl ChatAdmissionBinding {
    pub fn nonce(&self) -> [u8; 32] {
        self.nonce
    }
    pub fn domain(&self) -> [u8; 32] {
        self.domain
    }
    pub fn publication_authority(&self) -> RequestCancellation {
        self.publication.clone()
    }
    pub fn is_live(&self) -> bool {
        !self.publication.is_cancelled()
    }
    /// Negative-only original-owner handle, safe to call before waiting for retirement storage.
    pub fn revoker(&self) -> ChatAdmissionRevoker {
        self.revoker.clone()
    }
}

/// Revokes only the original nonce owner. It cannot publish, issue a binding, access a later
/// admission or keep the publisher alive. Durable retirement is still separately required.
#[derive(Clone)]
pub struct ChatAdmissionRevoker(revocation::Handle);
impl ChatAdmissionRevoker {
    pub fn revoke(&self) {
        self.0.revoke();
    }
}
impl std::fmt::Debug for ChatAdmissionRevoker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatAdmissionRevoker")
            .finish_non_exhaustive()
    }
}

/// Host-held negative-only handle for revocation BEFORE waiting for a mounted Store guard.
/// Reopening/replacing a Store never changes which physical lifetime this handle targets.
#[derive(Clone)]
pub struct ChatMountRevoker(revocation::Handle);
impl ChatMountRevoker {
    pub fn revoke(&self) {
        self.0.revoke();
    }
}
impl std::fmt::Debug for ChatMountRevoker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatMountRevoker").finish_non_exhaustive()
    }
}

impl catcoms_sync::local_chat_scope::ManagedChatScopeRevocation for ChatAdmissionRevoker {
    fn revoke(&self) {
        ChatAdmissionRevoker::revoke(self);
    }
}

impl catcoms_sync::local_chat_scope::ManagedChatScopeReservation for ChatAdmissionBinding {
    fn group(&self) -> &[u8] {
        self.key.group()
    }
    fn signer(&self) -> &[u8; 32] {
        self.key.signer()
    }
    fn domain(&self) -> [u8; 32] {
        self.domain
    }
    fn nonce(&self) -> [u8; 32] {
        self.nonce
    }
    fn owner_identity(&self) -> Arc<()> {
        self.owner_identity.clone()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        self.publication.clone()
    }
    fn revocation(&self) -> Arc<dyn catcoms_sync::local_chat_scope::ManagedChatScopeRevocation> {
        Arc::new(self.revoker.clone())
    }
}
impl catcoms_sync::local_chat_scope::ManagedChatScopeBinding for ChatAdmissionBinding {
    fn publication_authority(&self) -> RequestCancellation {
        self.publication.clone()
    }
}

impl catcoms_sync::local_chat_scope::ManagedChatScopeReservation for ChatAdmissionPreparation {
    fn group(&self) -> &[u8] {
        self.key.group()
    }
    fn signer(&self) -> &[u8; 32] {
        self.key.signer()
    }
    fn domain(&self) -> [u8; 32] {
        self.domain
    }
    fn nonce(&self) -> [u8; 32] {
        self.record.nonce
    }
    fn owner_identity(&self) -> Arc<()> {
        self.owner_identity.clone()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        self.admission.clone()
    }
    fn revocation(&self) -> Arc<dyn catcoms_sync::local_chat_scope::ManagedChatScopeRevocation> {
        Arc::new(self.revoker.clone())
    }
}

struct AdmissionOwner {
    nonce: [u8; 32],
    revision: u64,
    life: Arc<revocation::Owner>,
}
#[derive(Default)]
struct Runtime {
    table: Option<BTreeMap<ChatAdmissionKey, Record>>,
    dirty: bool,
    managed: bool,
    format: Option<catcoms_storage::VaultFormat>,
    owners: BTreeMap<ChatAdmissionKey, AdmissionOwner>,
    // Weak original-task exclusions, including tasks captured against absent/Retired rows.
    // The sole typed creator owns its sender; no Store/debt can keep it running after Drop.
    predecessors: predecessor::Registry,
}
#[cfg(test)]
type BeforeWriterHook = Box<dyn FnOnce(&Mutex<Runtime>) + Send>;
/// Owned by one physical ServerStore. Observers never retain these revoking senders.
pub(super) struct MountOwner {
    identity: Arc<()>,
    life: Arc<revocation::Owner>,
    runtime: Mutex<Runtime>,
    #[cfg(test)]
    before_writer: Mutex<Option<BeforeWriterHook>>,
}
impl MountOwner {
    pub(super) fn new() -> Self {
        Self {
            identity: Arc::new(()),
            life: revocation::Owner::new(),
            runtime: Mutex::new(Runtime::default()),
            #[cfg(test)]
            before_writer: Mutex::new(None),
        }
    }
    pub(super) fn observer(&self) -> RequestCancellation {
        self.life.observer()
    }
    pub(super) fn revoke(&self) {
        self.life.revoke();
    }
    /// Test-only observation immediately before the next actual admission lock. Expose only
    /// contention, never the private Runtime or a fake permit, so races can assert that a real
    /// retirement is waiting behind admitted subordinate filesystem work without sleeps.
    #[cfg(test)]
    pub(in crate::store) fn observe_next_writer_contention(
        &self,
        observer: impl FnOnce(bool) + Send + 'static,
    ) {
        *self.before_writer.lock().unwrap() = Some(Box::new(move |runtime| {
            observer(matches!(
                runtime.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
        }));
    }
    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Runtime>, AppError> {
        if !self.life.live() {
            return Err(refused());
        }
        #[cfg(test)]
        {
            // One bounded scheduling seam. Release its own mutex before executing the hook;
            // tests can prove real writer contention without ambient sleeps or polling races.
            let hook = self.before_writer.lock().map_err(|_| refused())?.take();
            if let Some(hook) = hook {
                hook(&self.runtime);
            }
        }
        let runtime = self.runtime.lock().map_err(|_| refused())?;
        // Revocation can happen while this synchronous writer waits for earlier admitted work.
        if !self.life.live() {
            return Err(refused());
        }
        Ok(runtime)
    }
}
impl Drop for MountOwner {
    fn drop(&mut self) {
        self.revoke();
    }
}

#[derive(Debug)]
struct AdmissionAuthority {
    mount: RequestCancellation,
    admission: RequestCancellation,
}
#[async_trait::async_trait]
impl RequestAuthority for AdmissionAuthority {
    fn authorized(&self) -> bool {
        !self.mount.is_cancelled() && !self.admission.is_cancelled()
    }
    async fn revoked(&self) {
        let mut mount = self.mount.clone();
        let mut admission = self.admission.clone();
        if self.authorized() {
            tokio::select! {
                _ = mount.cancelled() => {},
                _ = admission.cancelled() => {},
            }
        }
    }
}

fn encode(
    table: &BTreeMap<ChatAdmissionKey, Record>,
    domain: &[u8; 32],
) -> Result<Vec<u8>, AppError> {
    if table.len() > MAX_CHAT_ADMISSION_IDENTITIES
        || table
            .len()
            .checked_mul(RESERVED_ROW_BYTES)
            .and_then(|n| n.checked_add(128))
            .is_none_or(|n| n > MAX_BYTES)
    {
        return Err(refused());
    }
    let mut e = Encoder::new();
    e.put_str(DOMAIN).map_err(|_| refused())?;
    e.put_bytes(domain).map_err(|_| refused())?;
    e.put_u32(table.len() as u32);
    for (key, record) in table {
        e.put_bytes(&key.group).map_err(|_| refused())?;
        e.put_bytes(&key.signer).map_err(|_| refused())?;
        e.put_bytes(&record.nonce).map_err(|_| refused())?;
        e.put_u64(record.revision);
        e.put_u8(match record.phase {
            Phase::Initializing => 0,
            Phase::Current => 1,
            Phase::Retired => 2,
        });
        e.put_u8(match record.origin {
            Origin::Legacy => 0,
            Origin::Fresh => 1,
        });
        e.put_bytes(&record.provenance).map_err(|_| refused())?;
    }
    Ok(e.finish())
}
fn decode(bytes: &[u8], domain: &[u8; 32]) -> Result<BTreeMap<ChatAdmissionKey, Record>, AppError> {
    if bytes.len() > MAX_BYTES {
        return Err(refused());
    }
    let mut d = Decoder::new(bytes);
    if d.get_str().map_err(|_| refused())? != DOMAIN
        || d.get_bytes().map_err(|_| refused())? != domain
    {
        return Err(refused());
    }
    let count = d.get_u32().map_err(|_| refused())? as usize;
    if count > MAX_CHAT_ADMISSION_IDENTITIES || count > d.remaining() / MIN_ROW_BYTES {
        return Err(refused());
    }
    let mut table = BTreeMap::new();
    for _ in 0..count {
        let group = d.get_bytes().map_err(|_| refused())?;
        let signer = d
            .get_bytes()
            .map_err(|_| refused())?
            .try_into()
            .map_err(|_| refused())?;
        let key = ChatAdmissionKey::new(group, signer)?;
        let nonce = d
            .get_bytes()
            .map_err(|_| refused())?
            .try_into()
            .map_err(|_| refused())?;
        let revision = d.get_u64().map_err(|_| refused())?;
        let phase = match d.get_u8().map_err(|_| refused())? {
            0 => Phase::Initializing,
            1 => Phase::Current,
            2 => Phase::Retired,
            _ => return Err(refused()),
        };
        let origin = match d.get_u8().map_err(|_| refused())? {
            0 => Origin::Legacy,
            1 => Origin::Fresh,
            _ => return Err(refused()),
        };
        let provenance = d
            .get_bytes()
            .map_err(|_| refused())?
            .try_into()
            .map_err(|_| refused())?;
        if revision == 0
            || table
                .last_key_value()
                .is_some_and(|(previous, _)| previous >= &key)
        {
            return Err(refused());
        }
        table.insert(
            key,
            Record {
                nonce,
                revision,
                phase,
                origin,
                provenance,
            },
        );
    }
    d.finish().map_err(|_| refused())?;
    Ok(table)
}

impl ServerStore {
    /// The actual authenticated mounted format, not a live authoring permit. Restore consumers
    /// must require managed mode for no-tail bytes too, BEFORE any owner run/network handoff.
    /// A raw snapshot does not supply this physical-vault classification.
    pub fn chat_vault_format(&self) -> Result<catcoms_storage::VaultFormat, AppError> {
        self.chat_mount.locked()?.format.ok_or_else(refused)
    }

    /// Called only by the actual Store constructor under its exclusive physical mount, before
    /// exposure. Staged empty inventory under a legacy wrapper stays inert; previously issued
    /// lineage cannot be cleared by restoring an older authenticated legacy wrapper.
    pub(super) fn open_chat_admission_inventory(
        &self,
        format: catcoms_storage::VaultFormat,
    ) -> Result<(), AppError> {
        let mut runtime = self.chat_mount.locked()?;
        if runtime.table.is_some() || runtime.format.is_some() {
            // This is a constructor-only transition. A later reload must never discard live
            // negative custody or replace an uncertain candidate with older installed bytes.
            return Err(refused());
        }
        let managed = match format {
            catcoms_storage::VaultFormat::Legacy => false,
            catcoms_storage::VaultFormat::ChatScope
            | catcoms_storage::VaultFormat::OwnerEvidence
            | catcoms_storage::VaultFormat::OwnerOperations
            | catcoms_storage::VaultFormat::CriticalNamespaces => true,
        };
        let table = self.read_chat_admissions(managed)?;
        if !managed && !table.is_empty() {
            return Err(refused());
        }
        runtime.table = Some(table);
        runtime.managed = managed;
        runtime.format = Some(format);
        Ok(())
    }

    /// Revoke before any replacement/unmount await. Locking the UI alone does not unmount.
    pub fn revoke_chat_mount(&self) {
        self.chat_mount.revoke();
    }

    /// Capture this while the Store is mounted; revoke before any replacement/unmount await.
    /// The returned weak negative handle cannot keep the Store/session alive or affect a new mount.
    pub fn chat_mount_revoker(&self) -> ChatMountRevoker {
        ChatMountRevoker(self.chat_mount.life.handle())
    }

    /// Nonsecret comparison domain, not a runtime binding or source of admission authority.
    pub fn chat_vault_domain(&self) -> Result<[u8; 32], AppError> {
        let key = Zeroizing::new(self.keys.db_key()?);
        Ok(*blake3::keyed_hash(&key, b"catcoms/chat-vault-domain/v1").as_bytes())
    }

    fn read_chat_admissions(
        &self,
        managed: bool,
    ) -> Result<BTreeMap<ChatAdmissionKey, Record>, AppError> {
        let path = self.dir.join(FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return if managed {
                    Err(refused())
                } else {
                    Ok(BTreeMap::new())
                };
            }
            Ok(metadata) if metadata.file_type().is_file() => {}
            _ => return Err(refused()),
        }
        let Some(raw) = catcoms_storage::blob::read_bounded(&path, MAX_BYTES + 40)? else {
            return if managed {
                Err(refused())
            } else {
                Ok(BTreeMap::new())
            };
        };
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&raw)?)?);
        decode(&plain, &self.chat_vault_domain()?)
    }

    fn flush_chat_admissions(
        &self,
        runtime: &mut Runtime,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        runtime.dirty = true;
        let table = runtime.table.as_ref().ok_or_else(refused)?;
        let bytes = Zeroizing::new(encode(table, &self.chat_vault_domain()?)?);
        let sealed = seal(&self.keys.db_key()?, &bytes, rng)?;
        if !self.chat_mount.life.live() {
            return Err(refused());
        }
        self.write_chat_admissions_record(&frame(&sealed))?;
        runtime.dirty = false;
        Ok(())
    }

    /// Host-only mandatory format activation. This does not issue a scope or return a binding.
    /// Inventory first, cryptographically distinct wrapper second; a staged legacy inventory
    /// must remain empty/inert. No usable managed Store may be returned after any failure.
    pub fn enable_chat_scope_management(
        &self,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let mut runtime = self.chat_mount.locked()?;
        let (opened, format) =
            open_or_create_vault_classified(&self.dir, passphrase, rng)?.into_parts();
        if opened.db_key()? != self.keys.db_key()? {
            self.chat_mount.revoke();
            return Err(refused());
        }
        let managed = match format {
            catcoms_storage::VaultFormat::Legacy => false,
            catcoms_storage::VaultFormat::ChatScope
            | catcoms_storage::VaultFormat::OwnerEvidence
            | catcoms_storage::VaultFormat::OwnerOperations
            | catcoms_storage::VaultFormat::CriticalNamespaces => true,
        };
        if runtime.table.is_none() {
            runtime.table = Some(self.read_chat_admissions(managed)?);
        }
        if !managed && !runtime.table.as_ref().ok_or_else(refused)?.is_empty() {
            return Err(refused());
        }
        self.flush_chat_admissions(&mut runtime, rng)?;
        map_vault_passphrase_change(catcoms_storage::vault::activate_chat_scope_vault(
            &self.dir, passphrase, rng,
        ))?;
        runtime.managed = true;
        runtime.format = Some(match format {
            catcoms_storage::VaultFormat::CriticalNamespaces => {
                catcoms_storage::VaultFormat::CriticalNamespaces
            }
            catcoms_storage::VaultFormat::OwnerOperations => {
                catcoms_storage::VaultFormat::OwnerOperations
            }
            catcoms_storage::VaultFormat::OwnerEvidence => {
                catcoms_storage::VaultFormat::OwnerEvidence
            }
            _ => catcoms_storage::VaultFormat::ChatScope,
        });
        Ok(())
    }

    /// Constructor-only pending-v5 recovery. The actual authenticated intent makes existing
    /// lineage mandatory even if an older authentic wrapper was replayed. It never fabricates a
    /// wrapper classification or initializes missing rows. No Store escapes this temporary state
    /// before the SAME keys authenticate v5 and confirm_owner_operations_format completes.
    pub(in crate::store) fn open_chat_admissions_for_operations_recovery(
        &self,
        actual_format: catcoms_storage::VaultFormat,
        intent: &super::owner_evidence::operations::ValidatedOwnerOperationsIntent,
    ) -> Result<(), AppError> {
        intent.validate_original(self)?;
        let mut runtime = self.chat_mount.locked()?;
        if runtime.table.is_some() || runtime.format.is_some() {
            return Err(refused());
        }
        runtime.table = Some(self.read_chat_admissions(true)?);
        runtime.managed = true;
        runtime.format = Some(actual_format);
        Ok(())
    }

    /// Actual opaque authenticated result, not a later public-header read or caller boolean.
    pub(in crate::store) fn confirm_owner_operations_format(
        &self,
        opened: catcoms_storage::OpenedVault,
        intent: &super::owner_evidence::operations::ValidatedOwnerOperationsIntent,
    ) -> Result<(), AppError> {
        intent.validate_original(self)?;
        let (keys, format) = opened.into_parts();
        if !format.requires_owner_operations() || keys.db_key()? != self.keys.db_key()? {
            return Err(refused());
        }
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed || runtime.dirty || runtime.table.is_none() {
            return Err(refused());
        }
        runtime.format = Some(format);
        Ok(())
    }

    /// Only the actual pre-exposure catalog transaction calls this with its original known
    /// files and freshly authenticated SAME-DEK wrapper. Decoded catalog bytes/format metadata
    /// alone cannot update a mount or construct an admission. Exact v4 negative gates remain v4.
    pub(super) fn confirm_critical_namespace_format(
        &self,
        opened: catcoms_storage::OpenedVault,
        catalog: &super::critical_namespaces::ValidatedEmptyCatalog,
    ) -> Result<(), AppError> {
        catalog.validate_original(self)?;
        let (keys, format) = opened.into_parts();
        if format != catcoms_storage::VaultFormat::CriticalNamespaces
            || keys.db_key()? != self.keys.db_key()?
        {
            return Err(refused());
        }
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed || runtime.dirty || runtime.table.is_none() {
            return Err(refused());
        }
        runtime.format = Some(format);
        Ok(())
    }

    /// Capture before the first join/issuance await. The private record is an exact CAS basis.
    pub fn capture_chat_admission(
        &self,
        key: ChatAdmissionKey,
    ) -> Result<ChatAdmissionCapture, AppError> {
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed || runtime.dirty {
            return Err(refused());
        }
        // Capturing a legacy lineage is already too late once a group-wide V2 reservation
        // exists. Never mint an inert old capture that a later path could mistake for vacancy.
        self.require_no_v2_group_directory(key.group())?;
        let original = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&key)
            .cloned();
        let admission = match &original {
            Some(record) if record.phase != Phase::Retired => Some(
                self.reservation_locked(&mut runtime, &key, record)?
                    .admission,
            ),
            _ => None,
        };
        Ok(ChatAdmissionCapture {
            key,
            original,
            mount_identity: self.chat_mount.identity.clone(),
            mount: self.chat_mount.observer(),
            admission,
            issued: Mutex::new(None),
        })
    }

    /// Reserve one identity's fixed retirement footprint before exposing a context. Fresh
    /// provenance comes from the host's verified live Found/Welcome completion, never wire data.
    /// The separate entry points keep restore from impersonating that provenance.
    fn prepare_chat_admission(
        &self,
        capture: &ChatAdmissionCapture,
        provenance: [u8; 32],
        origin: Origin,
        rng: &mut impl CryptoRngCore,
    ) -> Result<ChatAdmissionPreparation, AppError> {
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed
            || capture.mount.is_cancelled()
            || !Arc::ptr_eq(&capture.mount_identity, &self.chat_mount.identity)
        {
            return Err(refused());
        }
        // An actual source/plan owns this predecessor. Refuse BEFORE issued retry/dirty flush
        // as well as RNG/new-owner creation: weak expiry is not durable source/debt absence.
        // Future intent-backed consumers require their own exact original path; this function
        // has no kind/status bypass or positive permission derived from a negative registry.
        if runtime.predecessors.excludes_generic(&capture.key) {
            return Err(refused());
        }
        // A supported V2 namespace may retain a source/intent even with no old scope row.
        // Inspect its original root under this SAME serializer before issued retry/dirty
        // flush as well as pre-RNG issuance. Unknown target presence is never old absence.
        // Cold/paused JOIN source has no runtime task yet but still owns this exact basis.
        // Check BEFORE both issued retry/dirty flush and pre-RNG admission. Keep catalog before
        // any subordinate pins/head/Root; admission remains held through the eventual effect.
        self.require_no_pending_checkpoint_join_source(capture.key.group(), capture.key.signer())?;
        self.require_no_archive_bootstrap_target(capture.key.group(), capture.key.signer())?;
        // Lock order is Store writer -> brief issued-slot access. No slot guard is held while
        // waiting for the writer or during I/O, and no slot reader ever waits for Store custody.
        // Re-read under the writer so simultaneous retries cannot each sample a new nonce.
        let issued = capture.issued.lock().map_err(|_| refused())?.clone();
        let current = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&capture.key)
            .cloned();
        if let Some(issued) = issued {
            if issued.record.origin != origin
                || issued.record.provenance != provenance
                || issued.admission.is_cancelled()
                || current.as_ref().is_none_or(|row| {
                    row.nonce != issued.record.nonce
                        || row.revision != issued.record.revision
                        || row.origin != origin
                        || row.provenance != provenance
                        || row.phase == Phase::Retired
                })
            {
                return Err(refused());
            }
            // This is the SAME privately issued candidate, not permission to advance again.
            // Its own observer still refuses any later removal, revision or mount replacement.
            if runtime.dirty {
                self.flush_chat_admissions(&mut runtime, rng)?;
            }
            return Ok(issued);
        }
        if capture
            .admission
            .as_ref()
            .is_some_and(RequestCancellation::is_cancelled)
        {
            return Err(refused());
        }
        if current.as_ref().is_some_and(|r| {
            r.phase == Phase::Retired
                && r.origin == Origin::Fresh
                && origin == Origin::Fresh
                && r.provenance == provenance
        }) {
            return Err(refused());
        }
        let record = if let Some(record) = current.as_ref().filter(|r| {
            r.origin == origin
                && r.provenance == provenance
                && (r.phase == Phase::Initializing
                    || (origin == Origin::Fresh && r.phase == Phase::Current))
        }) {
            record.clone()
        } else {
            if current != capture.original || (origin == Origin::Legacy && current.is_some()) {
                return Err(refused());
            }
            if current.is_none()
                && runtime.table.as_ref().ok_or_else(refused)?.len()
                    >= MAX_CHAT_ADMISSION_IDENTITIES
            {
                return Err(refused());
            }
            let revision = match current {
                Some(ref old) => old.revision.checked_add(1).ok_or_else(refused)?,
                None => 1,
            };
            let mut nonce = [0; 32];
            rng.fill_bytes(&mut nonce);
            // The persistent revision also separates admissions when a faulty/restored RNG
            // repeats bytes. A new admission cannot adopt an older scope merely by nonce reuse.
            let mut identity = blake3::Hasher::new();
            identity.update(b"catcoms/chat-admission-nonce/v1\0");
            identity.update(&revision.to_be_bytes());
            identity.update(&nonce);
            nonce = *identity.finalize().as_bytes();
            Record {
                nonce,
                revision,
                phase: Phase::Initializing,
                origin,
                provenance,
            }
        };
        let unchanged = current
            .as_ref()
            .is_some_and(|old| old.nonce == record.nonce && old.revision == record.revision);
        let (preparation, new_owner) = if unchanged {
            (
                self.reservation_locked(&mut runtime, &capture.key, &record)?,
                None,
            )
        } else {
            let (preparation, owner) = self.new_reservation(&capture.key, &record)?;
            (preparation, Some(owner))
        };
        // Final fresh-issuance admission point. Revocation is atomic and does not wait for this
        // writer, so recheck AFTER nonce/RNG/custody construction, immediately before the advance.
        // Revocation after this read may leave already-admitted original-custody work to finish.
        if capture
            .admission
            .as_ref()
            .is_some_and(RequestCancellation::is_cancelled)
            || preparation.admission.is_cancelled()
        {
            return Err(refused());
        }
        // Cache BEFORE revoking the previous nonce or touching disk. A failed rename can retry
        // exactly this candidate, even though issuing it already invalidated the old observer.
        *capture.issued.lock().map_err(|_| refused())? = Some(preparation.clone());
        if let Some(owner) = new_owner {
            runtime
                .predecessors
                .before_row_change(&capture.key, current.as_ref());
            if let Some(old) = runtime.owners.insert(capture.key.clone(), owner) {
                old.life.revoke();
            }
            runtime
                .table
                .as_mut()
                .ok_or_else(refused)?
                .insert(capture.key.clone(), record.clone());
        }
        if record.phase == Phase::Current {
            // Same verified attempt: preserve its unchanged owner and never reinstall old data.
            if runtime.dirty {
                self.flush_chat_admissions(&mut runtime, rng)?;
            }
            return Ok(preparation);
        }
        // Keep the EXACT candidate in memory even after a before-rename failure. Old observers
        // remain revoked; a retry flushes this same reservation instead of sampling a new nonce.
        self.flush_chat_admissions(&mut runtime, rng)?;
        Ok(preparation)
    }

    /// One-time bootstrap of previously unmanaged bytes. Any recorded identity, including a
    /// retired record or later fresh admission, closes this path permanently.
    pub fn prepare_legacy_chat_admission(
        &self,
        capture: &ChatAdmissionCapture,
        exact_snapshot: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<ChatAdmissionPreparation, AppError> {
        self.prepare_chat_admission(
            capture,
            *blake3::hash(exact_snapshot).as_bytes(),
            Origin::Legacy,
            rng,
        )
    }

    /// Storage-unit fixture only. Production renewal needs an opaque verified completion that
    /// owns the original pre-await capture and exact live KeyPackage evidence; it is not supplied
    /// by this unwired storage foundation and must never be minted from this scalar test seam.
    #[cfg(test)]
    fn prepare_fresh_chat_admission(
        &self,
        capture: &ChatAdmissionCapture,
        verified_attempt: [u8; 32],
        rng: &mut impl CryptoRngCore,
    ) -> Result<ChatAdmissionPreparation, AppError> {
        self.prepare_chat_admission(capture, verified_attempt, Origin::Fresh, rng)
    }

    /// Complete the exact reservation after a covering server save, then flush Current before
    /// returning any runtime binding. `save` is synchronous and must not re-enter admission APIs.
    /// All final server-storage writes and retirement share this inventory owner.
    pub fn finish_chat_admission<R: CryptoRngCore>(
        &self,
        preparation: &ChatAdmissionPreparation,
        rng: &mut R,
        save: impl FnOnce(&Self, &mut R) -> Result<(), AppError>,
    ) -> Result<ChatAdmissionFinish, AppError> {
        self.finish_chat_admission_with_storage(preparation, rng, |permit, rng| {
            save(permit.store(), rng)
        })
    }

    /// Cover private initialization under the already-held admission writer. The permit cannot
    /// escape the callback or be sent to an asynchronous worker. A subordinate owner-head saver
    /// takes its own writer AFTER this permit, then revalidates the original permit before I/O;
    /// it must never re-enter an outer admission API. Initializing permits grant no publication.
    pub(crate) fn finish_chat_admission_with_storage<R: CryptoRngCore>(
        &self,
        preparation: &ChatAdmissionPreparation,
        rng: &mut R,
        save: impl for<'writer> FnOnce(
            ChatAdmissionStoragePermit<'writer>,
            &mut R,
        ) -> Result<(), AppError>,
    ) -> Result<ChatAdmissionFinish, AppError> {
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed
            || preparation.domain != self.chat_vault_domain()?
            || preparation.mount.is_cancelled()
            || preparation.admission.is_cancelled()
            || !Arc::ptr_eq(&preparation.mount_identity, &self.chat_mount.identity)
        {
            return Err(refused());
        }
        // An old preparation may predate a newly saved JOIN source. It cannot advance that
        // source's captured Initializing row or flush/rebind an old Current result afterward.
        self.require_no_pending_checkpoint_join_source(
            preparation.key.group(),
            preparation.key.signer(),
        )?;
        self.require_no_archive_bootstrap_target(
            preparation.key.group(),
            preparation.key.signer(),
        )?;
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&preparation.key)
            .cloned()
            .ok_or_else(refused)?;
        if row.nonce != preparation.record.nonce
            || row.revision != preparation.record.revision
            || row.origin != preparation.record.origin
            || row.provenance != preparation.record.provenance
            || row.phase == Phase::Retired
        {
            return Err(refused());
        }
        let already_current = row.phase == Phase::Current;
        if runtime.dirty {
            self.flush_chat_admissions(&mut runtime, rng)?;
        }
        if already_current {
            // A delayed duplicate may carry S0 after ordinary work saved S1. Current never
            // authorizes rerunning the covering callback or claiming that it saved fresh data.
            return self
                .bind_chat_admission_locked(
                    &mut runtime,
                    &preparation.key,
                    preparation.record.nonce,
                )
                .map(ChatAdmissionFinish::AlreadyCurrent);
        }
        // No await between final lineage check, covering original-store save and Current commit.
        if preparation.mount.is_cancelled() || preparation.admission.is_cancelled() {
            return Err(refused());
        }
        let permit = ChatAdmissionStoragePermit::initializing(self, &runtime, preparation);
        permit.validate_original(self, preparation.key.group(), preparation.key.signer())?;
        save(permit, rng)?;
        if preparation.admission.is_cancelled() {
            return Err(refused());
        }
        runtime
            .predecessors
            .before_row_change(&preparation.key, Some(&row));
        runtime
            .table
            .as_mut()
            .ok_or_else(refused)?
            .get_mut(&preparation.key)
            .ok_or_else(refused)?
            .phase = Phase::Current;
        self.flush_chat_admissions(&mut runtime, rng)?;
        self.bind_chat_admission_locked(&mut runtime, &preparation.key, preparation.record.nonce)
            .map(ChatAdmissionFinish::Saved)
    }

    fn bind_chat_admission_locked(
        &self,
        runtime: &mut Runtime,
        key: &ChatAdmissionKey,
        nonce: [u8; 32],
    ) -> Result<ChatAdmissionBinding, AppError> {
        if !runtime.managed || runtime.dirty || !self.chat_mount.life.live() {
            return Err(refused());
        }
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(key)
            .cloned()
            .ok_or_else(refused)?;
        if row.phase != Phase::Current || row.nonce != nonce {
            return Err(refused());
        }
        let custody = self.reservation_locked(runtime, key, &row)?;
        Ok(ChatAdmissionBinding {
            key: key.clone(),
            nonce,
            revision: row.revision,
            domain: custody.domain,
            mount_identity: custody.mount_identity,
            mount: custody.mount,
            publication: custody.admission,
            owner_identity: custody.owner_identity,
            revoker: custody.revoker,
            owner_writer_identity: None,
        })
    }

    /// Resolve saved scope labels against the independent current-admission record. Raw decode
    /// cannot bypass this method, and no-tail restore uses the separate closed bootstrap path.
    pub fn resolve_chat_admission(
        &self,
        key: &ChatAdmissionKey,
        nonce: [u8; 32],
    ) -> Result<ChatAdmissionResolution, AppError> {
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed || runtime.dirty {
            return Err(refused());
        }
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(key)
            .cloned()
            .ok_or_else(refused)?;
        // A genuine later Welcome changes the nonce, making every older capture non-current.
        if row.nonce != nonce || row.phase == Phase::Retired {
            return Ok(ChatAdmissionResolution::Retired);
        }
        // Retired above remains a negative result. Positive Initializing/Current resolution
        // must not reopen an older generic owner over a retained private JOIN source.
        self.require_no_pending_checkpoint_join_source(key.group(), key.signer())?;
        self.require_no_archive_bootstrap_target(key.group(), key.signer())?;
        match row.phase {
            Phase::Current => self
                .bind_chat_admission_locked(&mut runtime, key, nonce)
                .map(ChatAdmissionResolution::Current),
            Phase::Initializing => self
                .reservation_locked(&mut runtime, key, &row)
                .map(ChatAdmissionResolution::Initializing),
            Phase::Retired => Ok(ChatAdmissionResolution::Retired),
        }
    }

    fn original_chat_store(&self, binding: &ChatAdmissionBinding) -> Result<(), AppError> {
        if !Arc::ptr_eq(&binding.mount_identity, &self.chat_mount.identity)
            || binding.mount.is_cancelled()
            || binding.domain != self.chat_vault_domain()?
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Final positive storage admission. Holding the global writer until actual callback return
    /// orders all snapshot replacement against durable negative retirement and current renewal.
    pub fn with_chat_admission_storage<T>(
        &self,
        binding: &ChatAdmissionBinding,
        write: impl FnOnce(&Self) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        self.with_chat_admission_storage_permit(binding, |permit| write(permit.store()))
    }

    /// Supply exact original-store custody without recursively reacquiring the admission writer.
    /// The synchronous callback may take a subordinate logical head writer, and must revalidate
    /// this permit after that wait. Its return value cannot borrow the callback-local permit.
    pub(crate) fn with_chat_admission_storage_permit<T>(
        &self,
        binding: &ChatAdmissionBinding,
        write: impl for<'writer> FnOnce(ChatAdmissionStoragePermit<'writer>) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let runtime = self.chat_mount.locked()?;
        self.original_chat_store(binding)?;
        if !runtime.managed || runtime.dirty {
            return Err(refused());
        }
        // Fence even bindings captured before source save. Check while admission is held and
        // BEFORE a subordinate saver/head lock; source admission cannot race this later write.
        self.require_no_pending_checkpoint_join_source(binding.key.group(), binding.key.signer())?;
        self.require_no_archive_bootstrap_target(binding.key.group(), binding.key.signer())?;
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&binding.key)
            .ok_or_else(refused)?;
        if row.phase != Phase::Current
            || row.nonce != binding.nonce
            || row.revision != binding.revision
        {
            return Err(refused());
        }
        // Final positive storage admission read. A later revoke cannot recall this original
        // synchronous write; the writer still prevents it from running after a durable fence.
        if !binding.is_live() {
            return Err(refused());
        }
        let permit = ChatAdmissionStoragePermit::current(self, &runtime, binding);
        permit.validate_original(self, binding.key.group(), binding.key.signer())?;
        write(permit)
    }

    /// Negative history repair bypasses role/UI, not physical original-store lifetime. It cannot
    /// change external lineage or authorize publication. Same-nonce retirement remains repairable;
    /// a newer admission must merge old evidence through its own transaction. Host custody still
    /// supplies the exact server-incarnation lease; this gate cannot identify numeric slot owners.
    pub fn with_original_chat_store<T>(
        &self,
        binding: &ChatAdmissionBinding,
        write: impl FnOnce(&Self) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let runtime = self.chat_mount.locked()?;
        self.original_chat_store(binding)?;
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&binding.key)
            .ok_or_else(refused)?;
        if row.nonce != binding.nonce {
            return Err(refused());
        }
        if binding.mount.is_cancelled() {
            return Err(refused());
        }
        write(self)
    }

    /// Retire using already reserved bytes, synchronously revoking EVERY prior observer before
    /// disk I/O. Failure retains the exact negative candidate and dirty debt; retry cannot rebind
    /// the previous Current row even if its older bytes still occupy the file before rename.
    pub fn retire_chat_admission(
        &self,
        binding: &ChatAdmissionBinding,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let mut runtime = self.chat_mount.locked()?;
        self.original_chat_store(binding)?;
        self.retire_chat_row(
            &mut runtime,
            &binding.key,
            binding.nonce,
            binding.revision,
            rng,
        )
    }

    /// A reserved Initializing scope can be removed before Current is issued. This original
    /// negative custody remains usable after its publication/liveness observer is revoked.
    pub fn retire_chat_preparation(
        &self,
        preparation: &ChatAdmissionPreparation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let mut runtime = self.chat_mount.locked()?;
        if preparation.mount.is_cancelled()
            || !Arc::ptr_eq(&preparation.mount_identity, &self.chat_mount.identity)
            || preparation.domain != self.chat_vault_domain()?
        {
            return Err(refused());
        }
        self.retire_chat_row(
            &mut runtime,
            &preparation.key,
            preparation.record.nonce,
            preparation.record.revision,
            rng,
        )
    }

    fn retire_chat_row(
        &self,
        runtime: &mut Runtime,
        key: &ChatAdmissionKey,
        nonce: [u8; 32],
        revision: u64,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let original = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(key)
            .cloned()
            .ok_or_else(refused)?;
        if original.nonce != nonce {
            return Err(refused());
        }
        if original.phase != Phase::Retired {
            if original.revision != revision {
                return Err(refused());
            }
            runtime.predecessors.before_row_change(key, Some(&original));
            let row = runtime
                .table
                .as_mut()
                .ok_or_else(refused)?
                .get_mut(key)
                .ok_or_else(refused)?;
            row.phase = Phase::Retired;
            // Retirement still works at revision exhaustion; future renewal's checked_add refuses.
            row.revision = row.revision.saturating_add(1);
        }
        if let Some(owner) = runtime.owners.remove(key) {
            owner.life.revoke();
        }
        self.flush_chat_admissions(runtime, rng)
    }

    /// Metadata-only debt can be flushed by the original mounted persistence worker while UI is
    /// locked. It does not complete missing server initialization or issue a new runtime binding.
    pub fn repair_chat_admission_inventory(
        &self,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let mut runtime = self.chat_mount.locked()?;
        if runtime.dirty {
            self.flush_chat_admissions(&mut runtime, rng)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
