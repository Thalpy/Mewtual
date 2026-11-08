//! Actual first empty operational promotion, paired with fresh Current/lane admissions.
//!
//! Declared with the concrete owning host and its affine Current/lane admissions. The
//! same candidate frames survive every refusal. The finite worker closes every new workspace
//! File before releasing its actual physical-session lease; dormant debt is bytes/facts only.
use super::archive::{TextArchiveData, TextArchiveOperationData, TextArchiveSpecData};
use super::capture::{canonical_source_context_digest_data, original_context};
use super::common::{refused, TextHeadIdData};
use super::context::TextContextData;
use super::control::PreparedTextControlFrameData;
use super::head::{TextHeadData, TextParentData, TextPendingData, TextRootData};
use super::io::{archive_child, fixed_child, read_control, require_named, MAX_TEXT_PATH};
use super::resources::{
    V2TextReadResources, V2TextWorkResources, TEXT_READ_METADATA_FEE, TEXT_WORK_BUFFER_FEE,
};
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextPromotionCommit, AdmittedV2TextPromotionStage, V2TextCurrentGroupGuard,
    V2_TEXT_PROMOTION_ADMISSION_LAYOUT_BYTES,
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_replication::chat::acceptance_tree::TreeContext;
use catcoms_replication::chat::cold_index::{
    encode_leaf_data, DirectoryData, DirectoryTreeData, IndexContextData, IndexScopeData,
    LeafRowData, NodePlacementData, NodeRefData, NodeSummaryData, ValueData,
};
use catcoms_replication::chat::cold_manifest::ArchiveHeadId;
use catcoms_replication::chat::owner_header::SignedOwnerHeader;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::kept::SessionId;
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use zeroize::Zeroizing;

mod classify;
mod exact_repair;
mod recovery;
mod scratch;
pub(super) mod seed;
mod work;
pub(in crate::store) use classify::V2TextPromotionRecoveryClassData;
pub(in crate::store) use exact_repair::{
    CapturedV2TextPromotionSeedRepair, PreparedV2TextPromotionSeedRepair,
};
pub(in crate::store) use recovery::V2TextPromotionRecoveryResources;
pub(in crate::store) use scratch::{
    CapturedV2TextPromotionScratch, PreparedV2TextPromotionScratch,
};
#[cfg(all(test, windows))]
pub(in crate::store) use work::V2TextPromotionTestCut;

const FULL_CREATE_RESERVED_BYTES: u128 = 1_614_671;
const FULL_CREATE_RESERVED_OBJECTS: u128 = 101;
/// Fixed mutable-control/original-lineage capacity is separate from immutable history quota.
pub(in crate::store) const V2_TEXT_FIXED_CONTROL_BYTES: u128 = 4_280_640;
pub(in crate::store) const V2_TEXT_FIXED_CONTROL_OBJECTS: u128 = 8;

/// Trusted HOST configuration, not a source grant, a memory payment or a WC refund token.
/// No implicit/unbounded default. Exact selected counters and admitted remaining work must
/// be restored from authenticated O/WAL; a new mount cannot reset this prospective budget.
#[derive(Clone, Copy, Debug)]
pub(in crate::store) struct V2TextQuotaPolicy {
    immutable_bytes: u128,
    immutable_objects: u128,
}
impl V2TextQuotaPolicy {
    pub(in crate::store) fn new(bytes: u128, objects: u128) -> Result<Self, AppError> {
        if bytes < FULL_CREATE_RESERVED_BYTES || objects < FULL_CREATE_RESERVED_OBJECTS {
            return Err(refused());
        }
        // Fixed controls/original-lineage inodes remain additional trusted host capacity;
        // reject a limit whose total accounting cannot even represent that mandatory class.
        bytes
            .checked_add(V2_TEXT_FIXED_CONTROL_BYTES)
            .ok_or_else(refused)?;
        objects
            .checked_add(V2_TEXT_FIXED_CONTROL_OBJECTS)
            .ok_or_else(refused)?;
        Ok(Self {
            immutable_bytes: bytes,
            immutable_objects: objects,
        })
    }
    pub(super) fn require_initial_capacity(
        &self,
        bytes: u128,
        objects: u128,
    ) -> Result<(), AppError> {
        if bytes
            .checked_add(FULL_CREATE_RESERVED_BYTES)
            .filter(|n| *n <= self.immutable_bytes)
            .is_none()
            || objects
                .checked_add(FULL_CREATE_RESERVED_OBJECTS)
                .filter(|n| *n <= self.immutable_objects)
                .is_none()
        {
            return Err(refused());
        }
        Ok(())
    }
}

