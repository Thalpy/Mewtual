//! Physical family plumbing for the local draft archive: naming, scope, bounds, an authenticated
//! bounded reader and accounting. It knows nothing about what an archive contains.
//!
//! The archive is a distinct **physical** record kind, with its own suffix, domain, scope, path,
//! reader and cap, while sharing the **Intents accounting class**: its records, record slots and
//! bytes are charged to [`EpochIntentBudget`](super::EpochIntentBudget) against
//! `MAX_VAULT_INTENT_BYTES`. It is not a new budget family.
//!
//! This module is Agent 2's seam, landed ahead of their implementation so that all three agents
//! rebase onto one variant rather than each adding the same conceptual enum arm. Nothing here
//! writes, releases or decodes an archive: the payload schema, the writer, the release path, the
//! disposal transaction, the reference collector and the archive sub-cap are Agent 2's, and are
//! built on top of this. A vault with no archive file therefore behaves exactly as it did before.

use std::collections::BTreeSet;
use std::io::Read;

use catcoms_replication::checkpoint::MAX_CHECKPOINT_BYTES;
use catcoms_replication::epoch::{MAX_INTENT_BYTES_PER_DOCUMENT, MAX_RECEIPT_BYTES};
use catcoms_replication::studio::ContentId;
use catcoms_replication::studio::MAX_STUDIO_OVERLAY_OPS;
use catcoms_replication::LogicalDocument;

use super::epoch_budget::{Footprint, StorageRecord};
use super::epoch_recovery::inventory::{is_link, regular_file};
use super::epoch_recovery::AuthenticatedEpochFileBytes;
use super::*;

pub(super) const RECORD_DOMAIN: &[u8] = b"catcoms/epoch-draft-archive-store/v1";

/// Fixed per-entry fields plus framing. Agent 2 ties this to the actual encoder with a static
/// assertion when the payload lands; until then it is the conservative scan rail.
const ARCHIVE_ENTRY_OVERHEAD_BYTES: usize = 128;
/// Deliberately generous; Agent 2 tightens it against the real header.
const ARCHIVE_HEADER_BYTES: usize = 2048;

/// Header, the covering receipt, the seed checkpoint, a document's worth of operations, and
/// per-entry overhead for a full overlay branch. Kept crate-internal because nothing outside the
/// store needs it yet; widen it and re-export it from `store.rs` when the payload encoder that
/// must satisfy it lands.
pub(super) const MAX_DRAFT_ARCHIVE_PAYLOAD_BYTES: usize = ARCHIVE_HEADER_BYTES
    + MAX_RECEIPT_BYTES
    + MAX_CHECKPOINT_BYTES
    + MAX_INTENT_BYTES_PER_DOCUMENT
    + MAX_STUDIO_OVERLAY_OPS * ARCHIVE_ENTRY_OVERHEAD_BYTES;
const MAX_DRAFT_ARCHIVE_RECORD_BYTES: usize = MAX_DRAFT_ARCHIVE_PAYLOAD_BYTES + 1024;
/// About 6 MiB + 35 KiB, deliberately larger than the intent record's 5 MiB + 1024: the archive is
/// its own record kind and was never obliged to obey the intent cap. Recovery remains the largest
/// family at 18 MiB + 2088, so no scan rail moves for this.
pub(super) const MAX_DRAFT_ARCHIVE_SEALED_BYTES: usize = MAX_DRAFT_ARCHIVE_RECORD_BYTES + 40;

impl ServerStore {
    pub(in crate::store) fn epoch_draft_archive_path(&self, scope: &[u8]) -> PathBuf {
        self.dir
            .join("servers")
            .join(format!("{}.draft-archive", blake3::hash(scope).to_hex()))
    }

    /// Authenticate framing under the same parent-directory and regular-file rails the intent
    /// reader uses, with this family's own cap. It performs no typed decode: there is no archive
    /// payload type yet, and the scanner needs only the authenticated scope and physical size.
    pub(super) fn read_epoch_draft_archive_plain(
        &self,
        path: &Path,
    ) -> Result<Option<AuthenticatedEpochFileBytes>, AppError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AppError::Io(e.to_string())),
        };
        if !regular_file(&metadata) || metadata.len() > MAX_DRAFT_ARCHIVE_SEALED_BYTES as u64 {
            return Err(invalid("draft archive file is not bounded and regular"));
        }
        let file = File::open(path).map_err(|e| AppError::Io(e.to_string()))?;
        if !regular_file(&file.metadata().map_err(|e| AppError::Io(e.to_string()))?) {
            return Err(invalid("opened draft archive file is not regular"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_DRAFT_ARCHIVE_SEALED_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Io(e.to_string()))?;
        if bytes.len() > MAX_DRAFT_ARCHIVE_SEALED_BYTES {
            return Err(invalid("draft archive file exceeds its bound"));
        }
        Ok(Some(AuthenticatedEpochFileBytes {
            plain: Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&bytes)?)?),
            physical_bytes: bytes.len() as u64,
        }))
    }

    /// The same read through the canonical path, with the parent-directory check the addressed
    /// intent reader performs.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Agent 2's writer and release path are \
        the first production callers; the seam is exercised by its own tests today"
        )
    )]
    pub(in crate::store) fn read_scoped_draft_archive_plain(
        &self,
        scope: &[u8],
    ) -> Result<Option<AuthenticatedEpochFileBytes>, AppError> {
        let parent = fs::symlink_metadata(self.dir.join("servers"))
            .map_err(|e| AppError::Io(e.to_string()))?;
        if !parent.is_dir() || is_link(&parent) {
            return Err(invalid("parent is not a regular directory"));
        }
        self.read_epoch_draft_archive_plain(&self.epoch_draft_archive_path(scope))
    }
}

