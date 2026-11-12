//! Exact retained-A candidate DATA. No filesystem, selector, acceptance or publication.
//!
//! A real host first proves its selected request hit/miss and active work ownership. This
//! producer only consumes the original opaque Create packet and builds immutable at-rest
//! pages under a NEW affine work payment. It never repeats the Create signer/exporter.
use super::archive::{TextArchiveData, TextArchiveOperationData, TextArchiveSpecData};
use super::artifacts::TextArtifactRoleData;
use super::capture::SelectedV2TextHead;
use super::common::{refused, TextHeadIdData, TextTransactionIdData};
use super::context::TextContextData;
use super::control::{PreparedTextControlFrameData, TextControlKindData};
use super::request::{TextRequestDigestData, TextRetainIdData};
use super::resources::{V2TextReadResources, V2TextWorkResources, TEXT_WORK_BUFFER_FEE};
use super::retain::{
    cipher_digest, CipherChunkData, RetainedActionData, RetainedActionSpecData,
    TextChunkDescriptorData, MAX_CIPHER_CHUNK_PLAIN, MAX_ENCRYPTED_PAYLOAD_BYTES,
};
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_replication::chat::cold_actions::{
    PreparedActionPages, ACTION_CHUNK_BYTES, MAX_ACTION_CHUNK_PAGE_BYTES,
};
use catcoms_replication::chat::cold_manifest::{
    encode_manifest, ArchiveHeadId, ArchiveOperation, ManifestSpec, PageDescriptor, PageRole,
};
use catcoms_replication::chat::owner_header::SignedOwnerHeader;
use catcoms_replication::chat::Body;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use catcoms_sync::owner_v2::{DetachedV2CreateData, V2CreateRequest, MAX_CREATE_ACTION_BYTES};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use zeroize::Zeroizing;

mod custody;
mod intent;
pub(in crate::store) use custody::PreparedV2RetainOriginalCustody;
pub(in crate::store) use intent::{
    encode_unsigned_intent_data, TextUnsignedIntentBindingsData, TextUnsignedIntentFrameData,
};

struct OriginalPage {
    page: PreparedPrivateChatPage,
    plain_bytes: u32,
}

struct RetainParentData {
    head: TextHeadIdData,
    archive: ArchiveHeadId,
    manifest: catcoms_storage::private_chat_page::PrivateChatPageId,
    manifest_sequence: u64,
    header: SignedOwnerHeader,
    image: Zeroizing<Vec<u8>>,
}

// The complete A/I/S and next-unsigned owning metadata peak measures 1,145,090
// bytes on the Windows test target. Reserve this whole independent class BEFORE
// any producer/copy; SDK, model, read, completed-parent and work fees remain
// additional. Every concrete layout assertion below remains authoritative on
// other targets and after future type changes. The global pool is unchanged.
pub(in crate::store) const TEXT_TRANSACTION_METADATA_FEE: usize = 1280 * 1024;
#[cfg(all(test, windows))]
thread_local! {
    // Installed only around the actual constructor, before its paid body is shared.
    static NEXT_TRANSACTION_FEE_NOTICE: std::cell::RefCell<Option<Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
}
struct TransactionMetadataPayment {
    workspace_busy: AtomicBool,
    pool: CriticalCatalogReadPool,
    // LAST: a new independent payment, never Selected-read or SDK-producer credit.
    fee: CatalogReadReservation,
}
#[derive(Clone)]
pub(in crate::store) struct V2TextTransactionMetadataResources {
    body: Arc<TransactionMetadataPayment>,
}
impl V2TextTransactionMetadataResources {
    /// Observe the exact new transaction debit, not another resource class's refund.
    /// The scoped reset also runs on panic, preventing leakage into a later fixture.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn with_fee_drop_notice_for_test<T>(
        notice: Arc<AtomicBool>,
        prepare: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                NEXT_TRANSACTION_FEE_NOTICE.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        NEXT_TRANSACTION_FEE_NOTICE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "nested exact transaction-fee observation"
            );
            *slot.borrow_mut() = Some(notice);
        });
        let _reset = Reset;
        let output = prepare();
        NEXT_TRANSACTION_FEE_NOTICE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "fixture did not acquire its actual transaction debit"
            );
        });
        output
    }
    pub(in crate::store) fn validate_prepaid_account(&self) -> Result<(), AppError> {
        self.body
            .fee
            .validate(&self.body.pool, TEXT_TRANSACTION_METADATA_FEE)
    }
}

