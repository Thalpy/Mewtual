//! A captured view must reach exactly the live group's verdict on every authority check the
//! repair and adoption paths make, and must stop matching the live group once anything it
//! depends on moves. These are the two properties a detached repair stage relies on.
use super::*;
use crate::epoch::InheritedCheckpoint;
use crate::{LogicalDocument, Receipt, ReceiptRepair, ReplError};
use catcoms_mls::MlsDevice;
use catcoms_wire::DocType;

struct Fixture {
    owner: MlsDevice,
    member: MlsDevice,
    group: ServerGroup,
    document: LogicalDocument,
}

fn fixture() -> Fixture {
    let owner = MlsDevice::generate().unwrap();
    let member = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    group
        .add_member(&owner, member.key_package().unwrap())
        .unwrap();
    let document =
        LogicalDocument::new(group.group_id(), DocType::StudioIndex, vec![7; 16]).unwrap();
    Fixture {
        owner,
        member,
        group,
        document,
    }
}

fn receipt(document: &LogicalDocument, tenure: u64, salt: u8, signer: &MlsDevice) -> Receipt {
    Receipt::sign(
        document.clone(),
        0,
        [salt; 32],
        [salt; 32],
        tenure,
        InheritedCheckpoint::EpochZero,
        signer,
    )
    .unwrap()
}

fn repair(selected: &Receipt, losing: &Receipt, tenure: u64, signer: &MlsDevice) -> ReceiptRepair {
    let mut hashes = [selected.hash(), losing.hash()];
    hashes.sort();
    ReceiptRepair::sign_in_tenure(
        selected.document.clone(),
        selected.tenure_id,
        hashes,
        selected.hash(),
        1,
        tenure,
        signer,
    )
    .unwrap()
}

/// The verdict as a comparable value: which error, or success.
fn verdict<T>(result: Result<T, ReplError>) -> Result<(), String> {
    result.map(|_| ()).map_err(|error| format!("{error:?}"))
}

#[test]
fn a_captured_view_answers_the_four_facts_for_the_committer_alone() {
    let f = fixture();
    let view = CapturedOwnerAuthority::capture(&f.group);
    let owner = f.owner.device_id();
    let member = f.member.device_id();
    assert_eq!(OwnerAuthority::group_id(&view), f.group.group_id());
    assert_eq!(OwnerAuthority::epoch(&view), f.group.epoch());
    assert_eq!(
        OwnerAuthority::designated_committer(&view),
        f.group.designated_committer()
    );
    assert_eq!(
        OwnerAuthority::member_signature_key(&view, &owner),
        f.group.member_signature_key(&owner)
    );
    // Deliberately narrower than the live group: no check on these paths asks for anyone else's
    // key, and a check that started to would fail closed here rather than pass on stale facts.
    assert!(f.group.member_signature_key(&member).is_some());
    assert_eq!(OwnerAuthority::member_signature_key(&view, &member), None);
    // Public keys are public, but the debug form need not carry them.
    let key = f.group.member_signature_key(&owner).unwrap();
    assert!(!format!("{view:?}").contains(&format!("{:?}", key)));
}

#[test]
fn receipt_and_repair_verdicts_match_the_live_group_case_by_case() {
    let f = fixture();
    let view = CapturedOwnerAuthority::capture(&f.group);
    let epoch = f.group.epoch();
    let selected = receipt(&f.document, 0, 1, &f.owner);
    let losing = receipt(&f.document, 0, 2, &f.owner);
    let foreign_group = ServerGroup::create(&f.owner).unwrap();
    let foreign_document =
        LogicalDocument::new(foreign_group.group_id(), DocType::StudioIndex, vec![7; 16]).unwrap();
    let receipts = [
        (
            "owner, current tenure",
            receipt(&f.document, 0, 3, &f.owner),
            0,
        ),
        (
            "owner, wrong expected tenure",
            receipt(&f.document, 0, 4, &f.owner),
            1,
        ),
        (
            "owner, tenure beyond the epoch",
            receipt(&f.document, epoch + 1, 5, &f.owner),
            epoch + 1,
        ),
        (
            "member, not the committer",
            receipt(&f.document, 0, 6, &f.member),
            0,
        ),
        (
            "another group's document",
            receipt(&foreign_document, 0, 7, &f.owner),
            0,
        ),
    ];
    let mut accepted = 0;
    for (case, candidate, expected) in &receipts {
        let live = verdict(candidate.verify_current_owner(&f.group, *expected));
        assert_eq!(
            verdict(candidate.verify_current_owner(&view, *expected)),
            live,
            "receipt case: {case}"
        );
        accepted += usize::from(live.is_ok());
    }
    assert_eq!(
        accepted, 1,
        "the matrix exercises refusals, not only acceptance"
    );

    let repairs = [
        (
            "owner, current tenure",
            repair(&selected, &losing, 0, &f.owner),
            0,
        ),
        (
            "owner, wrong expected tenure",
            repair(&selected, &losing, 0, &f.owner),
            1,
        ),
        (
            "owner, issuer tenure beyond the epoch",
            repair(&selected, &losing, epoch + 1, &f.owner),
            epoch + 1,
        ),
        ("member-signed", repair(&selected, &losing, 0, &f.member), 0),
    ];
    let mut accepted = 0;
    for (case, candidate, expected) in &repairs {
        let live = verdict(candidate.verify_current_owner(&f.group, *expected));
        assert_eq!(
            verdict(candidate.verify_current_owner(&view, *expected)),
            live,
            "repair case: {case}"
        );
        accepted += usize::from(live.is_ok());
    }
    assert_eq!(accepted, 1);
}

#[test]
fn a_view_stops_matching_once_the_group_moves() {
    let mut f = fixture();
    let before = CapturedOwnerAuthority::capture(&f.group);
    assert_eq!(before, CapturedOwnerAuthority::capture(&f.group));
    // Any commit advances the epoch, so a plan made against the old view cannot be committed
    // by a stage that recaptures and compares first.
    let other = MlsDevice::generate().unwrap();
    f.group
        .add_member(&f.owner, other.key_package().unwrap())
        .unwrap();
    let after = CapturedOwnerAuthority::capture(&f.group);
    assert_ne!(before, after);
    assert_eq!(
        OwnerAuthority::epoch(&after),
        OwnerAuthority::epoch(&before) + 1
    );
    // A different group with the same owner is a different authority too.
    let foreign = ServerGroup::create(&f.owner).unwrap();
    assert_ne!(CapturedOwnerAuthority::capture(&foreign), before);
}
