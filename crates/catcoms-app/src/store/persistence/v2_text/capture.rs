//! Paid, read-only physical O/H selection from the real original Current group guard.
//!
//! The selected frame is a physical observation, not a verified public prefix, a Saved
//! barrier, a clear transaction, signing permission or authority to publish. In particular
//! a complete S/B WAL/stage may coexist with the observed O. Every effect-capable caller
//! must first use the separate complete-workspace classifier and fresh commit admission.
use super::archive::{TextArchiveData, MAX_ARCHIVE_PLAIN};
use super::common::{refused, TextGenesisParentIdData, TextHeadIdData};
use super::context::{TextContextData, TextContextSpecData, MAX_CONTEXT_PLAIN};
use super::control::{open_control_data, TextControlKindData};
use super::head::{TextHeadData, MAX_TEXT_HEAD_FRAME};
use super::io::{archive_child, fixed_child, read_control, require_named, MAX_TEXT_PATH};
use super::resources::{
    V2TextReadResources, TEXT_READ_METADATA_FEE, TEXT_READ_RESOURCE_LAYOUT_BYTES,
};
use crate::store::critical_namespaces::archive_bootstrap::founder_codec;
use crate::store::critical_namespaces::archive_current_codec as genesis_codec;
use crate::store::critical_namespaces::current_layout::{
    V2TextCurrentGroupGuard, V2_TEXT_CURRENT_GROUP_GUARD_BYTES,
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_storage::kept::SessionId;
use std::fs::File;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

mod founder_anchor;
mod source_context;
pub(in crate::store) use founder_anchor::{
    V2TextFounderAnchorData, V2_TEXT_FOUNDER_ANCHOR_LAYOUT_BYTES,
};
pub(super) use source_context::canonical_source_context_digest_data;

/// Simultaneous capture/validation footprint, independently paid before allocating this
/// owner or waiting on the actual Current guard. Original SDK/read/Scope fees are additional.
/// O/H raw+copied AEAD ciphertext+plaintext and canonical pin/source/context overlaps are
/// distinct. Revalidation opens temporary controls but never retains an old operational FD.
pub(in crate::store) const V2_TEXT_PARENT_CAPTURE_LAYOUT_BYTES: usize = 2
    * (std::mem::size_of::<PreparedV2TextParentCapture>()
        + std::mem::size_of::<SelectedV2TextHead>())
    + TEXT_READ_RESOURCE_LAYOUT_BYTES
    + V2_TEXT_CURRENT_GROUP_GUARD_BYTES
    + 10 * (MAX_TEXT_PATH + 256)
    + 8 * (MAX_CONTEXT_PLAIN + 256)
    + 6 * (MAX_TEXT_HEAD_FRAME + MAX_ARCHIVE_PLAIN + 40)
    + 4 * (genesis_codec::MAX_PIN_FRAME + 692)
    + 20 * std::mem::size_of::<File>()
    + 64 * 1024;
const _: () = assert!(V2_TEXT_PARENT_CAPTURE_LAYOUT_BYTES <= TEXT_READ_METADATA_FEE);

/// Same paid preparation is retained on every refusal. There is no adopting constructor,
/// caller-chosen path/root or SDK getter. This owner contains no File/key/Store/session.
pub(in crate::store) struct PreparedV2TextParentCapture {
    selected: Option<SelectedV2TextHead>,
    // LAST: all frozen selected frames/paths/context drain before this possibly last fee.
    resources: V2TextReadResources,
}
impl PreparedV2TextParentCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<Self, AppError> {
        // The compile-time concrete invoice is checked above before the independent debit.
        // prepare_paid performs real original-source/mount checks on both sides of its wait.
        let resources = V2TextReadResources::prepare_paid(store, source)?;
        Ok(Self {
            selected: None,
            resources,
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.resources
    }
    /// Borrow cached DATA outside writers for independently paid query preparation.
    /// The actual query admission must repeat full current-selection proof.
    pub(in crate::store) fn selected_data(&self) -> Option<&SelectedV2TextHead> {
        self.selected.as_ref()
    }
    /// Cache the exact complete observed O/H before the fallible final namespace proof.
    /// A late failure retains that same physical observation; retry never replaces it with
    /// a newer parent and never converts an IO/decode failure into an empty token result.
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<&SelectedV2TextHead, AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        guard.validate_original_namespace()?;
        if self.selected.is_none() {
            self.selected = Some(read_selected(guard, &self.resources)?);
        }
        let selected = self.selected.as_ref().ok_or_else(refused)?;
        selected.validate_under_current_guard(guard)?;
        Ok(selected)
    }
}

/// Frozen physical facts only. Does not keep an obsolete operational O/WAL inode alive;
/// ephemeral held originals drain at the end of each real guarded comparison. Every later
/// use repeats the SAME source/Current/namespace and exact complete frame selection.
pub(in crate::store) struct SelectedV2TextHead {
    target: PathBuf,
    target_id: SessionId,
    pages: PathBuf,
    pages_id: SessionId,
    head_path: PathBuf,
    archive_path: PathBuf,
    head_frame: Zeroizing<Vec<u8>>,
    archive_frame: Zeroizing<Vec<u8>>,
    head: TextHeadData,
    archive: TextArchiveData,
    resources: V2TextReadResources,
}

fn owned_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().len() > MAX_TEXT_PATH {
        return Err(refused());
    }
    let mut owned = PathBuf::with_capacity(path.as_os_str().len());
    owned.push(path);
    if owned.capacity() > MAX_TEXT_PATH {
        return Err(refused());
    }
    Ok(owned)
}

