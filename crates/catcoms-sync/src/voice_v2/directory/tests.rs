use super::super::runtime::{Runtime, View};
use super::*;
use automerge::transaction::Transactable;
use automerge::{ReadDoc, ROOT};
use catcoms_crypto::DeviceKeypair;
use catcoms_rt::ManualClock;

fn reader(doc: Option<&AutoCommit>) -> Result<BTreeSet<u128>, VoiceError> {
    let Some(doc) = doc else {
        return Ok(BTreeSet::from([1]));
    };
    let mut rooms = BTreeSet::from([1]);
    for (index, key) in doc.keys(ROOT).enumerate() {
        if index >= MAX_DIRECTORY_KEYS {
            return Err(VoiceError::Capacity);
        }
        if let Ok(id) = key.parse::<u128>() {
            rooms.insert(id);
        }
        if rooms.len() > MAX_DIRECTORY_ROOMS {
            return Err(VoiceError::Capacity);
        }
    }
    Ok(rooms)
}
fn owner(runtime: &Arc<Runtime>, hash: [u8; 32]) -> ActorOwner {
    let me = DeviceKeypair::from_seed(&[1; 32]).device_id();
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, installation) = runtime.prepare_actor(hash, me, &ticket).unwrap();
    installation.register().unwrap();
    actor
        .sync_view(View {
            epoch: 1,
            members: BTreeSet::from([me]),
            policy: Some([7; 32]),
            eligible: true,
            indirect: true,
        })
        .unwrap();
    actor
}
fn scope(room: u128) -> Scope {
    Scope {
        group: [1; 32],
        policy: Some([7; 32]),
        room,
    }
}

#[test]
fn directory_deletion_recreation_and_drop_irreversibly_retire_exact_children_and_clones() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = owner(&runtime, [1; 32]);
    let mut directory =
        Directory::new(
            [1; 32],
            runtime.resources(),
            reader,
            |_| Ok(BTreeSet::new()),
        );
    let mut doc = AutoCommit::new();
    doc.put(ROOT, "2", true).unwrap();
    directory.observe(Some(&doc)).unwrap();
    let old = directory.authority(scope(2)).unwrap();
    let call = directory.call_lease(&actor, scope(2)).unwrap();
    let watch = directory.watch_lease(&actor, scope(2)).unwrap();
    doc.delete(ROOT, "2").unwrap();
    directory.observe(Some(&doc)).unwrap();
    doc.put(ROOT, "2", true).unwrap();
    directory.observe(Some(&doc)).unwrap();
    assert!(!old.lease.is_live() && !call.is_live() && !watch.is_live());
    let new = directory.authority(scope(2)).unwrap();
    assert!(!new.lease.is_same(&old.lease));
    let new_call = directory.call_lease(&actor, scope(2)).unwrap();
    drop(directory);
    assert!(!new.lease.is_live() && !new_call.is_live());
}

#[test]
fn directory_overflow_faults_closed_and_valid_read_opens_fresh_owners() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = owner(&runtime, [1; 32]);
    let mut directory =
        Directory::new(
            [1; 32],
            runtime.resources(),
            reader,
            |_| Ok(BTreeSet::new()),
        );
    directory.observe(None).unwrap();
    let old = directory.authority(scope(1)).unwrap();
    let child = directory.call_lease(&actor, scope(1)).unwrap();
    let mut doc = AutoCommit::new();
    for index in 0..=MAX_DIRECTORY_KEYS {
        doc.put(ROOT, format!("invalid{index}"), true).unwrap();
    }
    assert_eq!(directory.observe(Some(&doc)), Err(VoiceError::Capacity));
    assert!(!old.lease.is_live() && !child.is_live());
    assert!(directory.authority(scope(1)).is_err());
    directory.observe(None).unwrap();
    let new = directory.authority(scope(1)).unwrap();
    assert!(new.lease.is_live() && !new.lease.is_same(&old.lease));
}

