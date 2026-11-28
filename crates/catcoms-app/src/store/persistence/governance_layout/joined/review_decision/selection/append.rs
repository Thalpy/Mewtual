//! Encrypted write-ahead data for one later decision. The reader checks
//! both changed index paths against the selected old head and exact record.
use super::index::{
    append::{ReviewIndexDelta, MAX_INDEX_APPEND_BYTES},
    update::DecisionIndexUpdate,
    DecisionIndexQuery, DecisionIndexes,
};
use super::*;
use crate::store::persistence::v2_member_outbox::fs::MAX_FILE;
use crate::store::persistence::v2_text::private_index::ReviewIndexValueData;

pub(in super::super) const APPEND_FRAME: &[u8; 8] = b"CTV2RVJ\x03";
const APPEND: &[u8] = b"catcoms/private-chat-v2-review-journal-selection/v3\0";
pub(in super::super) const MAX_APPEND_BYTES: usize = WAL_HEADER
    + SEAL_BYTES
    + 4
    + APPEND.len()
    + 3 * DIGEST_FIELD
    + 3 * 4
    + 2 * HEAD_BYTES
    + MAX_RECORD_BYTES
    + MAX_INDEX_APPEND_BYTES;
const _: () = assert!(MAX_APPEND_BYTES <= MAX_FILE);

pub(in super::super) struct IndexedAppend<'a> {
    controls: [[u8; 32]; 3],
    pub(in super::super) predecessor: &'a [u8],
    pub(in super::super) record: &'a [u8],
    pub(in super::super) head: &'a [u8],
    pub(super) changes: [ReviewIndexDelta<'a>; 2],
}

impl<'a> IndexedAppend<'a> {
    pub(super) fn incomplete_scratch(frame: &[u8]) -> bool {
        if frame.len() < APPEND_FRAME.len() {
            return APPEND_FRAME.starts_with(frame);
        }
        if !frame.starts_with(APPEND_FRAME) {
            return false;
        }
        if frame.len() < WAL_HEADER {
            return true;
        }
        let Ok(size) = <[u8; 4]>::try_from(&frame[APPEND_FRAME.len()..WAL_HEADER]) else {
            return false;
        };
        let size = u32::from_be_bytes(size) as usize;
        size > frame.len() && (WAL_HEADER + SEAL_BYTES + 1..=MAX_APPEND_BYTES).contains(&size)
    }

    pub(in super::super) fn seal(
        key: &JournalKey,
        controls: [[u8; 32]; 3],
        (predecessor, record, head): (&[u8], &[u8], &[u8]),
        changes: &[DecisionIndexUpdate; 2],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        require_frames(predecessor, record, head)?;
        let mut plain = Encoder::with_capacity(MAX_APPEND_BYTES - WAL_HEADER - SEAL_BYTES);
        let [intent, attempt, raw] = controls;
        for field in [APPEND, &intent, &attempt, &raw, predecessor, record, head] {
            plain.put_bytes(field).map_err(|_| denied())?;
        }
        for change in changes {
            ReviewIndexDelta::encode(change, &mut plain)?;
        }
        if plain.len() > MAX_APPEND_BYTES - WAL_HEADER - SEAL_BYTES {
            return Err(denied());
        }
        let sealed = key.seal(plain, rng)?;
        let size = WAL_HEADER.checked_add(sealed.len()).ok_or_else(denied)?;
        let length = u32::try_from(size).map_err(|_| denied())?;
        let mut frame = Zeroizing::new(Vec::with_capacity(size));
        frame.extend_from_slice(APPEND_FRAME);
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(&sealed);
        Ok(frame)
    }

