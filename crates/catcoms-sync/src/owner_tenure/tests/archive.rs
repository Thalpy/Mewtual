//! CORE-005: the archived Observed-tenure witness.
//!
//! The witness is the one piece of owner HISTORY this device keeps, and Agent 3's historical report
//! admission trusts it in place of a pair's own self-signatures. So the questions these tests ask are
//! the ones that decide whether that trust is earned: is it minted only from a retirement this device
//! actually watched, does anything weaker leave it alone, does it survive a save and reload exactly,
//! does corruption refuse rather than read as "no history", and can the application reach it only
//! through a permit whose snapshot was durably saved?
use super::*;
use crate::receipt_head::DurableOwnerSnapshot;

/// Sync the Welcome joiners up to the founder's epoch and let them commit, as the rejoin test does.
/// The hub is returned so the caller keeps the network alive.
async fn members(count: u64) -> (impl Sized, Vec<Node>, Vec<DeviceId>) {
    let (hub, mut nodes, ids) = build_members(count).await;
    let initial: Vec<_> = nodes[0].commit_log.iter().cloned().collect();
    for node in nodes.iter_mut().skip(1) {
        for record in &initial {
            if record.commit_epoch == node.epoch() {
                assert!(node.apply_commit_in_order(record));
            }
        }
        node.config.max_committer_rank = 2;
        node.config.stage_decision_window_ms = 0;
    }
    (hub, nodes, ids)
}

/// Remove `leaving` through the real contested-commit path, committed by `committers` and applied
/// by every node in `appliers`.
async fn remove_through_contest(
    nodes: &mut [Node],
    committers: [usize; 2],
    appliers: std::ops::Range<usize>,
    leaving: &DeviceId,
) {
    for at in committers {
        nodes[at].remove(leaving).await.unwrap();
    }
    let candidates = committers.map(|at| nodes[at].pending.as_ref().unwrap().best.clone());
    for node in &mut nodes[appliers] {
        for candidate in &candidates {
            node.contest_commit(candidate.clone());
        }
        assert!(node.resolve_pending_if_expired());
    }
}

fn key_of(node: &Node) -> [u8; 32] {
    node.device.public_key_bytes().try_into().unwrap()
}

fn expected(
    group: &ServerGroup,
    key: [u8; 32],
    start: u64,
    retired_at: u64,
) -> ArchivedOwnerTenure {
    ArchivedOwnerTenure {
        owner_key: key,
        start,
        tenure_id: catcoms_replication::tenure_id(&group.group_id(), &key, start),
        retired_at,
    }
}

/// N50's sync half, with no injected witness: an observer watches B take office and then retire,
/// persists and reopens, and only then reaches B's tenure - through a durable permit.
///
/// It also pins the refusal on the other side of the same run. The Welcome joiners never observed
/// A's founding tenure begin, so A's retirement must NOT mint a witness on them: that tenure is
/// `Unknown` here, and CORE-005 refuses to archive what was never positively observed.
#[tokio::test]
async fn an_observer_archives_a_retirement_it_watched_and_only_that_one() {
    let (_hub, mut nodes, ids) = members(4).await;
    for node in &nodes[1..] {
        assert_eq!(
            observed_start(node),
            None,
            "precondition: the joiners never saw A's founding tenure begin"
        );
    }

    // A -> B. A's tenure was Unknown to every remaining node, so nothing is archived.
    remove_through_contest(&mut nodes, [1, 2], 1..4, &ids[0]).await;
    let b_start = observed_start(&nodes[2]).expect("B's tenure began in front of the observers");
    for node in &nodes[1..] {
        assert_eq!(node.designated_committer_id(), Some(ids[1]));
        assert_eq!(
            node.owner_tenure.archived(&node.group),
            None,
            "an Unknown departure must not mint a witness"
        );
    }

    // B -> C. B's tenure WAS observed, start to finish, by C and D.
    let b_key = key_of(&nodes[1]);
    remove_through_contest(&mut nodes, [2, 3], 2..4, &ids[1]).await;
    for node in &nodes[2..] {
        assert_eq!(node.designated_committer_id(), Some(ids[2]));
        let want = expected(&node.group, b_key, b_start, node.epoch());
        assert_eq!(node.owner_tenure.archived(&node.group), Some(want));
        assert!(b_start < node.epoch());
    }

    // Persist and reopen C, then reach B's tenure only through a durable permit.
    let c = &mut nodes[2];
    let mut reopened = restore(&c.snapshot().unwrap()).unwrap();
    let want = expected(&reopened.group, b_key, b_start, reopened.epoch());
    assert_eq!(
        reopened.owner_tenure.archived(&reopened.group),
        Some(want),
        "the witness must survive a save and reload exactly"
    );
    let failed: Result<DurableOwnerSnapshot, &str> = reopened
        .prepare_receipt_head_snapshot(|_, _| Err("disk full"))
        .unwrap();
    assert!(
        failed.is_err(),
        "a failed snapshot write must yield no permit, and so no witness"
    );
    let permit = reopened
        .prepare_receipt_head_snapshot(|_, _| Ok::<(), ()>(()))
        .unwrap()
        .unwrap();
    let seen = reopened
        .with_durable_owner_history(&permit, |_, _, _, tenure, archive| {
            (tenure, archive.copied())
        })
        .unwrap();
    assert_eq!(seen, (reopened.epoch(), Some(want)));
}

