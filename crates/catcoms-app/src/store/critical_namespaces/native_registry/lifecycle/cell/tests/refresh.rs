//! Real owning normal3 Refresh path, including the existing four physical cuts.
use super::super::super::refresh;
use super::*;

fn input(owner: &owning::Store) -> refresh::Request {
    let row = request(owner);
    refresh::Request {
        id: row.id,
        identity: row.identity,
        expected_revision: row.expected_revision,
        invite: "new route-aware invite".into(),
    }
}
fn invite(owner: &owning::Store) -> String {
    let value = selected(owner);
    let data::Backend::Legacy { invite, .. } = &value.rows[0].backend else {
        panic!("Legacy")
    };
    invite.clone()
}

#[test]
fn lifecycle3_refresh_cell_warm_cuts_keep_exact_tagged_request_and_ciphertext() {
    for (n, cut) in [
        cuts::Fault::Index3PartialStage,
        cuts::Fault::IndexStage,
        cuts::Fault::IndexRename,
        cuts::Fault::Index3Covered,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xca01 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let mut r = input(&owner);
        r.invite = "i".repeat(super::super::super::super::data::MAX_INVITE);
        let old = selected(&owner);
        let rename = request(&owner);
        cuts::arm_fault(cut);
        assert!(owner.refresh_legacy_invite_data(&r, &mut rng).is_err());
        assert!(cuts::take_fault_seen());
        for changed in [
            refresh::Request {
                id: r.id + 1,
                ..r.clone()
            },
            refresh::Request {
                identity: [99; 32],
                ..r.clone()
            },
            refresh::Request {
                expected_revision: 1,
                ..r.clone()
            },
            refresh::Request {
                invite: "edited".into(),
                ..r.clone()
            },
        ] {
            assert!(owner
                .refresh_legacy_invite_data(&changed, &mut NeverRng)
                .is_err());
        }
        assert!(
            owner.rename_metadata_data(&rename, &mut NeverRng).is_err(),
            "another operation cannot replace a retained request"
        );
        let frozen = if cut == cuts::Fault::IndexStage {
            Some(
                fs::read(file(
                    dir.path(),
                    super::super::super::super::preparation::INDEX_STAGE_NAME,
                ))
                .unwrap(),
            )
        } else {
            None
        };
        let (result, debt) = owner.refresh_legacy_invite_data(&r, &mut NeverRng).unwrap();
        let debt = debt.unwrap();
        if let Some(frame) = frozen {
            assert_eq!(frame, *debt._transaction.frame);
        }
        assert!(owner.row_count_data().is_err());
        drop(debt);
        let new = selected(&owner);
        assert_eq!(result.revision, 1);
        assert_eq!(invite(&owner), r.invite);
        assert_eq!(new.rows[0].name, old.rows[0].name);
        assert_eq!(new.rows[0].is_dm, old.rows[0].is_dm);
        assert!(new.origin == old.origin);
        assert_eq!(new.last_issued_id, old.last_issued_id);
        assert!(owner.refresh_legacy_invite_data(&r, &mut NeverRng).is_err());
        r.expected_revision = 1;
        let (_, no_debt) = owner.refresh_legacy_invite_data(&r, &mut NeverRng).unwrap();
        assert!(no_debt.is_none());
    }
}

#[test]
fn lifecycle3_refresh_cell_cold_cuts_recover_without_rng() {
    for (n, cut) in [
        cuts::Fault::Index3PartialStage,
        cuts::Fault::IndexStage,
        cuts::Fault::IndexRename,
        cuts::Fault::Index3Covered,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xca11 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let r = input(&owner);
        let old = invite(&owner);
        cuts::arm_fault(cut);
        assert!(owner.refresh_legacy_invite_data(&r, &mut rng).is_err());
        assert!(cuts::take_fault_seen());
        drop(owner);
        let mut cold = reopen(dir.path());
        let expected = if cut == cuts::Fault::Index3PartialStage {
            old
        } else {
            r.invite
        };
        assert_eq!(invite(&cold), expected);
        assert_eq!(
            selected(&cold).rows[0].metadata_revision,
            u64::from(cut != cuts::Fault::Index3PartialStage)
        );
        assert!(!file(
            dir.path(),
            super::super::super::super::preparation::INDEX_STAGE_NAME
        )
        .exists());
        assert_eq!(cold.row_count_data().unwrap(), 1);
    }
}

