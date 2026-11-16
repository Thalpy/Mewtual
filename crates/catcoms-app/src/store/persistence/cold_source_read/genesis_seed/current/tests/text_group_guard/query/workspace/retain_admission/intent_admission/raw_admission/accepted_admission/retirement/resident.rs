//! Genuine WC/lane -> SAME flat resident. The old B/read/query/SDK payments must
//! drain outside writers while original model/domain payments follow the resident.
use super::finish::{completed_lane_fixture, LaneFinishFixture};
use super::*;
use crate::store::owner_evidence::canonical::text_lane::text_query_control_io_for_test;
use crate::store::persistence::v2_text::PreparedV2TextAcceptedQuery;
use crate::store::persistence::{
    PreparedV2TextResidentProjection, PreparedV2UnsignedOriginalCustody,
};
use std::sync::atomic::AtomicUsize;

struct Notices {
    b: Arc<AtomicBool>,
    domain: Arc<AtomicBool>,
    query: Arc<AtomicBool>,
    query_parent: Arc<AtomicBool>,
}
fn observed_fixture(family: usize, number: u64) -> (LaneFinishFixture, Notices) {
    let n = Notices {
        b: Arc::new(AtomicBool::new(false)),
        domain: Arc::new(AtomicBool::new(false)),
        query: Arc::new(AtomicBool::new(false)),
        query_parent: Arc::new(AtomicBool::new(false)),
    };
    let fixture = PreparedV2UnsignedOriginalCustody::with_resident_domain_fee_notice_for_test(
        n.domain.clone(),
        || {
            PreparedV2TextAcceptedCreate::with_metadata_fee_drop_notice_for_test(
                n.b.clone(),
                || {
                    PreparedV2TextAcceptedQuery::with_prepare_observation_for_test(
                        n.query.clone(),
                        n.query_parent.clone(),
                        Box::new(|| {}),
                        || completed_lane_fixture(family, number),
                    )
                },
            )
        },
    );
    (fixture, n)
}

/// Faults occur in the actual Current callback only AFTER BOTH owning outputs
/// exist. The error was allocated outside writers. Scope/domain operations below
/// are real negative revocations, not flags used as proof or a fabricated Saved.
enum LateReturn {
    None,
    Lost,
    Domain,
    Scope,
}
fn finish(
    t: &mut LaneFinishFixture,
    resident: &mut Option<PreparedV2TextResidentProjection>,
    disposal: &mut Option<Box<PreparedV2TextAcceptedCreate>>,
    late: LateReturn,
) -> Result<(), AppError> {
    let accepted = t
        .accepted
        .as_ref()
        .ok_or_else(|| AppError::Invalid("missing actual B".into()))?;
    let saved = accepted.saved_retirement_completion()?.clone();
    let resources = accepted.resources().clone();
    let store = t.original.layout.checked_store()?;
    let owner = t.original.owner.borrow_current()?;
    let revoke = if matches!(late, LateReturn::Scope) {
        Some(
            owner
                .warm_scope()
                .ok_or_else(|| AppError::Invalid("missing warm scope".into()))?
                .revocation(),
        )
    } else {
        None
    };
    let mut lost = Some(AppError::Invalid(
        "injected lost resident callback return".into(),
    ));
    store.with_v2_text_current(&owner, |admission| {
        let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
        let finish = guard.admit_completed_finish(
            t.retired.as_ref().unwrap(),
            &saved,
            t.accepted.as_ref().unwrap(),
        )?;
        PreparedV2TextAcceptedCreate::finish_resident_into(
            &finish,
            &mut t.accepted,
            resident,
            disposal,
        )?;
        assert!(t.accepted.is_none() && resident.is_some() && disposal.is_some());
        // The original still-held genuine guard can repeat only cached association;
        // it cannot transfer/rederive/sign/advance a second mathematical owner.
        PreparedV2TextAcceptedCreate::finish_resident_into(
            &finish,
            &mut t.accepted,
            resident,
            disposal,
        )?;
        match late {
            LateReturn::None => finish.validate_negative(),
            LateReturn::Lost => Err(lost.take().unwrap()),
            LateReturn::Domain => {
                resident.as_ref().unwrap().cancel_original_domain_for_test();
                finish.validate_negative()
            }
            LateReturn::Scope => {
                revoke.as_ref().unwrap().revoke();
                Ok(()) // the real Current postcheck must observe actual ScopeStop
            }
        }
    })
}

