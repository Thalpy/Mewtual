//! Shared immutable recovery DATA and bounded held reads, not effect admissions.
//!
//! Kept undeclared until the distinct seed-repair/scratch callers and Factory markers are
//! composed. A matching native ID is never an original-incarnation grant. Actual mutation
//! must consume the new marker derived from an authenticated journal or classified scratch.
use super::super::archive::TextArchiveOperationData;
use super::super::control::{open_control_data, TextControlIdData, TextControlKindData};
use super::*;
use crate::store::critical_namespaces::archive_current_codec as genesis_codec;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use catcoms_replication::chat::cold_index::{NodeFrame, NodeKindData};
use catcoms_storage::kept::{session_at, session_id, SessionAt};
use catcoms_storage::private_chat_page::{open_private_chat_page, PrivateChatPageId};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) const RECOVERY_FEE_BYTES: usize = 4 * 1024 * 1024;

struct RecoveryPayment {
    pool: CriticalCatalogReadPool,
    // LAST: even an unstarted active-capture wrapper remains charged through cleanup.
    fee: CatalogReadReservation,
}
/// Resource-only payment loan. It contains no bulk frames, File, Store/key/SDK/source or
/// permission. Active capture owns this independently even BEFORE any source is cached.
#[derive(Clone)]
pub(in crate::store) struct V2TextPromotionRecoveryResources {
    body: Arc<RecoveryPayment>,
}
impl V2TextPromotionRecoveryResources {
    pub(in crate::store) fn validate_account(&self) -> Result<(), AppError> {
        self.body.fee.validate(&self.body.pool, RECOVERY_FEE_BYTES)
    }
}
impl std::fmt::Debug for V2TextPromotionRecoveryResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2TextPromotionRecoveryResources")
            .finish_non_exhaustive()
    }
}

/// Independent recovery metadata/buffer class. None of these copies/read/AEAD/native-path
/// overlaps borrows credit from original16MiB/source SDK/read1MiB or immutable disk quota.
pub(super) const PROMOTION_RECOVERY_LAYOUT_BYTES: usize =
    4 * std::mem::size_of::<RecoveryPlan>()
        + std::mem::size_of::<RecoveryPayment>()
        + 16 * std::mem::size_of::<V2TextPromotionRecoveryResources>()
        + 4 * 64
        + 2 * std::mem::size_of::<super::exact_repair::PreparedV2TextPromotionSeedRepair>()
        + 2 * std::mem::size_of::<super::exact_repair::CapturedV2TextPromotionSeedRepair>()
        + 2 * std::mem::size_of::<super::scratch::PreparedV2TextPromotionScratch>()
        + 2 * std::mem::size_of::<super::scratch::CapturedV2TextPromotionScratch>()
        + 32 * std::mem::size_of::<HeldRecoveryControl>()
        + 32 * (MAX_TEXT_PATH + 256)
        + 16 * (32768 + 40)
        + 8 * 16470
        + crate::store::critical_namespaces::current_layout::V2_TEXT_PROMOTION_REPAIR_ADMISSION_LAYOUT_BYTES
        // Root's ONE owning recovery enum (role + active bundle + admitted marker and
        // simultaneous transition tags/copies). Root separately asserts its actual <=8KiB.
        + 8 * 1024
        + 256 * 1024;
const _: () = assert!(PROMOTION_RECOVERY_LAYOUT_BYTES <= RECOVERY_FEE_BYTES);

