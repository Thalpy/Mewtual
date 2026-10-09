//! Design 9.1, "No graph restore on the commit path", and its implementation plan 9.1.1.
//!
//! H5 now takes the destination's accounting facts from H2 behind the stamp instead of restoring
//! the source before its write. After the write it proves the persisted record is the candidate
//! instead of restoring it again. These tests pin the absence of those restores, the proof's
//! refusals, and that the restart path still restores exactly once.
use super::*;
use crate::store::epoch_studio::source::studio_full_restores_for_test;

/// H1 to H4 for the fixture's branch, returning H4's commit. Nothing durable is written.
fn staged(f: &Fixture, store: &mut ServerStore, basis: [u8; 32]) -> StudioHandoffCommit {
    let mut b = budget(store, f);
    let start = store
        .start_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::None,
        )
        .expect("H1 refused the fixture branch");
    let crate::store::StudioHandoffStart::Captured(capture) = start else {
        panic!("H1 settled instead of capturing, so there is no H5 to test")
    };
    let mut plan = capture.prepare().expect("H2 reconstructs the candidate");
    assert!(
        plan.sign_slice(&f.device, &f.group, 0, false, usize::MAX, None)
            .unwrap()
            .complete(),
        "H3 did not sign the whole branch"
    );
    plan.assemble().expect("H4 assembles the candidate")
}

/// H5 alone, counting the full restores it performs.
fn commit_counting(
    f: &Fixture,
    store: &mut ServerStore,
    commit: StudioHandoffCommit,
    hooks: &mut WriteHooks<'_>,
) -> (Result<StudioHandoffOutcome, AppError>, usize) {
    let mut b = budget(store, f);
    let before = studio_full_restores_for_test();
    let outcome = store.commit_studio_handoff_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        commit,
        Some(0),
        &mut rng(),
        &mut b,
        hooks,
    );
    (outcome, studio_full_restores_for_test() - before)
}

/// The two Flipnotes [`index_with_two_references`] names.
const REFERENCED: [[u8; 16]; 2] = [[16; 16], [17; 16]];

/// An Index branch whose two PutObjects name two existing Flipnotes, plus those Flipnotes. Before
/// 9.1, H5 restored each of them once.
fn index_with_two_references(f: &Fixture, store: &mut ServerStore) -> [u8; 32] {
    let (close, basis) = closing(f, store);
    for (object, nonce) in REFERENCED.into_iter().zip([55, 57]) {
        let op = f.domain(
            IndexOp::PutObject {
                object,
                kind: StudioKind::Flipnote,
                title: format!("saved branch {nonce}"),
                created_by: f.device.device_id(),
                ts: 123,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap(),
            nonce,
        );
        save(f, store, &close, basis.fingerprint(), op, 123);
    }
    install(f, store, &close);
    for (object, nonce) in REFERENCED.into_iter().zip([56, 58]) {
        edit_referenced(f, store, object, nonce);
    }
    basis.fingerprint()
}

/// One title edit to the Flipnote `object` in the fixture's channel.
fn edit_referenced(f: &Fixture, store: &mut ServerStore, object: [u8; 16], nonce: u8) {
    let referenced = StudioTarget::Flipnote {
        channel: f.target.channel(),
        object,
    };
    let logical = referenced.document(&f.group.group_id()).unwrap();
    let body = DomainOp {
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
        nonce: [nonce; 16],
        body: FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("edit {nonce}")))
            .encode()
            .unwrap(),
    };
    let mut b = budget(store, f);
    let current = store
        .load_studio_epoch(SERVER, &f.group, referenced, &f.device)
        .unwrap()
        .map(|state| state.unit.doc_id())
        .unwrap_or_else(|| epoch_zero_id(logical.doc_type, &logical.logical_key));
    store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            referenced,
            current,
            &f.device,
            body,
            123,
            &mut rng(),
            &mut b,
        )
        .unwrap();
}