    pub(in super::super) fn open(
        key: &JournalKey,
        frame: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if !(WAL_HEADER + SEAL_BYTES + 1..=MAX_APPEND_BYTES).contains(&frame.len())
            || !frame.starts_with(APPEND_FRAME)
        {
            return Err(denied());
        }
        let length = u32::from_be_bytes(
            frame[APPEND_FRAME.len()..WAL_HEADER]
                .try_into()
                .map_err(|_| denied())?,
        );
        if length as usize != frame.len() {
            return Err(denied());
        }
        let plain = key.open(&frame[WAL_HEADER..])?;
        if read_field(&mut Decoder::new(&plain))? != APPEND {
            return Err(denied());
        }
        Ok(plain)
    }

    pub(in super::super) fn decode(plain: &'a [u8]) -> Result<Self, AppError> {
        if plain.len() > MAX_APPEND_BYTES - WAL_HEADER - SEAL_BYTES {
            return Err(denied());
        }
        let mut reader = Decoder::new(plain);
        if read_field(&mut reader)? != APPEND {
            return Err(denied());
        }
        let controls = [
            digest(&mut reader)?,
            digest(&mut reader)?,
            digest(&mut reader)?,
        ];
        let predecessor = read_field(&mut reader)?;
        let record = read_field(&mut reader)?;
        let head = read_field(&mut reader)?;
        require_frames(predecessor, record, head)?;
        let changes = [
            ReviewIndexDelta::decode(&mut reader)?,
            ReviewIndexDelta::decode(&mut reader)?,
        ];
        reader.finish().map_err(|_| denied())?;
        Ok(Self {
            controls,
            predecessor,
            record,
            head,
            changes,
        })
    }

    pub(in super::super) fn require_controls(
        &self,
        controls: [[u8; 32]; 3],
    ) -> Result<(), AppError> {
        if self.controls != controls {
            return Err(denied());
        }
        Ok(())
    }

    pub(in super::super) fn require_remaining_controls(
        &self,
        frames: [Option<&[u8]>; 3],
    ) -> Result<(), AppError> {
        require_remaining_controls(self.controls, frames)
    }

    pub(in super::super) fn verify_committed(
        &self,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
    ) -> Result<(DecisionHead, ReviewIndexValueData), AppError> {
        let old = DecisionHead::open(key, self.predecessor)?;
        old.roots.ok_or_else(denied)?;
        let selected = DecisionHead::open(key, self.head)?;
        let roots = selected.roots.ok_or_else(denied)?;
        let plain = RetainedDecisionRecord::open(key, self.record)?;
        let record = RetainedDecisionRecord::decode(pin, &plain)?;
        let entry = record.record.entry_data();
        record.verify_raw_predecessor(self.controls[2])?;
        selected.require_record(entry, self.record)?;
        selected.require_predecessor(entry, Some((old, self.predecessor)))?;
        for (change, root) in self.changes.iter().zip(roots) {
            if change.outputs.last().map(|(node, _)| *node) != Some(root) {
                return Err(denied());
            }
        }
        Ok((selected, DecisionIndexes::row(&record, self.record)))
    }

    pub(in super::super) fn verify_next(
        &self,
        key: &JournalKey,
        pin: &FounderGenesisAnchorData,
        (predecessor, tip): (&[u8], &ReviewStreamEntry),
        (entry, request): (&ReviewStreamEntry, [u8; 32]),
        prior_case: Option<(ReviewIndexValueData, &ExactReviewDecisionBody)>,
        mut read: impl FnMut(
            catcoms_storage::private_chat_page::PrivateChatPageId,
        ) -> Result<Zeroizing<Vec<u8>>, AppError>,
    ) -> Result<DecisionHead, AppError> {
        if self.predecessor != predecessor {
            return Err(denied());
        }
        let old = DecisionHead::open(key, predecessor)?;
        let old_roots = old.roots.ok_or_else(denied)?;
        if tip.position_data().sequence != old.sequence || tip.id().as_bytes() != &old.entry {
            return Err(denied());
        }
        let selected = DecisionHead::open(key, self.head)?;
        let roots = selected.roots.ok_or_else(denied)?;
        let plain = RetainedDecisionRecord::open(key, self.record)?;
        let record = RetainedDecisionRecord::decode(pin, &plain)?;
        if record.record.entry_data() != entry || record.request_digest != request {
            return Err(denied());
        }
        record.verify_raw_predecessor(self.controls[2])?;
        record
            .record
            .decision_data()
            .verify_new_transition_from(prior_case.as_ref().map(|(_, decision)| *decision))
            .map_err(|_| denied())?;
        entry
            .verify_next_for_pin(pin, Some(tip))
            .map_err(|_| denied())?;
        selected.require_record(entry, self.record)?;
        selected.require_predecessor(entry, Some((old, predecessor)))?;

        let value = DecisionIndexes::row(&record, self.record);
        let queries = [
            (DecisionIndexQuery::Intent(value.intent), None),
            (
                DecisionIndexQuery::Case(value.case),
                prior_case.map(|(prior, _)| prior),
            ),
        ];
        let indexes = key.indexes();
        for (((source, selected), (query, prior)), change) in old_roots
            .into_iter()
            .zip(roots)
            .zip(queries)
            .zip(&self.changes)
        {
            change.require_delta(
                &indexes,
                (source, selected, query),
                (prior, value),
                &mut read,
            )?;
        }
        Ok(selected)
    }
}

