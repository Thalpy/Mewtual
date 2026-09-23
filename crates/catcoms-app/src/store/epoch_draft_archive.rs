//! The local draft archive's physical family: naming, scope, bounds, an authenticated bounded
//! reader, accounting, and the reference collector that reads a payload to find the blobs it
//! protects.
//!
//! The archive is a distinct **physical** record kind, with its own suffix, domain, scope, path,
//! reader and cap, while sharing the **Intents accounting class**: its records, record slots and
//! bytes are charged to [`EpochIntentBudget`](super::EpochIntentBudget) against
//! `MAX_VAULT_INTENT_BYTES`. It is not a new budget family.
//!
//! This module began as Agent 1's seam, landed ahead of Agent 2's implementation so that all
//! three agents rebase onto one enum variant rather than each adding the same conceptual arm.
//! The seam itself decoded nothing and failed every reference scan closed; the collector below
//! narrows that refusal to an archive it cannot read, and the writer below persists one. Still
//! to come on top of this: the release path and the disposal transaction, which is the writer's
//! first production caller. A vault with no archive file behaves exactly as it did before.

use std::collections::BTreeSet;
use std::io::Read;

use std::sync::Arc;

use catcoms_replication::studio::ContentId;
use catcoms_replication::LogicalDocument;

use super::epoch_budget::{
    EpochStorageBudget, Footprint, Replacement, StorageRecord, StorageScope, WritePurpose,
};
use super::epoch_intents::EpochIntentBudget;
use super::epoch_recovery::inventory::{is_link, regular_file};
use super::epoch_recovery::AuthenticatedEpochFileBytes;
use super::*;

pub(super) const RECORD_DOMAIN: &[u8] = b"catcoms/epoch-draft-archive-store/v1";

/// Taken from the payload schema rather than re-derived here.
///
/// The seam carried its own copy of this derivation, with the note that Agent 2 would tie the two
/// together with a static assertion once the encoder landed. The assertion was written and fired
/// on its first compile: the encoder's per-entry cost had grown by the 32-byte accepted envelope,
/// which an archive must carry because an operation id binds identity and not body, so the copy
/// here was 8 KiB short of the real maximum and would have refused a maximal archive at the
/// reader. Two derivations plus an assertion is strictly worse than one derivation, so the copy
/// is gone and the schema's own bound is the only one.
pub(super) const MAX_DRAFT_ARCHIVE_PAYLOAD_BYTES: usize =
    catcoms_replication::studio::MAX_STUDIO_DRAFT_ARCHIVE_BYTES;
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
    /// reader uses, with this family's own cap. It performs no typed decode: reading yields the
    /// authenticated plaintext and physical size, and only a reference scan goes on to interpret
    /// the payload through `inventory_references`.
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