/// H1's pristine-successor probe runs before the Index object check, whose restores of every object
/// a PutObject names are the expensive part of H1 (implementation review of the 18.3 fixes, LOW-1).
/// So a non-pristine Index successor is refused having restored nothing; with the probe after the
/// check, as first built, this vault paid two restores every probe period before the refusal.
#[test]
fn studio_overlay_handoff_h1_refuses_a_non_pristine_index_successor_before_restoring_objects() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let basis = index_with_two_references(&f, &mut store);
    // The installed successor takes one ordinary operation, so it is no longer pristine.
    let mut successor = f.load(&store).unwrap().unit;
    let mut ordinary = f.title();
    ordinary.nonce = [79; 16];
    let packet = successor
        .edit_or_reseal(&f.device, &f.group, &mut rng(), &ordinary, 557)
        .unwrap();
    let mut b = budget(&mut store, &f);
    store
        .ingest_studio_epoch(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &packet,
            &mut rng(),
            &mut b,
        )
        .unwrap();

    let mut b = budget(&mut store, &f);
    let before = studio_full_restores_for_test();
    let refused = store
        .start_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::None,
        )
        .map(|_| ())
        .expect_err("H1 accepted a non-pristine Index successor");
    assert!(
        refused
            .to_string()
            .contains("successor is not transferable: SuccessorNotPristine"),
        "refused for another reason: {refused}"
    );
    assert_eq!(
        studio_full_restores_for_test() - before,
        0,
        "H1 restored the branch's objects before refusing a non-pristine successor"
    );
}

/// 9.1.1 step 7, the zero-restore claim, narrowed as its review asked (M-4): H5 calls neither
/// `restore_unit` nor `load_studio_epoch` (whose loads go through it), for a Flipnote and for an
/// Index whose branch names two objects. Before 9.1 this H5 restored twice for a Flipnote, and
/// twice plus once per PutObject for an Index.
#[test]
fn studio_overlay_handoff_commit_restores_nothing() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, expected) = prepare(&f, &mut store);
        let commit = staged(&f, &mut store, basis);
        let (outcome, restores) = commit_counting(&f, &mut store, commit, &mut WriteHooks::None);
        outcome.expect("the handoff did not commit");
        assert_eq!(restores, 0, "H5 restored a source (art {art})");
        assert_eq!(f.load(&store).unwrap().projection().unwrap(), expected);
        assert!(!store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .handoff_prepared());
    }

    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let basis = index_with_two_references(&f, &mut store);
    let commit = staged(&f, &mut store, basis);
    let (outcome, restores) = commit_counting(&f, &mut store, commit, &mut WriteHooks::None);
    outcome.expect("the Index handoff did not commit");
    assert_eq!(
        restores, 0,
        "H5 restored a source for an Index with PutObjects"
    );
}

/// 9.1.1 step 1's oracle, and the candidate side of step 3's. Before the write, the facts H2
/// carries describe the source exactly as a restore would: `stamped_studio_source` produces the
/// accounting record `checked_studio_source` produces over the same bytes, and the digest of the
/// plaintext actually on disk. After the write, restoring the persisted bytes gives back the
/// candidate's blob CIDs and snapshot, so classifying from the candidate is classifying from what
/// landed.
#[test]
fn studio_overlay_handoff_facts_match_a_restore_of_the_stamped_source() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, _) = prepare(&f, &mut store);
        let commit = staged(&f, &mut store, basis);
        let mut b = budget(&mut store, &f);
        let (observed, digest) = store
            .stamped_studio_source(&commit.stamp, &commit.facts, &mut b.storage)
            .expect("the stamped facts were refused");
        let mut b = budget(&mut store, &f);
        let (_, restored, _) = store
            .checked_studio_source(SERVER, &f.group, f.target, &f.device, false, &mut b.storage)
            .unwrap();
        assert_eq!(
            Some(observed),
            restored,
            "facts and restore disagree (art {art})"
        );
        let scope = scope_bytes(SERVER, &f.logical).unwrap();
        let on_disk = store.read_studio_record(&scope).unwrap().unwrap();
        assert_eq!(digest, blake3::hash(&on_disk.plain));

        let cids = commit.candidate.blob_cids().unwrap();
        let snapshot = blake3::hash(&commit.snapshot);
        let (outcome, _) = commit_counting(&f, &mut store, commit, &mut WriteHooks::None);
        outcome.expect("the handoff did not commit");
        let mut persisted = f.load(&store).unwrap();
        assert_eq!(
            persisted.unit.blob_cids().unwrap(),
            cids,
            "the persisted source names other blobs than the candidate (art {art})"
        );
        assert_eq!(
            blake3::hash(&persisted.unit.snapshot().unwrap()),
            snapshot,
            "the persisted source does not restore to the candidate's snapshot (art {art})"
        );
    }
}