fn require_frames(predecessor: &[u8], record: &[u8], head: &[u8]) -> Result<(), AppError> {
    if predecessor.len() != HEAD_BYTES
        || head.len() != HEAD_BYTES
        || !(SEAL_BYTES + 1..=MAX_RECORD_BYTES).contains(&record.len())
    {
        return Err(denied());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::persistence::governance_layout::joined::review_decision::selection::tests::signed_case_fixture;
    use crate::store::NativeTextInitialReviewRequest;
    use catcoms_crypto::DeviceKeypair;
    use catcoms_replication::chat::owner_bodies::{
        ReviewDecision, ReviewDecisionBody, ReviewStreamPosition, UnsignedReviewStreamEntry,
    };
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn sign(
        founder: &DeviceKeypair,
        pin: &FounderGenesisAnchorData,
        position: ReviewStreamPosition,
        decision: &ExactReviewDecisionBody,
    ) -> ReviewStreamEntry {
        let unsigned = UnsignedReviewStreamEntry::prepare(pin, position, decision).unwrap();
        let signature = founder.sign(unsigned.signing_bytes());
        unsigned.attach_signature(pin, signature).unwrap()
    }

    #[test]
    fn review_index_append_replays_exact_pages_and_requires_the_selected_case() {
        let mut rng = ChaCha20Rng::seed_from_u64(735);
        let key = JournalKey::new(&[35; 32], &[36; 32]);
        let (founder, pin, first_position, first, held) =
            signed_case_fixture(ReviewDecision::Reject);
        let tip = sign(&founder, &pin, first_position, &first);
        let old_record = RetainedDecisionRecord::prepare(
            &key,
            &pin,
            (&tip, &first, &held),
            (*first.id().as_bytes(), [37; 32]),
            &mut rng,
        )
        .unwrap();
        let plain = RetainedDecisionRecord::open(&key, &old_record).unwrap();
        let retained = RetainedDecisionRecord::decode(&pin, &plain).unwrap();
        let indexes = key.indexes();
        let old_pages = indexes
            .prepare_first(&retained, &old_record, &mut rng)
            .unwrap();
        let old_roots = old_pages.each_ref().map(|page| page.root);
        let old_head = DecisionHead::from_record(&tip, &old_record, None, old_roots).unwrap();
        let predecessor = old_head.seal(&key, &mut rng).unwrap();
        let original_case = DecisionIndexes::row(&retained, &old_record);
        let temp = tempfile::tempdir().unwrap();
        let root = HeldDirectory::open(temp.path().to_path_buf()).unwrap();
        std::fs::write(root.child(disk::HEAD_CURRENT), &predecessor).unwrap();
        let records = root.child(disk::RECORD_DIRECTORY);
        std::fs::create_dir(&records).unwrap();
        std::fs::write(records.join("0000000000000001.record"), &old_record).unwrap();
        for page in &old_pages {
            let name = format!("{}.page", hex::encode(page.page.id().as_bytes()));
            std::fs::write(records.join(name), page.page.as_bytes()).unwrap();
        }
        let mut snapshot = disk::indexed::IndexedDecisionSnapshot::read(&root, &key, &pin).unwrap();
        assert_eq!(snapshot.selected_data().1, predecessor.as_slice());
        assert_eq!(snapshot.selected_data().2, &tip);
        let lookup = snapshot
            .lookup(&root, &key, &pin, DecisionIndexQuery::Case(first.case_id()))
            .unwrap();
        let prior = lookup.record.unwrap();
        assert_eq!(prior.row, original_case);
        assert_eq!(prior.decision, first);

        let mut spec = first.decision().spec().clone();
        spec.intent = [38; 16];
        spec.decision = ReviewDecision::Reopen;
        spec.previous = Some(first.id());
        let next = ExactReviewDecisionBody::new(
            ReviewDecisionBody::new(spec).unwrap(),
            first.original_data(),
        )
        .unwrap();
        let position = ReviewStreamPosition {
            sequence: 2,
            previous: Some(tip.id()),
            ..first_position
        };
        let entry = sign(&founder, &pin, position, &next);
        let request = *next.id().as_bytes();
        let original_target = first.decision().spec().target.spec();
        let request_for = |decision: &ExactReviewDecisionBody| {
            let spec = decision.decision().spec();
            NativeTextInitialReviewRequest::new(
                original_target.channel,
                (
                    decision.case_id(),
                    original_target.action,
                    decision.original_data(),
                ),
                spec.intent,
                spec.decision,
            )
        };
        assert!(snapshot
            .check_request(&root, &key, &pin, &request_for(&next))
            .is_err());
        let successor = request_for(&next).for_successor(first.id());
        let disk::indexed::IndexedReviewRequest::Next(mut preflight) = snapshot
            .check_request(&root, &key, &pin, &successor)
            .unwrap()
        else {
            panic!("the next intent is still absent");
        };
        preflight.require_current().unwrap();
        let (mut signing_head, next_sequence) = snapshot.signing_base(&root).unwrap();
        signing_head.require().unwrap();
        assert_eq!(signing_head.bytes.as_slice(), predecessor.as_slice());
        assert_eq!(next_sequence, position.sequence);
        assert_eq!(
            snapshot
                .next_position((position.membership_header, position.membership_epoch))
                .unwrap(),
            position,
        );
        let next_case = snapshot
            .lookup(
                &root,
                &key,
                &pin,
                DecisionIndexQuery::Intent(next.decision().spec().intent),
            )
            .unwrap();
        assert!(next_case.record.is_none());
        let controls = [[39; 32], [40; 32], [41; 32]];
        let record = RetainedDecisionRecord::prepare(
            &key,
            &pin,
            (&entry, &next, &held),
            (request, controls[2]),
            &mut rng,
        )
        .unwrap();
        let plain = RetainedDecisionRecord::open(&key, &record).unwrap();
        let data = RetainedDecisionRecord::decode(&pin, &plain).unwrap();
        let replacement = DecisionIndexes::row(&data, &record);
        let read = |id| {
            let page = old_pages
                .iter()
                .find(|page| page.page.id() == id)
                .ok_or_else(denied)?;
            Ok(Zeroizing::new(page.page.as_bytes().to_vec()))
        };
        let changes = [
            indexes
                .prepare_update(
                    old_roots[0],
                    DecisionIndexQuery::Intent(replacement.intent),
                    (None, replacement),
                    read,
                    &mut rng,
                )
                .unwrap(),
            indexes
                .prepare_update(
                    old_roots[1],
                    DecisionIndexQuery::Case(replacement.case),
                    (Some(original_case), replacement),
                    read,
                    &mut rng,
                )
                .unwrap(),
        ];
        let roots = changes.each_ref().map(|change| change.root);
        let head =
            DecisionHead::from_record(&entry, &record, Some((old_head, &predecessor)), roots)
                .unwrap();
        let sealed_head = head.seal(&key, &mut rng).unwrap();
        let sealed = IndexedAppend::seal(
            &key,
            controls,
            (&predecessor, &record, &sealed_head),
            &changes,
            &mut rng,
        )
        .unwrap();
        let mut wrong_format = sealed.to_vec();
        wrong_format[7] = 2;
        assert!(IndexedAppend::open(&key, &wrong_format).is_err());
        let opened = IndexedAppend::open(&key, &sealed).unwrap();
        let append = IndexedAppend::decode(&opened).unwrap();
        append.require_controls(controls).unwrap();
        assert!(append.require_controls([[42; 32]; 3]).is_err());
        assert!(
            append
                .verify_next(
                    &key,
                    &pin,
                    (&predecessor, &tip),
                    (&entry, request),
                    preflight.case_data(),
                    read,
                )
                .unwrap()
                == head
        );
        assert!(append
            .verify_next(
                &key,
                &pin,
                (&predecessor, &tip),
                (&entry, request),
                None,
                read,
            )
            .is_err());
        let wrong_case = ReviewIndexValueData {
            record: [43; 32],
            ..original_case
        };
        assert!(append
            .verify_next(
                &key,
                &pin,
                (&predecessor, &tip),
                (&entry, request),
                Some((wrong_case, &first)),
                read,
            )
            .is_err());
        assert!(append
            .verify_next(
                &key,
                &pin,
                (&sealed_head, &tip),
                (&entry, request),
                Some((original_case, &first)),
                read,
            )
            .is_err());
        let mut cover = || {
            disk::cover_indexed_append(
                &root,
                position.sequence,
                |bytes| {
                    snapshot.require_current(&root)?;
                    preflight.require_current()?;
                    if let Some(bytes) = bytes {
                        let plain = IndexedAppend::open(&key, bytes)?;
                        let check = IndexedAppend::decode(&plain)?;
                        check.require_controls(controls)?;
                        check.verify_next(
                            &key,
                            &pin,
                            (&predecessor, &tip),
                            (&entry, request),
                            preflight.case_data(),
                            read,
                        )?;
                    }
                    Ok(())
                },
                || Ok(sealed.clone()),
            )
        };
        let (stopped, fault) = crate::store::persistence::governance_layout::joined::review_decision::with_review_control_fault(
            crate::store::persistence::governance_layout::joined::review_decision::ReviewControlFault::ShortSelectionScratch,
            &mut cover,
        );
        assert!(stopped.is_err() && fault);
        assert!(!root.child(disk::SELECTION_WAL).exists());
        let mut wal = cover().unwrap();
        snapshot
            .select_append(&root, &mut wal, &append, head, || Ok(()))
            .unwrap();

        let mut reopened = disk::indexed::IndexedDecisionSnapshot::read(&root, &key, &pin).unwrap();
        assert_eq!(reopened.selected_data().2, &entry);
        let old_outcome = reopened
            .lookup(
                &root,
                &key,
                &pin,
                DecisionIndexQuery::Intent(original_case.intent),
            )
            .unwrap()
            .record
            .unwrap();
        assert_eq!(old_outcome.entry, tip);
        assert_eq!(old_outcome.decision, first);
        let new_case = reopened
            .lookup(&root, &key, &pin, DecisionIndexQuery::Case(next.case_id()))
            .unwrap()
            .record
            .unwrap();
        assert_eq!(new_case.entry, entry);
        assert_eq!(new_case.decision, next);
        let first_request = request_for(&first);
        let disk::indexed::IndexedReviewRequest::Saved(old) = reopened
            .check_request(&root, &key, &pin, &first_request)
            .unwrap()
        else {
            panic!("retrying the already indexed old Reject must return its result");
        };
        assert_eq!(old.record.unwrap().entry, tip);
    }
}