/// Same shape as `epoch_intents::scope_bytes` under this family's own domain, so
/// `decode_record_scope`'s domain check and canonical re-derivation work unmodified and an
/// archive scope can never be mistaken for an intent scope.
pub(super) fn scope_bytes(server: u64, document: &LogicalDocument) -> Result<Vec<u8>, AppError> {
    super::epoch_recovery::scope_bytes(server, document)?;
    let mut e = Encoder::new();
    e.put_bytes(RECORD_DOMAIN).expect("constant fits");
    e.put_u64(server);
    e.put_bytes(&document.server_id).map_err(invalid)?;
    e.put_u16(document.doc_type.tag());
    e.put_bytes(&document.logical_key).map_err(invalid)?;
    Ok(e.finish())
}

/// Charged as content, in the Intents class. The id derives from this family's own scope, so an
/// archive and an intent ledger for the same logical document occupy distinct records.
pub(super) fn storage_record(
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    bytes: u64,
) -> Result<StorageRecord, AppError> {
    Ok(StorageRecord {
        id: *blake3::hash(scope).as_bytes(),
        document: *blake3::hash(&super::epoch_recovery::scope_bytes(server, document)?).as_bytes(),
        footprint: Footprint {
            content: bytes,
            ..Footprint::default()
        },
    })
}

/// One archive's accounting record and the conservative reference set it keeps alive.
pub(super) struct InspectedDraftArchive {
    pub(super) record: StorageRecord,
    pub(super) cids: BTreeSet<ContentId>,
}

/// Decode an archive body and yield the blob references it protects.
///
/// This is the narrowing of the seam's fail-closed reference arm, not its removal: it no longer
/// refuses every archive, it refuses one whose bounded canonical payload will not decode, which
/// is the rule every other family already applies to a corrupt record. Refusing is what keeps a
/// complete scan from installing a known set that omits an archive's CIDs and so making archived
/// pixels reclaimable, which is the single failure the archive exists to prevent.
///
/// The caller has already authenticated the file, validated the scope's domain against this
/// family, re-derived the scope canonically and checked that the filename matches it. What is
/// left to bind is the payload's own claim: an archive naming a different logical document than
/// the record it sits in would have its references attributed to the wrong group.
pub(super) fn inventory_references(
    plain: &[u8],
    server: u64,
    document: &LogicalDocument,
    scope: &[u8],
    bytes: u64,
) -> Result<InspectedDraftArchive, AppError> {
    let mut d = Decoder::new(plain);
    if d.get_bytes().map_err(invalid)? != scope {
        return Err(invalid("wrong sealed scope"));
    }
    let body = d.get_bytes().map_err(invalid)?;
    d.finish().map_err(invalid)?;
    let archive = catcoms_replication::studio::StudioDraftArchive::decode(body).map_err(invalid)?;
    if archive.document() != document {
        return Err(invalid("draft archive names another logical document"));
    }
    Ok(InspectedDraftArchive {
        record: storage_record(server, document, scope, bytes)?,
        cids: archive.blob_cids().map_err(invalid)?,
    })
}

fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("epoch draft archive: {error}"))
}

/// Test-only fixture: seal and frame an archive file at its canonical path.
///
/// There is deliberately no production writer in this seam, so a fixture cannot call one. It
/// bypasses nothing the seam validates: the scope comes from [`scope_bytes`], the sealing and
/// framing are the store's own, and the path is [`ServerStore::epoch_draft_archive_path`], so the
/// filename grammar, filename-to-scope agreement, bound, framing, authentication, domain check and
/// canonical scope re-derivation are all exercised for real. Only `body` is opaque, which is
/// exactly what this family does not interpret. Delete this when Agent 2's writer lands and have
/// the tests call that instead.
#[cfg(test)]
pub(in crate::store) fn write_draft_archive_for_test(
    store: &ServerStore,
    server: u64,
    document: &LogicalDocument,
    body: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<PathBuf, AppError> {
    let scope = scope_bytes(server, document)?;
    let mut e = Encoder::new();
    e.put_bytes(&scope).map_err(invalid)?;
    e.put_bytes(body).map_err(invalid)?;
    let plain = Zeroizing::new(e.finish());
    if plain.len() > MAX_DRAFT_ARCHIVE_RECORD_BYTES {
        return Err(invalid("record exceeds its bound"));
    }
    let sealed = seal(&store.keys.db_key()?, &plain, rng)?;
    let path = store.epoch_draft_archive_path(&scope);
    fs::write(&path, frame(&sealed)).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(path)
}
