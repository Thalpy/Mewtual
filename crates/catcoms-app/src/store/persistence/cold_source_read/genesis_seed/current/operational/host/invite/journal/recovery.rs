//! An independent read of the three original records. Warm inspection is
//! read-only. Cold publication can sync a complete, authenticated set as
//! held, without repairing an incomplete record or obtaining a key cut here.
use super::*;
use crate::store::persistence::v2_text::ColdInviteParent;
use crate::store::{NativeTextInviteRecovery, ServerStore};
use catcoms_replication::chat::{
    join_bindings::SignedInviteBindingData,
    owner_header::SignedOwnerHeader,
    owner_profile::{AdmissionBody, SignedOwnerProfile},
    ChatError,
};
use catcoms_sync::owner_v2::{
    BootstrapSourceContextData, OpenedBootstrapSourceData, V2GenesisSeedData,
};
use signing::{open_stage, FRAME_BYTES};

const READ_BYTES: usize = 6 * MAX_PATH + 6 * MAX_FRAME + 8 * FRAME_BYTES + 16 * 1024;

pub(in super::super) struct InviteRecoveryRead<'a> {
    store: &'a ServerStore,
    parent: Option<TextHeadIdData>,
    names: InviteJournalNames,
    context: Option<[u8; 32]>,
    pages: Option<Pages>,
    intent: Option<ReadObject>,
    attempt: Option<ReadObject>,
    raw: Option<ReadObject>,
    // Files and their scratch must drain before these outside-guard owners.
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

struct InviteGenesis<'a> {
    context: &'a BootstrapSourceContextData,
    profile: &'a SignedOwnerProfile,
    admission: &'a AdmissionBody,
    header: &'a SignedOwnerHeader,
}

impl<'a> From<V2GenesisSeedData<'a>> for InviteGenesis<'a> {
    fn from(original: V2GenesisSeedData<'a>) -> Self {
        Self {
            context: original.context,
            profile: original.profile,
            admission: original.admission,
            header: original.header,
        }
    }
}