/// A same-size substitute for a framed Studio source record, and the two snapshots involved.
struct Substitute {
    framed: Vec<u8>,
    original: Vec<u8>,
    substituted: Vec<u8>,
}

/// Rewrite an authenticated Studio source record with one byte of its snapshot's receipt book
/// changed, resealed under the vault key and keeping its size.
///
/// The receipt book is the one part of the snapshot `preserves_vault_source` skips. So this
/// substitute clears barrier 2's fence (the test asserts that precondition), and only the
/// post-write proof can tell it from the candidate. Every framing field, including the link
/// extension, is untouched, so it also decodes.
fn with_book_byte_flipped(key: &[u8; 32], framed: &[u8]) -> Substitute {
    let mut plain = catcoms_crypto::unseal(key, &unframe(framed).unwrap()).unwrap();
    let mut d = Decoder::new(&plain);
    d.get_bytes().unwrap(); // scope
    d.get_bytes().unwrap(); // channel
    let snapshot = d.get_bytes().unwrap();
    let at = snapshot.as_ptr() as usize - plain.as_ptr() as usize;
    let original = snapshot.to_vec();
    // The snapshot: prefix, channel, opening, seed, receipt book, gate, operations.
    let mut s = Decoder::new(&original);
    let prefix = s.get_u8().unwrap();
    assert!(prefix <= 2, "the fixture is not a repaired source");
    s.get_bytes().unwrap(); // channel
    s.get_bytes().unwrap(); // opening
    s.get_bytes().unwrap(); // seed
    let book = s.get_bytes().unwrap();
    assert!(!book.is_empty(), "the fixture's receipt book is empty");
    let inside = book.as_ptr() as usize - original.as_ptr() as usize + book.len() - 1;
    plain[at + inside] ^= 1;
    let substituted = plain[at..at + original.len()].to_vec();
    let out = frame(&seal(key, &plain, &mut rng()).unwrap());
    assert_eq!(
        out.len(),
        framed.len(),
        "the substitute must keep the record's size"
    );
    Substitute {
        framed: out,
        original,
        substituted,
    }
}