/// Context labels are derived from independent actual original source/Scope/pin receipts,
/// never chosen from the O being decoded. No native File/read/writer generation enters C.
pub(super) fn original_context(
    guard: &V2TextCurrentGroupGuard<'_, '_>,
    key: &[u8; 32],
) -> Result<TextContextData, AppError> {
    let (expected, genesis_frame, nonce) = guard.original_genesis_context_data()?;
    let public = guard.source().data();
    let source = public.context.spec_data();
    if expected.context != *public.context
        || expected.source_id != public.source
        || nonce != source.scope_nonce
        || expected.context.spec_data().profile != public.profile.id()
        || expected.context.spec_data().admission != public.admission.id()
        || expected.context.spec_data().header != public.header.id()
        || genesis_frame.is_empty()
        || genesis_frame.len() > genesis_codec::MAX_HEAD_FRAME
    {
        return Err(refused());
    }
    let pin_frame = guard.original_pin_data().2;
    let pin = genesis_codec::open_pin_data(key, expected.layout_data(), source.group, pin_frame)?;
    pin.require_source_data(expected)?;
    if pin.profile != *public.profile {
        return Err(refused());
    }
    let canonical_source = founder_codec::encode_context(&expected.context)?;
    TextContextData::from_data(TextContextSpecData {
        layout: expected.layout_data(),
        marker: expected.marker_id,
        terminal: expected.terminal_id,
        source: expected.source_id,
        source_context_digest: source_context::canonical_source_context_digest_data(
            &canonical_source,
        )?,
        profile: public.profile.id(),
        admission: public.admission.id(),
        signer: source.signer,
        nonce,
        pin: genesis_codec::V2GroupPinId::of(pin_frame),
        tenure: public.profile.tenure(),
        genesis_parent: TextGenesisParentIdData::of_data(genesis_frame),
        group: source.group,
    })
}

fn require_archive_context(head: &TextHeadData, archive: &TextArchiveData) -> Result<(), AppError> {
    let context = head.context.spec_data();
    let a = archive.spec_data();
    let public = a.header.spec();
    if a.context != head.context.id_data()?
        || public.profile != context.profile
        || public.group != context.group
        || public.issuer != context.signer
        || public.tenure != context.tenure
    {
        return Err(refused());
    }
    // TextArchiveData already verifies its signature and exact StateFrame/header binding.
    // Complete selected-prefix replay and accepted-index lookup remain separate consumers.
    Ok(())
}

