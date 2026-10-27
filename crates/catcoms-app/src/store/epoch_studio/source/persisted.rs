//! The one way to prove that the source an H5 handoff commit wrote is the candidate it meant to
//! write, without restoring it (design 9.1, and 9.1.1 step 3).
//!
//! H5 used to restore the destination source after writing it, so the resolution that writes
//! Completed could classify the persisted bytes. That restore is driven by the source's
//! structure, about 240 ms for 128 frames, and C-3 runtime design 15.7 makes removing it the first
//! prerequisite of C-3 step 3. The writer already returns the digest and size of exactly the
//! plaintext it encoded. This module re-reads what landed, requires it to match, and requires the
//! landed snapshot to be the candidate's. Only then may resolve classify from the candidate unit
//! the commit already holds.

use super::*;

/// A destination source the handoff commit wrote and then re-read, proved byte-identical to the
/// candidate unit it carries.
///
/// **Holding one is the proof.** Its fields are private and its only constructor is
/// [`ServerStore::verify_persisted_studio_source`], so nothing outside this module can build one
/// from a worker's claim. It binds the mount, the numeric server, the complete target, the version
/// the writer returned, the candidate unit, the landed snapshot, and the inventory generation it
/// was verified under (amendment A3). So it cannot be spent after another write.
///
/// **How the unit is tied to the bytes.** The constructor does not re-encode the unit; that would
/// cost a full snapshot encode. It relies on where an `EpochStudioState` comes from. Only the
/// store builds one: its writer, its loader, and `preparation.rs`. Each derives the version from
/// that unit's own encoded plaintext **at construction**. A holder of `&mut EpochStudioState`
/// can still change the unit afterwards, as `with_prepared_studio_source`'s closure and
/// `ingest_studio_epoch_reusing` do. So the pairing is guaranteed only for a state passed here
/// as the writer returned it, which is what H5 does. In H5, barrier 2 has also just checked
/// `hash(unit.snapshot())` against the snapshot hash this constructor requires. And a mismatched
/// unit would in any case meet resolve's flush-only fence, which compares the landed snapshot
/// with the unit's before anything is written.
pub(in crate::store) struct VerifiedPersistedSource {
    server: u64,
    target: StudioTarget,
    generation: Arc<()>,
    version: SourceVersion,
    unit: StudioEpoch,
    /// The snapshot bytes that actually landed, proved equal to the candidate's encoding.
    snapshot: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for VerifiedPersistedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VerifiedPersistedSource { .. }")
    }
}

impl ServerStore {
    /// Re-read the source the handoff commit just wrote, and require it to be exactly the
    /// candidate (design 9.1, "After the Source write").
    ///
    /// `written` is what `save_studio_source_checked` returned. `candidate_snapshot` is the hash
    /// barrier 2 already proved equal to the hash of the candidate's own snapshot (the write
    /// capability's `source`). Every check is against bytes read back under this borrow:
    ///
    /// - the writer returned a version, which it does only for a replacement, the one branch H5
    ///   takes;
    /// - the version names this mount, this server and the target;
    /// - the landed record's physical size and plaintext digest are the version's, so what landed
    ///   is what the writer encoded;
    /// - the landed record's channel is the target's, and the hash of its snapshot is the
    ///   candidate's, so the unit carried here is the unit those bytes describe;
    /// - the source-to-intent link is valid, as `checked_studio_source` would require.
    ///
    /// The digest overlaps the field checks, except at one byte: the link check accepts an
    /// **unlinked** record, and dropping the link leaves the snapshot unchanged. So a destination
    /// whose link was dropped is refused here only by the size-and-digest comparison (design 18.3
    /// review, F7; `studio_overlay_handoff_refuses_a_persisted_source_whose_link_was_dropped`).
    ///
    /// The re-read is bounded by the family's sealed cap, not the 8 MiB retained-source bound, so a
    /// successor that has grown past 8 MiB is still verified rather than refused.
    ///
    /// **Any error is a verification failure,** including a re-read that is missing or does not
    /// authenticate. The caller must then refuse the commit with Prepared retained.
    pub(in crate::store) fn verify_persisted_studio_source(
        &self,
        server: u64,
        target: StudioTarget,
        written: EpochStudioState,
        candidate_snapshot: [u8; 32],
    ) -> Result<VerifiedPersistedSource, AppError> {
        let EpochStudioState { unit, source } = written;
        let version =
            source.ok_or_else(|| invalid("the handoff source write produced no new version"))?;
        if !Arc::ptr_eq(&version.mount, &self.registry_mount())
            || version.server != server
            || unit.target() != target
        {
            return Err(invalid(
                "the written handoff source belongs to another mount, server or target",
            ));
        }
        let document = unit.document().clone();
        let scope = scope_bytes(server, &document)?;
        let landed = self
            .read_studio_record(&scope)?
            .ok_or_else(|| invalid("the written handoff source is missing"))?;
        if landed.physical_bytes != version.bytes || blake3::hash(&landed.plain) != version.digest {
            return Err(invalid(
                "the written handoff source differs from what the writer encoded",
            ));
        }
        let (stored, snapshot) = decode_record(&landed.plain, &scope, &document)?;
        if stored != target || *blake3::hash(snapshot).as_bytes() != candidate_snapshot {
            return Err(invalid("the written handoff source is not the candidate"));
        }
        self.check_studio_intent_link(server, &document, &scope, &landed.plain)?;
        let snapshot = Zeroizing::new(snapshot.to_vec());
        Ok(VerifiedPersistedSource {
            server,
            target,
            generation: self.inventory_generation.clone(),
            version,
            unit,
            snapshot,
        })
    }
}

impl VerifiedPersistedSource {
    /// Spend the proof where resolve would otherwise call `checked_studio_source`, returning the
    /// same `(unit, observed, before)` without a restore.
    ///
    /// The bindings are rechecked against the store as it is now: the same mount, server, target
    /// and group, and **no five-family write since verification** (the inventory generation).
    /// The budget's own view of the record is verified as `checked_studio_source` verifies it.
    /// Any failure spends the budget, because the commit's reservations described these bytes.
    pub(in crate::store) fn into_checked(
        self,
        store: &ServerStore,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        budget: &mut EpochStorageBudget,
    ) -> Result<CheckedStudioSource, AppError> {
        let bound = Arc::ptr_eq(&self.version.mount, &store.registry_mount())
            && self.server == server
            && self.target == target
            && self.unit.document().server_id == group.group_id()
            && Arc::ptr_eq(&self.generation, &store.inventory_generation);
        if !bound {
            budget.invalidate();
            return Err(invalid(
                "the verified handoff source no longer describes this store",
            ));
        }
        let scope = scope_bytes(server, self.unit.document())?;
        let checked = StorageScope::new(server, &self.unit.document().server_id)
            .map_err(invalid)
            .and_then(|storage| {
                budget
                    .verify_record(
                        &storage,
                        *blake3::hash(&scope).as_bytes(),
                        Some(self.version.record),
                    )
                    .map_err(invalid)
            });
        if let Err(error) = checked {
            budget.invalidate();
            return Err(error);
        }
        Ok((self.unit, Some(self.version.record), self.snapshot))
    }
}
