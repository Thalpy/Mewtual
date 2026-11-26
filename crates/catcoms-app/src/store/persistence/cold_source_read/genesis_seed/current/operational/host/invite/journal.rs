//! One immutable, covered unsigned intent inside the original text.pages.
//!
//! One persistent intent is retained per selected parent. A torn or unknown
//! object blocks further work on that parent;
//! only the same warm owner may finish its own exact partially written frame.
//! The signing stages use separate immutable files under the same held directory.
use super::*;
use crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard;
use crate::store::persistence::v2_text::TextHeadIdData;
use catcoms_crypto::{seal, unseal};
use catcoms_replication::chat::join_bindings::InviteBindingSpecData;
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use catcoms_storage::VaultPhysicalWorkLease;
use catcoms_wire::{Decoder, Encoder};
use std::borrow::Cow;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const NAME: &str = "invite.intent.v1";
const TAG: &[u8] = b"mewtual/private-chat/invite-intent/v1\0";
const KEY_DOMAIN: &str = "mewtual/private-chat/invite-intent-key/v1";
const CONTEXT_DOMAIN: &[u8] = b"mewtual/private-chat/invite-context/v1\0";
const SELECTED_CONTEXT_DOMAIN: &str = "mewtual/private-chat/invite-context-selected/v2";
const MAX_NAME: usize = "invite.attempt.v2.".len() + 64;
const OBJECTS: u32 = 1;
const BYTES: u32 = 8192;
const MAX_PLAIN: usize = 4096;
const MAX_FRAME: usize = MAX_PLAIN + 40;
const MAX_PATH: usize = 32 * 1024 + 256;
mod recovery;
mod signing;
pub(super) use recovery::InviteRecoveryRead;
pub(super) use signing::{InviteSigningJournal, SIGNING_JOURNAL_BYTES};

type Pages = (PathBuf, File, SessionId);

struct InviteJournalNames {
    intent: Cow<'static, str>,
    attempt: Cow<'static, str>,
    raw: Cow<'static, str>,
}

impl InviteJournalNames {
    fn for_parent(parent: Option<TextHeadIdData>) -> Self {
        if let Some(parent) = parent {
            let suffix = hex::encode(parent.as_bytes());
            Self {
                intent: format!("invite.intent.v2.{suffix}").into(),
                attempt: format!("invite.attempt.v2.{suffix}").into(),
                raw: format!("invite.raw.v2.{suffix}").into(),
            }
        } else {
            Self {
                intent: NAME.into(),
                attempt: signing::ATTEMPT.into(),
                raw: signing::RAW.into(),
            }
        }
    }
}

impl Default for InviteJournalNames {
    fn default() -> Self {
        Self::for_parent(None)
    }
}