/// Paid original genesis read only. The real original settled inventory and exact retained
/// genesis ciphertext are selected under Current; no operational Create has run at this cut.
pub(in crate::store) struct PreparedV2TextPromotionCapture {
    captured: Option<CapturedGenesisParent>,
    resources: V2TextReadResources,
    work: Option<V2TextWorkResources>,
}
struct CapturedGenesisParent {
    context: TextContextData,
    original_head: Zeroizing<Vec<u8>>,
    header: SignedOwnerHeader,
    image: Zeroizing<Vec<u8>>,
    manifest: catcoms_storage::private_chat_page::PrivateChatPageId,
    target: PathBuf,
    target_id: SessionId,
}

pub(in crate::store) const V2_TEXT_PROMOTION_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<PreparedV2TextPromotionCapture>()
        + 2 * std::mem::size_of::<PreparedV2TextPromotion>()
        + 4 * std::mem::size_of::<ReadyV2TextPromotionSelection>()
        + 4 * std::mem::size_of::<SavedV2TextPromotion>()
        + V2_TEXT_PROMOTION_ADMISSION_LAYOUT_BYTES
        + 40 * (MAX_TEXT_PATH + 256)
        + 8 * 32768
        + 8 * 16470
        + 256 * 1024;
const _: () = assert!(V2_TEXT_PROMOTION_LAYOUT_BYTES <= TEXT_WORK_BUFFER_FEE);

impl PreparedV2TextPromotionCapture {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<Self, AppError> {
        // This independently pays the audited strict first-inventory traversal and concrete
        // candidate/worker layout, rather than silently enlarging the smaller read loan.
        let resources = V2TextReadResources::prepare_paid(store, source)?;
        let work = V2TextWorkResources::prepare_paid(store, source, &resources)?;
        Ok(Self {
            captured: None,
            resources,
            work: Some(work),
        })
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.resources
    }
    pub(in crate::store) fn capture_genesis_parent(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.work
            .as_ref()
            .ok_or_else(refused)?
            .validate_prepaid_account()?;
        guard.validate_original_namespace()?;
        work::require_original_genesis_inventory(guard)?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        let context = original_context(guard, &key)?;
        let (_, original_head, _) = guard.original_genesis_context_data()?;
        let (target, _, target_id) = guard.target_directory_data();
        let head_path = fixed_child(target, "head.bin")?;
        let head = read_control(
            &head_path,
            crate::store::critical_namespaces::archive_current_codec::MAX_HEAD_FRAME,
        )?;
        if head.frame.as_slice() != original_head {
            return Err(refused());
        }
        if let Some(previous) = &self.captured {
            if previous.context != context
                || previous.original_head.as_slice() != original_head
                || previous.target != target
                || previous.target_id != target_id
            {
                return Err(refused());
            }
        } else {
            let public = guard.source().data();
            let mut target_owned = PathBuf::with_capacity(target.as_os_str().len());
            target_owned.push(target);
            if target_owned.capacity() > MAX_TEXT_PATH {
                return Err(refused());
            }
            self.captured = Some(CapturedGenesisParent {
                context,
                original_head: Zeroizing::new(original_head.to_vec()),
                header: public.header.clone(),
                image: Zeroizing::new(public.state.to_vec()),
                manifest: public.context.spec_data().manifest,
                target: target_owned,
                target_id,
            });
        }
        // Cache the actual immutable selection before its fallible final proof. Inventory
        // work precedes the last source/ID3/name chain; only final names/negatives follow it.
        work::require_original_genesis_inventory(guard)?;
        guard.validate_original_namespace()?;
        require_named(&head_path, &head)?;
        guard.validate_source_only()
    }
    /// No sign/seal/IO. The same captured parent and work payment move once into the host's
    /// outside slot; every fallible check precedes the unique transfer.
    pub(in crate::store) fn take_promotion_into(
        &mut self,
        policy: V2TextQuotaPolicy,
        slot: &mut Option<PreparedV2TextPromotion>,
    ) -> Result<(), AppError> {
        if slot.is_some() || self.captured.is_none() || self.work.is_none() {
            return Err(refused());
        }
        self.work
            .as_ref()
            .ok_or_else(refused)?
            .validate_prepaid_account()?;
        // Allocation is independently prepaid and occurs outside every global writer.
        let identity = Arc::new(());
        *slot = Some(PreparedV2TextPromotion {
            captured: self.captured.take().expect("checked actual capture"),
            identity,
            policy,
            archive: None,
            directory: None,
            head: None,
            seed: None,
            slots: work::PromotionSlots::default(),
            ready: None,
            saved: None,
            #[cfg(all(test, windows))]
            facts_drop_notice: None,
            #[cfg(test)]
            hooks: work::PromotionTestHooks::default(),
            resources: self.resources.clone(),
            work: Arc::new(self.work.take().expect("checked original work payment")),
        });
        Ok(())
    }
}