/// M17, the post-write proof. The Source write lands authenticated bytes of the same size that
/// are not the ones the writer encoded: one receipt-book byte differs. Barrier 2's fence ignores
/// that byte, as the test asserts first. So with the post-write proof removed, resolve would
/// classify the candidate as Complete and write Completed over a record that is not it. With the
/// proof, H5 refuses with Prepared retained, writes no Completed, and restores nothing.
///
/// Which of the proof's checks fires is not pinned, and cannot be. The digest and the snapshot
/// hash both see this byte, and each is redundant with the other by construction. The mutation
/// this kills is the whole re-read removed: Completed is then written, which the state assertion
/// catches whatever the message says.
#[test]
fn studio_overlay_handoff_refuses_a_persisted_source_that_is_not_the_candidate() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, _) = prepare(&f, &mut store);
        let commit = staged(&f, &mut store, basis);
        let key = store.keys.db_key().unwrap();
        let substitute = std::cell::RefCell::new(None);
        let (outcome, restores) = commit_counting(
            &f,
            &mut store,
            commit,
            &mut WriteHooks::Hooked {
                before: Some(&mut |at: WriteTag, _: &Path, bytes: &[u8]| {
                    if at != WriteTag::Source {
                        return Intercept::Continue;
                    }
                    let replaced = substitute
                        .borrow_mut()
                        .get_or_insert_with(|| with_book_byte_flipped(&key, bytes))
                        .framed
                        .clone();
                    Intercept::Replace(replaced)
                }),
                before_sync: None,
                before_unlink: None,
                after: None,
            },
        );
        let substitute = substitute
            .into_inner()
            .expect("the Source write was never reached");
        // The precondition: barrier 2's fence accepts the substitute, so only the proof stops it.
        let mut candidate = StudioEpoch::restore(
            &substitute.original,
            &f.group,
            f.target,
            f.device.device_id(),
        )
        .unwrap();
        assert!(
            candidate
                .preserves_vault_source(&substitute.substituted, &Default::default())
                .unwrap(),
            "precondition: the fence must accept the substitute, or it would stop it by itself"
        );
        let error = outcome.expect_err("H5 completed from a persisted source it did not write");
        assert!(
            error.to_string().contains(
                "persisted handoff source could not be proved to be the written candidate"
            ),
            "refused by something other than the post-write proof (art {art}): {error}"
        );
        assert_eq!(restores, 0, "the refusal restored a source");
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert!(state.handoff_prepared(), "Prepared was not retained");
    }
}

/// The one byte, of the proof's checks, only the size-and-digest comparison sees (design 18.3
/// review, F7).
///
/// H5 writes its destination linked to the handoff metadata: the plaintext ends in a link byte.
/// This substitute is the candidate's own plaintext with that byte dropped, resealed. It decodes
/// as an unlinked record with the candidate's channel and exactly the candidate's snapshot, so the
/// snapshot-hash check passes and `check_studio_intent_link`, which accepts an unlinked record,
/// passes too. Within the proof, only the comparison with what the writer encoded refuses it.
///
/// It is not the last line of defence. The encoding is canonical (the link, if present, is a
/// trailing `1`), so dropping it always shrinks the record, and resolve's flush-only fence, which
/// checks the file's length against the written version, refuses it too: later, and with
/// "retry file changed". What this pins is the designed refusal point: the proof, which spends the
/// budget and runs before resolve reads anything. CI's `proof-digest` entry removes the
/// comparison and requires this test to fail at that assertion.
#[test]
fn studio_overlay_handoff_refuses_a_persisted_source_whose_link_was_dropped() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    let commit = staged(&f, &mut store, basis);
    let key = store.keys.db_key().unwrap();
    let written = std::cell::RefCell::new(None);
    let (outcome, restores) = commit_counting(
        &f,
        &mut store,
        commit,
        &mut WriteHooks::Hooked {
            before: Some(&mut |at: WriteTag, _: &Path, bytes: &[u8]| {
                if at != WriteTag::Source {
                    return Intercept::Continue;
                }
                let plain = catcoms_crypto::unseal(&key, &unframe(bytes).unwrap()).unwrap();
                let replaced = frame(&seal(&key, &plain[..plain.len() - 1], &mut rng()).unwrap());
                *written.borrow_mut() = Some(plain);
                Intercept::Replace(replaced)
            }),
            before_sync: None,
            before_unlink: None,
            after: None,
        },
    );
    let plain = written
        .into_inner()
        .expect("the Source write was never reached");
    // The preconditions: the candidate was linked, and dropping the link changes nothing else the
    // proof's other checks read.
    let scope = scope_bytes(SERVER, &f.logical).unwrap();
    let (target, snapshot, linked) =
        crate::store::epoch_studio::decode_record_link(&plain, &scope, &f.logical).unwrap();
    let (dropped_target, dropped_snapshot, dropped_linked) =
        crate::store::epoch_studio::decode_record_link(
            &plain[..plain.len() - 1],
            &scope,
            &f.logical,
        )
        .unwrap();
    assert!(linked, "precondition: H5 writes a linked destination");
    assert!(!dropped_linked, "precondition: the substitute is unlinked");
    assert_eq!((dropped_target, dropped_snapshot), (target, snapshot));

    let error = outcome.expect_err("H5 completed from a persisted source whose link was dropped");
    assert!(
        error
            .to_string()
            .contains("persisted handoff source could not be proved to be the written candidate"),
        "refused by something other than the post-write proof: {error}"
    );
    assert_eq!(restores, 0, "the refusal restored a source");
    assert!(
        store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .handoff_prepared(),
        "Prepared was not retained"
    );
}

