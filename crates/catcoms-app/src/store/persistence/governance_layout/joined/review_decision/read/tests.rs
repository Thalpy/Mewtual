//! Actual authenticated heads, signed records and keyed Case pages. These
//! are structural DATA fixtures, not a manufactured native/source grant.
use super::super::selection::{tests::signed_case_fixture, DecisionHead, RetainedDecisionRecord};
use super::*;
use catcoms_replication::chat::{
    owner_bodies::{ExactReviewDecisionBody, ReviewDecision, UnsignedReviewStreamEntry},
    SignedAction, V2RemovedCreateRecord,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::path::PathBuf;

struct Fixture {
    root: HeldDirectory,
    key: JournalKey,
    pin: FounderGenesisAnchorData,
    case: NativeTextHeldCase,
    decision: ExactReviewDecisionBody,
    entry: catcoms_replication::chat::owner_bodies::ReviewStreamEntry,
    case_page: PathBuf,
    temp: tempfile::TempDir,
}

fn fixture(public: bool) -> Fixture {
    let mut rng = ChaCha20Rng::seed_from_u64(0x4911);
    let key = JournalKey::new(&[91; 32], &[92; 32]);
    let (founder, pin, position, decision, held) = signed_case_fixture(ReviewDecision::Reject);
    let original = V2RemovedCreateRecord::decode(&held).unwrap();
    let exact = decision.original_data();
    let case = NativeTextHeldCase {
        case: decision.case_id(),
        original: exact.digest,
        removal: (exact.remove_header, exact.remove_epoch),
        admission: original.target_data().spec().context.admission,
        signed: SignedAction::decode(original.original_bytes_data()).unwrap(),
        decision: NativeTextHeldReviewDecision::Unavailable,
    };
    let unsigned = if public {
        UnsignedReviewStreamEntry::prepare_public(&pin, position, &decision)
    } else {
        UnsignedReviewStreamEntry::prepare(&pin, position, &decision)
    }
    .unwrap();
    let signature = founder.sign(unsigned.signing_bytes());
    let entry = unsigned.attach_signature(&pin, signature).unwrap();
    let record = RetainedDecisionRecord::prepare(
        &key,
        &pin,
        (&entry, &decision, &held),
        (*decision.id().as_bytes(), [93; 32]),
        &mut rng,
    )
    .unwrap();
    let plain = RetainedDecisionRecord::open(&key, &record).unwrap();
    let retained = RetainedDecisionRecord::decode(&pin, &plain).unwrap();
    let pages = key
        .indexes()
        .prepare_first(&retained, &record, &mut rng)
        .unwrap();
    let head = DecisionHead::from_record(
        &entry,
        &record,
        None,
        pages.each_ref().map(|page| page.root),
    )
    .unwrap()
    .seal(&key, &mut rng)
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("journal");
    std::fs::create_dir(&directory).unwrap();
    let root = HeldDirectory::open(directory).unwrap();
    std::fs::write(root.child(HEAD_NAME), head).unwrap();
    let records = root.child(selection::disk::RECORD_DIRECTORY);
    std::fs::create_dir(&records).unwrap();
    std::fs::write(records.join("0000000000000001.record"), record).unwrap();
    let case_page = records.join(format!(
        "{}.page",
        hex::encode(pages[1].page.id().as_bytes())
    ));
    for page in pages {
        std::fs::write(
            records.join(format!("{}.page", hex::encode(page.page.id().as_bytes()))),
            page.page.as_bytes(),
        )
        .unwrap();
    }
    Fixture {
        root,
        key,
        pin,
        case,
        decision,
        entry,
        case_page,
        temp,
    }
}

fn read(fixture: &Fixture) -> Result<NativeTextHeldReviewDecision, AppError> {
    let channel = fixture.decision.decision().spec().target.spec().channel;
    let (states, _proof) = read_cases(
        &fixture.root,
        &fixture.key,
        &fixture.pin,
        channel,
        std::slice::from_ref(&fixture.case),
        &mut |position| {
            if let Some(position) = position {
                assert_eq!(position, fixture.entry.position_data());
            }
            Ok(())
        },
    )?;
    Ok(states[0])
}

#[test]
fn founder_status_uses_the_selected_case_and_distinct_exact_body_and_entry_ids() {
    let fixture = fixture(true);
    assert_eq!(
        read(&fixture).unwrap(),
        NativeTextHeldReviewDecision::Selected {
            choice: ReviewDecision::Reject,
            decision_body: fixture.decision.id(),
            public_entry: fixture.entry.id(),
        }
    );
    // Changing only the expected exact original cannot borrow this case's
    // decision. No ordinary projection or key operation is exercised.
    let mut changed = fixture;
    changed.case.removal.1 += 1;
    assert!(read(&changed).is_err());
    assert_eq!(
        changed.case.decision,
        NativeTextHeldReviewDecision::Unavailable
    );
}

#[test]
fn founder_status_proves_empty_and_preserves_missing_damaged_or_interrupted_evidence() {
    let fixture = fixture(true);
    let selected = std::fs::read(fixture.root.child(HEAD_NAME)).unwrap();
    let records = fixture.root.child(selection::disk::RECORD_DIRECTORY);
    let parked = fixture.temp.path().join("parked-records");
    std::fs::rename(&records, &parked).unwrap();
    std::fs::write(fixture.root.child(HEAD_NAME), fixture.key.initial_head()).unwrap();
    assert_eq!(
        read(&fixture).unwrap(),
        NativeTextHeldReviewDecision::Undecided
    );
    for name in [
        "intent.pending",
        "key.attempt",
        "key.raw",
        "selection.pending",
        "selection.scratch",
        "head.stage",
        "foreign",
    ] {
        let path = fixture.root.child(name);
        std::fs::write(&path, b"preserved uncertain bytes").unwrap();
        assert!(read(&fixture).is_err(), "{name}");
        assert_eq!(std::fs::read(&path).unwrap(), b"preserved uncertain bytes");
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_file(fixture.root.child(HEAD_NAME)).unwrap();
    assert!(read(&fixture).is_err());
    std::fs::write(fixture.root.child(HEAD_NAME), [0; 32]).unwrap();
    assert!(read(&fixture).is_err());
    // An authentic empty head with surviving outputs is not an empty
    // baseline. Restore the exact selected files as a positive control.
    std::fs::write(fixture.root.child(HEAD_NAME), fixture.key.initial_head()).unwrap();
    std::fs::rename(&parked, &records).unwrap();
    assert!(read(&fixture).is_err());
    std::fs::write(fixture.root.child(HEAD_NAME), selected).unwrap();
    assert!(matches!(
        read(&fixture).unwrap(),
        NativeTextHeldReviewDecision::Selected { .. }
    ));
}

#[test]
fn founder_status_refuses_legacy_or_missing_selected_pages_without_repair() {
    let legacy = fixture(false);
    assert!(read(&legacy).is_err());
    let fixture = fixture(true);
    let page = &fixture.case_page;
    let original = std::fs::read(page).unwrap();
    std::fs::remove_file(page).unwrap();
    assert!(read(&fixture).is_err());
    assert!(!page.exists());
    std::fs::write(&page, original).unwrap();
    assert!(read(&fixture).is_ok());
    assert!(read_cases(
        &fixture.root,
        &fixture.key,
        &fixture.pin,
        fixture.decision.decision().spec().target.spec().channel,
        std::slice::from_ref(&fixture.case),
        &mut |_| Err(denied())
    )
    .is_err());
}