fn dispose_old(
    t: LaneFinishFixture,
    disposal: Option<Box<PreparedV2TextAcceptedCreate>>,
    resident: &PreparedV2TextResidentProjection,
    notices: &Notices,
    read_alive: impl Fn() -> bool,
    expected_header: [u8; 32],
) -> TextFixture {
    let LaneFinishFixture {
        accepted,
        predecessor,
        original,
        model_dropped,
        sdk_dropped,
        basis,
        lane,
        drained,
        retired,
    } = t;
    assert!(accepted.is_none() && lane.is_none() && drained.is_none());
    assert!(read_alive());
    assert!(!notices.b.load(Ordering::SeqCst));
    assert!(!sdk_dropped.load(Ordering::SeqCst));
    assert!(!model_dropped.load(Ordering::SeqCst));
    // These are actual removed-lane/basis/predecessor tails. They also retain old
    // metadata/read payments, so disposing only B is insufficient and not claimed.
    drop(predecessor);
    drop(basis);
    drop(retired);
    drop(disposal); // actual whole SDK/query/WAL/transaction payload, OUTSIDE writers
    assert!(
        !read_alive(),
        "resident must not keep original transaction read1MiB"
    );
    assert!(
        notices.b.load(Ordering::SeqCst),
        "resident must not keep B2MiB"
    );
    assert!(
        notices.query.load(Ordering::SeqCst),
        "actual accepted query fee drains"
    );
    assert!(
        notices.query_parent.load(Ordering::SeqCst),
        "actual query parent drains"
    );
    assert!(
        sdk_dropped.load(Ordering::SeqCst),
        "actual original SDK2MiB drains"
    );
    assert!(
        !model_dropped.load(Ordering::SeqCst),
        "SAME model1MiB follows resident"
    );
    assert!(
        !notices.domain.load(Ordering::SeqCst),
        "SAME domain16KiB follows resident"
    );
    let (actual, selected, sequence) = resident.resident_header_state_for_test();
    assert_eq!(actual, expected_header);
    assert_eq!(selected, expected_header);
    assert_eq!(sequence, 2);
    // Only historical last-request recognition survives disposal/Stop. Hashing
    // occurs here outside all writers and grants no canonical token/effect role.
    let request = V2CreateRequest {
        token: [31; 32],
        channel: 7,
        text: "retained before acceptance",
    };
    resident.require_completed_request_data(&request).unwrap();
    for changed in [
        V2CreateRequest {
            token: [32; 32],
            channel: 7,
            text: request.text,
        },
        V2CreateRequest {
            token: request.token,
            channel: 8,
            text: request.text,
        },
        V2CreateRequest {
            token: request.token,
            channel: request.channel,
            text: "changed request",
        },
    ] {
        assert!(resident.require_completed_request_data(&changed).is_err());
    }
    resident.require_completed_request_data(&request).unwrap();
    assert_eq!(
        text_query_control_io_for_test(original.layout.checked_store().unwrap()),
        0
    );
    original
}

