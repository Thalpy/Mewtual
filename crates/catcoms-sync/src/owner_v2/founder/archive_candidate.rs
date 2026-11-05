//! Original founder/private-S0 -> one immutable public genesis archive candidate. No I/O,
//! mounted key, fees, covering save, Current, publisher or positive authority is created here.
use super::{FounderPrivateSnapshotView, FrozenFounder};
use crate::{ChannelSync, SyncError};
use catcoms_replication::chat::{
    cold_manifest::{
        encode_manifest, ArchiveOperation, ManifestSpec, PageDescriptor, PageRole,
        MAX_MANIFEST_PAGE_BYTES,
    },
    cold_records::{
        encode_record_batch, BatchBinding, RecordSource, MAX_HEADER_ROW_BYTES,
        MAX_RECORD_BATCH_BYTES, RECORD_BATCH_METADATA_BYTES,
    },
    owner_facts::OwnerFactId,
    owner_header::{OwnerHeaderId, SignedOwnerHeader, MAX_HEADER_BYTES},
    owner_profile::{AdmissionBody, SignedOwnerProfile, MAX_ADMISSION_BYTES, MAX_PROFILE_BYTES},
    MAX_GROUP_ID_BYTES,
};
use catcoms_rt::{CryptoRngCore, MeshTransport};
use catcoms_storage::private_chat_page::{
    PreparedPrivateChatPage, PrivateChatPageId, MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES,
    MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES,
};
use zeroize::Zeroizing;
#[path = "bootstrap_source/mod.rs"]
mod bootstrap_source;
pub use bootstrap_source::{
    open_bootstrap_source_data, prepare_v2_genesis_seed, BootstrapSourceContextData,
    BootstrapSourceContextSpec, BootstrapSourceDataView, BootstrapSourceId, BootstrapSourceRefusal,
    FounderBootstrapPreparation, OpenedBootstrapSourceData, OriginalV2GenesisSeedObservation,
    PreparedBootstrapSource, PreparedUnboundV2GenesisNode, ReturnedV2GenesisAttach,
    UnboundV2GenesisNode, UnboundV2GenesisSeed, V2GenesisRuntimeParts, V2GenesisRuntimeRefusal,
    V2GenesisSeedData, V2GenesisSeedLimits, BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE,
    BOOTSTRAP_SOURCE_OPEN_BUFFER_BYTES, BOOTSTRAP_SOURCE_SEAL_BUFFER_BYTES,
    MAX_BOOTSTRAP_SOURCE_FRAME_BYTES, MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES,
    V2_GENESIS_SEED_METADATA_BYTES,
};

const KEY_BINDING_DOMAIN: &str = "catcoms/private-chat-genesis-archive-key-binding/v1";
/// Named logical original-package/encoding allowance, NOT a prepaid fee or SDK/whole-host cap.
/// The actual owner separately retains the founder public-source allowance, private S0/raw/
/// provider/backing and source-comparison fees before entering this producer.
pub const GENESIS_ARCHIVE_PRIVATE_INVOICE: usize = 512 * 1024;

// Batch phase: exact requested batch output + AEAD ciphertext + sealed record frame + fixed
// batch arrays + composite/signature/typed-group scratch. Manifest phase retains that SAME
// record frame and adds one small encoder, descriptor scratch and ciphertext/frame. Summing
// both phases conservatively is explicit overpayment, not a whole-host/RSS assertion.
const NAMED_LAYOUT: usize = MAX_RECORD_BATCH_BYTES
    + (MAX_RECORD_BATCH_BYTES + 16)
    + MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES
    + RECORD_BATCH_METADATA_BYTES
    + MAX_HEADER_ROW_BYTES
    + MAX_HEADER_BYTES
    + MAX_PROFILE_BYTES
    + MAX_ADMISSION_BYTES
    + 4 * MAX_GROUP_ID_BYTES
    + 8 * 1024 // bounded nested semantic/signature/genesis scratch, separate from original frames
    + MAX_MANIFEST_PAGE_BYTES
    + 3 * 39
    + (MAX_MANIFEST_PAGE_BYTES + 16)
    + (MAX_MANIFEST_PAGE_BYTES + MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES
        - MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES)
    + std::mem::size_of::<FounderArchivePreparation>()
    + std::mem::size_of::<ArchiveSourceRefusal>()
    + std::mem::size_of::<FrozenFounder>(); // boxed original-source error storage, if refused