#[test]
fn lifecycle3_refresh_and_rename_share_revision_and_never_rebase_delayed_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xca21);
    let mut owner = mounted(dir.path(), &mut rng);
    let mut refresh = input(&owner);
    let stale_name = request(&owner);
    let (_, debt) = owner
        .refresh_legacy_invite_data(&refresh, &mut rng)
        .unwrap();
    drop(debt);
    assert!(owner
        .rename_metadata_data(&stale_name, &mut NeverRng)
        .is_err());
    let mut current_name = stale_name.clone();
    current_name.expected_revision = 1;
    let (_, debt) = owner.rename_metadata_data(&current_name, &mut rng).unwrap();
    drop(debt);
    refresh.expected_revision = 1;
    assert!(
        owner
            .refresh_legacy_invite_data(&refresh, &mut NeverRng)
            .is_err(),
        "same invite is not proof of an old invocation"
    );
    refresh.expected_revision = 2;
    let (observed, debt) = owner
        .refresh_legacy_invite_data(&refresh, &mut NeverRng)
        .unwrap();
    assert_eq!(observed.revision, 2);
    assert!(debt.is_none());
    let mut name = request(&owner);
    name.name = "another local name".into();
    cuts::arm_fault(cuts::Fault::IndexStage);
    assert!(owner.rename_metadata_data(&name, &mut rng).is_err());
    assert!(cuts::take_fault_seen());
    let same_payload = refresh::Request {
        invite: name.name.clone(),
        ..refresh
    };
    assert!(
        owner
            .refresh_legacy_invite_data(&same_payload, &mut NeverRng)
            .is_err(),
        "tag is part of exact retry identity"
    );
    let (_, debt) = owner.rename_metadata_data(&name, &mut NeverRng).unwrap();
    drop(debt);
}

#[test]
fn lifecycle3_refresh_close_on_both_admission_sides_preserves_cold_truth() {
    for (n, edge) in [Edge::BeforeMetadataAdmission, Edge::AfterMetadataCommit]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0xca31 + n as u64);
        let mut owner = mounted(dir.path(), &mut rng);
        let r = input(&owner);
        let old = invite(&owner);
        let close = owner.handler_for_test();
        at_edge(edge, Box::new(move || close.close()));
        assert!(owner.refresh_legacy_invite_data(&r, &mut rng).is_err());
        assert!(owner.refresh_legacy_invite_data(&r, &mut NeverRng).is_err());
        drop(owner);
        let cold = reopen(dir.path());
        assert_eq!(
            invite(&cold),
            if edge == Edge::BeforeMetadataAdmission {
                old
            } else {
                r.invite
            }
        );
    }
}

#[test]
fn lifecycle3_refresh_physical_capsule_uses_same_cell_and_poison() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xca41);
    let owner = mounted(dir.path(), &mut rng);
    let r = input(&owner);
    let rename = request(&owner);
    let mut work = owner.capture_metadata_work().unwrap();
    cuts::arm_fault(cuts::Fault::IndexStage);
    assert!(work.refresh_legacy_invite_data(&r, &mut rng).is_err());
    assert!(cuts::take_fault_seen());
    drop(work);
    let mut work = owner.capture_metadata_work().unwrap();
    assert!(work.rename_metadata_data(&rename, &mut NeverRng).is_err());
    let (_, debt) = work.refresh_legacy_invite_data(&r, &mut NeverRng).unwrap();
    drop(debt);
    drop(work);
    let r = refresh::Request {
        invite: "one more".into(),
        expected_revision: 1,
        ..r
    };
    let mut work = owner.capture_metadata_work().unwrap();
    at_edge(
        Edge::AfterMetadataCommit,
        Box::new(|| panic!("actual post-effect unwind")),
    );
    assert!(catch_unwind(AssertUnwindSafe(
        || work.refresh_legacy_invite_data(&r, &mut rng)
    ))
    .is_err());
    drop(work);
    let mut retry = owner.capture_metadata_work().unwrap();
    assert!(retry.refresh_legacy_invite_data(&r, &mut NeverRng).is_err());
    drop(retry);
    drop(owner);
    let cold = reopen(dir.path());
    assert_eq!(invite(&cold), r.invite);
}

#[test]
fn lifecycle3_refresh_oversize_cell_entry_refuses_before_current_read_and_rng() {
    use crate::store::critical_namespaces::current_layout::take_checked_store_entries_for_test as reads;
    let dir = tempfile::tempdir().unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0xca61);
    let mut owner = mounted(dir.path(), &mut rng);
    let mut r = input(&owner);
    let mut name = request(&owner);
    let original = invite(&owner);
    let path = file(
        dir.path(),
        super::super::super::super::preparation::INDEX_NAME,
    );
    let stage = file(
        dir.path(),
        super::super::super::super::preparation::INDEX_STAGE_NAME,
    );
    let before = fs::read(&path).unwrap();
    r.invite = "x".repeat(super::super::super::super::data::MAX_INVITE + 1);
    name.name = "x".repeat(super::super::super::super::data::MAX_NAME + 1);
    reads();
    assert!(owner.refresh_legacy_invite_data(&r, &mut NeverRng).is_err());
    assert_eq!(reads(), 0);
    assert!(owner.rename_metadata_data(&name, &mut NeverRng).is_err());
    assert_eq!(reads(), 0);
    let mut work = owner.capture_metadata_work().unwrap();
    assert!(work.refresh_legacy_invite_data(&r, &mut NeverRng).is_err());
    assert_eq!(reads(), 0);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!stage.exists());
    // Positive control proves this observes the real read boundary. It must
    // remain current/no-change and draw no randomness or produce a WAL.
    r.invite = original;
    let (_, debt) = work.refresh_legacy_invite_data(&r, &mut NeverRng).unwrap();
    assert!(debt.is_none());
    assert_eq!(reads(), 1);
    assert_eq!(fs::read(path).unwrap(), before);
    assert!(!stage.exists());
}