impl<'a> InviteRecoveryRead<'a> {
    pub(in super::super) fn prepare(
        store: &'a ServerStore,
        parent: Option<TextHeadIdData>,
    ) -> Result<Self, AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(READ_BYTES)?;
        fee.validate(pool, READ_BYTES)?;
        Ok(Self {
            store,
            parent,
            names: InviteJournalNames::for_parent(parent),
            context: None,
            pages: None,
            intent: None,
            attempt: None,
            raw: None,
            _lease: store._session.retain_for_physical_work(),
            _fee: fee,
        })
    }

    pub(in super::super) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.store, guard.store()) || self.pages.is_some() {
            return Err(refused());
        }
        self.context = Some(bound_context(guard, self.parent)?);
        self.pages = Some(open_pages(guard.target_directory_data().0)?);
        self.capture_slots(false)?;
        if self.intent.is_none() {
            return Err(refused());
        }
        Ok(())
    }

    /// Classify all three exact names. Cold work can start only when none
    /// exists, even if a torn or reordered directory update lost the intent.
    pub(in super::super) fn capture_cold(
        &mut self,
        guard: &ColdInviteParent<'_>,
    ) -> Result<(), AppError> {
        let (parent, base) = guard.context_data();
        if !std::ptr::eq(self.store, guard.store())
            || self.parent != Some(parent)
            || self.pages.is_some()
        {
            return Err(refused());
        }
        self.context = Some(selected_context(base, self.parent));
        self.pages = Some(open_pages(guard.target())?);
        self.capture_slots(true)?;
        if self.intent.is_none() && (self.attempt.is_some() || self.raw.is_some()) {
            return Err(refused());
        }
        Ok(())
    }

    fn capture_slots(&mut self, for_cover: bool) -> Result<(), AppError> {
        let pages = self.pages.as_ref().ok_or_else(refused)?;
        self.intent = ReadObject::open(&pages.0.join(&*self.names.intent), MAX_FRAME, for_cover)?;
        self.attempt =
            ReadObject::open(&pages.0.join(&*self.names.attempt), FRAME_BYTES, for_cover)?;
        self.raw = ReadObject::open(&pages.0.join(&*self.names.raw), FRAME_BYTES, for_cover)?;
        Ok(())
    }

    /// A second live Current guard revalidates the same handles, bytes and
    /// observed absences after authentication outside all writers.
    pub(in super::super) fn require_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.store, guard.store())
            || self.context != Some(bound_context(guard, self.parent)?)
        {
            return Err(refused());
        }
        self.require_slots()
    }

    pub(in super::super) fn require_current_cold(
        &mut self,
        guard: &ColdInviteParent<'_>,
    ) -> Result<(), AppError> {
        let (parent, base) = guard.context_data();
        if !std::ptr::eq(self.store, guard.store())
            || self.parent != Some(parent)
            || self.context != Some(selected_context(base, self.parent))
        {
            return Err(refused());
        }
        self.require_slots()
    }

    pub(in super::super) fn cover_current_cold(
        &mut self,
        guard: &ColdInviteParent<'_>,
    ) -> Result<(), AppError> {
        self.require_current_cold(guard)?;
        for object in [&self.intent, &self.attempt, &self.raw] {
            object
                .as_ref()
                .ok_or_else(refused)?
                .file
                .sync_all()
                .map_err(io)?;
        }
        crate::store::persistence::sync_directory(&self.pages.as_ref().ok_or_else(refused)?.0)
            .map_err(io)?;
        self.require_current_cold(guard)
    }

    fn require_slots(&mut self) -> Result<(), AppError> {
        let pages = self.pages.as_ref().ok_or_else(refused)?;
        require_pages(pages)?;
        require_slot(&pages.0.join(&*self.names.intent), self.intent.as_mut())?;
        require_slot(&pages.0.join(&*self.names.attempt), self.attempt.as_mut())?;
        require_slot(&pages.0.join(&*self.names.raw), self.raw.as_mut())?;
        require_pages(pages)
    }

    pub(in super::super) fn authenticate(
        &self,
        request: V2FounderInviteRequestData,
        route: &[u8],
        genesis: V2GenesisSeedData<'_>,
    ) -> Result<NativeTextInviteRecovery, AppError> {
        let (intent, signed) = self.recover(route, genesis.into())?;
        intent.result(request, route)?;
        Ok(match signed {
            Some(signed) => NativeTextInviteRecovery::SignatureRecovered {
                binding_id: signed.id_data(),
                expires_at_ms: intent.expires_at_ms,
            },
            None if self.attempt.is_some() => NativeTextInviteRecovery::KeyAttemptHeld,
            None => NativeTextInviteRecovery::IntentOnly,
        })
    }

    pub(in super::super) fn require_received_invite(
        &self,
        incoming: &SignedInviteBindingData,
        route: &[u8],
        genesis: V2GenesisSeedData<'_>,
    ) -> Result<(), AppError> {
        let (_, signed) = self.recover(route, genesis.into())?;
        let signed = signed.ok_or_else(refused)?;
        if incoming.encode() != signed.encode() {
            return Err(refused());
        }
        Ok(())
    }

    /// Return only a fully authenticated, already covered signature or a
    /// complete absence. Incomplete states cannot seed a cold signer.
    pub(in super::super) fn recover_cold(
        &self,
        request: V2FounderInviteRequestData,
        route: &[u8],
        source: &OpenedBootstrapSourceData,
    ) -> Result<Option<SignedInviteBindingData>, AppError> {
        if self.intent.is_none() && self.attempt.is_none() && self.raw.is_none() {
            self.pages.as_ref().ok_or_else(refused)?;
            return Ok(None);
        }
        let original = source.genesis_public_data();
        let genesis = InviteGenesis {
            context: original.context_data(),
            profile: original.profile_data(),
            admission: original.admission_data(),
            header: original.header_data(),
        };
        let (intent, signed) = self.recover(route, genesis)?;
        intent.result(request, route)?;
        signed.map(Some).ok_or_else(refused)
    }

    fn recover(
        &self,
        route: &[u8],
        genesis: InviteGenesis<'_>,
    ) -> Result<(Intent, Option<SignedInviteBindingData>), AppError> {
        let context = self.context.ok_or_else(refused)?;
        let intent_frame = self.intent.as_ref().ok_or_else(refused)?;
        let key = InviteIntentJournal::key(self.store)?;
        let sealed = crate::store::unframe(&intent_frame.frame)?;
        let plain = Zeroizing::new(unseal(&key, &sealed)?);
        let intent = decode(&plain, context)?;
        if intent.route != route {
            return Err(refused());
        }
        let spec = InviteBindingSpecData::decode_signing_bytes_data(&intent.preimage)
            .map_err(|_| refused())?;
        let expected = genesis.context.spec_data();
        if spec.anchor.group.as_slice() != expected.group
            || spec.anchor.founder != expected.signer
            || spec.inviter != expected.signer
            || spec.anchor.profile != *genesis.profile.id().as_bytes()
            || spec.anchor.admission != *genesis.admission.id().as_bytes()
            || spec.anchor.tenure != *genesis.profile.tenure().as_bytes()
            || spec.anchor.header != *genesis.header.id().as_bytes()
            || spec.anchor.state != *genesis.header.spec().state.as_bytes()
        {
            return Err(refused());
        }
        let first = intent_frame.commitment();
        let Some(attempt) = self.attempt.as_ref() else {
            return if self.raw.is_some() {
                Err(refused())
            } else {
                Ok((intent, None))
            };
        };
        let _ = open_stage(self.store, &attempt.frame, context, first, None)?;
        let Some(raw) = self.raw.as_ref() else {
            return Ok((intent, None));
        };
        let signature = open_stage(
            self.store,
            &raw.frame,
            context,
            first,
            Some(attempt.commitment()),
        )?
        .ok_or_else(refused)?;
        // Materialization uses only cached bytes and verifies the signature.
        let signed = SignedInviteBindingData::sign_using(spec, |bytes| {
            if bytes != intent.preimage.as_slice() {
                return Err(ChatError::Malformed);
            }
            Ok(signature)
        })
        .map_err(|_| refused())?;
        Ok((intent, Some(signed)))
    }
}