#[cfg(test)]
std::thread_local! {
    static FAIL_PARTIAL_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
pub(super) fn fail_partial_write_for_test() {
    FAIL_PARTIAL_WRITE.with(|fault| fault.set(true));
}

pub(super) const JOURNAL_BYTES: usize = 8 * MAX_FRAME + 4 * MAX_PATH + 16 * 1024;

struct Intent {
    request: V2FounderInviteRequestData,
    route: Vec<u8>,
    preimage: Vec<u8>,
    expires_at_ms: u64,
}
impl Intent {
    fn result(
        &self,
        request: V2FounderInviteRequestData,
        route: &[u8],
    ) -> Result<([u8; 32], u64), AppError> {
        if self.request != request || self.route != route {
            return Err(refused());
        }
        Ok((*blake3::hash(&self.preimage).as_bytes(), self.expires_at_ms))
    }
}

/// Private affine IO owner inside the SAME synchronous RuntimeLoan. Its sole
/// constructor is used by the native original's Host, never a cold/library
/// adopter. Native Source retirement waits for the loan. Actual Current4 and
/// the selected G1/member parent are proved around each use by the caller.
/// Files drain before the original vault's physical-work lease.
#[derive(Default)]
pub(super) struct InviteIntentJournal {
    pages: Option<Pages>,
    context: Option<[u8; 32]>,
    object: JournalFile,
    parent: Option<TextHeadIdData>,
    names: InviteJournalNames,
    intent: Option<Intent>,
    observed_absent: bool,
    observed_present: bool,
    loaded: bool,
    covered: bool,
    _lease: Option<VaultPhysicalWorkLease>,
}

#[derive(Default)]
struct JournalFile {
    file: Option<(File, SessionId)>,
    frame: Option<Zeroizing<Vec<u8>>>,
    create_entered: bool,
}

fn require_pages((path, file, id): &Pages) -> Result<(), AppError> {
    crate::store::persistence::v2_genesis_activation::require_directory_name(path, file, *id)
}

fn original_context(guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<[u8; 32], AppError> {
    let (expected, genesis, nonce) = guard.original_genesis_context_data()?;
    let encoded =
        crate::store::critical_namespaces::archive_bootstrap::founder_codec::encode_context(
            &expected.context,
        )?;
    let mut h = blake3::Hasher::new();
    h.update(CONTEXT_DOMAIN);
    h.update(&(encoded.len() as u64).to_be_bytes());
    h.update(&encoded);
    h.update(blake3::hash(genesis).as_bytes());
    h.update(&nonce);
    h.update(blake3::hash(guard.original_pin_data().2).as_bytes());
    Ok(*h.finalize().as_bytes())
}

fn bound_context(
    guard: &V2TextCurrentGroupGuard<'_, '_>,
    parent: Option<TextHeadIdData>,
) -> Result<[u8; 32], AppError> {
    let genesis = original_context(guard)?;
    if let Some(parent) = parent {
        let mut hash = blake3::Hasher::new_derive_key(SELECTED_CONTEXT_DOMAIN);
        hash.update(&genesis);
        hash.update(parent.as_bytes());
        Ok(*hash.finalize().as_bytes())
    } else {
        Ok(genesis)
    }
}

fn open_pages(guard: &V2TextCurrentGroupGuard<'_, '_>) -> Result<Pages, AppError> {
    let (target, _, _) = guard.target_directory_data();
    if target.as_os_str().len() + 1 + "text.pages".len() + 1 + MAX_NAME > MAX_PATH {
        return Err(refused());
    }
    let pages = target.join("text.pages");
    let file = crate::store::persistence::open_archive_directory(&pages)?;
    let id = session_id(&file).map_err(io)?;
    let pages = (pages, file, id);
    require_pages(&pages)?;
    Ok(pages)
}

fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}
fn regular(metadata: &fs::Metadata) -> bool {
    let plain = metadata.file_type().is_file() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        plain && metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    plain
}
fn named(path: &Path, file: &File, id: SessionId) -> Result<(), AppError> {
    if !regular(&file.metadata().map_err(io)?)
        || session_id(file).map_err(io)? != id
        || !matches!(session_at(path, id), SessionAt::Ours)
    {
        return Err(refused());
    }
    Ok(())
}
fn field<'a>(d: &mut Decoder<'a>, cap: usize) -> Result<&'a [u8], AppError> {
    let value = d.get_bytes().map_err(|_| refused())?;
    if value.len() > cap {
        return Err(refused());
    }
    Ok(value)
}
fn decode(plain: &[u8], context: [u8; 32]) -> Result<Intent, AppError> {
    if plain.len() > MAX_PLAIN {
        return Err(refused());
    }
    let mut d = Decoder::new(plain);
    if field(&mut d, TAG.len())? != TAG
        || d.get_u16().map_err(|_| refused())? != 1
        || d.get_u32().map_err(|_| refused())? != OBJECTS
        || d.get_u32().map_err(|_| refused())? != BYTES
        || field(&mut d, 32)? != context
    {
        return Err(refused());
    }
    let token: [u8; 32] = field(&mut d, 32)?.try_into().map_err(|_| refused())?;
    let lifetime_ms = d.get_u64().map_err(|_| refused())?;
    let route = field(&mut d, 1024)?;
    let preimage = field(&mut d, 1024)?;
    d.finish().map_err(|_| refused())?;
    let spec = InviteBindingSpecData::decode_signing_bytes_data(preimage).map_err(|_| refused())?;
    if token == [0; 32]
        || lifetime_ms == 0
        || lifetime_ms > MAX_FOUNDER_INVITE_LIFETIME_MS
        || route.is_empty()
        || spec.expires_at_ms < lifetime_ms
        || spec.signing_bytes_data().map_err(|_| refused())? != preimage
    {
        return Err(refused());
    }
    let public_routing_plan =
        catcoms_replication::chat::join_bindings::JoinPayloadKindData::RoutingPlan
            .commitment_data(route)
            .map_err(|_| refused())?;
    if public_routing_plan != spec.public_routing_plan {
        return Err(refused());
    }
    Ok(Intent {
        request: V2FounderInviteRequestData {
            token,
            lifetime_ms,
            public_routing_plan,
        },
        route: route.to_vec(),
        preimage: preimage.to_vec(),
        expires_at_ms: spec.expires_at_ms,
    })
}

