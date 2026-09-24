use super::*;

#[test]
fn studio_replay_reconciles_all_snapshots_and_holds_independent_branch_values() {
    let f = Fixture::new(false);
    let mut old = f.empty();
    let a = f.intent(title("A"), 1);
    let b = f.intent(title("B"), 2);
    f.edit(&mut old, &a);
    let first = f.history(&old);
    f.edit(&mut old, &b);
    let second = f.history(&old);
    let mut history = vec![first, second];
    let current = f.empty().projection().unwrap();
    for _ in 0..2 {
        assert_eq!(
            choose(&current, &BTreeMap::new(), &history, &a).unwrap(),
            ReplayChoice::Manual
        );
        assert_eq!(
            choose(&current, &BTreeMap::new(), &history, &b).unwrap(),
            ReplayChoice::Ready
        );
        history.reverse();
    }
    let mut branch_b = f.empty();
    f.edit(&mut branch_b, &b);
    let mut branch_a = f.empty();
    f.edit(&mut branch_a, &a);
    let history = vec![f.history(&branch_a), f.history(&branch_b)];
    let own = BTreeMap::from([
        (a.operation.id(&a.author), a),
        (b.operation.id(&b.author), b),
    ]);
    let mut choices = own
        .iter()
        .map(|(id, i)| {
            (
                *id,
                choose(&current, &BTreeMap::new(), &history, i).unwrap(),
            )
        })
        .collect();
    deconflict(&mut choices, &own).unwrap();
    assert!(choices.values().all(|c| *c == ReplayChoice::Manual));
}
use catcoms_mls::{MlsDevice, ServerGroup};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::slice;

struct Fixture {
    device: MlsDevice,
    group: ServerGroup,
    target: StudioTarget,
}
impl Fixture {
    fn new(index: bool) -> Self {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let target = if index {
            StudioTarget::Index { channel: [1; 16] }
        } else {
            StudioTarget::Flipnote {
                channel: [1; 16],
                object: [2; 16],
            }
        };
        Self {
            device,
            group,
            target,
        }
    }
    fn empty(&self) -> types::StudioEpoch {
        types::StudioEpoch::new(&self.group, self.target, self.device.device_id()).unwrap()
    }
    fn intent(&self, body: Vec<u8>, n: u8) -> LocalIntent {
        LocalIntent {
            author: self.device.device_id(),
            operation: domain(self.target, [n; 16], body),
        }
    }
    fn edit(&self, state: &mut types::StudioEpoch, intent: &LocalIntent) {
        state
            .edit_or_reseal(
                &self.device,
                &self.group,
                &mut ChaCha20Rng::seed_from_u64(17),
                &intent.operation,
                100,
            )
            .unwrap();
    }
    /// Merge a real concurrent second birth of an element id this state already holds: a branch
    /// that never observed the first one creates it against its own empty causal past, which the
    /// validator accepts, and this state ingests the signed change. Two births of one id are
    /// legitimate retained evidence, not a malformed slot.
    fn contest(
        &mut self,
        state: &mut types::StudioEpoch,
        body: impl Fn(&MlsDevice) -> Vec<u8>,
        n: u8,
        later_than: [u8; 32],
    ) {
        // Deliberately the LOSING birth, so the winner and every selected value stay exactly as
        // the agreed version recorded them and the extra evidence is the only difference.
        let (other, operation) = loop {
            let other = MlsDevice::generate().unwrap();
            let operation = domain(self.target, [n; 16], body(&other));
            if operation.id(&other.device_id()) > later_than {
                break (other, operation);
            }
        };
        let welcome = self
            .group
            .add_member(&self.device, other.key_package().unwrap())
            .unwrap()
            .welcome;
        let joined = ServerGroup::join(&other, &welcome).unwrap();
        let packet = types::StudioEpoch::new(&joined, self.target, other.device_id())
            .unwrap()
            .edit_or_reseal(
                &other,
                &joined,
                &mut ChaCha20Rng::seed_from_u64(19),
                &operation,
                100,
            )
            .unwrap();
        assert_eq!(
            state.ingest(&packet, &self.group, &self.device).unwrap(),
            catcoms_replication::Admission::Accepted
        );
    }
    fn history(&self, state: &types::StudioEpoch) -> StudioRecovery {
        let snapshot = StudioRecovery::snapshot(
            &state.projection().unwrap(),
            None,
            RecoveryReason::Excluded,
            [8; 32],
            &state.current_operations().unwrap(),
        )
        .unwrap();
        StudioRecovery::from_snapshot(&snapshot, state.document(), self.target.channel()).unwrap()
    }
}
fn title(value: &str) -> Vec<u8> {
    FlipnoteOp::SetHeader(FlipnoteHeader::Title(value.into()))
        .encode()
        .unwrap()
}
fn insert() -> Vec<u8> {
    FlipnoteOp::InsertFrame {
        frame: [4; 16],
        after: None,
        cid: [5; 32],
        bytes: 20,
    }
    .encode()
    .unwrap()
}
fn replace(cid: u8) -> Vec<u8> {
    FlipnoteOp::ReplaceFrame {
        frame: [4; 16],
        cid: [cid; 32],
        bytes: 20,
    }
    .encode()
    .unwrap()
}
fn insert_after() -> Vec<u8> {
    FlipnoteOp::InsertFrame {
        frame: [7; 16],
        after: Some([4; 16]),
        cid: [5; 32],
        bytes: 20,
    }
    .encode()
    .unwrap()
}

