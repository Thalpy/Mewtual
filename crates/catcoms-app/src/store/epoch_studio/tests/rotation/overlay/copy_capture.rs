//! C1: the composite destination capture, and what it refuses to call current.
//!
//! The capture's whole job is to let a plan be built off custody and then proved still true. So the
//! tests that matter are the ones where something moved underneath it: a destination edited, a
//! recovery record appearing where there was none, another device finishing someone else's capture.
use super::*;

/// A second Flipnote in the same group, which is the primary cross-document destination.
fn other_target(f: &Fixture) -> StudioTarget {
    StudioTarget::Flipnote {
        channel: f.target.channel(),
        object: [0x5e; 16],
    }
}

/// Give a target a real Studio source record, so it is a destination a copy could address.
///
/// Builds on whatever the target already has rather than starting a fresh epoch each time: two
/// independent `StudioEpoch::new` states for one document replay the same actor sequence and the
/// second ingest is refused as a duplicate, which is the store being right rather than the test
/// being unlucky.
fn seed(f: &Fixture, store: &mut ServerStore, target: StudioTarget, nonce: u8, body: Vec<u8>) {
    let logical = target.document(&f.group.group_id()).unwrap();
    let mut b = budget(store, f);
    store
        .edit_studio_epoch(
            SERVER,
            &f.group,
            target,
            epoch_zero_id(logical.doc_type, &logical.logical_key),
            &f.device,
            catcoms_replication::DomainOp {
                body,
                nonce: [nonce; 16],
                doc_type: logical.doc_type,
                logical_key: logical.logical_key.clone(),
            },
            100,
            &mut rng(),
            &mut b,
        )
        .expect("an ordinary local edit to the destination");
}

fn title(text: &str) -> Vec<u8> {
    FlipnoteOp::SetHeader(FlipnoteHeader::Title(text.into()))
        .encode()
        .unwrap()
}

#[test]
fn a_destination_capture_is_current_until_either_of_its_two_records_moves() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let destination = other_target(&f);
    seed(&f, &mut store, destination, 1, title("before"));

    let capture = store
        .capture_studio_destination(SERVER, &f.group, destination, &f.device)
        .expect("a seeded destination can be captured");
    assert!(
        store
            .studio_destination_is_current(SERVER, &f.group, destination, &f.device, &capture.stamp)
            .unwrap(),
        "nothing has moved yet"
    );

    // The capture carries the destination's actual authenticated plaintext, not a placeholder: the
    // detached planner has nothing else to build the destination projection from.
    assert!(!capture.source.is_empty());
    let again = store
        .capture_studio_destination(SERVER, &f.group, destination, &f.device)
        .unwrap();
    assert_eq!(
        capture.source, again.source,
        "two captures of an unchanged destination must carry the same bytes"
    );

    // The Studio source record changes: a copy planned against the old bytes is stale.
    seed(&f, &mut store, destination, 2, title("after"));
    assert!(
        !store
            .studio_destination_is_current(SERVER, &f.group, destination, &f.device, &capture.stamp)
            .unwrap(),
        "an edited destination must not still be current"
    );
    let moved = store
        .capture_studio_destination(SERVER, &f.group, destination, &f.device)
        .unwrap();
    assert_ne!(
        capture.source, moved.source,
        "and the bytes a later capture carries must be the new ones"
    );
}