/// The single unique mutable prepared promotion; complete ciphertexts are cached immediately
/// after their first producer. Retained state has no Store/key/source/SDK or workspace File.
pub(in crate::store) struct PreparedV2TextPromotion {
    captured: CapturedGenesisParent,
    identity: Arc<()>,
    policy: V2TextQuotaPolicy,
    archive: Option<PreparedTextControlFrameData>,
    directory: Option<PreparedPrivateChatPage>,
    head: Option<PreparedTextControlFrameData>,
    seed: Option<PreparedTextControlFrameData>,
    slots: work::PromotionSlots,
    ready: Option<ReadyV2TextPromotionSelection>,
    saved: Option<Arc<SavedV2TextPromotion>>,
    #[cfg(all(test, windows))]
    facts_drop_notice: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
    #[cfg(test)]
    hooks: work::PromotionTestHooks,
    resources: V2TextReadResources,
    // LAST: all candidate buffers and any cached historical/negative tails drain first.
    work: Arc<V2TextWorkResources>,
}

/// Exact-plan identity only; independently paid tiny weak tail, no source liveness, renewal,
/// IO, Store/SDK or physical lease. An already admitted finite effect remains identifiable
/// after cancellation, so comparison intentionally does not turn owner death into a new grant.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextPromotionObservation {
    identity: Weak<()>,
    resources: V2TextReadResources,
}
impl OriginalV2TextPromotionObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.resources.require_same_original(&other.resources)?;
        if !self.identity.ptr_eq(&other.identity) {
            return Err(refused());
        }
        Ok(())
    }
}

/// Genuine completed immutable staging outcome. It owns fixed original frames/facts ONLY;
/// actual Current/registered-lane/selected-genesis proof must admit the separate O mutation.
pub(in crate::store) struct ReadyV2TextPromotionSelection {
    original: OriginalV2TextPromotionObservation,
    facts: Arc<work::PromotionReadyFacts>,
    // Active immutable staging/commit overlap keeps the SAME work payment. This does not
    // move into Saved/negative tails; their bounded fixed facts retain only the read loan.
    work: Arc<V2TextWorkResources>,
}

/// Actual O_G file/parent barrier outcome, cached before the caller's final Current proof.
/// Historical metadata does not retain the replaced operational control FD or a lease.
pub(in crate::store) struct SavedV2TextPromotion {
    head: TextHeadIdData,
    facts: Arc<work::PromotionReadyFacts>,
    // LAST: a sole escaping Saved must destroy its complete facts/frames BEFORE the
    // independently retained1MiB read loan can refund. Prepared/Ready may already be gone.
    original: OriginalV2TextPromotionObservation,
}