impl InviteIntentJournal {
    pub(super) fn for_parent(parent: Option<TextHeadIdData>) -> Self {
        Self {
            parent,
            names: InviteJournalNames::for_parent(parent),
            ..Self::default()
        }
    }

    pub(super) fn parent_data(&self) -> Option<TextHeadIdData> {
        self.parent
    }

    fn bind(
        &mut self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        let context = bound_context(guard, self.parent)?;
        if self.context.is_some_and(|original| original != context) {
            return Err(refused());
        }
        if self.pages.is_none() {
            let lease = guard.store()._session.retain_for_physical_work();
            self.pages = Some(open_pages(guard)?);
            self.context = Some(context);
            self._lease = Some(lease);
        }
        require_pages(self.pages()?)
    }
    fn pages(&self) -> Result<&Pages, AppError> {
        self.pages.as_ref().ok_or_else(refused)
    }
    fn key(store: &crate::store::ServerStore) -> Result<Zeroizing<[u8; 32]>, AppError> {
        let key = Zeroizing::new(store.keys.db_key()?);
        Ok(Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key.as_ref())))
    }
    /// Exact token lookup BEFORE the retained node's Clock/RNG. Missing after
    /// creation, malformed quota/format, foreign MAC/context and torn files are
    /// errors, never empty usage. Absence can initialize only this unique warm
    /// original, before it has entered its one create; no cold adopter exists.
    pub(super) fn lookup(
        &mut self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
        request: V2FounderInviteRequestData,
        route: &[u8],
    ) -> Result<Option<([u8; 32], u64)>, AppError> {
        self.bind(guard)?;
        if let Some(intent) = &self.intent {
            let result = intent.result(request, route)?;
            if !self.covered && !self.loaded {
                return Ok(None);
            }
            self.object
                .sync(self.pages.as_ref().ok_or_else(refused)?, &self.names.intent)?;
            self.covered = true;
            return Ok(Some(result));
        }
        if self.observed_absent {
            return Ok(None);
        }
        let path = self.pages()?.0.join(&*self.names.intent);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !self.observed_present => {
                self.observed_absent = true;
                require_pages(self.pages()?)?;
                return Ok(None);
            }
            Ok(metadata) => {
                self.observed_present = true;
                if !regular(&metadata) || metadata.len() <= 40 || metadata.len() > MAX_FRAME as u64
                {
                    return Err(refused());
                }
            }
            Err(e) => {
                self.observed_present = true;
                return Err(io(e));
            }
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(io)?;
        let id = session_id(&file).map_err(io)?;
        named(&path, &file, id)?;
        let mut file = file;
        let frame = read(&mut file)?;
        named(&path, &file, id)?;
        let key = Self::key(guard.store())?;
        let plain = Zeroizing::new(unseal(&key, &crate::store::unframe(&frame)?)?);
        let intent = decode(&plain, self.context.ok_or_else(refused)?)?;
        intent.result(request, route)?;
        self.object.file = Some((file, id));
        self.object.frame = Some(frame);
        self.intent = Some(intent);
        self.loaded = true;
        self.object
            .sync(self.pages.as_ref().ok_or_else(refused)?, &self.names.intent)?;
        self.covered = true;
        Ok(Some(self.intent.as_ref().unwrap().result(request, route)?))
    }
    /// Seal/cache the original OUTSIDE Current/Root writers. No IO, SDK or key
    /// signing occurs here. A failed physical write never causes another seal.
    pub(super) fn prepare_data(
        &mut self,
        store: &crate::store::ServerStore,
        request: V2FounderInviteRequestData,
        route: &[u8],
        preimage: &[u8],
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.intent.is_none() {
            if !self.observed_absent || route.len() > 1024 || preimage.len() > 1024 {
                return Err(refused());
            }
            let mut e = Encoder::with_capacity(MAX_PLAIN);
            e.put_bytes(TAG).map_err(|_| refused())?;
            e.put_u16(1);
            e.put_u32(OBJECTS);
            e.put_u32(BYTES);
            e.put_bytes(&self.context.ok_or_else(refused)?)
                .map_err(|_| refused())?;
            e.put_bytes(&request.token).map_err(|_| refused())?;
            e.put_u64(request.lifetime_ms);
            e.put_bytes(route).map_err(|_| refused())?;
            e.put_bytes(preimage).map_err(|_| refused())?;
            let plain = Zeroizing::new(e.finish());
            let intent = decode(&plain, self.context.ok_or_else(refused)?)?;
            intent.result(request, route)?;
            let key = Self::key(store)?;
            let sealed = seal(&key, &plain, rng)?;
            self.object.frame = Some(Zeroizing::new(crate::store::frame(&sealed)));
            self.intent = Some(intent);
        }
        let intent = self.intent.as_ref().ok_or_else(refused)?;
        intent.result(request, route)?;
        if intent.preimage != preimage {
            return Err(refused());
        }
        Ok(())
    }
    /// Persist only the SAME presealed unsigned original under fresh actual
    /// Current4/physical/native checks. No RNG or signing callback can enter.
    pub(super) fn cover(
        &mut self,
        guard: &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
        request: V2FounderInviteRequestData,
        route: &[u8],
        preimage: &[u8],
    ) -> Result<([u8; 32], u64), AppError> {
        self.bind(guard)?;
        let intent = self.intent.as_ref().ok_or_else(refused)?;
        let result = intent.result(request, route)?;
        if intent.preimage != preimage {
            return Err(refused());
        }
        self.object
            .cover(self.pages.as_ref().ok_or_else(refused)?, &self.names.intent)?;
        self.covered = true;
        Ok(result)
    }

    fn require_original(&mut self) -> Result<(), AppError> {
        if !self.covered || self.loaded || !self.object.create_entered {
            return Err(refused());
        }
        self.object
            .require_exact(self.pages.as_ref().ok_or_else(refused)?, &self.names.intent)
    }
}