/// Stage one recovery snapshot for the destination and return it, so a second can differ from it.
fn stage_recovery(f: &Fixture, store: &mut ServerStore, destination: StudioTarget) {
    let logical = destination.document(&f.group.group_id()).unwrap();
    let projection = store
        .with_studio_source(SERVER, &f.group, destination, &f.device, |s| s.projection())
        .unwrap()
        .unwrap();
    let saved = catcoms_replication::studio::StudioRecovery::snapshot(
        &projection,
        None,
        catcoms_replication::RecoveryReason::Excluded,
        [7; 32],
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    store
        .update_epoch_recovery(
            SERVER,
            &logical,
            super::super::super::EpochRecoveryAction::Stage(saved),
            &ManualClock::new(100),
            &mut rng(),
        )
        .unwrap();
}

/// **A recovery record that CHANGES is a change too**, not only one that appears.
///
/// The appearing case below is caught by comparing `Option`s at all. This one is caught only by
/// comparing the digests inside them, and a review proved the difference: replacing the comparison
/// with `recovery.is_some() == stamp.recovery.is_some()` survived every other test in this file.
/// The consequence is the same as the appearing case and no less serious - a newly retained version
/// carries tombstones that can block a resurrection the plan thought was free, and a plan built
/// against the previous version has never seen them.
#[test]
fn a_changed_recovery_record_makes_a_destination_capture_stale() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let destination = other_target(&f);
    seed(&f, &mut store, destination, 1, title("first"));
    stage_recovery(&f, &mut store, destination);

    let capture = store
        .capture_studio_destination(SERVER, &f.group, destination, &f.device)
        .unwrap();
    assert!(
        capture.recovery.is_some(),
        "this case needs a destination that ALREADY has a recovery record"
    );
    assert!(store
        .studio_destination_is_current(SERVER, &f.group, destination, &f.device, &capture.stamp)
        .unwrap());

    // Move the destination, then stage a snapshot of the moved state. The record stays present and
    // its contents differ, which is exactly the case an `is_some()` comparison cannot see.
    seed(&f, &mut store, destination, 2, title("second"));
    stage_recovery(&f, &mut store, destination);
    let moved = store
        .capture_studio_destination(SERVER, &f.group, destination, &f.device)
        .unwrap();
    assert!(
        moved.recovery.is_some() && moved.recovery != capture.recovery,
        "the two recovery records must differ, or this proves nothing"
    );

    assert!(
        !store
            .studio_destination_is_current(SERVER, &f.group, destination, &f.device, &capture.stamp)
            .unwrap(),
        "a destination whose retained versions changed has changed"
    );
}

/// **A recovery record appearing where there was none is a change**, and this is the half that a
/// naive implementation gets wrong by reading the second record only when it captured one.
///
/// It matters because the planner consults the destination's retained versions for tombstones: a
/// deletion in a newly retained version can block a resurrection the plan thought was free. A plan
/// built before that record existed has never seen those tombstones.
#[test]
fn acquiring_a_recovery_record_makes_a_destination_capture_stale() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let destination = other_target(&f);
    seed(&f, &mut store, destination, 1, title("only"));

    let capture = store
        .capture_studio_destination(SERVER, &f.group, destination, &f.device)
        .unwrap();
    assert!(
        capture.recovery.is_none(),
        "this destination starts with no retained versions"
    );
    assert!(store
        .studio_destination_is_current(SERVER, &f.group, destination, &f.device, &capture.stamp)
        .unwrap());

    let logical = destination.document(&f.group.group_id()).unwrap();
    let projection = store
        .with_studio_source(SERVER, &f.group, destination, &f.device, |s| s.projection())
        .unwrap()
        .unwrap();
    let saved = catcoms_replication::studio::StudioRecovery::snapshot(
        &projection,
        None,
        catcoms_replication::RecoveryReason::Excluded,
        [7; 32],
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    store
        .update_epoch_recovery(
            SERVER,
            &logical,
            super::super::super::EpochRecoveryAction::Stage(saved),
            &ManualClock::new(100),
            &mut rng(),
        )
        .unwrap();

    assert!(
        !store
            .studio_destination_is_current(SERVER, &f.group, destination, &f.device, &capture.stamp)
            .unwrap(),
        "a destination that gained a recovery record has changed, and its tombstones are new"
    );
}

/// A capture is taken for one device, one server and one target, and finishing it as another is
/// refused before either record is read.
#[test]
fn a_destination_capture_belongs_to_the_device_target_and_server_that_took_it() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let destination = other_target(&f);
    seed(&f, &mut store, destination, 1, title("only"));
    seed(&f, &mut store, f.target, 3, title("the other document"));

    let capture = store
        .capture_studio_destination(SERVER, &f.group, destination, &f.device)
        .unwrap();
    assert!(
        !store
            .studio_destination_is_current(SERVER, &f.group, f.target, &f.device, &capture.stamp)
            .unwrap(),
        "a capture of one destination must not answer for another"
    );
    assert!(
        !store
            .studio_destination_is_current(
                SERVER + 1,
                &f.group,
                destination,
                &f.device,
                &capture.stamp
            )
            .unwrap(),
        "a capture of one server must not answer for another"
    );
    let other = MlsDevice::generate().unwrap();
    assert!(
        !store
            .studio_destination_is_current(SERVER, &f.group, destination, &other, &capture.stamp)
            .unwrap(),
        "a copy is authored by whoever asked for it; another device cannot finish this capture"
    );
}

/// A destination with no Studio source at all is refused at capture, not silently treated as empty.
#[test]
fn a_destination_that_does_not_exist_is_refused_at_capture() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let store = open(root.path());
    let error = store
        .capture_studio_destination(SERVER, &f.group, other_target(&f), &f.device)
        .expect_err("an absent destination is not an empty one");
    assert!(
        error.to_string().contains("no Studio source"),
        "said: {error}"
    );
}