/// Exact-plan association only. Its NEW independently paid transaction metadata tail
/// cannot renew the16MiB work, a Source, a Current writer or a signing grant.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2RetainObservation {
    metadata: V2TextTransactionMetadataResources,
}
impl OriginalV2RetainObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.metadata.body, &other.metadata.body) {
            return Err(refused());
        }
        Ok(())
    }
}

/// At most ONE actual workspace capture for this original pre-packet producer. Its owner
/// declares this LAST after Source Files/key and physical lease. A separate wrapper cannot
/// create its own counter and reuse the same sixteen-megabyte invoice concurrently.
pub(in crate::store) struct V2TextRetainWorkspaceClaim {
    _work: Arc<V2TextWorkResources>,
    metadata: V2TextTransactionMetadataResources,
}
impl Drop for V2TextRetainWorkspaceClaim {
    fn drop(&mut self) {
        self.metadata
            .body
            .workspace_busy
            .store(false, Ordering::Release);
    }
}

/// Original packet, all cached ciphertexts and their unique payment stay together on
/// every refusal. There is no decoded-packet or caller-selected source constructor.
pub(in crate::store) struct PreparedV2RetainData {
    context: TextContextData,
    token: [u8; 32],
    channel: u128,
    request: TextRequestDigestData,
    transaction: TextTransactionIdData,
    parent: RetainParentData,
    packet: Option<DetachedV2CreateData>,
    original: Option<PreparedActionPages>,
    actions: Vec<OriginalPage>,
    ciphers: Vec<OriginalPage>,
    retain: Option<PreparedTextControlFrameData>,
    manifest: Option<PreparedPrivateChatPage>,
    archive: Option<PreparedTextControlFrameData>,
    // LAST: original packet/pages/contexts drain before the new work payment refunds.
    work: Arc<V2TextWorkResources>,
    // LAST: future small logical/closing tails retain their OWN1MiB after buffers drain.
    metadata: V2TextTransactionMetadataResources,
}

pub(in crate::store) const RETAIN_DATA_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<PreparedV2RetainData>()
        + 2 * std::mem::size_of::<PreparedV2RetainOriginalCustody>()
        + 10 * std::mem::size_of::<OriginalPage>()
        + 8 * MAX_CREATE_ACTION_BYTES
        + 8 * MAX_ENCRYPTED_PAYLOAD_BYTES
        + 8 * MAX_ACTION_CHUNK_PAGE_BYTES
        + 8 * MAX_CIPHER_CHUNK_PLAIN
        + 8 * intent::MAX_UNSIGNED_INTENT_PLAIN
        + 8 * (634 + 676 + 1110 + 1519)
        + 16 * std::mem::size_of::<OriginalV2RetainObservation>()
        + 256 * 1024;
const _: () = assert!(RETAIN_DATA_LAYOUT_BYTES <= TEXT_WORK_BUFFER_FEE);
pub(in crate::store) const TRANSACTION_METADATA_BASE_LAYOUT_BYTES: usize =
    4 * std::mem::size_of::<PreparedV2RetainData>()
        + 32 * std::mem::size_of::<OriginalV2RetainObservation>()
        + 8 * std::mem::size_of::<V2TextRetainWorkspaceClaim>()
        + 8 * std::mem::size_of::<TransactionMetadataPayment>()
        + 8192
        + 32 * 1024;
const _: () = assert!(TRANSACTION_METADATA_BASE_LAYOUT_BYTES <= TEXT_TRANSACTION_METADATA_FEE);