const _: () = assert!(NAMED_LAYOUT <= GENESIS_ARCHIVE_PRIVATE_INVOICE);

/// Pre-candidate refusal preserves the SAME original private owner. It can be recovered to
/// finish an uncached source; no page, key binding or possible I/O candidate exists on this path.
pub struct ArchiveSourceRefusal {
    original: Box<FrozenFounder>,
}
impl ArchiveSourceRefusal {
    pub fn into_original(self) -> FrozenFounder {
        *self.original
    }
}
impl std::fmt::Debug for ArchiveSourceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveSourceRefusal")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for ArchiveSourceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private archive source is unavailable")
    }
}
impl std::error::Error for ArchiveSourceRefusal {}

struct OriginalGenesisArchive {
    // Encrypted outputs drop BEFORE their original secret-derived comparison binding. Neither
    // is a resource fee; the actual host must keep its separate leases alive through all copies.
    records: PreparedPrivateChatPage,
    manifest: PreparedPrivateChatPage,
    key_binding: Zeroizing<[u8; 32]>,
    records_plaintext_bytes: usize,
    profile: OwnerFactId,
    header: OwnerHeaderId,
}

/// Owns the actual frozen founder and its cached private S0; no mutable source/Server escape
/// permits a different source to be placed behind already prepared ciphertext. Retry/error
/// borrows retain the whole original package. Actual App I/O must retain this owner on ambiguity.
pub struct FounderArchivePreparation {
    package: Option<OriginalGenesisArchive>,
    original: FrozenFounder,
    vault: [u8; 32],
}
impl std::fmt::Debug for FounderArchivePreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderArchivePreparation")
            .finish_non_exhaustive()
    }
}

/// Borrowed private ciphertext DATA only. These values are not a covering head, permission to
/// read/write a path, a fee, or Current/publication evidence. They never implement native serde.
pub struct GenesisArchiveView<'a> {
    pub vault: &'a [u8; 32],
    pub profile: OwnerFactId,
    pub header: OwnerHeaderId,
    pub records_id: PrivateChatPageId,
    pub records_frame: &'a [u8],
    pub records_plaintext_bytes: usize,
    pub manifest_id: PrivateChatPageId,
    pub manifest_frame: &'a [u8],
}
impl std::fmt::Debug for GenesisArchiveView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenesisArchiveView").finish_non_exhaustive()
    }
}

fn unavailable() -> SyncError {
    SyncError::ChatScopeUnavailable
}

impl FounderArchivePreparation {
    /// Consume ONLY after exact S0 exists. Original managed-domain DATA comes from the SAME
    /// opaque observation, not a scalar label or downloaded Profile. Refusal returns the source.
    pub fn from_cached_source<T: MeshTransport, R: CryptoRngCore>(
        original: FrozenFounder,
        node: &ChannelSync<T, R>,
        expected_vault: [u8; 32],
    ) -> Result<Self, ArchiveSourceRefusal> {
        let valid = original
            .original
            .scope
            .managed_archive_domain(&node.local_chat_scope)
            .is_ok_and(|domain| domain == expected_vault)
            && original
                .borrow_private_snapshot_for_covering_save(node)
                .is_ok();
        if !valid {
            return Err(ArchiveSourceRefusal {
                original: Box::new(original),
            });
        }
        Ok(Self {
            package: None,
            original,
            vault: expected_vault,
        })
    }

    fn check_source<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<(), SyncError> {
        let domain = self
            .original
            .original
            .scope
            .managed_archive_domain(&node.local_chat_scope)?;
        if domain != self.vault {
            return Err(unavailable());
        }
        self.original
            .borrow_private_snapshot_for_covering_save(node)?;
        Ok(())
    }