/// A Source write that lands bytes which do not authenticate at all is a proof failure too, not
/// an error that escapes the proof: H5 refuses with Prepared retained, and the budget that
/// reserved a footprint for the candidate is spent (9.1.1 step 4).
#[test]
fn studio_overlay_handoff_refuses_and_spends_the_budget_when_the_written_source_does_not_authenticate(
) {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (_, basis, _) = prepare(&f, &mut store);
    let commit = staged(&f, &mut store, basis);
    let mut b = budget(&mut store, &f);
    let outcome = store.commit_studio_handoff_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        commit,
        Some(0),
        &mut rng(),
        &mut b,
        &mut WriteHooks::Hooked {
            before: Some(&mut |at: WriteTag, _: &Path, bytes: &[u8]| {
                if at != WriteTag::Source {
                    return Intercept::Continue;
                }
                Intercept::Replace(vec![0xA5; bytes.len()])
            }),
            before_sync: None,
            before_unlink: None,
            after: None,
        },
    );
    let error = outcome.expect_err("H5 completed over a source that does not authenticate");
    assert!(
        error
            .to_string()
            .contains("persisted handoff source could not be proved to be the written candidate"),
        "an unauthenticated re-read escaped the proof: {error}"
    );
    assert!(
        b.storage.requires_reconciliation(),
        "the budget that reserved the candidate's footprint was not spent"
    );
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .handoff_prepared());
}

/// The pre-write half: a source replaced between H4 and H5 by another authenticated record, here
/// the earlier Closing version of the same document, is refused by the stamp, never restored, and
/// leaves the records as they were.
#[test]
fn studio_overlay_handoff_refuses_a_changed_source_without_restoring() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, _, closing_record) = prepare_keeping_closing(&f, &mut store);
        let commit = staged(&f, &mut store, basis);
        fs::write(f.path(&store), closing_record).unwrap();
        let records = canonical(&store);
        let (outcome, restores) = commit_counting(&f, &mut store, commit, &mut WriteHooks::None);
        let error = outcome.expect_err("H5 committed over a source the stamp did not describe");
        assert!(
            error
                .to_string()
                .contains("overlay records or context changed"),
            "refused for another reason (art {art}): {error}"
        );
        assert_eq!(restores, 0, "the stamp refusal restored a source");
        assert_eq!(canonical(&store), records, "a refused H5 wrote");
    }
}

/// A1: H5's header-only Index check accepts a referenced Flipnote edited between H1 and H5,
/// because it still holds work. The removed-object case is
/// `studio_overlay_handoff_rechecks_index_object_sources_at_commit_not_only_at_capture`.
#[test]
fn studio_overlay_handoff_commit_accepts_a_reference_edited_since_capture() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let basis = index_with_two_references(&f, &mut store);
    let commit = staged(&f, &mut store, basis);
    edit_referenced(&f, &mut store, REFERENCED[0], 59);
    let (outcome, restores) = commit_counting(&f, &mut store, commit, &mut WriteHooks::None);
    outcome.expect("an edited reference that still holds work was refused at H5");
    assert_eq!(restores, 0);
}

/// The source on disk, as `load` returns it with its real version, and its snapshot's hash: the
/// two inputs the proof is built from.
fn loaded(f: &Fixture, store: &ServerStore) -> (EpochStudioState, [u8; 32]) {
    let mut state = f.load(store).unwrap();
    let hash = *blake3::hash(&state.unit.snapshot().unwrap()).as_bytes();
    (state, hash)
}