/// Every weaker departure leaves the witness alone, and only a fully observed one replaces it.
///
/// Driven at the `applied` level, on the leaf-change step the rejoin test also uses, because the
/// interesting inputs are the device's own prior knowledge - Observed, Imported, none - and those
/// are exactly what a real group cannot be made to vary on demand.
#[tokio::test]
async fn only_a_fully_observed_contiguous_retirement_mints_or_replaces_the_witness() {
    let (_, nodes, _) = build_members(3).await;
    let group = &nodes[0].group;
    let live = Position::of(group);
    assert!(live.epoch >= 2, "the gap case needs two predecessor epochs");
    let key = live.owner_key.expect("the committer's key is readable");
    let (index, digest) = live.leaf.unwrap();
    let mut other = digest;
    other[0] ^= 0xff;
    // A contiguous step onto a new leaf identity: a new tenure under the same owner.
    let new_tenure = Position {
        leaf: Some((index, other)),
        epoch: live.epoch - 1,
        ..live
    };
    // The same, across a gap: contiguity cannot be established, so no tenure is derived.
    let gap = Position {
        epoch: live.epoch - 2,
        ..new_tenure
    };
    // An ordinary same-owner step: knowledge is preserved, no tenure ends.
    let preserve = Position {
        epoch: live.epoch - 1,
        ..live
    };
    let prior = expected(group, [7; 32], 0, 1);
    let run = |before: Position, start: Option<u64>, imported: bool| {
        let mut state = OwnerTenure::unknown(group);
        state.position = before;
        state.start = start;
        state.imported = imported;
        state.archive = Some(prior);
        state.applied(before, group);
        state.archive
    };

    assert_eq!(
        run(new_tenure, Some(0), false),
        Some(expected(group, key, 0, live.epoch)),
        "a fully observed retirement replaces the previous witness"
    );
    assert_eq!(
        run(new_tenure, Some(0), true),
        Some(prior),
        "an Imported tenure cannot mint or replace"
    );
    assert_eq!(
        run(new_tenure, None, false),
        Some(prior),
        "an Unknown tenure cannot mint or replace"
    );
    assert_eq!(
        run(gap, Some(0), false),
        Some(prior),
        "a gap cannot mint or replace"
    );
    assert_eq!(
        run(preserve, Some(0), false),
        Some(prior),
        "a same-owner commit that ends no tenure leaves the witness alone"
    );

    // A saved position that is not exactly `before` is a missed hook, not observed history.
    let mut stale = OwnerTenure::unknown(group);
    stale.position = preserve;
    stale.start = Some(0);
    stale.archive = Some(prior);
    stale.applied(new_tenure, group);
    assert_eq!(stale.archive, Some(prior));
}