impl PreparedV2TextPromotion {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_fault_for_test(&mut self, point: V2TextPromotionTestCut) {
        self.hooks.fault = Some(point);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_work_pause_for_test(
        &mut self,
        point: V2TextPromotionTestCut,
        callback: Box<dyn FnOnce() + Send>,
    ) {
        self.hooks.pause = Some((point, callback));
    }
    /// Observe actual frozen-facts destruction against this fixture's EXACT read debit.
    /// The observer wrapper is installed LAST in the real shared facts, so its Drop runs
    /// after all retained frame/path fields, not at the beginning of the outer Saved drop.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn observe_saved_facts_drop_for_test(
        &mut self,
        facts_dropped: Arc<std::sync::atomic::AtomicBool>,
        exact_fee_dropped: Arc<std::sync::atomic::AtomicBool>,
    ) {
        assert!(self.ready.is_none() && self.saved.is_none());
        self.facts_drop_notice = Some((facts_dropped, exact_fee_dropped));
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextPromotionObservation {
        OriginalV2TextPromotionObservation {
            identity: Arc::downgrade(&self.identity),
            resources: self.resources.clone(),
        }
    }
    /// Fixed pure-data production only, outside writers. No existing complete frame is ever
    /// resealed on retry, including when the original source dies just after a producer cut.
    pub(in crate::store) fn prepare_candidates(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.work.validate_prepaid_account()?;
        self.resources.validate_original(store, source)?;
        let source_data = source.data();
        let source_context =
            crate::store::critical_namespaces::archive_bootstrap::founder_codec::encode_context(
                source_data.context,
            )?;
        if source_data.source != self.captured.context.spec_data().source
            || canonical_source_context_digest_data(&source_context)?
                != self.captured.context.spec_data().source_context_digest
            || source_data.header.encode() != self.captured.header.encode()
            || source_data.state != self.captured.image.as_slice()
        {
            return Err(refused());
        }
        self.resources
            .require_bound_original_negative(store, source)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        if self.archive.is_none() {
            let archive = TextArchiveData::from_data(TextArchiveSpecData {
                context: self.captured.context.id_data()?,
                manifest: self.captured.manifest,
                manifest_sequence: 1,
                header: &self.captured.header,
                image: &self.captured.image,
                previous: None,
                operation: TextArchiveOperationData::Genesis,
            })?;
            self.archive = Some(archive.prepare_frame_data(&key, rng)?);
            self.resources
                .require_bound_original_negative(store, source)?;
        }
        let context = self.captured.context.spec_data();
        let archive = self.archive.as_ref().ok_or_else(refused)?;
        let archive_id = ArchiveHeadId::from_bytes(*archive.id_data().as_bytes());
        if self.directory.is_none() {
            let source_data = source.data();
            let directory = DirectoryData {
                tree: DirectoryTreeData::Empty,
                accepted_leaf_count: 0,
                unique_action_count: 0,
                accepted_root: TreeContext {
                    profile: context.profile,
                    admission: source_data.admission.admission_id(),
                }
                .empty_hash(),
                own_tip_header: self.captured.header.id(),
                tip_body: context.admission,
                prefix_manifest: self.captured.manifest,
                prefix_sequence: 1,
                prefix_head: archive_id,
            };
            let plain = encode_leaf_data(
                IndexContextData {
                    vault: context.layout.vault,
                    group: context.group,
                    profile: context.profile,
                    scope: IndexScopeData::Directory,
                },
                &[LeafRowData {
                    key: *context.admission.as_bytes(),
                    value: ValueData::Directory(directory),
                }],
                NodePlacementData::Root,
            )
            .map_err(|_| refused())?;
            self.resources
                .require_bound_original_negative(store, source)?;
            self.directory = Some(PreparedPrivateChatPage::prepare(&key, &plain, rng)?);
            self.resources
                .require_bound_original_negative(store, source)?;
        }
        let directory = self.directory.as_ref().ok_or_else(refused)?;
        let bytes = archive
            .frame_data()
            .len()
            .checked_add(directory.as_bytes().len())
            .ok_or_else(refused)? as u128;
        self.policy.require_initial_capacity(bytes, 2)?;
        if self.head.is_none() {
            let head = TextHeadData {
                context: self.captured.context.clone(),
                generation: 1,
                previous: TextParentData::Genesis(context.genesis_parent),
                archive: archive_id,
                directory: TextRootData::Present(NodeRefData {
                    page: directory.id(),
                    summary: NodeSummaryData {
                        height: 0,
                        keys: 1,
                        min: *context.admission.as_bytes(),
                        max: *context.admission.as_bytes(),
                    },
                }),
                tokens: TextRootData::Empty,
                publications: TextRootData::Empty,
                queue_front: 0,
                queue_next: 0,
                pending: TextPendingData::None,
                live_bytes: bytes,
                live_objects: 2,
                selection_attempt: 1,
            };
            self.resources
                .require_bound_original_negative(store, source)?;
            self.head = Some(head.prepare_frame_data(&key, rng)?);
            self.resources
                .require_bound_original_negative(store, source)?;
        }
        if self.seed.is_none() {
            self.resources
                .require_bound_original_negative(store, source)?;
            self.seed = Some(seed::prepare(self, &key, rng)?);
            self.resources
                .require_bound_original_negative(store, source)?;
        }
        // Late negatives leave every already-produced byte in this same prepared owner.
        self.resources.validate_original(store, source)
    }
    pub(in crate::store) fn ready_selection(
        &self,
    ) -> Result<&ReadyV2TextPromotionSelection, AppError> {
        self.ready.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn saved_promotion(&self) -> Result<&Arc<SavedV2TextPromotion>, AppError> {
        self.saved.as_ref().ok_or_else(refused)
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
redacted_debug!(
    PreparedV2TextPromotionCapture,
    PreparedV2TextPromotion,
    OriginalV2TextPromotionObservation,
    ReadyV2TextPromotionSelection,
    SavedV2TextPromotion,
);
