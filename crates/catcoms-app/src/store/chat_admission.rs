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
use tokio::sync::watch;

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
}

/// An exact initialization reservation, never a publication/storage capability.
#[derive(Clone)]
pub struct ChatAdmissionPreparation {
    key: ChatAdmissionKey,
    record: Record,
    domain: [u8; 32],
    mount_identity: Arc<()>,
    mount: RequestCancellation,
}

/// Restore classification. Initializing resumes the same recorded nonce; Retired permits only
/// retained reads/original-store negative repair, never automatic bootstrap or publication.
pub enum ChatAdmissionResolution {
    Current(ChatAdmissionBinding),
    Initializing(ChatAdmissionPreparation),
    Retired,
}
/// Saved covers the supplied snapshot callback; AlreadyCurrent is only an exact scope replay
/// and cannot acknowledge a newer dirty snapshot ticket or reinstall an old candidate graph.
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
    pub fn nonce(&self) -> [u8; 32] {
        self.record.nonce
    }
    pub fn domain(&self) -> [u8; 32] {
        self.domain
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
}

struct AdmissionOwner {
    nonce: [u8; 32],
    revision: u64,
    signal: watch::Sender<bool>,
}
#[derive(Default)]
struct Runtime {
    table: Option<BTreeMap<ChatAdmissionKey, Record>>,
    dirty: bool,
    managed: bool,
    owners: BTreeMap<ChatAdmissionKey, AdmissionOwner>,
}
/// Owned by one physical ServerStore. Observers never retain these revoking senders.
pub(super) struct MountOwner {
    identity: Arc<()>,
    signal: watch::Sender<bool>,
    runtime: Mutex<Runtime>,
}
impl MountOwner {
    pub(super) fn new() -> Self {
        Self {
            identity: Arc::new(()),
            signal: watch::channel(false).0,
            runtime: Mutex::new(Runtime::default()),
        }
    }
    fn observer(&self) -> RequestCancellation {
        RequestCancellation::new(self.signal.subscribe(), None)
    }
    pub(super) fn revoke(&self) {
        self.signal.send_replace(true);
    }
    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Runtime>, AppError> {
        if *self.signal.borrow() {
            return Err(refused());
        }
        let runtime = self.runtime.lock().map_err(|_| refused())?;
        // Revocation can happen while this synchronous writer waits for earlier admitted work.
        if *self.signal.borrow() {
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
    admission: watch::Receiver<bool>,
}
#[async_trait::async_trait]
impl RequestAuthority for AdmissionAuthority {
    fn authorized(&self) -> bool {
        !self.mount.is_cancelled()
            && !*self.admission.borrow()
            && self.admission.has_changed().is_ok()
    }
    async fn revoked(&self) {
        let mut mount = self.mount.clone();
        let mut admission = self.admission.clone();
        while self.authorized() {
            tokio::select! {
                _ = mount.cancelled() => return,
                changed = admission.changed() => if changed.is_err() { return; },
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
    /// Revoke before any replacement/unmount await. Locking the UI alone does not unmount.
    pub fn revoke_chat_mount(&self) {
        self.chat_mount.revoke();
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
        if *self.chat_mount.signal.borrow() {
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
        let opened = open_or_create_vault(&self.dir, passphrase, rng)?;
        if opened.db_key()? != self.keys.db_key()? {
            self.chat_mount.revoke();
            return Err(refused());
        }
        let managed = catcoms_storage::vault::chat_scope_vault_managed(&self.dir)?;
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
        Ok(())
    }

    /// Capture before the first join/issuance await. The private record is an exact CAS basis.
    pub fn capture_chat_admission(
        &self,
        key: ChatAdmissionKey,
    ) -> Result<ChatAdmissionCapture, AppError> {
        let runtime = self.chat_mount.locked()?;
        if !runtime.managed || runtime.dirty {
            return Err(refused());
        }
        let original = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&key)
            .cloned();
        Ok(ChatAdmissionCapture {
            key,
            original,
            mount_identity: self.chat_mount.identity.clone(),
            mount: self.chat_mount.observer(),
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
        let table = runtime.table.as_mut().ok_or_else(refused)?;
        let current = table.get(&capture.key).cloned();
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
            if current.is_none() && table.len() >= MAX_CHAT_ADMISSION_IDENTITIES {
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
        if record.phase == Phase::Current {
            // Same verified attempt, already complete: do not revoke its unchanged owner or
            // rerun initialization. Finish reports this as AlreadyCurrent.
            return Ok(ChatAdmissionPreparation {
                key: capture.key.clone(),
                record,
                domain: self.chat_vault_domain()?,
                mount_identity: self.chat_mount.identity.clone(),
                mount: self.chat_mount.observer(),
            });
        }
        if let Some(old) = runtime.owners.remove(&capture.key) {
            old.signal.send_replace(true);
        }
        runtime
            .table
            .as_mut()
            .ok_or_else(refused)?
            .insert(capture.key.clone(), record.clone());
        // Keep the EXACT candidate in memory even after a before-rename failure. Old observers
        // remain revoked; a retry flushes this same reservation instead of sampling a new nonce.
        self.flush_chat_admissions(&mut runtime, rng)?;
        Ok(ChatAdmissionPreparation {
            key: capture.key.clone(),
            record,
            domain: self.chat_vault_domain()?,
            mount_identity: self.chat_mount.identity.clone(),
            mount: self.chat_mount.observer(),
        })
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
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed
            || preparation.domain != self.chat_vault_domain()?
            || preparation.mount.is_cancelled()
            || !Arc::ptr_eq(&preparation.mount_identity, &self.chat_mount.identity)
        {
            return Err(refused());
        }
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(&preparation.key)
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
        if preparation.mount.is_cancelled() {
            return Err(refused());
        }
        save(self, rng)?;
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
        if !runtime.managed || runtime.dirty || *self.chat_mount.signal.borrow() {
            return Err(refused());
        }
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(key)
            .ok_or_else(refused)?;
        if row.phase != Phase::Current || row.nonce != nonce {
            return Err(refused());
        }
        let revision = row.revision;
        if runtime
            .owners
            .get(key)
            .is_some_and(|owner| owner.nonce != nonce || owner.revision != revision)
        {
            if let Some(old) = runtime.owners.remove(key) {
                old.signal.send_replace(true);
            }
        }
        let owner = runtime
            .owners
            .entry(key.clone())
            .or_insert_with(|| AdmissionOwner {
                nonce,
                revision,
                signal: watch::channel(false).0,
            });
        let mount = self.chat_mount.observer();
        let admission = owner.signal.subscribe();
        let publication = RequestCancellation::new(admission.clone(), None).with_authority(
            Arc::new(AdmissionAuthority {
                mount: mount.clone(),
                admission,
            }),
        );
        Ok(ChatAdmissionBinding {
            key: key.clone(),
            nonce,
            revision,
            domain: self.chat_vault_domain()?,
            mount_identity: self.chat_mount.identity.clone(),
            mount,
            publication,
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
        match row.phase {
            Phase::Current => self
                .bind_chat_admission_locked(&mut runtime, key, nonce)
                .map(ChatAdmissionResolution::Current),
            Phase::Initializing => Ok(ChatAdmissionResolution::Initializing(
                ChatAdmissionPreparation {
                    key: key.clone(),
                    record: row,
                    domain: self.chat_vault_domain()?,
                    mount_identity: self.chat_mount.identity.clone(),
                    mount: self.chat_mount.observer(),
                },
            )),
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
        let runtime = self.chat_mount.locked()?;
        self.original_chat_store(binding)?;
        if !runtime.managed || runtime.dirty || !binding.is_live() {
            return Err(refused());
        }
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
        write(self)
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
        let row = runtime
            .table
            .as_mut()
            .ok_or_else(refused)?
            .get_mut(&binding.key)
            .ok_or_else(refused)?;
        if row.nonce != binding.nonce {
            return Err(refused());
        }
        if row.phase != Phase::Retired {
            if row.revision != binding.revision {
                return Err(refused());
            }
            row.phase = Phase::Retired;
            // Retirement still works at revision exhaustion; future renewal's checked_add refuses.
            row.revision = row.revision.saturating_add(1);
        }
        if let Some(owner) = runtime.owners.remove(&binding.key) {
            owner.signal.send_replace(true);
        }
        self.flush_chat_admissions(&mut runtime, rng)
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