#[test]
fn actual_three_family_wc_finish_moves_same_model_and_drops_every_old_chain_even_after_lost_return()
{
    for family in 0..3 {
        let core = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
        let sign = Arc::new(AtomicUsize::new(0));
        PreparedV2UnsignedOriginalCustody::with_accepted_advance_observation_for_test(
            core.clone(),
            || {
                PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
                    sign.clone(),
                    None,
                    || {
                        let (mut t, notices) = observed_fixture(family, 0x82600 + family as u64);
                        let quota = {
                            let source = t
                                .accepted
                                .as_ref()
                                .unwrap()
                                .borrow_completed_retirement_source_data()
                                .unwrap();
                            let head = source.head_data();
                            let reclaimed = source.reclaimed_data();
                            (
                                head.live_bytes,
                                head.live_objects,
                                reclaimed.bytes,
                                reclaimed.objects,
                            )
                        };
                        let effective = t
                            .accepted
                            .as_ref()
                            .unwrap()
                            .saved_retirement_completion()
                            .unwrap()
                            .effective_baseline_data()
                            .unwrap();
                        let header = t
                            .accepted
                            .as_ref()
                            .unwrap()
                            .accepted_header_state_for_test()
                            .unwrap()
                            .0;
                        let read_alive = t
                            .accepted
                            .as_ref()
                            .unwrap()
                            .resources()
                            .original_body_liveness_for_test();
                        let model_notice = t.model_dropped.clone();
                        let target = t
                            .original
                            .owner
                            .borrow_current()
                            .unwrap()
                            .source()
                            .source()
                            .capture
                            .target
                            .to_path_buf();
                        let paths = [target.join("head.bin"), target.join("text.wal")];
                        let bytes: Vec<_> =
                            paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
                        let mut resident = None;
                        let mut disposal = None;
                        let late = match family {
                            0 => LateReturn::None,
                            1 => LateReturn::Lost,
                            _ => LateReturn::Domain,
                        };
                        let result = finish(&mut t, &mut resident, &mut disposal, late);
                        assert_eq!(result.is_ok(), family == 0);
                        // Lost return/Stop cannot renew positive authority, but exact
                        // cached owning outputs remain recognizable for finite disposal.
                        PreparedV2TextAcceptedCreate::require_finished_resident_outputs(
                            &t.accepted,
                            resident.as_ref().unwrap(),
                            &disposal,
                        )
                        .unwrap();
                        for (path, expected) in paths.iter().zip(bytes) {
                            assert_eq!(std::fs::read(path).unwrap(), expected);
                        }
                        let resident_quota =
                            resident.as_ref().unwrap().resident_quota_state_for_test();
                        assert_eq!(
                            resident_quota, quota,
                            "raw O_B baseline and exact WC totals stay separate"
                        );
                        assert_eq!(
                            resident_quota.0.checked_sub(resident_quota.2),
                            Some(effective.bytes)
                        );
                        assert_eq!(
                            resident_quota.1.checked_sub(resident_quota.3),
                            Some(effective.objects)
                        );
                        PreparedV2TextAcceptedCreate::require_finished_resident_outputs(
                            &t.accepted,
                            resident.as_ref().unwrap(),
                            &disposal,
                        )
                        .unwrap();
                        assert_eq!(
                            resident.as_ref().unwrap().resident_quota_state_for_test(),
                            quota,
                            "lost-return retry must not subtract or add WC totals again"
                        );
                        let original = dispose_old(
                            t,
                            disposal,
                            resident.as_ref().unwrap(),
                            &notices,
                            read_alive,
                            header,
                        );
                        assert_eq!(core[0].load(Ordering::SeqCst), 1);
                        assert_eq!(core[1].load(Ordering::SeqCst), 1);
                        assert_eq!(sign.load(Ordering::SeqCst), 1);
                        assert_eq!(
                            resident
                                .as_ref()
                                .unwrap()
                                .validate_negative(original.layout.checked_store().unwrap())
                                .is_ok(),
                            family != 2
                        );
                        drop(resident);
                        assert!(model_notice.load(Ordering::SeqCst));
                        assert!(notices.domain.load(Ordering::SeqCst));
                        drop(original);
                    },
                )
            },
        );
    }
}