#[test]
fn call_grant_provenance_requires_exact_current_actor_and_room_issuers() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = owner(&runtime, [1; 32]);
    let mut directory =
        Directory::new(
            [1; 32],
            runtime.resources(),
            reader,
            |_| Ok(BTreeSet::new()),
        );
    let mut doc = AutoCommit::new();
    doc.put(ROOT, "2", true).unwrap();
    directory.observe(Some(&doc)).unwrap();
    let grant = directory.call_grant(&actor, scope(2)).unwrap();
    directory.validate_call_grant(&actor, &grant).unwrap();
    let foreign = super::super::CallGrant::from_lease(CallLease::default(), scope(2));
    assert_eq!(
        directory.validate_call_grant(&actor, &foreign),
        Err(VoiceError::Unauthorized)
    );
    let actor_only = super::super::CallGrant::from_lease(actor.child_lease().unwrap(), scope(2));
    assert_eq!(
        directory.validate_call_grant(&actor, &actor_only),
        Err(VoiceError::Unauthorized)
    );
    let wrong_room = super::super::CallGrant::from_lease(grant.lease(), scope(1));
    assert_eq!(
        directory.validate_call_grant(&actor, &wrong_room),
        Err(VoiceError::Unauthorized)
    );
    // These scope wrappers share revocation custody intentionally; keep them until after the
    // positive check so their affine Drop does not itself invalidate the original grant.
    directory.validate_call_grant(&actor, &grant).unwrap();
    doc.delete(ROOT, "2").unwrap();
    directory.observe(Some(&doc)).unwrap();
    doc.put(ROOT, "2", true).unwrap();
    directory.observe(Some(&doc)).unwrap();
    assert_eq!(
        directory.validate_call_grant(&actor, &grant),
        Err(VoiceError::Unauthorized)
    );
    let fresh = directory.call_grant(&actor, scope(2)).unwrap();
    directory.validate_call_grant(&actor, &fresh).unwrap();
    let replacement = owner(&runtime, [1; 32]);
    assert_eq!(
        directory.validate_call_grant(&replacement, &fresh),
        Err(VoiceError::Unauthorized)
    );
    let current = directory.call_grant(&replacement, scope(2)).unwrap();
    directory
        .validate_call_grant(&replacement, &current)
        .unwrap();
}

#[test]
fn failed_room_allocation_revokes_previous_authority_without_refunding_retained_room_owner() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let _actor = owner(&runtime, [1; 32]);
    let mut directory =
        Directory::new(
            [1; 32],
            runtime.resources(),
            reader,
            |_| Ok(BTreeSet::new()),
        );
    directory.observe(None).unwrap();
    let retained = directory.authority(scope(1)).unwrap();
    let before = runtime.resources().counts().0;
    let fill = runtime
        .resources()
        .charge([1; 32], 256 * 1024 - before, ResourceKind::Retained)
        .unwrap();
    let mut doc = AutoCommit::new();
    doc.put(ROOT, "2", true).unwrap();
    assert_eq!(directory.observe(Some(&doc)), Err(VoiceError::Capacity));
    assert!(!retained.lease.is_live());
    assert_eq!(runtime.resources().counts().0, 256 * 1024);
    drop(fill);
    drop(retained);
    assert_eq!(runtime.resources().counts().0, before - 1024);
    directory.observe(Some(&doc)).unwrap();
    assert!(directory.authority(scope(2)).is_ok());
}

#[test]
fn room_caps_require_exact_group_and_share_actor_child_limit() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = owner(&runtime, [1; 32]);
    let other = owner(&runtime, [2; 32]);
    let mut directory =
        Directory::new(
            [1; 32],
            runtime.resources(),
            reader,
            |_| Ok(BTreeSet::new()),
        );
    directory.observe(None).unwrap();
    assert_eq!(
        directory.call_lease(&other, scope(1)).err(),
        Some(VoiceError::Scope)
    );
    let mut wrong = scope(1);
    wrong.group = [2; 32];
    assert!(directory.authority(wrong).is_err());
    let children = (0..ROOM_CHILDREN)
        .map(|_| directory.call_lease(&actor, scope(1)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        directory.call_lease(&actor, scope(1)).err(),
        Some(VoiceError::Capacity)
    );
    children[0].revoke();
    directory.call_lease(&actor, scope(1)).unwrap();
}