/// The proof's bindings (9.1.1 step 3 and A3). It is built only for the target the record names
/// and only for the candidate whose snapshot actually landed, and it cannot be spent after a
/// five-family write lands between verification and use.
#[test]
fn studio_overlay_handoff_verified_source_binds_target_candidate_and_generation() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        prepare(&f, &mut store);

        let (state, hash) = loaded(&f, &store);
        let verified = store
            .verify_persisted_studio_source(SERVER, f.target, state, hash)
            .expect("the record on disk was not verified as itself");
        let mut b = budget(&mut store, &f);
        verified
            .into_checked(&store, SERVER, &f.group, f.target, &mut b.storage)
            .expect("an untouched proof could not be spent");

        let (state, mut hash) = loaded(&f, &store);
        hash[0] ^= 1;
        let refused = store.verify_persisted_studio_source(SERVER, f.target, state, hash);
        assert!(
            matches!(refused, Err(ref e) if e.to_string().contains("not the candidate")),
            "another candidate's snapshot hash was accepted (art {art}): {refused:?}"
        );

        let (state, hash) = loaded(&f, &store);
        let other = StudioTarget::Flipnote {
            channel: [0xEE; 16],
            object: [0xEE; 16],
        };
        assert!(
            store
                .verify_persisted_studio_source(SERVER, other, state, hash)
                .is_err(),
            "a proof was built for a target the record does not name"
        );

        let (state, hash) = loaded(&f, &store);
        let verified = store
            .verify_persisted_studio_source(SERVER, f.target, state, hash)
            .unwrap();
        store.epoch_mutation_guard();
        let mut b = budget(&mut store, &f);
        let spent = verified.into_checked(&store, SERVER, &f.group, f.target, &mut b.storage);
        assert!(
            matches!(spent, Err(ref e) if e.to_string().contains("no longer describes")),
            "a proof was spent after a write landed (art {art}): {spent:?}"
        );
    }
}

/// 9.1.1 step 5: a verified source with anything but Complete evidence refuses without writing.
/// Here the proof is of the source as it stood before the branch, behind an interrupted Prepared
/// record, so its evidence is Absent. Resolving from disk would return the branch to Active;
/// resolving from a proof must not, because the commit only ever proves a candidate that carries
/// the whole branch, and anything else means the proof and the record disagree.
#[test]
fn studio_overlay_handoff_verified_resolution_accepts_only_complete_evidence() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, _) = prepare(&f, &mut store);
        super::fences::interrupt(&f, &mut store, basis, WriteTag::Source);
        assert!(store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .handoff_prepared());
        let (state, hash) = loaded(&f, &store);
        let verified = store
            .verify_persisted_studio_source(SERVER, f.target, state, hash)
            .unwrap();
        let records = canonical(&store);
        let mut b = budget(&mut store, &f);
        let refused = store.resolve_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            Some(verified),
            &mut rng(),
            &mut b,
            &mut WriteHooks::None,
        );
        assert!(
            matches!(refused, Err(ref e) if e.to_string().contains("does not carry the whole branch")),
            "a proof with Absent evidence was resolved (art {art}): {refused:?}"
        );
        assert_eq!(
            canonical(&store),
            records,
            "a refused verified resolution wrote"
        );
    }
}

/// Replace a referenced Flipnote's record with an authenticated one saying `channel` at the
/// record level, around `snapshot`, carrying the source-to-intent link extension when `linked`.
fn rewrite_referenced(
    f: &Fixture,
    store: &ServerStore,
    object: [u8; 16],
    channel: [u8; 16],
    snapshot: &[u8],
    linked: bool,
) {
    let referenced = StudioTarget::Flipnote {
        channel: f.target.channel(),
        object,
    };
    let logical = referenced.document(&f.group.group_id()).unwrap();
    let scope = scope_bytes(SERVER, &logical).unwrap();
    let mut e = Encoder::new();
    e.put_bytes(&scope).unwrap();
    e.put_bytes(&channel).unwrap();
    e.put_bytes(snapshot).unwrap();
    if linked {
        e.put_u8(1);
    }
    let sealed = seal(&store.keys.db_key().unwrap(), &e.finish(), &mut rng()).unwrap();
    fs::write(store.studio_epoch_path(&scope), frame(&sealed)).unwrap();
}