fn read_selected(
    guard: &V2TextCurrentGroupGuard<'_, '_>,
    resources: &V2TextReadResources,
) -> Result<SelectedV2TextHead, AppError> {
    guard.require_text_control_write_idle_for_query()?;
    let context_key = Zeroizing::new(guard.store().keys.db_key()?);
    let expected_context = original_context(guard, &context_key)?;
    let (target, target_file, target_id) = guard.target_directory_data();
    let target_owned = owned_path(target)?;
    let head_path = fixed_child(target, "head.bin")?;
    let pages = fixed_child(target, "text.pages")?;
    let pages_file = crate::store::persistence::open_archive_directory(&pages)?;
    let pages_id = catcoms_storage::kept::session_id(&pages_file)
        .map_err(|error| AppError::Io(error.to_string()))?;
    let head_control = read_control(&head_path, MAX_TEXT_HEAD_FRAME)?;
    let head_plain = open_control_data(
        &context_key,
        TextControlKindData::Head.id_data(&head_control.frame),
        &head_control.frame,
    )?;
    let head = TextHeadData::decode_data(&head_plain)?;
    if head.context != expected_context {
        return Err(refused());
    }
    let archive_path = archive_child(&pages, head.archive.as_bytes())?;
    let archive_control = read_control(&archive_path, MAX_ARCHIVE_PLAIN + 40)?;
    let archive_plain = open_control_data(
        &context_key,
        super::control::TextControlIdData::from_data(
            TextControlKindData::Archive,
            *head.archive.as_bytes(),
        ),
        &archive_control.frame,
    )?;
    let archive = TextArchiveData::decode_data(&archive_plain)?;
    require_archive_context(&head, &archive)?;
    // Key/decode/account waits precede the final source/ID3/name/negative chain. Both held
    // operational control objects remain alive through that final name observation only.
    guard.validate_original_namespace()?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        target,
        target_file,
        target_id,
    )?;
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        &pages,
        &pages_file,
        pages_id,
    )?;
    require_named(&archive_path, &archive_control)?;
    require_named(&head_path, &head_control)?;
    guard.validate_source_only()?;
    // Only infallible moves after the final negative proof. Paths are fully allocated first
    // so a later allocator/validation cannot masquerade as part of that final proof chain.
    Ok(SelectedV2TextHead {
        target: target_owned,
        target_id,
        pages,
        pages_id,
        head_path,
        archive_path,
        head_frame: head_control.frame,
        archive_frame: archive_control.frame,
        head,
        archive,
        resources: resources.clone(),
    })
}

/// Exact borrowed facts from a genuine selected capture. This supplies no File, key,
/// source owner, caller-selected path or permission to read/write a physical object.
pub(super) struct SelectedV2TextQueryParentData<'selected> {
    pub(super) target: &'selected Path,
    pub(super) target_id: SessionId,
    pub(super) pages: &'selected Path,
    pub(super) pages_id: SessionId,
    pub(super) head_path: &'selected Path,
    pub(super) archive_path: &'selected Path,
    pub(super) head_frame: &'selected [u8],
    pub(super) archive_frame: &'selected [u8],
}
impl std::fmt::Debug for SelectedV2TextQueryParentData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectedV2TextQueryParentData")
            .finish_non_exhaustive()
    }
}

impl SelectedV2TextHead {
    /// The new query's actual independent payment must precede any copied DATA. This
    /// immutable loan does not establish that its observed root is still selected.
    pub(super) fn physical_query_data(&self) -> SelectedV2TextQueryParentData<'_> {
        SelectedV2TextQueryParentData {
            target: &self.target,
            target_id: self.target_id,
            pages: &self.pages,
            pages_id: self.pages_id,
            head_path: &self.head_path,
            archive_path: &self.archive_path,
            head_frame: &self.head_frame,
            archive_frame: &self.archive_frame,
        }
    }
    /// The SAME independently paid observation body, never a Current or query permit.
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.resources
    }
    pub(in crate::store) fn validate_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        // No selected control handle may open while an admitted selector writer owns
        // the SAME group's gate, including ephemeral post-result comparisons.
        guard.require_text_control_write_idle_for_query()?;
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.head.context {
            return Err(refused());
        }
        let (target, target_file, target_id) = guard.target_directory_data();
        if target != self.target || target_id != self.target_id {
            return Err(refused());
        }
        let pages_file = crate::store::persistence::open_archive_directory(&self.pages)?;
        let head = read_control(&self.head_path, self.head_frame.len())?;
        let archive = read_control(&self.archive_path, self.archive_frame.len())?;
        if head.frame != self.head_frame || archive.frame != self.archive_frame {
            return Err(refused());
        }
        guard.validate_original_namespace()?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target,
            target_file,
            target_id,
        )?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            &self.pages,
            &pages_file,
            self.pages_id,
        )?;
        require_named(&self.archive_path, &archive)?;
        require_named(&self.head_path, &head)?;
        guard.validate_source_only()
    }
    pub(in crate::store) fn head_data(&self) -> &TextHeadData {
        &self.head
    }
    pub(in crate::store) fn archive_data(&self) -> &TextArchiveData {
        &self.archive
    }
    pub(in crate::store) fn head_id_data(&self) -> TextHeadIdData {
        TextHeadIdData::of_data(&self.head_frame)
    }
}

macro_rules! redacted_debug {
    ($($ty:ident),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_debug!(PreparedV2TextParentCapture, SelectedV2TextHead);