/// Only a real opaque prepared promotion can supply these complete original frames. The
/// plan contains no source/SDK/Store/key/session or caller-picked filename. All copies share
/// one NEW payment, whose concrete complete invoice is checked before this allocation.
pub(super) struct RecoveryPlan {
    pub(super) original: OriginalV2TextPromotionObservation,
    pub(super) context: TextContextData,
    pub(super) target: PathBuf,
    pub(super) target_id: SessionId,
    pub(super) paths: RecoveryPaths,
    pub(super) original_head: Zeroizing<Vec<u8>>,
    pub(super) seed: Zeroizing<Vec<u8>>,
    pub(super) archive: Zeroizing<Vec<u8>>,
    pub(super) archive_id: [u8; 32],
    pub(super) directory: Zeroizing<Vec<u8>>,
    pub(super) directory_id: PrivateChatPageId,
    pub(super) head: Zeroizing<Vec<u8>>,
    pub(super) policy: V2TextQuotaPolicy,
    // Resource-only keepalive for returning the existing Ready family to its original
    // preparation. NONE of the independent recovery invoice borrows this16MiB credit.
    pub(super) original_work: Arc<V2TextWorkResources>,
    // LAST: all copied frames/path/identity metadata precede this independent actual debit.
    pub(super) payment: V2TextPromotionRecoveryResources,
}
impl RecoveryPlan {
    pub(super) fn from_prepared(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        prepared: &PreparedV2TextPromotion,
    ) -> Result<Arc<Self>, AppError> {
        prepared.resources.validate_original(store, source)?;
        if prepared.saved.is_some() {
            return Err(refused());
        }
        let seed = prepared.seed.as_ref().ok_or_else(refused)?;
        let archive = prepared.archive.as_ref().ok_or_else(refused)?;
        let directory = prepared.directory.as_ref().ok_or_else(refused)?;
        let head = prepared.head.as_ref().ok_or_else(refused)?;
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let fee = pool.try_reserve(RECOVERY_FEE_BYTES)?;
        fee.validate(&pool, RECOVERY_FEE_BYTES)?;
        // Reservation may WAIT. Repeat actual original observations before allocating/copying;
        // acquiring this independent resource class renews no signing or execution authority.
        prepared.resources.validate_original(store, source)?;
        let payment = V2TextPromotionRecoveryResources {
            body: Arc::new(RecoveryPayment { pool, fee }),
        };
        let mut target = PathBuf::with_capacity(prepared.captured.target.as_os_str().len());
        target.push(&prepared.captured.target);
        if target.capacity() > MAX_TEXT_PATH {
            return Err(refused());
        }
        let paths = RecoveryPaths::prepare(
            &target,
            archive.id_data().as_bytes(),
            directory.id().as_bytes(),
        )?;
        Ok(Arc::new(Self {
            original: prepared.observe_original(),
            context: prepared.captured.context.clone(),
            target,
            target_id: prepared.captured.target_id,
            paths,
            original_head: Zeroizing::new(prepared.captured.original_head.to_vec()),
            seed: Zeroizing::new(seed.frame_data().to_vec()),
            archive: Zeroizing::new(archive.frame_data().to_vec()),
            archive_id: *archive.id_data().as_bytes(),
            directory: Zeroizing::new(directory.as_bytes().to_vec()),
            directory_id: directory.id(),
            head: Zeroizing::new(head.frame_data().to_vec()),
            policy: prepared.policy,
            original_work: prepared.work.clone(),
            payment,
        }))
    }
    pub(super) fn resources(&self) -> &V2TextReadResources {
        &self.original.resources
    }
    pub(super) fn require_prepared(
        &self,
        prepared: &PreparedV2TextPromotion,
    ) -> Result<(), AppError> {
        self.original
            .require_same_original(&prepared.observe_original())?;
        if prepared.saved.is_some()
            || prepared.seed.as_ref().ok_or_else(refused)?.frame_data() != self.seed.as_slice()
            || prepared.head.as_ref().ok_or_else(refused)?.frame_data() != self.head.as_slice()
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn validate_account(&self) -> Result<(), AppError> {
        self.payment.validate_account()
    }
    pub(super) fn require_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        self.validate_account()?;
        self.resources()
            .validate_original(guard.store(), guard.source())?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.context {
            return Err(refused());
        }
        let (target, _, id) = guard.target_directory_data();
        if target != self.target || id != self.target_id {
            return Err(refused());
        }
        Ok(())
    }
}

/// All closed paths allocate OUTSIDE AD before any source File is opened. A later final
/// source/ID3 proof is followed only by held-name checks over these existing paths.
pub(super) struct RecoveryPaths {
    pub(super) seed: PathBuf,
    pub(super) seed_stage: PathBuf,
    pub(super) head: PathBuf,
    pub(super) head_stage: PathBuf,
    pub(super) pages: PathBuf,
    pub(super) archive: PathBuf,
    pub(super) directory: PathBuf,
    pub(super) wal: PathBuf,
    pub(super) wal_stage: PathBuf,
}
impl RecoveryPaths {
    fn prepare(target: &Path, archive: &[u8; 32], directory: &[u8; 32]) -> Result<Self, AppError> {
        let pages = fixed_child(target, "text.pages")?;
        Ok(Self {
            seed: fixed_child(target, "text.seed")?,
            seed_stage: fixed_child(target, "text.seed.stage.v1")?,
            head: fixed_child(target, "head.bin")?,
            head_stage: fixed_child(target, "head.stage")?,
            archive: archive_child(&pages, archive)?,
            directory: work::page_child(&pages, directory)?,
            pages,
            wal: fixed_child(target, "text.wal")?,
            wal_stage: fixed_child(target, "text.wal.stage.v1")?,
        })
    }
}

pub(super) fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