    /// First prepare constructs all private outputs, then repeats original-source equality before
    /// returning ANY view. Subsequent retries verify the same key/source and borrow SAME bytes;
    /// they never re-sign/reseal or draw randomness. A key argument is not a mounted-key permit.
    pub fn prepare<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        node: &ChannelSync<T, R>,
        original_vault_db_key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<GenesisArchiveView<'_>, SyncError> {
        self.check_source(node)?; // BEFORE key hashing, record encoders, RNG or AEAD.
        let key_binding = Zeroizing::new(blake3::derive_key(
            KEY_BINDING_DOMAIN,
            original_vault_db_key,
        ));
        if let Some(package) = &self.package {
            if *package.key_binding != *key_binding {
                return Err(unavailable());
            }
        } else {
            let package = make_original_package(
                &self.original,
                node,
                self.vault,
                original_vault_db_key,
                key_binding,
                rng,
            )?;
            // A pre-return cancellation discards only an unexposed local preparation. There was
            // no I/O/handoff; it grants no attempt refund and cannot replace an existing package.
            self.check_source(node)?;
            self.package = Some(package);
        }
        self.borrow_for_covering_save(node)
    }

    /// Repeat exact original/private-S0 checks; do not infer currentness from cached ciphertext.
    /// The future saver still supplies atomic original scope/head/native admission through I/O.
    pub fn borrow_for_covering_save<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<GenesisArchiveView<'_>, SyncError> {
        self.check_source(node)?;
        let package = self.package.as_ref().ok_or_else(unavailable)?;
        Ok(GenesisArchiveView {
            vault: &self.vault,
            profile: package.profile,
            header: package.header,
            records_id: package.records.id(),
            records_frame: package.records.as_bytes(),
            records_plaintext_bytes: package.records_plaintext_bytes,
            manifest_id: package.manifest.id(),
            manifest_frame: package.manifest.as_bytes(),
        })
    }

    /// No mutable source exposure: the actual saver may borrow only this SAME checked cached S0.
    pub fn borrow_private_snapshot_for_covering_save<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<FounderPrivateSnapshotView<'_>, SyncError> {
        self.check_source(node)?;
        self.original
            .borrow_private_snapshot_for_covering_save(node)
    }
}

fn make_original_package<T: MeshTransport, R: CryptoRngCore>(
    original: &FrozenFounder,
    node: &ChannelSync<T, R>,
    vault: [u8; 32],
    key: &[u8; 32],
    key_binding: Zeroizing<[u8; 32]>,
    rng: &mut dyn CryptoRngCore,
) -> Result<OriginalGenesisArchive, SyncError> {
    let (records, records_plaintext_bytes, profile_id, header_id, body) = {
        let data = original.borrow_for_covering_save(node)?;
        let profile = SignedOwnerProfile::decode(data.profile).map_err(|_| unavailable())?;
        let admission = AdmissionBody::decode(data.admission).map_err(|_| unavailable())?;
        let header = SignedOwnerHeader::decode(data.header).map_err(|_| unavailable())?;
        let operation = ArchiveOperation::Genesis {
            body: admission.id(),
        };
        let batch = encode_record_batch(
            BatchBinding {
                vault: &vault,
                profile: &profile,
                role: PageRole::GenesisRecords,
                operation: &operation,
                append: None,
            },
            &[
                RecordSource::Profile(&profile),
                RecordSource::Header {
                    header: &header,
                    image: data.state,
                },
                RecordSource::Admission(&admission),
            ],
        )
        .map_err(|_| unavailable())?;
        let records =
            PreparedPrivateChatPage::prepare(key, &batch, rng).map_err(|_| unavailable())?;
        (
            records,
            batch.len(),
            profile.id(),
            header.id(),
            admission.id(),
        )
        // Batch plaintext, typed copies and fixed batch arrays drop BEFORE manifest encoding.
    };
    let page = PageDescriptor {
        role: PageRole::GenesisRecords,
        page: records.id(),
        plaintext_bytes: records_plaintext_bytes,
        entries: 3,
    };
    let operation = ArchiveOperation::Genesis { body };
    let manifest = encode_manifest(&ManifestSpec {
        vault,
        group: node.group_id_ref(),
        profile: profile_id,
        sequence: 1,
        previous: None,
        source_head: None,
        before_header: None,
        after_header: header_id,
        operation,
        pages: &[page],
    })
    .map_err(|_| unavailable())?;
    let manifest =
        PreparedPrivateChatPage::prepare(key, &manifest, rng).map_err(|_| unavailable())?;
    Ok(OriginalGenesisArchive {
        records,
        manifest,
        key_binding,
        records_plaintext_bytes,
        profile: profile_id,
        header: header_id,
    })
}

#[cfg(test)]
mod tests;