#[test]
fn actual_fresh_wc_finish_refuses_damage_before_take_and_late_scope_stop_preserves_both_owners() {
    let (mut t, notices) = observed_fixture(0, 0x82610);
    let header = t
        .accepted
        .as_ref()
        .unwrap()
        .accepted_header_state_for_test()
        .unwrap()
        .0;
    let read_alive = t
        .accepted
        .as_ref()
        .unwrap()
        .resources()
        .original_body_liveness_for_test();
    let target = t
        .original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .to_path_buf();
    let wal = target.join("text.wal");
    let original_wal = std::fs::read(&wal).unwrap();
    let mut damaged = original_wal.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    std::fs::write(&wal, &damaged).unwrap();
    let mut resident = None;
    let mut disposal = None;
    assert!(finish(&mut t, &mut resident, &mut disposal, LateReturn::None).is_err());
    assert!(t.accepted.is_some() && resident.is_none() && disposal.is_none());
    assert_eq!(
        t.accepted
            .as_ref()
            .unwrap()
            .accepted_header_state_for_test()
            .unwrap()
            .0,
        header
    );
    assert_eq!(std::fs::read(&wal).unwrap(), damaged);
    std::fs::write(&wal, &original_wal).unwrap();
    assert!(finish(&mut t, &mut resident, &mut disposal, LateReturn::Scope).is_err());
    assert!(t.accepted.is_none() && resident.is_some() && disposal.is_some());
    PreparedV2TextAcceptedCreate::require_finished_resident_outputs(
        &t.accepted,
        resident.as_ref().unwrap(),
        &disposal,
    )
    .unwrap();
    assert!(resident
        .as_ref()
        .unwrap()
        .validate_negative(t.original.layout.checked_store().unwrap())
        .is_err());
    assert_eq!(std::fs::read(&wal).unwrap(), original_wal);
    let model_notice = t.model_dropped.clone();
    let original = dispose_old(
        t,
        disposal,
        resident.as_ref().unwrap(),
        &notices,
        read_alive,
        header,
    );
    drop(resident);
    assert!(model_notice.load(Ordering::SeqCst));
    assert!(notices.domain.load(Ordering::SeqCst));
    drop(original);
}

#[test]
fn actual_finish_refuses_an_occupied_other_resident_without_taking_either_original() {
    let (mut a, na) = observed_fixture(0, 0x82620);
    let mut ra = None;
    let mut da = None;
    finish(&mut a, &mut ra, &mut da, LateReturn::None).unwrap();
    let a_header = ra.as_ref().unwrap().resident_header_state_for_test();
    let (mut b, nb) = observed_fixture(1, 0x82621);
    let b_header = b
        .accepted
        .as_ref()
        .unwrap()
        .accepted_header_state_for_test()
        .unwrap();
    let mut db = None;
    assert!(finish(&mut b, &mut ra, &mut db, LateReturn::None).is_err());
    assert!(b.accepted.is_some() && db.is_none());
    assert_eq!(
        b.accepted
            .as_ref()
            .unwrap()
            .accepted_header_state_for_test()
            .unwrap(),
        b_header
    );
    assert_eq!(
        ra.as_ref().unwrap().resident_header_state_for_test(),
        a_header
    );
    assert!(!a.model_dropped.load(Ordering::SeqCst) && !b.model_dropped.load(Ordering::SeqCst));
    assert!(!na.b.load(Ordering::SeqCst) && !nb.b.load(Ordering::SeqCst));
    let mut rb = None;
    finish(&mut b, &mut rb, &mut db, LateReturn::None).unwrap();
    PreparedV2TextAcceptedCreate::require_finished_resident_outputs(
        &a.accepted,
        ra.as_ref().unwrap(),
        &da,
    )
    .unwrap();
    PreparedV2TextAcceptedCreate::require_finished_resident_outputs(
        &b.accepted,
        rb.as_ref().unwrap(),
        &db,
    )
    .unwrap();
    assert!(
        PreparedV2TextAcceptedCreate::require_finished_resident_outputs(
            &b.accepted,
            ra.as_ref().unwrap(),
            &db
        )
        .is_err()
    );
    // All actual outside owners drain before either resident; no new Current or
    // claimed third source can substitute for destroying the old owning chains.
    drop(a);
    drop(b);
    drop(da);
    drop(db);
    assert!(na.b.load(Ordering::SeqCst) && nb.b.load(Ordering::SeqCst));
    assert!(!na.domain.load(Ordering::SeqCst) && !nb.domain.load(Ordering::SeqCst));
    drop(ra);
    drop(rb);
    assert!(na.domain.load(Ordering::SeqCst) && nb.domain.load(Ordering::SeqCst));
}