/// A complete original read or a bounded zero/partial regular object. Present is explicit:
/// an empty/torn file is never decoded as missing. No key or effect permission is retained.
pub(super) struct HeldRecoveryControl {
    file: Option<File>,
    id: Option<SessionId>,
    raw: Option<Zeroizing<Vec<u8>>>,
    used: usize,
    complete: bool,
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

pub(super) fn require_control_name(
    path: &Path,
    control: &HeldRecoveryControl,
) -> Result<(), AppError> {
    let id = control.id()?;
    if session_id(control.file()?).map_err(io)? != id
        || !matches!(session_at(path, id), SessionAt::Ours)
    {
        return Err(refused());
    }
    Ok(())
}

impl HeldRecoveryControl {
    pub(super) fn empty() -> Self {
        Self {
            file: None,
            id: None,
            raw: None,
            used: 0,
            complete: false,
        }
    }
    pub(super) fn file(&self) -> Result<&File, AppError> {
        self.file.as_ref().ok_or_else(refused)
    }
    pub(super) fn id(&self) -> Result<SessionId, AppError> {
        self.id.ok_or_else(refused)
    }
    pub(super) fn raw(&self) -> Result<&[u8], AppError> {
        if !self.complete {
            return Err(refused());
        }
        Ok(&self.raw.as_ref().ok_or_else(refused)?[..self.used])
    }
    /// This owner is already installed in the affine Source+lease bundle. Cache each File
    /// and buffer BEFORE metadata/seek/read/proof can fail, so no selected File is abandoned
    /// as a local during an error under AD. The outer bundle drains outside all writers.
    pub(super) fn capture(&mut self, path: &Path, maximum: usize) -> Result<(), AppError> {
        if maximum == 0 || maximum > 32768 + 40 {
            return Err(refused());
        }
        if self.complete {
            return self.require_unchanged(path);
        }
        if self.file.is_none() {
            let before = fs::symlink_metadata(path).map_err(io)?;
            if !regular(&before) || before.len() > maximum as u64 {
                return Err(refused());
            }
            self.file = Some(File::open(path).map_err(io)?);
        }
        let metadata = self.file()?.metadata().map_err(io)?;
        if !regular(&metadata) || metadata.len() > maximum as u64 {
            return Err(refused());
        }
        if self.id.is_none() {
            self.id = Some(session_id(self.file()?).map_err(io)?);
        }
        require_control_name(path, self)?;
        let length = metadata.len() as usize;
        if self.raw.is_none() {
            self.raw = Some(Zeroizing::new(vec![0; maximum + 1]));
        }
        let file = self.file.as_mut().ok_or_else(refused)?;
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        let raw = self.raw.as_mut().ok_or_else(refused)?;
        let mut used = 0;
        while used < raw.len() {
            match file.read(&mut raw[used..]) {
                Ok(0) => break,
                Ok(n) => used += n,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        if used != length {
            return Err(refused());
        }
        self.used = used;
        self.complete = true;
        require_control_name(path, self)
    }
    pub(super) fn require_unchanged(&self, path: &Path) -> Result<(), AppError> {
        require_control_name(path, self)?;
        let expected = self.raw()?;
        // Positioned reads use the already-cached actual FD. No duplicate File can be
        // created then stranded/dropped inside a writer when metadata or a read fails.
        let file = self.file()?;
        let metadata = file.metadata().map_err(io)?;
        if !regular(&metadata) || metadata.len() != expected.len() as u64 {
            return Err(refused());
        }
        let mut offset = 0;
        let mut buffer = [0u8; 4096];
        while offset < expected.len() {
            let wanted = buffer.len().min(expected.len() - offset);
            match positioned_read(file, &mut buffer[..wanted], offset as u64) {
                Ok(0) => return Err(refused()),
                Ok(n) => {
                    if buffer[..n] != expected[offset..offset + n] {
                        return Err(refused());
                    }
                    offset += n;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        loop {
            match positioned_read(file, &mut buffer[..1], offset as u64) {
                Ok(0) => break,
                Ok(_) => return Err(refused()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io(error)),
            }
        }
        require_control_name(path, self)
    }
}

pub(super) fn positioned_read(
    file: &File,
    bytes: &mut [u8],
    offset: u64,
) -> std::io::Result<usize> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        file.seek_read(bytes, offset)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_at(bytes, offset)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (file, bytes, offset);
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

pub(super) fn require_absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused()),
    }
}

/// A cached directory read. The active capture stores its real File before native metadata
/// or name checks can fail; its readonly identity does not select a head or authorize IO.
pub(super) struct HeldRecoveryDirectory {
    file: Option<File>,
    id: Option<SessionId>,
}
impl HeldRecoveryDirectory {
    pub(super) fn empty() -> Self {
        Self {
            file: None,
            id: None,
        }
    }
    pub(super) fn file(&self) -> Result<&File, AppError> {
        self.file.as_ref().ok_or_else(refused)
    }
    pub(super) fn id(&self) -> Result<SessionId, AppError> {
        self.id.ok_or_else(refused)
    }
    pub(super) fn capture(&mut self, path: &Path) -> Result<(), AppError> {
        if self.file.is_none() {
            #[cfg(windows)]
            let file = {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
                    FILE_SHARE_READ, FILE_SHARE_WRITE,
                };
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(path)
                    .map_err(io)?
            };
            #[cfg(not(windows))]
            let file = File::open(path).map_err(io)?;
            self.file = Some(file);
        }
        let metadata = self.file()?.metadata().map_err(io)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(refused());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(refused());
            }
        }
        if self.id.is_none() {
            self.id = Some(session_id(self.file()?).map_err(io)?);
        }
        self.require_name(path)
    }
    pub(super) fn require_name(&self, path: &Path) -> Result<(), AppError> {
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            path,
            self.file()?,
            self.id()?,
        )
    }
}