/// One frame id has one replacement register, so a contested birth is not another incarnation
/// to write onto: it is two versions disagreeing about the element's identity. Retained slots
/// are newest-first and the staged slot is newer still while sorting last, so a check applied
/// to whichever version is found first is decided by slot order rather than by the evidence.
#[test]
fn studio_replay_holds_when_another_version_shows_a_second_frame_birth() {
    let mut f = Fixture::new(false);
    let mut old = f.empty();
    let birth = f.intent(insert(), 1);
    let replace = f.intent(replace(6), 2);
    let follower = f.intent(insert_after(), 3);
    f.edit(&mut old, &birth);
    f.edit(&mut old, &replace);
    f.edit(&mut old, &follower);
    let agreed = f.history(&old);
    f.contest(
        &mut old,
        |_| insert(),
        200,
        birth.operation.id(&birth.author),
    );
    let contested = f.history(&old);
    let StudioProjection::Flipnote(shown) = contested.projection() else {
        panic!("flipnote")
    };
    assert_eq!(
        shown.frames[&[4; 16]].insertions.len(),
        2,
        "the fixture must really contest the birth"
    );
    let mut now = f.empty();
    f.edit(&mut now, &birth);
    let current = now.projection().unwrap();
    let held = now.current_operations().unwrap();
    let empty = f.empty().projection().unwrap();
    assert_eq!(
        choose(&current, &held, slice::from_ref(&agreed), &replace).unwrap(),
        ReplayChoice::Ready
    );
    assert_eq!(
        choose(
            &empty,
            &BTreeMap::new(),
            slice::from_ref(&agreed),
            &follower
        )
        .unwrap(),
        ReplayChoice::After(birth.operation.id(&birth.author))
    );
    let mut history = vec![agreed, contested];
    for _ in 0..2 {
        assert_eq!(
            choose(&current, &held, &history, &replace).unwrap(),
            ReplayChoice::Manual,
            "the contested version is evidence in either slot"
        );
        assert_eq!(
            choose(&empty, &BTreeMap::new(), &history, &follower).unwrap(),
            ReplayChoice::Manual,
            "a contested predecessor is not an automatic dependency edge"
        );
        history.reverse();
    }
}

/// Same rule on the Index side: `t/<object>` and `e/<object>` are one register per object id,
/// and concurrent creations of one id are explicitly legal.
#[test]
fn studio_replay_holds_when_another_version_shows_a_second_object_creation() {
    let mut f = Fixture::new(true);
    let mut old = f.empty();
    let birth = f.intent(
        IndexOp::PutObject {
            object: [3; 16],
            kind: StudioKind::Flipnote,
            title: "initial".into(),
            created_by: f.device.device_id(),
            ts: 100,
            expiry: StudioExpiry::Unrecorded,
        }
        .encode()
        .unwrap(),
        1,
    );
    let rename = f.intent(
        IndexOp::SetTitle {
            object: [3; 16],
            title: "wanted".into(),
        }
        .encode()
        .unwrap(),
        2,
    );
    f.edit(&mut old, &birth);
    f.edit(&mut old, &rename);
    let agreed = f.history(&old);
    f.contest(
        &mut old,
        |other| {
            IndexOp::PutObject {
                object: [3; 16],
                kind: StudioKind::Flipnote,
                title: "rival".into(),
                created_by: other.device_id(),
                ts: 100,
                expiry: StudioExpiry::Unrecorded,
            }
            .encode()
            .unwrap()
        },
        200,
        birth.operation.id(&birth.author),
    );
    let contested = f.history(&old);
    let StudioProjection::Index(shown) = contested.projection() else {
        panic!("index")
    };
    assert_eq!(
        shown.objects[&[3; 16]].creations.len(),
        2,
        "the fixture must really contest the creation"
    );
    let mut now = f.empty();
    f.edit(&mut now, &birth);
    let current = now.projection().unwrap();
    let held = now.current_operations().unwrap();
    assert_eq!(
        choose(&current, &held, slice::from_ref(&agreed), &rename).unwrap(),
        ReplayChoice::Ready
    );
    let mut history = vec![agreed, contested];
    for _ in 0..2 {
        assert_eq!(
            choose(&current, &held, &history, &rename).unwrap(),
            ReplayChoice::Manual
        );
        history.reverse();
    }
}