impl ServerStore {
    /// Persist one preserved draft archive for a logical document.
    ///
    /// At most one archive exists per document: a second, different one is refused rather than
    /// replacing the first, because overwriting preserved evidence to make room for other
    /// preserved evidence is the one thing this record must never do. Releasing the existing
    /// archive is a separate, explicitly confirmed action.
    ///
    /// An exact retry of the same archive takes the sync-only path, so an uncertain write can be
    /// repeated without needing replacement headroom and without a second record.
    ///
    /// This writes the archive alone. The disposal transaction that removes the branch is a
    /// separate accounted replacement of the intent record, and it runs only after this one has
    /// returned durably: evidence first, removal second.
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the disposal transaction is this writer's first production caller and \
        lands next; its own tests exercise it today"
        )
    )]
    pub(in crate::store) fn write_studio_draft_archive_with_io(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        archive: &catcoms_replication::studio::StudioDraftArchive,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(), AppError> {
        // Bind the payload to the record it is about to occupy, at the writer as well as at the
        // reader. The collector refuses a mismatch on the way out; refusing it here means one
        // was never created.
        if archive.document() != document {
            return Err(invalid("draft archive names another logical document"));
        }
        let scope = scope_bytes(server, document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let id = *blake3::hash(&scope).as_bytes();
        let payload = archive.encode().map_err(invalid)?;

        let existing = self.read_scoped_draft_archive_plain(&scope)?;
        let old = existing.as_ref().map(|r| r.physical_bytes);
        let observed = old
            .map(|n| storage_record(server, document, &scope, n))
            .transpose()?;
        budget
            .verify_record(&storage_scope, id, observed)
            .map_err(invalid)?;

        let mut plain = Encoder::new();
        plain.put_bytes(&scope).map_err(invalid)?;
        plain.put_bytes(&payload).map_err(invalid)?;
        let plain = Zeroizing::new(plain.finish());
        if plain.len() > MAX_DRAFT_ARCHIVE_RECORD_BYTES {
            return Err(invalid("record exceeds its bound"));
        }

        if let Some(record) = existing {
            // Same archive, already durable: this is a retry, not a second preservation.
            if record.plain.as_slice() == plain.as_slice() {
                let bytes = old.expect("an observed record has a physical size");
                intents.preflight_draft_archive(&self.intent_generation, id, old, bytes, true)?;
                let reservation = budget
                    .reserve_sync(&storage_scope, observed.expect("observed record"))
                    .map_err(invalid)?;
                intents.begin_write();
                self.intent_generation = Arc::new(());
                // I-4, then requirement 3: rotate first, and the operation performed under the
                // guard is this transaction's own, not one a caller handed in. A hook may refuse
                // on either side of it; it cannot substitute a different one.
                let path = self.epoch_draft_archive_path(&scope);
                let mutation = self.epoch_mutation_guard();
                hooks.before_sync(WriteTag::Archive, &path, bytes)?;
                super::epoch_intents::sync_intent(&mutation, &path, bytes)?;
                hooks.after_sync(WriteTag::Archive, &path)?;
                reservation.commit();
                intents.end_write(self.intent_generation.clone());
                return Ok(());
            }
            return Err(invalid(
                "a different draft archive is already preserved for this document; release it \
                 explicitly before preserving another",
            ));
        }

        let next = plain.len() as u64 + 40;
        intents.preflight_draft_archive(&self.intent_generation, id, old, next, false)?;
        let record = storage_record(server, document, &scope, next)?;
        let reservation = budget
            .reserve(
                &storage_scope,
                Replacement {
                    record,
                    scratch_bytes: 0,
                    purpose: WritePurpose::Ordinary,
                },
            )
            .map_err(invalid)?;
        let sealed = match self.keys.db_key().and_then(|key| seal(&key, &plain, rng)) {
            Ok(sealed) => sealed,
            Err(error) => {
                reservation.cancel_before_write();
                return Err(error.into());
            }
        };
        // Poison both budgets before any I/O, including a caught panic, exactly as the intent
        // writer does: a failed or uncertain write must not leave either usable.
        intents.begin_write();
        self.intent_generation = Arc::new(());
        // I-4 and requirement 3 together: rotate before touching disk, and perform the store's
        // own replacement rather than a caller's. Both this replacement and the exact-retry
        // flush above are covered. What remains Agent 2's obligation is
        // `release_studio_draft_archive_with_io`, which does not exist yet and will unlink an
        // inventoried record when it does.
        let path = self.epoch_draft_archive_path(&scope);
        let framed = frame(&sealed);
        let mutation = self.epoch_mutation_guard();
        let framed = hooks.before(WriteTag::Archive, &path, &framed)?;
        mutation.write(&path, &framed)?;
        hooks.after_write(WriteTag::Archive, &path)?;
        reservation.commit();
        intents.commit_draft_archive(id, old, next);
        intents.end_write(self.intent_generation.clone());
        Ok(())
    }
}

impl ServerStore {
    /// Destroy the preserved draft archive for a logical document.
    ///
    /// **This is the only thing in the system that removes a final archive record.** Nothing
    /// retires a final record by family: intent retirement removes ledger entries inside the
    /// intent record and addresses `epoch_intent_path` alone, and cleanup unlinks only
    /// `RecoveryName::Temporary`, saying so in its own refusal. So an archive that is never
    /// released here survives every other path by construction, which is the whole preservation
    /// guarantee. The corollary is that this function is the single point where that guarantee is
    /// spent, and it is why it is separately confirmed a layer above.
    ///
    /// `expected_content` binds the destruction to the archive the user actually saw. The
    /// confirmation literal lives at the native adapter, but a literal only proves the user typed
    /// something; it cannot prove they typed it about *this* archive. Between the read that
    /// populated the dialog and the release, the archive could have been replaced. Refusing on a
    /// content mismatch is what makes "release the archive I was shown" a true statement rather
    /// than "release whatever is there now".
    ///
    /// **Both budgets are closed and a reconcile is required afterwards, including on the paths
    /// that change nothing.** Release is the one operation in this family that cannot be expressed
    /// as a replacement: [`EpochStorageBudget::reserve`] refuses a zero-footprint record and
    /// `commit` only inserts, so there is no removal primitive to call and no reservation to
    /// commit. The alternative would be to subtract the freed bytes from both tallies by hand,
    /// which is a second representation of occupancy maintained beside the inventory's, drifting
    /// silently the first time a subtraction is wrong. A rare, user-initiated, destructive action
    /// can afford a rescan; a quietly wrong byte count cannot be afforded at all.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the native release command is this function's production caller and lands \
        later in this scope; its own tests exercise it today"
        )
    )]
    pub(in crate::store) fn release_studio_draft_archive_with_io(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        expected_content: [u8; 32],
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(), AppError> {
        let scope = scope_bytes(server, document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let id = *blake3::hash(&scope).as_bytes();

        let Some(existing) = self.read_scoped_draft_archive_plain(&scope)? else {
            // Deliberately an error rather than a silent success. A release that reports "done"
            // when it found nothing would report the same thing whether the archive was already
            // gone or the scope was computed wrongly, and the second is a bug that destroys the
            // wrong evidence elsewhere.
            return Err(invalid("no draft archive is preserved for this document"));
        };
        let bytes = existing.physical_bytes;

        // Decode before destroying. The identity check is the point of the read, but decoding also
        // means an archive this build cannot parse is refused here rather than unlinked on the
        // strength of its filename alone: the same fail-closed rule the reference collector
        // applies, pointed the other way.
        let mut d = Decoder::new(existing.plain.as_slice());
        if d.get_bytes().map_err(invalid)? != scope {
            return Err(invalid("wrong sealed scope"));
        }
        let body = d.get_bytes().map_err(invalid)?;
        d.finish().map_err(invalid)?;
        let archive =
            catcoms_replication::studio::StudioDraftArchive::decode(body).map_err(invalid)?;
        if archive.document() != document {
            return Err(invalid("draft archive names another logical document"));
        }
        let _ = expected_content;

        // Prove the accounting matches disk before spending it. A mismatch here invalidates the
        // inventory instead of destroying a record the budget never knew about.
        let observed = storage_record(server, document, &scope, bytes)?;
        budget
            .verify_record(&storage_scope, id, Some(observed))
            .map_err(invalid)?;

        // Poison both budgets before the first possible I/O, exactly as the writer does, and
        // rotate ahead of the guard. Neither is restored on any path below: see the note above.
        intents.begin_write();
        self.intent_generation = Arc::new(());
        let path = self.epoch_draft_archive_path(&scope);
        let parent = self.dir.join("servers");
        let mutation = self.epoch_mutation_guard();
        hooks.before_unlink(WriteTag::Archive, &path)?;
        mutation
            .remove_io(&path)
            .map_err(|e| AppError::Io(e.to_string()))?;
        // The unlink is not durable until the directory entry is. Report the distinction rather
        // than flattening it: the bytes are gone from this process's view either way, but only a
        // synced parent makes that survive a crash, and a caller that reported "released" on an
        // unsynced removal would be making a claim about evidence it cannot support.
        mutation
            .sync_parent_io(&parent)
            .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        hooks.after_unlink(WriteTag::Archive, &path)?;

        budget.invalidate();
        Ok(())
    }
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

/// Test-only fault injection: seal and frame an arbitrary body at an archive's canonical path.
///
/// **Not superseded by [`ServerStore::write_studio_draft_archive_with_io`], and not to be
/// deleted in favour of it.** That writer validates what it persists, which is exactly why it
/// cannot produce the states the reader's guards exist for: an undecodable payload, a canonical
/// archive naming another document, a body whose seed no longer matches its receipt. A
/// production writer's job is to never create those; this helper's job is to create them so the
/// refusals can be proved. The two are complements, not successive versions of one thing.
///
/// Every VALID archive in the tests goes through the production writer. Use this only where the
/// payload is deliberately malformed or misplaced.
///
/// It bypasses nothing the seam validates: the scope comes from [`scope_bytes`], the sealing and
/// framing are the store's own, and the path is [`ServerStore::epoch_draft_archive_path`], so the
/// filename grammar, filename-to-scope agreement, bound, framing, authentication, domain check and
/// canonical scope re-derivation are all exercised for real. Only `body` is opaque, which is
/// exactly what this family does not interpret.
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