/// Full independently anchored promotion-source verification. No parsed status, valid MAC,
/// matching label or caller-selected image can supply this source by itself. All inner
/// public signatures, old source/pin/genesis binding, O/H/index contexts and exact empty
/// genesis arithmetic are checked before any SeedExactRepair admission can be issued.
pub(super) fn verify_complete_seed(
    plan: &RecoveryPlan,
    guard: &V2TextCurrentGroupGuard<'_, '_>,
    raw_seed: &[u8],
) -> Result<(), AppError> {
    plan.require_guard(guard)?;
    verify_seed_payload(&SeedExpectation::from_plan(plan), guard, raw_seed)
}

/// Borrowed expected DATA from one of two closed genuine owners. No buffers or caller raw
/// snapshot are adopted and no permission is issued by this shared full-verification walk.
struct SeedExpectation<'a> {
    context: &'a TextContextData,
    original_head: &'a [u8],
    seed: &'a [u8],
    archive: &'a [u8],
    archive_id: [u8; 32],
    directory: &'a [u8],
    directory_id: PrivateChatPageId,
    head: &'a [u8],
    policy: V2TextQuotaPolicy,
}
impl<'a> SeedExpectation<'a> {
    fn from_plan(plan: &'a RecoveryPlan) -> Self {
        Self {
            context: &plan.context,
            original_head: &plan.original_head,
            seed: &plan.seed,
            archive: &plan.archive,
            archive_id: plan.archive_id,
            directory: &plan.directory,
            directory_id: plan.directory_id,
            head: &plan.head,
            policy: plan.policy,
        }
    }
    fn from_original(original: &'a PreparedV2TextPromotion) -> Result<Self, AppError> {
        Ok(Self {
            context: &original.captured.context,
            original_head: &original.captured.original_head,
            seed: original.seed.as_ref().ok_or_else(refused)?.frame_data(),
            archive: original.archive.as_ref().ok_or_else(refused)?.frame_data(),
            archive_id: *original
                .archive
                .as_ref()
                .ok_or_else(refused)?
                .id_data()
                .as_bytes(),
            directory: original.directory.as_ref().ok_or_else(refused)?.as_bytes(),
            directory_id: original.directory.as_ref().ok_or_else(refused)?.id(),
            head: original.head.as_ref().ok_or_else(refused)?.frame_data(),
            policy: original.policy,
        })
    }
}
pub(super) fn verify_original_seed_data(
    original: &PreparedV2TextPromotion,
    guard: &V2TextCurrentGroupGuard<'_, '_>,
    raw_seed: &[u8],
) -> Result<(), AppError> {
    original.work.validate_prepaid_account()?;
    original
        .resources
        .require_same_original(guard.resources())?;
    original
        .resources
        .validate_original(guard.store(), guard.source())?;
    let key = Zeroizing::new(guard.store().keys.db_key()?);
    if original_context(guard, &key)? != original.captured.context {
        return Err(refused());
    }
    verify_seed_payload(&SeedExpectation::from_original(original)?, guard, raw_seed)
}
fn verify_seed_payload(
    plan: &SeedExpectation<'_>,
    guard: &V2TextCurrentGroupGuard<'_, '_>,
    raw_seed: &[u8],
) -> Result<(), AppError> {
    let key = Zeroizing::new(guard.store().keys.db_key()?);
    let plain = open_control_data(&key, TextControlKindData::Seed.id_data(raw_seed), raw_seed)?;
    let seed = seed::parse_data(&plain)?;
    if seed.context != *plan.context
        || raw_seed != plan.seed
        || seed.original_head != plan.original_head
        || seed.archive_frame != plan.archive
        || seed.head_frame != plan.head
        || seed.directory_frame != plan.directory
    {
        return Err(refused());
    }
    let (expected, old_head, nonce) = guard.original_genesis_context_data()?;
    let public = guard.source().data();
    if old_head != seed.original_head || nonce != plan.context.spec_data().nonce {
        return Err(refused());
    }
    let pin_frame = guard.original_pin_data().2;
    let pin = genesis_codec::open_pin_data(
        &key,
        expected.layout_data(),
        public.context.spec_data().group,
        pin_frame,
    )?;
    pin.require_source_data(expected)?;
    let genesis = genesis_codec::open_head_data(
        &key,
        expected,
        genesis_codec::V2GroupPinId::of(pin_frame),
        &pin,
        seed.original_head,
    )?;
    genesis.require_original_public_data(
        &pin,
        &genesis_codec::GenesisPublicData {
            profile: public.profile,
            admission: public.admission,
            header: public.header,
            image: public.state,
        },
    )?;
    let head_plain = open_control_data(
        &key,
        TextControlKindData::Head.id_data(seed.head_frame),
        seed.head_frame,
    )?;
    let head = TextHeadData::decode_data(&head_plain)?;
    let archive_id = TextControlKindData::Archive.id_data(seed.archive_frame);
    if head.context != *plan.context
        || head.generation != 1
        || head.selection_attempt != 1
        || head.previous != TextParentData::Genesis(plan.context.spec_data().genesis_parent)
        || head.pending != TextPendingData::None
        || head.tokens != TextRootData::Empty
        || head.publications != TextRootData::Empty
        || head.queue_front != 0
        || head.queue_next != 0
        || head.archive.as_bytes() != archive_id.as_bytes()
        || archive_id.as_bytes() != &plan.archive_id
    {
        return Err(refused());
    }
    let archive_plain = open_control_data(
        &key,
        TextControlIdData::from_data(TextControlKindData::Archive, *head.archive.as_bytes()),
        seed.archive_frame,
    )?;
    let archive = TextArchiveData::decode_data(&archive_plain)?;
    let a = archive.spec_data();
    if a.context != plan.context.id_data()?
        || a.operation != TextArchiveOperationData::Genesis
        || a.previous.is_some()
        || a.manifest_sequence != 1
        || a.manifest != public.context.spec_data().manifest
        || a.header.encode() != public.header.encode()
        || a.image != public.state
    {
        return Err(refused());
    }
    let TextRootData::Present(directory_ref) = head.directory else {
        return Err(refused());
    };
    if directory_ref.page != plan.directory_id {
        return Err(refused());
    }
    let directory_plain = open_private_chat_page(&key, directory_ref.page, seed.directory_frame)?;
    let directory =
        NodeFrame::decode(&directory_plain, NodePlacementData::Root).map_err(|_| refused())?;
    let c = plan.context.spec_data();
    if directory.context()
        != (IndexContextData {
            vault: c.layout.vault,
            group: c.group,
            profile: c.profile,
            scope: IndexScopeData::Directory,
        })
        || directory.summary() != directory_ref.summary
        || directory.kind() != NodeKindData::Leaf
        || directory.row_count() != 1
    {
        return Err(refused());
    }
    directory
        .check_claim_bounds_data(None)
        .map_err(|_| refused())?;
    let row = directory.leaf_row(0).map_err(|_| refused())?;
    let expected_directory = DirectoryData {
        tree: DirectoryTreeData::Empty,
        accepted_leaf_count: 0,
        unique_action_count: 0,
        accepted_root: TreeContext {
            profile: public.profile.id(),
            admission: public.admission.admission_id(),
        }
        .empty_hash(),
        own_tip_header: public.header.id(),
        tip_body: public.admission.id(),
        prefix_manifest: public.context.spec_data().manifest,
        prefix_sequence: 1,
        prefix_head: head.archive,
    };
    if row.key != *public.admission.id().as_bytes()
        || row.value != ValueData::Directory(expected_directory)
        || head.live_bytes != (seed.archive_frame.len() + seed.directory_frame.len()) as u128
        || head.live_objects != 2
    {
        return Err(refused());
    }
    plan.policy
        .require_initial_capacity(head.live_bytes, head.live_objects)
}

impl std::fmt::Debug for RecoveryPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryPlan").finish_non_exhaustive()
    }
}
