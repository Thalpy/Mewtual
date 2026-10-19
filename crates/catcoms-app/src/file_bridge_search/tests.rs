use super::*;
use catcoms_mls::MlsDevice;
use std::collections::{BTreeMap, BTreeSet};

fn roster(others: usize) -> (Vec<MlsDevice>, ServerGroup) {
    let mut devices = vec![MlsDevice::generate().unwrap()];
    let mut group = ServerGroup::create(&devices[0]).unwrap();
    for _ in 0..others {
        let device = MlsDevice::generate().unwrap();
        group
            .add_member(&devices[0], device.key_package().unwrap())
            .unwrap();
        devices.push(device);
    }
    assert_eq!(group.member_count(), devices.len());
    (devices, group)
}
fn helper(device: &MlsDevice, peer: u64) -> Helper {
    Helper {
        device: device.device_id(),
        peer: PeerId::from_u64(peer),
    }
}
fn ids(choice: &Choice) -> Vec<DeviceId> {
    choice
        .sources
        .iter()
        .map(MemberCatalogEntry::device_id)
        .collect()
}
fn ready(step: Step) -> Choice {
    match step {
        Step::Ready(choice) => choice,
        other => panic!("expected choice, got {other:?}"),
    }
}

#[test]
fn proposal_survives_local_busy_without_spending_coverage_and_excludes_actual_c_and_b() {
    let (devices, group) = roster(7);
    let c = devices[0].device_id();
    let b = helper(&devices[1], 1);
    let mut search = Search::new(c, group.current_member_catalog().unwrap());
    let first = ready(
        search
            .propose(&group, &[[1; 32]], &[helper(&devices[0], 99), b])
            .unwrap(),
    );
    assert_eq!(
        search.helpers,
        vec![b.device],
        "actual C cannot become its own helper"
    );
    assert_eq!(first.sources.len(), 4);
    assert!(!ids(&first).contains(&c) && !ids(&first).contains(&b.device));
    let retry = ready(search.propose(&group, &[[1; 32]], &[b]).unwrap());
    assert_eq!(ids(&first), ids(&retry));
    assert_eq!(first.ticket, retry.ticket);
    assert!(search.admitted(&retry));
    assert!(
        !search.admitted(&first),
        "same preparation cannot consume a second batch"
    );
    let second = ready(search.propose(&group, &[[1; 32]], &[b]).unwrap());
    let expected: BTreeSet<_> = devices[2..].iter().map(MlsDevice::device_id).collect();
    let actual: BTreeSet<_> = ids(&first).into_iter().chain(ids(&second)).collect();
    assert_eq!(actual, expected, "four misses cannot hide the fifth source");
    assert!(search.admitted(&second));
    assert!(matches!(
        search.propose(&group, &[[1; 32]], &[b]).unwrap(),
        Step::Exhausted
    ));
}

#[test]
fn each_helper_and_exact_variant_covers_its_own_complete_native_catalog() {
    let (devices, group) = roster(10);
    let helpers = [helper(&devices[1], 1), helper(&devices[2], 2)];
    let variants = [[1; 32], [2; 32]];
    let mut search = Search::new(
        devices[0].device_id(),
        group.current_member_catalog().unwrap(),
    );
    let mut seen = BTreeMap::<([u8; 32], DeviceId), BTreeSet<DeviceId>>::new();
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(
            steps < 100,
            "fixed native catalog must make finite progress"
        );
        match search.propose(&group, &variants, &helpers).unwrap() {
            Step::Ready(choice) => {
                assert!(ids(&choice).windows(2).all(|p| p[0] < p[1]));
                seen.entry((choice.variant, choice.helper.device))
                    .or_default()
                    .extend(ids(&choice));
                assert!(search.admitted(&choice));
            }
            Step::Yield => {}
            Step::Exhausted => break,
        }
    }
    assert_eq!(seen.len(), 4);
    for variant in variants {
        for b in helpers {
            let expected: BTreeSet<_> = devices
                .iter()
                .map(MlsDevice::device_id)
                .filter(|id| *id != devices[0].device_id() && *id != b.device)
                .collect();
            assert_eq!(seen[&(variant, b.device)], expected);
        }
    }
}

#[test]
fn helper_replacement_and_temporarily_unavailable_proposal_do_not_reset_or_spend_coverage() {
    let (devices, group) = roster(8);
    let b = helper(&devices[1], 1);
    let d = helper(&devices[2], 2);
    let mut search = Search::new(
        devices[0].device_id(),
        group.current_member_catalog().unwrap(),
    );
    let old = ready(search.propose(&group, &[[1; 32]], &[b, d]).unwrap());
    let replacement = Helper {
        peer: PeerId::from_u64(99),
        ..b
    };
    let replaced = ready(
        search
            .propose(&group, &[[1; 32]], &[replacement, d])
            .unwrap(),
    );
    assert_eq!(ids(&old), ids(&replaced));
    assert_ne!(old.ticket, replaced.ticket);
    assert!(
        !search.admitted(&old),
        "stale route cannot commit replacement preparation"
    );
    let alternate = ready(search.propose(&group, &[[1; 32]], &[d]).unwrap());
    assert_eq!(alternate.helper, d);
    assert!(search.admitted(&alternate));
    let resumed = ready(
        search
            .propose(&group, &[[1; 32]], &[replacement, d])
            .unwrap(),
    );
    assert_eq!(resumed.helper, replacement);
    assert_eq!(ids(&resumed), ids(&old));
    assert!(search.admitted(&resumed));
    search.unsupported(b.device);
    for _ in 0..10 {
        match search
            .propose(&group, &[[1; 32]], &[replacement, d])
            .unwrap()
        {
            Step::Ready(choice) => {
                assert_eq!(choice.helper, d);
                assert!(search.admitted(&choice));
            }
            Step::Yield => {}
            Step::Exhausted => return,
        }
    }
    panic!("unsupported helper must not restart coverage");
}