struct ReadObject {
    file: File,
    id: SessionId,
    frame: Zeroizing<Vec<u8>>,
}

impl ReadObject {
    fn open(path: &Path, cap: usize, for_cover: bool) -> Result<Option<Self>, AppError> {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
            Ok(m) if !regular(&m) || m.len() <= 40 || m.len() > cap as u64 => return Err(refused()),
            Ok(_) => {}
        }
        let mut file = open_held(path, for_cover)?;
        let id = session_id(&file).map_err(io)?;
        named(path, &file, id)?;
        let frame = read_capped(&mut file, cap)?;
        if frame.len() <= 40 {
            return Err(refused());
        }
        named(path, &file, id)?;
        Ok(Some(Self { file, id, frame }))
    }

    fn commitment(&self) -> [u8; 32] {
        *blake3::hash(&self.frame).as_bytes()
    }

    fn require_exact(&mut self, path: &Path) -> Result<(), AppError> {
        named(path, &self.file, self.id)?;
        if read_capped(&mut self.file, self.frame.len())?.as_slice() != self.frame.as_slice() {
            return Err(refused());
        }
        named(path, &self.file, self.id)
    }
}

fn require_slot(path: &Path, record: Option<&mut ReadObject>) -> Result<(), AppError> {
    if let Some(record) = record {
        return record.require_exact(path);
    }
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused()),
    }
}

#[cfg(target_os = "linux")]
fn open_held(path: &Path, for_cover: bool) -> Result<File, AppError> {
    use rustix::fs::{open, Mode, OFlags};
    open(
        path,
        (if for_cover {
            OFlags::RDWR
        } else {
            OFlags::RDONLY
        }) | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(io)
}

#[cfg(windows)]
fn open_held(path: &Path, for_cover: bool) -> Result<File, AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
    OpenOptions::new()
        .read(true)
        .write(for_cover)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(io)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_held(_path: &Path, _for_cover: bool) -> Result<File, AppError> {
    Err(refused())
}