#[test]
fn retained_call_grants_hold_room_and_declared_bytes_until_actual_final_owner_drop() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = owner(&runtime, [1; 32]);
    let resources = runtime.resources();
    let mut directory = Directory::new([1; 32], resources.clone(), reader, |_| Ok(BTreeSet::new()));
    let mut doc = AutoCommit::new();
    doc.put(ROOT, "2", true).unwrap();
    directory.observe(Some(&doc)).unwrap();
    let native = Arc::new(directory.call_grant(&actor, scope(2)).unwrap());
    let actual_last_owner = native.clone();
    let old = native.lease();
    let before = resources.counts().0;
    doc.delete(ROOT, "2").unwrap();
    directory.observe(Some(&doc)).unwrap();
    assert!(!old.is_live());
    assert_eq!(
        resources.counts().0,
        before,
        "deletion revokes without refunding held Room/grant custody"
    );
    doc.put(ROOT, "2", true).unwrap();
    directory.observe(Some(&doc)).unwrap();
    assert_eq!(resources.counts().0, before + 1024);
    let other_grant = directory.call_grant(&actor, scope(2)).unwrap();
    let other_lease = other_grant.lease();
    let replacement = owner(&runtime, [1; 32]);
    assert!(
        !other_lease.is_live(),
        "actor replacement cannot revive the retained grant"
    );
    let held = resources.counts().0;
    let fill = resources
        .charge([1; 32], 256 * 1024 - held, ResourceKind::Retained)
        .unwrap();
    assert_eq!(
        directory.call_grant(&replacement, scope(2)).err(),
        Some(VoiceError::Capacity)
    );
    drop(native);
    assert_eq!(
        resources.counts().0,
        256 * 1024,
        "the last real Arc still owns custody"
    );
    drop(actual_last_owner);
    assert_eq!(resources.counts().0, 256 * 1024 - 1024 - 256);
    let fresh = directory.call_grant(&replacement, scope(2)).unwrap();
    assert!(fresh.lease().is_live());
    assert!(!old.is_live() && !other_lease.is_live());
    drop(fill);
    drop(directory);
    assert!(!fresh.lease().is_live());
    let before = resources.counts().0;
    drop(other_grant);
    assert_eq!(
        resources.counts().0,
        before - 256,
        "same current room remains held by the fresh grant"
    );
    drop(fresh);
    assert_eq!(resources.counts().0, before - 256 - 256 - 1024);
}

#[test]
fn call_grant_charge_refusal_issues_no_live_actor_or_room_child() {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1000)));
    let actor = owner(&runtime, [1; 32]);
    let resources = runtime.resources();
    let mut directory = Directory::new([1; 32], resources.clone(), reader, |_| Ok(BTreeSet::new()));
    directory.observe(None).unwrap();
    let before = resources.counts().0;
    let fill = resources
        .charge([1; 32], 256 * 1024 - before, ResourceKind::Retained)
        .unwrap();
    assert_eq!(
        directory.call_grant(&actor, scope(1)).err(),
        Some(VoiceError::Capacity)
    );
    assert!(directory.rooms[&1].children.lock().unwrap().is_empty());
    drop(fill);
    let children: Vec<_> = (0..ROOM_CHILDREN)
        .map(|_| actor.child_lease().unwrap())
        .collect();
    assert!(children.iter().all(CallLease::is_live));
    assert!(
        actor.child_lease().is_err(),
        "the failed grant consumed none of the 64 live child slots"
    );
}