/// v3 is v2 plus one framed witness, written only when a witness exists, and every check restore
/// can make refuses the whole tail rather than dropping the witness.
#[tokio::test]
async fn the_witness_round_trips_exactly_and_corruption_refuses_instead_of_reading_as_none() {
    let (_, nodes, _) = build_members(2).await;
    let group = &nodes[0].group;
    let live = Position::of(group);
    let (index, digest) = live.leaf.unwrap();
    let mut other = digest;
    other[0] ^= 0xff;
    let before = Position {
        leaf: Some((index, other)),
        epoch: live.epoch - 1,
        ..live
    };
    let mut state = OwnerTenure::unknown(group);
    state.position = before;
    state.start = Some(0);
    state.applied(before, group);
    let witness = state.archive.expect("this step mints");

    let without = OwnerTenure {
        archive: None,
        ..OwnerTenure::unknown(group)
    };
    let v2 = OwnerTenure {
        start: state.start,
        ..without
    }
    .encode(group)
    .unwrap();
    let bytes = state.encode(group).unwrap();
    assert_eq!(v2[0], 2, "a state with no witness keeps its exact v2 bytes");
    assert_eq!(bytes[0], 3);
    assert_eq!(&bytes[1..98], &v2[1..], "v3 is v2 with one field appended");
    let mut tail = 80u32.to_be_bytes().to_vec();
    tail.extend_from_slice(witness.owner_key());
    tail.extend_from_slice(&witness.start().to_be_bytes());
    tail.extend_from_slice(witness.tenure_id());
    tail.extend_from_slice(&witness.retired_at().to_be_bytes());
    assert_eq!(&bytes[98..], &tail[..], "pin the witness layout");
    assert!(bytes.len() - 98 <= MAX_HISTORICAL_OWNER_WITNESS_BYTES);
    assert_eq!(
        OwnerTenure::decode(&bytes, group).unwrap().archive,
        Some(witness)
    );
    assert_eq!(OwnerTenure::decode(&v2, group).unwrap().archive, None);

    // Key, start and id are bound together by the derived id: any flip refuses.
    for at in [102, 133, 141, 150, 173] {
        let mut corrupt = bytes.clone();
        corrupt[at] ^= 1;
        assert!(
            OwnerTenure::decode(&corrupt, group).is_err(),
            "corrupting witness byte {at} must refuse"
        );
    }
    // `retired_at` is range-checked, not bound: a flip that stays inside `start < retired_at <=
    // epoch` is not detectable here. The vault's authentication of the whole snapshot is what
    // catches that, as it does for every other field; these checks are the defence behind it.
    let retired = |value: u64| {
        let mut changed = bytes.clone();
        changed[174..182].copy_from_slice(&value.to_be_bytes());
        changed
    };
    for bad in [witness.start(), live.epoch + 1] {
        assert!(
            OwnerTenure::decode(&retired(bad), group).is_err(),
            "retired_at {bad} is outside start < retired_at <= epoch"
        );
    }
    // Framing: an empty witness, a short one, a long one, trailing bytes, an unknown version.
    let mut empty = bytes[..98].to_vec();
    empty.extend_from_slice(&0u32.to_be_bytes());
    let mut short = bytes[..98].to_vec();
    short.extend_from_slice(&79u32.to_be_bytes());
    short.extend_from_slice(&bytes[102..181]);
    let mut long = bytes[..98].to_vec();
    long.extend_from_slice(&81u32.to_be_bytes());
    long.extend_from_slice(&bytes[102..]);
    long.push(0);
    let mut trailing = bytes.clone();
    trailing.push(0);
    let mut v4 = bytes.clone();
    v4[0] = 4;
    let mut v2_with_witness = bytes.clone();
    v2_with_witness[0] = 2;
    for (name, case) in [
        ("empty", empty),
        ("short", short),
        ("long", long),
        ("trailing", trailing),
        ("v4", v4),
        ("v2 with a witness", v2_with_witness),
    ] {
        assert!(
            OwnerTenure::decode(&case, group).is_err(),
            "{name} must refuse"
        );
    }
}