#[test]
fn studio_replay_uses_register_provenance_not_hash_order_and_holds_newer_values() {
    let f = Fixture::new(false);
    let mut old = f.empty();
    let mut pair = None;
    for n in 1..255 {
        let a = f.intent(title("A"), n);
        let b = f.intent(title("B"), n + 1);
        if a.operation.id(&a.author) > b.operation.id(&b.author) {
            pair = Some((a, b));
            break;
        }
    }
    let (a, b) = pair.unwrap();
    f.edit(&mut old, &a);
    f.edit(&mut old, &b);
    let history = [f.history(&old)];
    let mut now = f.empty();
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &a
        )
        .unwrap(),
        ReplayChoice::Manual
    );
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &b
        )
        .unwrap(),
        ReplayChoice::Ready
    );
    f.edit(&mut now, &f.intent(title("newer"), 0));
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &b
        )
        .unwrap(),
        ReplayChoice::Manual
    );
    let mut mismatch = b.clone();
    mismatch.operation.body = title("forged same nonce");
    f.edit(&mut now, &b);
    assert!(choose(
        &now.projection().unwrap(),
        &now.current_operations().unwrap(),
        &history,
        &mismatch
    )
    .is_err());
}

#[test]
fn studio_replay_creation_then_selected_replace_and_tombstone_screening() {
    let f = Fixture::new(false);
    let mut old = f.empty();
    let birth = f.intent(insert(), 1);
    let replace = f.intent(replace(6), 2);
    f.edit(&mut old, &birth);
    f.edit(&mut old, &replace);
    let history = [f.history(&old)];
    let mut now = f.empty();
    let id = birth.operation.id(&birth.author);
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &BTreeMap::new(),
            &history,
            &birth
        )
        .unwrap(),
        ReplayChoice::Ready
    );
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &BTreeMap::new(),
            &history,
            &replace
        )
        .unwrap(),
        ReplayChoice::After(id)
    );
    f.edit(&mut now, &birth);
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &replace
        )
        .unwrap(),
        ReplayChoice::Ready
    );
    let newer = f.intent(self::replace(7), 3);
    f.edit(&mut now, &newer);
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &replace
        )
        .unwrap(),
        ReplayChoice::Manual
    );
    let delete = f.intent(
        FlipnoteOp::RemoveFrame { frame: [4; 16] }.encode().unwrap(),
        4,
    );
    f.edit(&mut old, &delete);
    let history = [history.into_iter().next().unwrap(), f.history(&old)];
    assert_eq!(
        choose(
            &f.empty().projection().unwrap(),
            &BTreeMap::new(),
            &history,
            &birth
        )
        .unwrap(),
        ReplayChoice::Manual
    );
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &birth
        )
        .unwrap(),
        ReplayChoice::Current
    );
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &delete
        )
        .unwrap(),
        ReplayChoice::Manual
    );
}

#[test]
fn studio_replay_index_creation_dependency_and_newer_scalar_hold() {
    let f = Fixture::new(true);
    let mut old = f.empty();
    let birth = f.intent(
        IndexOp::PutObject {
            object: [3; 16],
            kind: StudioKind::Flipnote,
            title: "initial".into(),
            created_by: f.device.device_id(),
            ts: 100,
            expiry: StudioExpiry::Unrecorded,
        }
        .encode()
        .unwrap(),
        1,
    );
    let rename = f.intent(
        IndexOp::SetTitle {
            object: [3; 16],
            title: "wanted".into(),
        }
        .encode()
        .unwrap(),
        2,
    );
    f.edit(&mut old, &birth);
    f.edit(&mut old, &rename);
    let history = [f.history(&old)];
    let mut now = f.empty();
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &BTreeMap::new(),
            &history,
            &rename
        )
        .unwrap(),
        ReplayChoice::After(birth.operation.id(&birth.author))
    );
    f.edit(&mut now, &birth);
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &rename
        )
        .unwrap(),
        ReplayChoice::Ready
    );
    let newer = f.intent(
        IndexOp::SetTitle {
            object: [3; 16],
            title: "newer".into(),
        }
        .encode()
        .unwrap(),
        3,
    );
    f.edit(&mut now, &newer);
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &history,
            &rename
        )
        .unwrap(),
        ReplayChoice::Manual
    );
    assert_eq!(
        choose(
            &now.projection().unwrap(),
            &now.current_operations().unwrap(),
            &[],
            &rename
        )
        .unwrap(),
        ReplayChoice::NoEvidence
    );
}