impl PreparedV2RetainData {
    /// Reserve BEFORE context/vector copies. The selected parent supplies exact context
    /// DATA and a paid original read loan; this API does not establish a current miss.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        request: V2CreateRequest<'_>,
    ) -> Result<Self, AppError> {
        selected.resources().validate_original(store, source)?;
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .clone();
        let metadata_fee = pool.try_reserve(TEXT_TRANSACTION_METADATA_FEE)?;
        #[cfg(all(test, windows))]
        let metadata_fee = {
            let mut metadata_fee = metadata_fee;
            NEXT_TRANSACTION_FEE_NOTICE.with(|slot| {
                if let Some(notice) = slot.borrow_mut().take() {
                    metadata_fee.observe_drop_for_test(notice);
                }
            });
            metadata_fee
        };
        metadata_fee.validate(&pool, TEXT_TRANSACTION_METADATA_FEE)?;
        selected.resources().validate_original(store, source)?;
        let work = V2TextWorkResources::prepare_paid(store, source, selected.resources())?;
        let context = &selected.head_data().context;
        let request_digest = TextRequestDigestData::from_request_data(
            context.id_data()?,
            request.token,
            request.channel,
            request.text,
        )?;
        work.validate_prepaid_account()?;
        selected
            .resources()
            .require_bound_original_negative(store, source)?;
        let parent = selected.archive_data().spec_data();
        parent
            .manifest_sequence
            .checked_add(1)
            .ok_or_else(refused)?;
        Ok(Self {
            context: context.clone(),
            token: request.token,
            channel: request.channel,
            request: request_digest,
            transaction: TextTransactionIdData::from_request_data(
                context.id_data()?,
                request.token,
                request_digest,
            ),
            parent: RetainParentData {
                head: selected.head_id_data(),
                archive: selected.head_data().archive,
                manifest: parent.manifest,
                manifest_sequence: parent.manifest_sequence,
                header: parent.header.clone(),
                image: Zeroizing::new(parent.image.to_vec()),
            },
            packet: None,
            original: None,
            actions: Vec::with_capacity(2),
            ciphers: Vec::with_capacity(3),
            retain: None,
            manifest: None,
            archive: None,
            work: Arc::new(work),
            metadata: V2TextTransactionMetadataResources {
                body: Arc::new(TransactionMetadataPayment {
                    workspace_busy: AtomicBool::new(false),
                    pool,
                    fee: metadata_fee,
                }),
            },
        })
    }

    /// Resource-only loans for one independently bounded active workspace/IO owner. Any
    /// escaping File bundle must retain this SAME16MiB Arc after every File/lease payload.
    pub(in crate::store) fn work_resources(&self) -> &Arc<V2TextWorkResources> {
        &self.work
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.work.read_resources()
    }
    pub(in crate::store) fn metadata_resources(&self) -> &V2TextTransactionMetadataResources {
        &self.metadata
    }
    /// Validate actual separate payments and the SAME original read/SDK association.
    /// This may wait on accounts and supplies no Current, physical IO or signing grant.
    pub(in crate::store) fn validate_resources_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        self.work.validate_original(store, source)
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2RetainObservation {
        OriginalV2RetainObservation {
            metadata: self.metadata.clone(),
        }
    }
    /// Claim BEFORE allocation/lease. The successful affine claim keeps BOTH new payers
    /// alive; no Source, SDK, File or work admission is supplied by resource custody.
    pub(in crate::store) fn claim_workspace_preparation(
        &self,
    ) -> Result<V2TextRetainWorkspaceClaim, AppError> {
        self.metadata.validate_prepaid_account()?;
        self.work.validate_prepaid_account()?;
        self.metadata
            .body
            .workspace_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| refused())?;
        Ok(V2TextRetainWorkspaceClaim {
            _work: self.work.clone(),
            metadata: self.metadata.clone(),
        })
    }

    /// Only this original local packet slot is unmaterialized. This does not prove that
    /// a persisted WAL or another historical packet is absent; the workspace issuer must
    /// establish those facts independently before a new Create predecessor is admitted.
    pub(in crate::store) fn require_unmaterialized_data(&self) -> Result<(), AppError> {
        if self.packet.is_some()
            || self.original.is_some()
            || !self.actions.is_empty()
            || !self.ciphers.is_empty()
            || self.retain.is_some()
            || self.manifest.is_some()
            || self.archive.is_some()
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Compare immutable DATA from the SAME originally paid capture, with no account,
    /// filesystem, decoder or SDK call. A true comparison is not present selection.
    pub(in crate::store) fn require_selected_data(
        &self,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.resources()
            .require_same_original(selected.resources())?;
        let a = selected.archive_data().spec_data();
        if self.context != selected.head_data().context
            || self.parent.head != selected.head_id_data()
            || self.parent.archive != selected.head_data().archive
            || self.parent.manifest != a.manifest
            || self.parent.manifest_sequence != a.manifest_sequence
            || self.parent.header != *a.header
            || self.parent.image.as_slice() != a.image
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn context_data(&self) -> &TextContextData {
        &self.context
    }
    pub(in crate::store) fn token_data(&self) -> [u8; 32] {
        self.token
    }
    pub(in crate::store) fn request_data(&self) -> TextRequestDigestData {
        self.request
    }
    /// Retry input equality only. It cannot replace the retained original packet or
    /// establish a live Source/Current, complete miss, saved state or signing permission.
    pub(in crate::store) fn require_request_data(
        &self,
        request: &V2CreateRequest<'_>,
    ) -> Result<(), AppError> {
        if request.token != self.token
            || request.channel != self.channel
            || TextRequestDigestData::from_request_data(
                self.context.id_data()?,
                request.token,
                request.channel,
                request.text,
            )? != self.request
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn transaction_data(&self) -> TextTransactionIdData {
        self.transaction
    }
    pub(in crate::store) fn parent_head_data(&self) -> TextHeadIdData {
        self.parent.head
    }
    pub(in crate::store) fn parent_archive_data(&self) -> ArchiveHeadId {
        self.parent.archive
    }

    /// A borrowed original action, never accepted/served/publishable merely by existing.
    pub(in crate::store) fn action_data(
        &self,
    ) -> Result<&catcoms_replication::chat::SignedAction, AppError> {
        Ok(self.packet.as_ref().ok_or_else(refused)?.view_data().action)
    }
    /// Fixed original-representation commitment already produced with the actual action
    /// pages. Borrowers need not allocate/encode the complete action again to name it.
    /// This DATA label supplies no accepted-history, Current or publication authority.
    pub(in crate::store) fn original_action_digest_data(
        &self,
    ) -> Result<catcoms_replication::chat::cold_actions::OriginalActionDigest, AppError> {
        Ok(self
            .original
            .as_ref()
            .ok_or_else(refused)?
            .original_digest())
    }
    /// Cached semantic ID from the SAME original action pages; no encoding/allocation.
    pub(in crate::store) fn original_action_id_data(
        &self,
    ) -> Result<catcoms_replication::chat::ActionId, AppError> {
        Ok(self.original.as_ref().ok_or_else(refused)?.action_id())
    }
    pub(in crate::store) fn has_complete_data(&self) -> bool {
        self.archive.is_some() && self.manifest.is_some() && self.retain.is_some()
    }
    /// The original SDK packet was already transferred, even when page sealing stopped
    /// halfway. A retry must finish THESE bytes, never call the Create signer again.
    pub(in crate::store) fn has_packet_data(&self) -> bool {
        self.packet.is_some()
    }

    /// Exact originals borrowed from this owning cell. Callers must check completion and
    /// fund all WAL/COW/sealing copies before assembly; this iterator owns no frame, path,
    /// payment or effect permission. The physical cell consumes this WHOLE producer so
    /// originals and their single16MiB payment cannot be separated by a fallible transfer.
    pub(in crate::store) fn artifact_frames_data(
        &self,
    ) -> impl Iterator<Item = (TextArtifactRoleData, [u8; 32], &[u8])> + '_ {
        self.actions
            .iter()
            .map(|p| {
                (
                    TextArtifactRoleData::ActionChunk,
                    *p.page.id().as_bytes(),
                    p.page.as_bytes(),
                )
            })
            .chain(self.ciphers.iter().map(|p| {
                (
                    TextArtifactRoleData::CipherChunk,
                    *p.page.id().as_bytes(),
                    p.page.as_bytes(),
                )
            }))
            .chain(self.retain.iter().map(|p| {
                (
                    TextArtifactRoleData::Retain,
                    *p.id_data().as_bytes(),
                    p.frame_data(),
                )
            }))
            .chain(self.manifest.iter().map(|p| {
                (
                    TextArtifactRoleData::Manifest,
                    *p.id().as_bytes(),
                    p.as_bytes(),
                )
            }))
            .chain(self.archive.iter().map(|p| {
                (
                    TextArtifactRoleData::Archive,
                    *p.id_data().as_bytes(),
                    p.frame_data(),
                )
            }))
    }

    /// Every check precedes taking the caller's original packet. A changed token/body,
    /// foreign signer/realm or late source refusal preserves both owning input slots.
    pub(in crate::store) fn attach_packet(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        packet: &mut Option<DetachedV2CreateData>,
    ) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        self.work.validate_original(store, source)?;
        if let Some(original) = &self.packet {
            if packet
                .as_ref()
                .is_some_and(|input| original.id_data() != input.id_data())
            {
                return Err(refused());
            }
            // This is only an idempotent DATA comparison. Repeat the original negative
            // observations even on this branch; an earlier successful transfer leaves
            // the same caller slot None. A second matching input is left untouched.
            return self
                .resources()
                .require_bound_original_negative(store, source);
        }
        let input = packet.as_ref().ok_or_else(refused)?;
        let view = input.view_data();
        let data = source.data();
        let c = self.context.spec_data();
        let Body::Create {
            text,
            reply_to: None,
        } = &view.action.spec().body
        else {
            return Err(refused());
        };
        if view.token != self.token
            || view.channel != self.channel
            || view.epoch != 0
            || view.action.spec().realm.group() != c.group
            || view.action.spec().realm.channel() != self.channel
            || view.action.signer() != &c.signer
            || !view.action.spec().parents.is_empty()
            || !view.action.verify()
            || data.source != c.source
            || data.profile.id() != c.profile
            || data.admission.id() != c.admission
            || TextRequestDigestData::from_request_data(
                self.context.id_data()?,
                self.token,
                self.channel,
                text,
            )? != self.request
            || view.action_bytes.len() > MAX_CREATE_ACTION_BYTES
            || 24usize
                .checked_add(view.ciphertext.len())
                .filter(|n| *n <= MAX_ENCRYPTED_PAYLOAD_BYTES)
                .is_none()
        {
            return Err(refused());
        }
        data.admission
            .matches_action_delegation(view.action)
            .map_err(|_| refused())?;
        // Validate the SAME original read/SDK negative, not just equal context labels.
        self.resources()
            .require_bound_original_negative(store, source)?;
        self.packet = packet.take();
        Ok(())
    }

    /// Seal each original page at most once and cache it immediately. Every retry first
    /// proves the original resource/source; after cancellation, only existing DATA may
    /// be retained, never another seal/sign/export. This performs no storage mutation.
    pub(in crate::store) fn prepare_pages(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        self.work.validate_original(store, source)?;
        source.validate_original(store)?;
        let packet = self.packet.as_ref().ok_or_else(refused)?;
        let view = packet.view_data();
        let key = Zeroizing::new(store.keys.db_key()?);
        if self.original.is_none() {
            self.original = Some(
                PreparedActionPages::prepare(
                    self.context.spec_data().layout.vault,
                    self.context.spec_data().profile,
                    view.action,
                )
                .map_err(|_| refused())?,
            );
        }
        let original = self.original.as_ref().ok_or_else(refused)?;
        if original.chunk_count() > 2 {
            return Err(refused());
        }
        while self.actions.len() < original.chunk_count() {
            self.resources()
                .require_bound_original_negative(store, source)?;
            let plain = original
                .encode_chunk(self.actions.len())
                .map_err(|_| refused())?;
            let plain_bytes = u32::try_from(plain.len()).map_err(|_| refused())?;
            self.resources()
                .require_bound_original_negative(store, source)?;
            let page = PreparedPrivateChatPage::prepare(&key, &plain, rng)?;
            self.actions.push(OriginalPage { page, plain_bytes });
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        let count = (24 + view.ciphertext.len()).div_ceil(ACTION_CHUNK_BYTES);
        while self.ciphers.len() < count {
            self.resources()
                .require_bound_original_negative(store, source)?;
            let plain = CipherChunkData::encode_data(
                self.context.id_data()?,
                self.transaction,
                view.nonce,
                view.ciphertext,
                self.ciphers.len(),
            )?;
            let plain_bytes = u32::try_from(plain.len()).map_err(|_| refused())?;
            self.resources()
                .require_bound_original_negative(store, source)?;
            let page = PreparedPrivateChatPage::prepare(&key, &plain, rng)?;
            self.ciphers.push(OriginalPage { page, plain_bytes });
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.retain.is_none() {
            let actions: Vec<_> = self
                .actions
                .iter()
                .map(|p| TextChunkDescriptorData {
                    page: p.page.id(),
                    plain_bytes: p.plain_bytes,
                })
                .collect();
            let ciphers: Vec<_> = self
                .ciphers
                .iter()
                .map(|p| TextChunkDescriptorData {
                    page: p.page.id(),
                    plain_bytes: p.plain_bytes,
                })
                .collect();
            let retain = RetainedActionData::from_data(RetainedActionSpecData {
                context: self.context.id_data()?,
                transaction: self.transaction,
                token: self.token,
                request: self.request,
                action: view.action.id(),
                original: original.original_digest(),
                cipher: cipher_digest(view.nonce, view.ciphertext)?,
                action_bytes: u32::try_from(view.action_bytes.len()).map_err(|_| refused())?,
                cipher_bytes: u32::try_from(24 + view.ciphertext.len()).map_err(|_| refused())?,
                channel: self.channel,
                actions: &actions,
                ciphers: &ciphers,
            })?;
            let plain = retain.encode_data()?;
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.retain = Some(PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Retain,
                &key,
                &plain,
                rng,
            )?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.manifest.is_none() {
            let descriptors: Vec<_> = self
                .actions
                .iter()
                .map(|p| PageDescriptor {
                    role: PageRole::OriginalActionChunk,
                    page: p.page.id(),
                    plaintext_bytes: p.plain_bytes as usize,
                    entries: 1,
                })
                .collect();
            let c = self.context.spec_data();
            let plain = encode_manifest(&ManifestSpec {
                vault: c.layout.vault,
                group: c.group,
                profile: c.profile,
                sequence: self
                    .parent
                    .manifest_sequence
                    .checked_add(1)
                    .ok_or_else(refused)?,
                previous: Some(self.parent.manifest),
                source_head: Some(self.parent.archive),
                before_header: Some(self.parent.header.id()),
                after_header: self.parent.header.id(),
                operation: ArchiveOperation::RetainAction {
                    action: view.action.id(),
                    original: original.original_digest(),
                    total_bytes: u32::try_from(view.action_bytes.len()).map_err(|_| refused())?,
                },
                pages: &descriptors,
            })
            .map_err(|_| refused())?;
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.manifest = Some(PreparedPrivateChatPage::prepare(&key, &plain, rng)?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.archive.is_none() {
            let archive = TextArchiveData::from_data(TextArchiveSpecData {
                context: self.context.id_data()?,
                manifest: self.manifest.as_ref().ok_or_else(refused)?.id(),
                manifest_sequence: self
                    .parent
                    .manifest_sequence
                    .checked_add(1)
                    .ok_or_else(refused)?,
                header: &self.parent.header,
                image: &self.parent.image,
                previous: Some(self.parent.archive),
                operation: TextArchiveOperationData::RetainAction,
            })?;
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.archive = Some(archive.prepare_frame_data(&key, rng)?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        Ok(())
    }

    /// Complete immutable frame DATA only, not RetainedA or acceptance. The borrowed
    /// frame stays funded by this original owner; it conveys no DB key or permission.
    pub(in crate::store) fn retain_frame_data(
        &self,
    ) -> Result<Option<(&[u8], TextRetainIdData)>, AppError> {
        self.retain
            .as_ref()
            .map(|r| {
                RetainedActionData::frame_id_data(r.frame_data()).map(|id| (r.frame_data(), id))
            })
            .transpose()
    }
}

impl std::fmt::Debug for PreparedV2RetainData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2RetainData")
            .finish_non_exhaustive()
    }
}

macro_rules! redacted_retain_metadata {
    ($($t:ty),+ $(,)?) => {$(
        impl std::fmt::Debug for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("RetainMetadata { .. }")
            }
        }
    )+};
}
redacted_retain_metadata!(
    V2TextTransactionMetadataResources,
    OriginalV2RetainObservation,
    V2TextRetainWorkspaceClaim
);