impl JournalFile {
    fn commitment(&self) -> Result<[u8; 32], AppError> {
        let frame = self.frame.as_ref().ok_or_else(refused)?;
        Ok(*blake3::hash(frame).as_bytes())
    }

    fn cover(&mut self, pages: &Pages, name: &str) -> Result<(), AppError> {
        self.write_same(pages, name)?;
        self.sync(pages, name)
    }

    fn write_same(&mut self, pages: &Pages, name: &str) -> Result<(), AppError> {
        let frame = self.frame.as_ref().ok_or_else(refused)?;
        require_pages(pages)?;
        let path = pages.0.join(name);
        if self.file.is_none() {
            if self.create_entered {
                return Err(refused());
            }
            self.create_entered = true;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(io)?;
            let id = session_id(&file).map_err(io)?;
            self.file = Some((file, id));
        }
        let (file, id) = self.file.as_mut().ok_or_else(refused)?;
        named(&path, file, *id)?;
        let present = read_capped(file, frame.len())?;
        if !frame.starts_with(&present) {
            return Err(refused());
        }
        file.seek(SeekFrom::End(0)).map_err(io)?;
        #[cfg(test)]
        if FAIL_PARTIAL_WRITE.with(|fault| fault.replace(false)) {
            let end = (present.len() + 128).min(frame.len());
            file.write_all(&frame[present.len()..end]).map_err(io)?;
            return Err(refused());
        }
        file.write_all(&frame[present.len()..]).map_err(io)?;
        named(&path, file, *id)?;
        require_pages(pages)
    }
    fn require_exact(&mut self, pages: &Pages, name: &str) -> Result<(), AppError> {
        require_pages(pages)?;
        let path = pages.0.join(name);
        let (file, id) = self.file.as_mut().ok_or_else(refused)?;
        named(&path, file, *id)?;
        let frame = self.frame.as_deref().ok_or_else(refused)?;
        if read_capped(file, frame.len())?.as_slice() != frame.as_slice() {
            return Err(refused());
        }
        named(&path, file, *id)?;
        require_pages(pages)
    }
    fn sync(&mut self, pages: &Pages, name: &str) -> Result<(), AppError> {
        self.require_exact(pages, name)?;
        self.file
            .as_ref()
            .ok_or_else(refused)?
            .0
            .sync_all()
            .map_err(io)?;
        crate::store::persistence::sync_directory(&pages.0).map_err(io)?;
        self.require_exact(pages, name)
    }
}

/// A single capped buffer and EOF probe. Empty is permitted only so the SAME
/// warm writer can finish its own prefix; lookup rejects it before this read.
fn read(file: &mut File) -> Result<Zeroizing<Vec<u8>>, AppError> {
    read_capped(file, MAX_FRAME)
}

fn read_capped(file: &mut File, maximum: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let metadata = file.metadata().map_err(io)?;
    if !regular(&metadata) || metadata.len() > maximum as u64 {
        return Err(refused());
    }
    let length = metadata.len() as usize;
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut bytes = Zeroizing::new(vec![0; length + 1]);
    let mut used = 0;
    while used < bytes.len() {
        match file.read(&mut bytes[used..]) {
            Ok(0) => break,
            Ok(n) => used += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io(e)),
        }
    }
    if used != length {
        return Err(refused());
    }
    bytes.truncate(used);
    Ok(bytes)
}

#[cfg(test)]
mod tests;