#[test]
fn helper_and_variant_slots_remain_bounded_without_roster_or_source_count_cutoff() {
    let (devices, group) = roster(6);
    let helpers: Vec<_> = devices[1..]
        .iter()
        .enumerate()
        .map(|(i, d)| helper(d, i as u64 + 1))
        .collect();
    let variants = [[1; 32], [2; 32], [3; 32], [4; 32], [5; 32]];
    let mut search = Search::new(
        devices[0].device_id(),
        group.current_member_catalog().unwrap(),
    );
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 100);
        match search.propose(&group, &variants, &helpers).unwrap() {
            Step::Ready(choice) => {
                assert_ne!(choice.variant, [5; 32]);
                assert!(helpers[..4].contains(&choice.helper));
                assert!(search.admitted(&choice));
            }
            Step::Yield => {}
            Step::Exhausted => break,
        }
    }
    assert_eq!(search.progress.len(), HELPERS * VARIANTS);
    assert!(search.pending.is_none());
    assert!(matches!(
        search.propose(&group, &[[9; 32]], &[helpers[5]]).unwrap(),
        Step::Exhausted
    ));
}

#[test]
fn even_exhausted_or_pending_search_refuses_a_new_epoch() {
    for consume in [false, true] {
        let (devices, mut group) = roster(if consume { 1 } else { 2 });
        let c = devices[0].device_id();
        let b = helper(&devices[1], 1);
        let mut search = Search::new(c, group.current_member_catalog().unwrap());
        if consume {
            assert!(matches!(
                search.propose(&group, &[[1; 32]], &[b]).unwrap(),
                Step::Yield
            ));
            assert!(matches!(
                search.propose(&group, &[[1; 32]], &[b]).unwrap(),
                Step::Exhausted
            ));
        } else {
            assert!(matches!(
                search.propose(&group, &[[1; 32]], &[b]).unwrap(),
                Step::Ready(_)
            ));
        }
        let joined = MlsDevice::generate().unwrap();
        group
            .add_member(&devices[0], joined.key_package().unwrap())
            .unwrap();
        assert!(matches!(
            search.propose(&group, &[[1; 32]], &[b]),
            Err(MemberCatalogError::Stale)
        ));
    }
}

#[test]
fn admitted_choice_is_bound_to_its_original_search_and_exact_proposed_fields() {
    let (devices, group) = roster(6);
    let b = helper(&devices[1], 1);
    let view = group.current_member_catalog().unwrap();
    let mut first = Search::new(devices[0].device_id(), view.clone());
    let mut second = Search::new(devices[0].device_id(), view);
    let a = ready(first.propose(&group, &[[1; 32]], &[b]).unwrap());
    let b_choice = ready(second.propose(&group, &[[1; 32]], &[b]).unwrap());
    assert_eq!(a.ticket, b_choice.ticket);
    assert!(
        !second.admitted(&a),
        "matching metadata cannot consume another job's proposal"
    );
    let mut changed = b_choice.clone();
    changed.variant = [2; 32];
    assert!(!second.admitted(&changed));
    let mut changed = b_choice.clone();
    changed.sources.pop();
    assert!(!second.admitted(&changed));
    assert!(second.admitted(&b_choice));
    assert!(first.admitted(&a));
}

#[test]
fn helper_peer_cycle_cannot_make_an_old_unstarted_choice_current_again() {
    let (devices, group) = roster(6);
    let first_peer = helper(&devices[1], 1);
    let second_peer = Helper {
        peer: PeerId::from_u64(2),
        ..first_peer
    };
    let mut search = Search::new(
        devices[0].device_id(),
        group.current_member_catalog().unwrap(),
    );
    let original = ready(search.propose(&group, &[[1; 32]], &[first_peer]).unwrap());
    let changed = ready(search.propose(&group, &[[1; 32]], &[second_peer]).unwrap());
    let returned = ready(search.propose(&group, &[[1; 32]], &[first_peer]).unwrap());
    assert_eq!(ids(&original), ids(&changed));
    assert_eq!(ids(&original), ids(&returned));
    assert!(
        !search.admitted(&original),
        "original PeerId clone cannot revive after replacement"
    );
    assert!(!search.admitted(&changed));
    let busy = ready(search.propose(&group, &[[1; 32]], &[first_peer]).unwrap());
    assert_eq!(
        returned.ticket, busy.ticket,
        "unchanged local Busy keeps its exact proposal"
    );
    assert!(search.admitted(&returned));
}