/// A1 at commit: a reference that passed H1's full load and is then made pristine, relabelled
/// under another channel, or linked to intent metadata that does not exist, is refused at H5
/// before anything is written, and nothing is restored. The removed-object case is
/// `studio_overlay_handoff_rechecks_index_object_sources_at_commit_not_only_at_capture`.
#[test]
fn studio_overlay_handoff_commit_refuses_a_reference_made_pristine_relabelled_or_unlinked() {
    #[derive(Debug, Clone, Copy)]
    enum Change {
        Pristine,
        Relabelled,
        Unlinked,
    }
    let channel = Fixture::new(false).target.channel();
    let other = [channel[0] ^ 0xFF; 16];
    for change in [Change::Pristine, Change::Relabelled, Change::Unlinked] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(false);
        let mut store = open(root.path());
        let basis = index_with_two_references(&f, &mut store);
        let commit = staged(&f, &mut store, basis);
        let referenced = StudioTarget::Flipnote {
            channel,
            object: REFERENCED[0],
        };
        // Minted before the record is rewritten. In production H5's budget comes from an
        // inventory taken in the same custody turn, which validates every record and refuses
        // these hand-made ones itself, so the commit would stop before H5's check. This
        // exercises the check's own predicate. The reachable equivalents are internally
        // consistent records (for example, one whose record and snapshot both name another
        // channel), and they take the same branches.
        let mut b = budget(&mut store, &f);
        let mut current = store
            .load_studio_epoch(SERVER, &f.group, referenced, &f.device)
            .unwrap()
            .unwrap();
        let snapshot = current.unit.snapshot().unwrap();
        match change {
            Change::Relabelled => {
                rewrite_referenced(&f, &store, REFERENCED[0], other, &snapshot, false)
            }
            Change::Unlinked => {
                // A source claiming a handoff link, for an object with no handoff metadata.
                rewrite_referenced(&f, &store, REFERENCED[0], channel, &snapshot, true)
            }
            Change::Pristine => {
                let mut pristine =
                    StudioEpoch::new(&f.group, referenced, f.device.device_id()).unwrap();
                let snapshot = pristine.snapshot().unwrap();
                rewrite_referenced(&f, &store, REFERENCED[0], channel, &snapshot, false);
            }
        }
        let records = canonical(&store);
        let before = studio_full_restores_for_test();
        let outcome = store.commit_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            commit,
            Some(0),
            &mut rng(),
            &mut b,
            &mut WriteHooks::None,
        );
        let restores = studio_full_restores_for_test() - before;
        let error = outcome.expect_err("H5 committed an Index entry naming an unusable Flipnote");
        let expected = match change {
            Change::Unlinked => "required handoff metadata missing",
            Change::Pristine | Change::Relabelled => "unavailable Flipnote",
        };
        assert!(
            error.to_string().contains(expected),
            "refused for another reason ({change:?}): {error}"
        );
        assert_eq!(restores, 0, "the H5 Index check restored a source");
        assert_eq!(canonical(&store), records, "a refused H5 wrote");
    }
}

/// The restart path is unchanged: resolving an interrupted Prepared record from disk restores the
/// destination source exactly once, as it did before 9.1.
#[test]
fn studio_overlay_handoff_restart_resolution_still_restores_once() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (_, basis, _) = prepare(&f, &mut store);
        super::fences::interrupt(&f, &mut store, basis, WriteTag::Completed);
        drop(store);
        let mut store = open(root.path());
        let mut b = budget(&mut store, &f);
        let before = studio_full_restores_for_test();
        store
            .resolve_studio_handoff(SERVER, &f.group, f.target, &f.device, &mut rng(), &mut b)
            .expect("the interrupted handoff did not resolve");
        assert_eq!(
            studio_full_restores_for_test() - before,
            1,
            "the restart path's restore changed (art {art})"
        );
    }
}