#[test]
fn studio_replay_topology_handles_reverse_ids_missing_prerequisites_and_max_chain_without_recursion(
) {
    let id = |n: u32| {
        let mut id = [0; 32];
        id[..4].copy_from_slice(&n.to_be_bytes());
        id
    };
    let count = catcoms_replication::epoch::MAX_INTENTS_PER_DOCUMENT as u32;
    let mut choices = BTreeMap::new();
    choices.insert(id(count), ReplayChoice::Ready);
    for n in 1..count {
        choices.insert(id(n), ReplayChoice::After(id(n + 1)));
    }
    let (order, manual) = ordered(&choices);
    assert!(manual.is_empty());
    assert_eq!(order.len(), count as usize);
    assert_eq!(order.front(), Some(&id(count)));
    assert_eq!(order.back(), Some(&id(1)));
    choices.insert(id(count), ReplayChoice::After(id(1)));
    let (order, manual) = ordered(&choices);
    assert!(order.is_empty());
    assert_eq!(manual.len(), count as usize);
    choices.insert(id(count), ReplayChoice::After(id(count + 1)));
    assert_eq!(ordered(&choices).1.len(), count as usize);
}

/// R4 / N25. Accepted Closing-overlay entries must be excluded from ordinary replay SELECTION,
/// not merely left to `choose` returning `NoEvidence` for them. The assertion is on the actual
/// `ReplayEvidence.own` set produced by the production path, with a comparable unannotated own
/// intent present as the control: a test of the `is_overlay` predicate alone would not cover the
/// call site and would survive deletion of the filter.
#[tokio::test]
async fn studio_replay_evidence_excludes_accepted_overlay_ids_and_keeps_ordinary_own_intents() {
    use crate::store::EpochStudioBudget;
    use crate::studio::StudioRequest;
    use catcoms_mls::MlsDevice;
    use catcoms_replication::studio::{IndexOp, StudioExpiry, StudioKind, StudioOverlaySave};
    use catcoms_rt::{Hub, ManualClock, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    const SERVER: u64 = 7;
    let rng = || ChaCha20Rng::seed_from_u64(451);
    let hub = Hub::new();
    let clock = ManualClock::new(1000);
    let mut server = Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(clock.clone()),
        "replay-exclusion",
    )
    .unwrap();
    let channel = crate::channel_id("general").to_be_bytes();
    let target = StudioTarget::Index { channel };
    let root = tempfile::tempdir().unwrap();
    let mut store = ServerStore::open(root.path(), b"replay-exclusion", &mut rng()).unwrap();
    let logical = target.document(&server.group_id()).unwrap();
    let budget = |store: &mut ServerStore, server: &mut Server<_, _>| -> EpochStudioBudget {
        let mut scan = store.scan_epoch_storage_with_studio().unwrap();
        while !scan.step().unwrap().complete {}
        let inventory = scan.finish().unwrap();
        server
            .sync
            .with_registry_context(|g, _, _, _| store.studio_storage_budget(SERVER, g, &inventory))
            .unwrap()
    };
    let domain = |body: Vec<u8>, nonce: u8| DomainOp {
        body,
        nonce: [nonce; 16],
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
    };
    let put = |object: [u8; 16], title: &str, device: crate::DeviceId| {
        IndexOp::PutObject {
            object,
            kind: StudioKind::Flipnote,
            title: title.into(),
            created_by: device,
            ts: 100,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap()
    };

    // An installed source with one signed operation, so the document exists and can be closed.
    let mut b = budget(&mut store, &mut server);
    let close = server
        .sync
        .with_registry_context(|g, d, _, r| {
            let mut source =
                catcoms_replication::studio::StudioEpoch::new(g, target, d.device_id()).unwrap();
            let seed = domain(put([4; 16], "shared seed", d.device_id()), 1);
            let packet = source.edit_or_reseal(d, g, r, &seed, 100).unwrap();
            store.ingest_studio_epoch(SERVER, g, target, d, &packet, r, &mut b)?;
            // Real large signed operations, as the shared overlay fixture does, so the source
            // actually reaches the production rotation threshold.
            let title = domain(
                IndexOp::SetTitle {
                    object: [4; 16],
                    title: "retained local draft".into(),
                }
                .encode()
                .unwrap(),
                9,
            );
            for n in 10..20u8 {
                let mut op = title.clone();
                op.nonce = [n; 16];
                let mut copy = catcoms_replication::studio::StudioEpoch::restore(
                    &source.snapshot().unwrap(),
                    g,
                    target,
                    d.device_id(),
                )
                .unwrap();
                let packet = copy.edit_or_reseal(d, g, r, &op, 100).unwrap();
                let opened = packet
                    .open(&g.channel_secret(d, packet.doc_type, packet.doc_id).unwrap())
                    .unwrap();
                let mut change = automerge::Change::from_bytes(opened.delta)
                    .unwrap()
                    .decode();
                change.message = Some("x".repeat(220_000));
                let change = automerge::Change::from(change);
                let signed = catcoms_replication::SignedOp::sign_domain(
                    d,
                    logical.doc_type,
                    source.doc_id(),
                    change.raw_bytes().to_vec(),
                    &op,
                )
                .unwrap();
                let packet = catcoms_replication::SealedOp::seal(&signed, g, d, r).unwrap();
                source.ingest(&packet, g, d).unwrap();
                store.ingest_studio_epoch(SERVER, g, target, d, &packet, r, &mut b)?;
            }
            let decision = source.new_owner_decision(g, d, 0, None).unwrap();
            store.seal_studio_epoch(
                SERVER,
                g,
                target,
                d,
                decision.receipt().clone(),
                0,
                r,
                &mut b,
            )?;
            Ok::<_, crate::AppError>(decision.close().clone())
        })
        .unwrap();

    // The accepted Closing-overlay entry.
    let annotated = domain(put([6; 16], "accepted overlay", server.device_id()), 3);
    let annotated_id = annotated.id(&server.device_id());
    let mut b = budget(&mut store, &mut server);
    let basis = server
        .prepare_studio_closing_overlay(&mut store, SERVER, target, &close, &mut b)
        .unwrap()
        .fingerprint();
    let StudioOverlaySave::Local(draft) = server
        .save_studio_closing_overlay(&mut store, SERVER, target, &close, basis, annotated, &mut b)
        .unwrap()
    else {
        panic!("expected actual local acceptance")
    };
    assert_eq!(draft.accepted(), 1);

    // Install the pristine successor, so the current source is Open and replay selection runs.
    // A large source must pass the existing detached preparation before rotation touches it.
    let capture = server
        .sync
        .with_registry_context(|g, d, _, _| store.capture_studio_source(SERVER, g, target, d))
        .unwrap()
        .expect("the installed source is present");
    let prepared = capture.rebuild().unwrap();
    assert!(server
        .sync
        .with_registry_context(|g, d, _, _| store.install_prepared_studio_source(g, d, prepared))
        .unwrap());
    let mut b = budget(&mut store, &mut server);
    server
        .sync
        .with_registry_context(|g, d, _, r| {
            store.install_sealed_studio_successor_for_test(SERVER, g, target, d, &close, r, &mut b)
        })
        .unwrap();
    let epoch = server
        .studio_transaction(&mut store, SERVER, StudioRequest::Read { target })
        .unwrap()
        .expect("the successor is readable")
        .epoch_id;

    // The control: an ordinary own intent on the successor, unannotated and still pending.
    let ordinary_body = put([5; 16], "ordinary pending", server.device_id());
    let ordinary_id = domain(ordinary_body.clone(), 2).id(&server.device_id());
    server
        .studio_transaction(
            &mut store,
            SERVER,
            StudioRequest::Apply {
                target,
                epoch_id: epoch,
                nonce: [2; 16],
                body: ordinary_body,
            },
        )
        .unwrap();

    // Both entries are pending and authored locally; only the annotation differs.
    let pending = store
        .load_epoch_intents_structural(SERVER, &logical)
        .unwrap();
    assert!(pending.pending().any(|(id, _)| *id == ordinary_id));
    assert!(pending.pending().any(|(id, _)| *id == annotated_id));
    assert!(pending.is_overlay(&annotated_id));
    assert!(!pending.is_overlay(&ordinary_id));

    // The production selection boundary.
    let evidence = server
        .studio_replay_evidence(&mut store, SERVER, target, epoch)
        .unwrap()
        .expect("an Open successor must produce replay evidence");
    assert!(
        evidence.own.contains_key(&ordinary_id),
        "an ordinary unannotated own intent must remain selectable"
    );
    assert!(
        !evidence.own.contains_key(&annotated_id),
        "an accepted overlay id reached ordinary replay selection"
    );
}
