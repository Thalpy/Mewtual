//! Genuine production verifier/issuer tests with independent device keys. HOST trust and
//! selected save expectations are explicit test selections, not proof of App/MLS/network
//! integration. No cfg(test) grant/witness issuer supplies any positive evidence here.
use super::*;

mod admission_prepare;
mod read_accessors;
mod review_regressions;
mod reviewed_create;
mod selected_history;
mod signing;
use crate::chat::owner_bodies::{AcceptanceSpec, AppendBody, AppendSpec};
use crate::chat::owner_header::{HeaderKind, HeaderSpec, OwnerBodyId};
use crate::chat::owner_profile::{AdmissionOrigin, AdmissionSpec, ProfileSpec, SignedOwnerProfile};
use crate::chat::owner_state::{
    encode_state, virtual_genesis_state, CurrentAdmissionSlot, StateFrame, StateMeta,
};
use catcoms_crypto::DeviceKeypair;
use std::cell::Cell;

const GROUP: &[u8] = b"general-open-actual-production-issuer";
#[derive(Debug)]
struct Negative(AtomicBool);
impl V2OriginNegative for Negative {
    fn is_live(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct Owner {
    key: DeviceKeypair,
    calls: Cell<usize>,
}
impl Owner {
    fn header(&self, image: &[u8]) -> SignedOwnerHeader {
        let frame = StateFrame::decode(image).unwrap();
        let m = frame.meta();
        SignedOwnerHeader::sign_using(
            HeaderSpec {
                group: m.group.to_vec(),
                profile: m.profile,
                issuer: m.issuer,
                tenure: m.tenure,
                parent: m.parent,
                sequence: m.sequence,
                kind: m.kind,
                body: m.body,
                state: frame.commitment(),
            },
            |bytes| {
                self.calls.set(self.calls.get() + 1);
                Ok(self.key.sign(bytes))
            },
        )
        .unwrap()
    }
}
struct Fixture {
    verifier: V2GeneralPrefixVerifier,
    negative: Arc<Negative>,
    group_view: ProjectionGroupView,
    _group: ProjectionGroup,
    owner: Owner,
}
impl Fixture {
    fn new() -> Self {
        Self::with_owner_seed(7)
    }
    fn with_owner_seed(seed: u8) -> Self {
        let owner = Owner {
            key: DeviceKeypair::from_seed(&[seed; 32]),
            calls: Cell::new(0),
        };
        let profile = SignedOwnerProfile::sign_using(
            ProfileSpec {
                group: GROUP.to_vec(),
                founder: owner.key.verifying_key().to_bytes(),
                creation: [8; 32],
            },
            |bytes| Ok(owner.key.sign(bytes)),
        )
        .unwrap();
        let founder = AdmissionBody::new(AdmissionSpec {
            group: GROUP.to_vec(),
            profile: profile.id(),
            subject: profile.spec().founder,
            origin: AdmissionOrigin::Founder {
                creation: profile.spec().creation,
            },
        })
        .unwrap();
        let context = TreeContext {
            profile: profile.id(),
            admission: founder.admission_id(),
        };
        let image = encode_state(
            StateMeta {
                group: GROUP,
                profile: profile.id(),
                issuer: profile.spec().founder,
                tenure: profile.tenure(),
                parent: profile.genesis_parent(),
                previous: virtual_genesis_state(&profile).unwrap(),
                sequence: 1,
                kind: HeaderKind::Admission,
                body: founder.id(),
                mls_epoch: 0,
            },
            &[CurrentAdmissionSlot {
                admission: founder.id(),
                subject: profile.spec().founder,
                tip_body: founder.id(),
                accepted_count: 0,
                accepted_root: context.empty_hash(),
            }],
        )
        .unwrap();
        let header = owner.header(&image);
        let expectation = V2GeneralExpectation::from_trusted_host(FounderGenesisAnchorData {
            group: GROUP.to_vec(),
            founder: profile.spec().founder,
            profile: profile.id(),
            admission: founder.admission_id(),
            tenure: profile.tenure(),
            header: header.id(),
            state: header.spec().state,
        })
        .unwrap();
        let mut group = ProjectionGroup::new(GROUP).unwrap();
        let mut group_view = group.begin_view();
        assert!(group_view.try_ready());
        let negative = Arc::new(Negative(AtomicBool::new(true)));
        let origin = V2ProjectionOrigin::new(negative.clone());
        let verifier = V2GeneralPrefixVerifier::from_genesis(
            expectation,
            &profile,
            &founder,
            &header,
            &image,
            &group_view,
            origin,
        )
        .unwrap();
        Self {
            verifier,
            negative,
            group_view,
            _group: group,
            owner,
        }
    }
    fn image(
        &self,
        kind: HeaderKind,
        body: OwnerBodyId,
        epoch: u64,
        mut slots: Vec<CurrentAdmissionSlot>,
    ) -> Vec<u8> {
        let base = self.verifier.replay_data();
        slots.sort_by_key(|s| s.admission);
        encode_state(
            StateMeta {
                group: GROUP,
                profile: base.profile().id(),
                issuer: base.profile().spec().founder,
                tenure: base.profile().tenure(),
                parent: base.header().id(),
                previous: base.header().spec().state,
                sequence: base.header().spec().sequence + 1,
                kind,
                body,
                mls_epoch: epoch,
            },
            &slots,
        )
        .unwrap()
    }
    fn epoch(&self) -> u64 {
        StateFrame::decode(self.verifier.replay_data().image())
            .unwrap()
            .meta()
            .mls_epoch
    }
    fn advance(
        &mut self,
        header: &SignedOwnerHeader,
        body: ReplayBody<'_>,
        image: &[u8],
    ) -> V2GeneralRetiredData {
        let candidate = self.verifier.derive_successor(header, body, image).unwrap();
        let selected =
            V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
        let mut candidate = Some(candidate);
        let retired = self
            .verifier
            .advance_host_selected(&selected, &mut candidate)
            .unwrap();
        assert!(candidate.is_none());
        retired
    }
    fn add(&mut self, key: &DeviceKeypair, marker: u8) -> V2AuthenticatedAdmission {
        let body = AdmissionBody::new(AdmissionSpec {
            group: GROUP.to_vec(),
            profile: self.verifier.replay_data().profile().id(),
            subject: key.verifying_key().to_bytes(),
            origin: AdmissionOrigin::Add {
                commit: [marker; 32],
                before_epoch: self.epoch(),
                after_epoch: self.epoch() + 1,
                key_package: [marker.wrapping_add(1); 32],
                credential: [marker.wrapping_add(2); 32],
            },
        })
        .unwrap();
        let context = TreeContext {
            profile: body.spec().profile,
            admission: body.admission_id(),
        };
        let mut slots: Vec<_> = self
            .verifier
            .replay_data()
            .slots()
            .map(|s| s.image)
            .collect();
        slots.push(CurrentAdmissionSlot {
            admission: body.id(),
            subject: body.spec().subject,
            tip_body: body.id(),
            accepted_count: 0,
            accepted_root: context.empty_hash(),
        });
        let image = self.image(HeaderKind::Admission, body.id(), self.epoch() + 1, slots);
        let header = self.owner.header(&image);
        drop(self.advance(&header, ReplayBody::Admission(&body), &image));
        self.verifier.admission(body.admission_id()).unwrap()
    }
    fn remove(&mut self, admission: &V2AuthenticatedAdmission) -> V2GeneralRetiredData {
        let base = self.verifier.replay_data();
        let slot = base.slot(admission.core.body.id()).unwrap();
        let step = base
            .prepare_removal_data(admission.core.body.id(), [199; 32])
            .unwrap();
        assert_eq!(step.body_data().spec().tip_header, slot.tip_header);
        assert_eq!(step.body_data().spec().frozen_tip, slot.frontier.tip());
        let image = step.image_data();
        let header = self.owner.header(image);
        assert_eq!(header.spec(), step.header_spec_data());
        self.advance(&header, ReplayBody::Removal(step.body_data()), image)
    }
    fn append_one(
        &mut self,
        admission: &V2AuthenticatedAdmission,
        action: &SignedAction,
    ) -> (AcceptanceRecord, V2GeneralRetiredData) {
        let context = TreeContext {
            profile: admission.core.body.spec().profile,
            admission: admission.core.body.admission_id(),
        };
        let record = AcceptanceRecord::new(AcceptanceSpec {
            group: GROUP.to_vec(),
            context,
            channel: action.spec().realm.channel(),
            action: action.id(),
            subject: *action.signer(),
            grant: action.spec().grant,
            basis: action.spec().basis,
            create: action.spec().body.create(),
        })
        .unwrap();
        let base = self.verifier.replay_data();
        let slot = base.slot(admission.core.body.id()).unwrap();
        let prepared = slot.frontier.prepare_append(&[record.leaf()]).unwrap();
        let body = AppendBody::new(AppendSpec {
            group: GROUP.to_vec(),
            previous_tip: slot.tip_header,
            old_tip: prepared.old_tip(),
            new_tip: prepared.new_tip(),
            leaves: vec![record.leaf()],
        })
        .unwrap();
        let mut slots: Vec<_> = base.slots().map(|s| s.image).collect();
        let changed = slots
            .iter_mut()
            .find(|s| s.admission == admission.core.body.id())
            .unwrap();
        changed.tip_body = body.id();
        changed.accepted_count = 1;
        changed.accepted_root = prepared.new_tip().root;
        let image = self.image(HeaderKind::AcceptanceAppend, body.id(), self.epoch(), slots);
        let header = self.owner.header(&image);
        (
            record,
            self.advance(&header, ReplayBody::Append(&body), &image),
        )
    }
    fn ledger(&self) -> (ChatLedger, ProjectionView) {
        let mut ledger = ChatLedger::new(
            Realm::new(GROUP, 42).unwrap(),
            LedgerLimits {
                max_actions: 512,
                max_action_bytes: 8 * 1024 * 1024,
            },
        )
        .unwrap();
        let mut view = ledger.begin_projection_view_in(&self.group_view).unwrap();
        assert!(view.try_ready());
        (ledger, view)
    }
}
fn action(
    key: &DeviceKeypair,
    admission: &V2AuthenticatedAdmission,
    nonce: u8,
    body: Body,
    mut parents: Vec<ActionId>,
) -> SignedAction {
    let realm = Realm::new(GROUP, 42).unwrap();
    let labels = admission.admission_data().ordinary_labels(&realm).unwrap();
    parents.sort();
    parents.dedup();
    SignedAction::sign(
        key,
        ActionSpec {
            realm,
            basis: labels.basis,
            grant: labels.grant,
            nonce: [nonce; 16],
            presented_ms: u64::MAX - u64::from(nonce),
            parents,
            body,
        },
    )
    .unwrap()
}
fn create(key: &DeviceKeypair, admission: &V2AuthenticatedAdmission, nonce: u8) -> SignedAction {
    action(
        key,
        admission,
        nonce,
        Body::Create {
            text: format!("ordinary {nonce}"),
            reply_to: None,
        },
        vec![],
    )
}
fn retain(
    f: &Fixture,
    ledger: &mut ChatLedger,
    a: &SignedAction,
    admission: &V2AuthenticatedAdmission,
    deps: &[V2OrdinaryDependency<'_>],
) {
    ledger.import(a.clone()).unwrap();
    ledger
        .authorize(
            a.id(),
            f.verifier
                .verify_action_semantics(a, admission, deps)
                .unwrap(),
        )
        .unwrap();
    while ledger.advance(512).remaining {}
    assert_eq!(ledger.state(a.id()), Some(ActionState::Valid));
}
// Sort untrusted action DATA before constructing dependency loans; tests need no
// access to the issuer module's private fields or additional public accessors.
fn ordered_dependencies<'a>(
    mut items: Vec<(&'a SignedAction, &'a V2AuthenticatedAdmission)>,
) -> Vec<V2OrdinaryDependency<'a>> {
    items.sort_by_key(|(action, _)| action.id());
    items
        .into_iter()
        .map(|(action, admission)| V2OrdinaryDependency::new(action, admission))
        .collect()
}
fn install(
    f: &Fixture,
    ledger: &mut ChatLedger,
    view: &ProjectionView,
    a: &SignedAction,
    admission: &V2AuthenticatedAdmission,
    deps: &[V2OrdinaryDependency<'_>],
) {
    let token = f
        .verifier
        .provisional_action(a, admission, deps, view)
        .unwrap();
    ledger.admit_projection(&token, 512).unwrap();
}

#[test]
fn ordinary_receive_reports_only_the_exact_ledger_admission_limit() {
    let mut fixture = Fixture::new();
    let signer = DeviceKeypair::from_seed(&[90; 32]);
    let admission = fixture.add(&signer, 20);
    let first = create(&signer, &admission, 1);
    let next = create(&signer, &admission, 2);
    let cache = || {
        let mut ledger = ChatLedger::new(
            Realm::new(GROUP, 42).unwrap(),
            LedgerLimits {
                max_actions: 1,
                max_action_bytes: MAX_ACTION_BYTES,
            },
        )
        .unwrap();
        let mut view = ledger
            .begin_projection_view_in(&fixture.group_view)
            .unwrap();
        assert!(view.try_ready());
        (ledger, view)
    };
    let stage = |ledger: &mut ChatLedger, view: &ProjectionView, action: &SignedAction| {
        fixture
            .verifier
            .stage_provisional_from_ledger(ledger, view, action, &admission, 64)
    };
    let (mut ledger, view) = cache();
    assert_eq!(
        stage(&mut ledger, &view, &first),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    );
    assert_eq!(
        stage(&mut ledger, &view, &next),
        Err(ChatError::LedgerCapacity)
    );
    assert_eq!(ledger.action(first.id()), Some(&first));
    assert!(ledger.action(next.id()).is_none());
    drop((view, ledger));

    let (mut replacement, view) = cache();
    assert_eq!(
        stage(&mut replacement, &view, &next),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    );
    drop(view);
    let (mut foreign, foreign_view) = cache();
    assert_eq!(
        stage(&mut foreign, &foreign_view, &first),
        Ok(V2OrdinaryReceiveStatus::Retained(ActionState::Valid))
    );
    // A view cannot substitute for the underlying ledger or its scope.
    let (other, view) = cache();
    assert_eq!(stage(&mut foreign, &view, &next), Err(ChatError::Scope));
    drop(other);
}

#[test]
fn ordinary_receiver_waits_for_exact_valid_dependencies_then_projects() {
    let mut f = Fixture::new();
    let original = DeviceKeypair::from_seed(&[90; 32]);
    let responder = DeviceKeypair::from_seed(&[91; 32]);
    let first = f.add(&original, 20);
    let second = f.add(&responder, 21);
    let target = create(&original, &first, 1);
    let reply = action(
        &responder,
        &second,
        2,
        Body::Create {
            text: "ordered reply".into(),
            reply_to: Some(target.id()),
        },
        vec![target.id()],
    );
    let (mut ledger, view) = f.ledger();
    let receive = |ledger: &mut ChatLedger, action, admission| {
        f.verifier
            .project_provisional_from_ledger(ledger, &view, action, admission, 64)
    };
    assert_eq!(
        receive(&mut ledger, &reply, &first),
        Err(ChatError::Authority)
    );
    assert_eq!(
        receive(&mut ledger, &reply, &second).unwrap(),
        V2OrdinaryReceiveStatus::WaitingForDependencies(vec![target.id()])
    );
    assert!(ledger.action(reply.id()).is_none());
    ledger.import(target.clone()).unwrap();
    assert_eq!(
        receive(&mut ledger, &reply, &second).unwrap(),
        V2OrdinaryReceiveStatus::WaitingForDependencies(vec![target.id()])
    );
    assert!(ledger.action(reply.id()).is_none());
    assert_eq!(
        receive(&mut ledger, &target, &first).unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert_eq!(
        receive(&mut ledger, &reply, &second).unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert!(
        matches!(ledger.guarded_projection(reply.id(), 64).unwrap().with_publication(Clone::clone).unwrap(),
        GuardedProjection::Visible { text, reply_to: Some(id), certainty: ProjectionCertainty::Provisional, .. }
        if text == "ordered reply" && id == target.id())
    );
    assert_eq!(ledger.action(reply.id()), Some(&reply));
    assert!(ledger.matches_valid_ordinary_original(&reply, second.core.body.admission_id()));
    assert!(!ledger.matches_valid_ordinary_original(&reply, first.core.body.admission_id()));
    let original_effect = f
        .verifier
        .observe_live_member(second.core.body.admission_id())
        .unwrap();
    let epoch = f.epoch();
    drop(f.append_one(&second, &reply));
    assert_eq!(f.epoch(), epoch);
    assert!(!original_effect.is_live_cached());
    assert!(second.is_open_data());
    assert!(f
        .verifier
        .observe_live_member(second.core.body.admission_id())
        .unwrap()
        .is_live_cached());
}

#[test]
fn staged_live_mutation_has_no_projection_until_explicit_admission() {
    let mut fixture = Fixture::new();
    let author = DeviceKeypair::from_seed(&[94; 32]);
    let admission = fixture.add(&author, 20);
    let create = create(&author, &admission, 1);
    let delete = action(
        &author,
        &admission,
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    let (mut ledger, view) = fixture.ledger();
    let verifier = &fixture.verifier;
    verifier
        .project_provisional_from_ledger(&mut ledger, &view, &create, &admission, 64)
        .unwrap();
    assert_eq!(
        verifier
            .stage_provisional_from_ledger(&mut ledger, &view, &delete, &admission, 64)
            .unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert!(
        ledger.matches_valid_ordinary_original(&delete, admission.admission_data().admission_id())
    );
    // If selection fails before writing a WAL, a staged mutation cannot
    // alter the live body or require its own undo.
    let read = |ledger: &mut ChatLedger| {
        ledger
            .guarded_projection(create.id(), 64)
            .unwrap()
            .with_publication(Clone::clone)
            .unwrap()
    };
    assert!(matches!(
        read(&mut ledger),
        GuardedProjection::Visible { .. }
    ));
    verifier
        .project_provisional_from_ledger(&mut ledger, &view, &delete, &admission, 64)
        .unwrap();
    assert!(matches!(
        read(&mut ledger),
        GuardedProjection::Deleted { .. }
    ));
}

#[test]
fn ordinary_history_replay_restores_dependencies_without_publishing_removed_originals() {
    let mut f = Fixture::new();
    let author = DeviceKeypair::from_seed(&[92; 32]);
    let responder = DeviceKeypair::from_seed(&[93; 32]);
    let first = f.add(&author, 20);
    let second = f.add(&responder, 21);
    let target = create(&author, &first, 1);
    let reply = action(
        &responder,
        &second,
        2,
        Body::Create {
            text: "reply from the archive".into(),
            reply_to: Some(target.id()),
        },
        vec![target.id()],
    );
    let mut removed = f.remove(&first);
    let frozen = removed.take_frozen_data().unwrap();
    assert!(removed.take_close_completion().unwrap().try_complete());
    let (mut ledger, view) = f.ledger();
    let replay = |ledger: &mut ChatLedger, action, admission, frozen: &[&V2FrozenAdmission]| {
        f.verifier
            .replay_ordinary_history_original(ledger, &view, action, admission, frozen, 64)
    };
    assert_eq!(
        replay(&mut ledger, &reply, &second, &[]).unwrap(),
        V2OrdinaryReceiveStatus::WaitingForDependencies(vec![target.id()])
    );
    assert!(ledger.action(reply.id()).is_none());
    assert_eq!(
        replay(&mut ledger, &target, &second, &[]),
        Err(ChatError::Authority)
    );
    assert!(ledger.action(target.id()).is_none());
    assert_eq!(
        replay(&mut ledger, &target, &first, &[]).unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert_eq!(
        replay(&mut ledger, &reply, &second, &[]),
        Err(ChatError::Dependency)
    );
    let mut other = Fixture::new();
    let equal_data = other.add(&author, 20);
    other.add(&responder, 21);
    assert_eq!(
        equal_data.admission_data().admission_id(),
        first.admission_data().admission_id()
    );
    let foreign_frozen = other.remove(&equal_data).take_frozen_data().unwrap();
    assert_eq!(
        replay(&mut ledger, &reply, &second, &[&foreign_frozen]),
        Err(ChatError::Scope)
    );
    assert!(ledger.action(reply.id()).is_none());
    assert_eq!(
        replay(&mut ledger, &reply, &second, &[&frozen]).unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    for original in [&target, &reply] {
        assert_eq!(ledger.action(original.id()), Some(original));
        assert!(ledger
            .guarded_projection(original.id(), 64)
            .unwrap()
            .with_publication(|item| matches!(item, GuardedProjection::Held { .. }))
            .unwrap());
    }
    assert!(f
        .verifier
        .project_provisional_from_ledger(&mut ledger, &view, &target, &first, 64)
        .is_err());
    assert_eq!(
        f.verifier
            .project_provisional_from_ledger(&mut ledger, &view, &reply, &second, 64),
        Err(ChatError::Dependency)
    );
}

#[test]
fn actual_twenty_and_fifty_independent_keys_edit_delete_reply_and_merge_without_owner_signatures() {
    for count in [20, 50] {
        let mut f = Fixture::new();
        let members: Vec<_> = (0..count - 1)
            .map(|i| {
                let key = DeviceKeypair::from_seed(&[80 + i as u8; 32]);
                let admission = f.add(&key, 30 + i as u8);
                (key, admission)
            })
            .collect();
        assert_eq!(f.verifier.replay_data().slots().len(), count);
        let owner_calls = f.owner.calls.get();
        let (mut left, lv) = f.ledger();
        let (mut right, rv) = f.ledger();
        let mut records = Vec::new();
        for (i, (key, admission)) in members.iter().enumerate() {
            let c = create(key, admission, i as u8);
            let e = action(
                key,
                admission,
                100 + i as u8,
                Body::Edit {
                    create: c.id(),
                    text: format!("edit {i}"),
                },
                vec![c.id()],
            );
            let d = action(
                key,
                admission,
                200 + i as u8,
                Body::Delete { create: c.id() },
                vec![e.id()],
            );
            let (ledger, view) = if i % 2 == 0 {
                (&mut left, &lv)
            } else {
                (&mut right, &rv)
            };
            retain(&f, ledger, &c, admission, &[]);
            install(&f, ledger, view, &c, admission, &[]);
            let ed = [V2OrdinaryDependency::new(&c, admission)];
            retain(&f, ledger, &e, admission, &ed);
            install(&f, ledger, view, &e, admission, &ed);
            let dd = ordered_dependencies(vec![(&c, admission), (&e, admission)]);
            retain(&f, ledger, &d, admission, &dd);
            install(&f, ledger, view, &d, admission, &dd);
            records.push((c, e, d));
        }
        // Converge both additive bubbles in opposite import orders. Each action still passes
        // the real production verifier; no owner key is used during ordinary work or merge.
        for (ledger, view, reverse) in [(&mut left, &lv, false), (&mut right, &rv, true)] {
            let indices: Vec<_> = if reverse {
                (0..members.len()).rev().collect()
            } else {
                (0..members.len()).collect()
            };
            for i in indices {
                let admission = &members[i].1;
                let (c, e, d) = &records[i];
                retain(&f, ledger, c, admission, &[]);
                install(&f, ledger, view, c, admission, &[]);
                let ed = [V2OrdinaryDependency::new(c, admission)];
                retain(&f, ledger, e, admission, &ed);
                install(&f, ledger, view, e, admission, &ed);
                let dd = ordered_dependencies(vec![(c, admission), (e, admission)]);
                retain(&f, ledger, d, admission, &dd);
                install(&f, ledger, view, d, admission, &dd);
                let projection = ledger.guarded_projection(c.id(), 512).unwrap();
                assert!(projection
                    .with_publication(|p| matches!(
                        p,
                        GuardedProjection::Deleted {
                            certainty: ProjectionCertainty::Provisional,
                            ..
                        }
                    ))
                    .unwrap());
            }
        }
        let (key, admission) = &members[1];
        let target = &records[0].0;
        let reply = action(
            key,
            admission,
            77,
            Body::Create {
                text: "cross-bubble reply".into(),
                reply_to: Some(target.id()),
            },
            vec![target.id()],
        );
        let deps = [V2OrdinaryDependency::new(target, &members[0].1)];
        retain(&f, &mut left, &reply, admission, &deps);
        install(&f, &mut left, &lv, &reply, admission, &deps);
        assert_eq!(
            f.owner.calls.get(),
            owner_calls,
            "no per-action owner certification"
        );
    }
}

#[test]
fn selected_local_archive_preserves_edits_deletes_and_replies_after_a1_removal() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[93; 32]);
    let other = DeviceKeypair::from_seed(&[94; 32]);
    let a1 = f.add(&key, 20);
    let b1 = f.add(&other, 21);
    let a_id = a1.admission_data().admission_id();
    let b_id = b1.admission_data().admission_id();
    let message = create(&key, &a1, 1);
    let edit = action(
        &key,
        &a1,
        2,
        Body::Edit {
            create: message.id(),
            text: "retained before removal".into(),
        },
        vec![message.id()],
    );
    let hidden = create(&key, &a1, 3);
    let delete = action(
        &key,
        &a1,
        4,
        Body::Delete {
            create: hidden.id(),
        },
        vec![hidden.id()],
    );
    let reply = action(
        &other,
        &b1,
        5,
        Body::Create {
            text: "reply from a current member".into(),
            reply_to: Some(message.id()),
        },
        vec![message.id()],
    );
    let own_outbox = create(&key, &a1, 6);
    let mut retired = f.remove(&a1);
    let frozen = retired.take_frozen_data().unwrap();
    assert!(retired.take_close_completion().unwrap().try_complete());
    drop(retired);
    let a2 = f.add(&key, 22);
    let archive =
        |action: &SignedAction, admission, ledger: &mut ChatLedger, view: &ProjectionView| {
            f.verifier.project_local_archive_original(
                ledger,
                view,
                V2LocalArchiveOriginal::from_trusted_local_selection(
                    action,
                    admission,
                    &action.encode(),
                )
                .unwrap(),
                &[&frozen],
                128,
            )
        };
    let (mut ledger, live) = f.ledger();
    assert!(archive(&message, a_id, &mut ledger, &live).is_err());
    let mut view = ledger
        .begin_local_archive_projection_view_in(&f.group_view)
        .unwrap();
    assert!(view.try_ready());
    let current = create(&other, &b1, 7);
    let draft = ActionDraft::new(current.spec().clone(), *current.signer()).unwrap();
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &b1, &[], &view)
        .is_err());
    assert!(f
        .verifier
        .provisional_action(&current, &b1, &[], &view)
        .is_err());
    assert!(f
        .verifier
        .project_provisional_from_ledger(&mut ledger, &view, &current, &b1, 128)
        .is_err());
    assert!(ledger.action(current.id()).is_none());
    assert!(archive(
        &message,
        a2.admission_data().admission_id(),
        &mut ledger,
        &view
    )
    .is_err());
    let mut changed = message.encode();
    *changed.last_mut().unwrap() ^= 1;
    assert!(
        V2LocalArchiveOriginal::from_trusted_local_selection(&message, a_id, &changed).is_err()
    );
    assert_eq!(
        archive(&reply, b_id, &mut ledger, &view).unwrap(),
        V2OrdinaryReceiveStatus::WaitingForDependencies(vec![message.id()]),
    );
    assert!(ledger.action(reply.id()).is_none());
    for (original, admission) in [
        (&message, a_id),
        (&edit, a_id),
        (&hidden, a_id),
        (&reply, b_id),
    ] {
        assert_eq!(
            archive(original, admission, &mut ledger, &view).unwrap(),
            V2OrdinaryReceiveStatus::Retained(ActionState::Valid),
        );
    }
    assert!(ledger.guarded_projection(message.id(), 128).unwrap()
        .with_publication(|p| matches!(p, GuardedProjection::Visible {
            text, certainty: ProjectionCertainty::Provisional, .. } if text == "retained before removal"))
        .unwrap());
    let before_delete = ledger.guarded_projection(hidden.id(), 128).unwrap();
    assert!(before_delete
        .with_publication(|projection| matches!(projection, GuardedProjection::Visible { .. }))
        .unwrap());
    assert!(ledger.projectable_causal_heads(hidden.id(), 128).is_err());
    assert_eq!(
        archive(&delete, a_id, &mut ledger, &view).unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert!(before_delete.with_publication(|_| ()).is_err());
    assert!(ledger.guarded_projection(hidden.id(), 128).unwrap()
        .with_publication(|p| matches!(p, GuardedProjection::Deleted {
            delete: applied, certainty: ProjectionCertainty::Provisional, .. } if *applied == delete.id()))
        .unwrap());
    assert!(ledger.guarded_projection(reply.id(), 128).unwrap()
        .with_publication(|p| matches!(p, GuardedProjection::Visible {
            reply_to: Some(parent), certainty: ProjectionCertainty::Provisional, .. } if *parent == message.id()))
        .unwrap());
    assert_eq!(f.verifier.replay_local_archive_dependency(
        &mut ledger, &view, &own_outbox, a_id, &[&frozen], 128,
    ).unwrap(), V2OrdinaryReceiveStatus::Retained(ActionState::Valid));
    assert!(ledger
        .guarded_projection(own_outbox.id(), 128)
        .unwrap()
        .with_publication(|p| matches!(p, GuardedProjection::Held { .. }))
        .unwrap());
    assert!(f
        .verifier
        .provisional_action(&own_outbox, &a1, &[], &view)
        .is_err());
}

#[test]
fn current_signer_can_only_reply_to_an_exact_selected_retired_create() {
    let mut f = Fixture::new();
    let signer = DeviceKeypair::from_seed(&[115; 32]);
    let first = f.add(&signer, 39);
    let parent = create(&signer, &first, 1);
    let parent_raw = parent.encode();
    let archived = || {
        V2LocalArchiveOriginal::from_trusted_local_selection(
            &parent,
            first.admission_data().admission_id(),
            &parent_raw,
        )
        .unwrap()
    };
    let mut removed = f.remove(&first);
    assert!(removed.take_close_completion().unwrap().try_complete());
    let retirements = [removed.take_frozen_data().unwrap()];
    drop(removed);
    let replacement = f.add(&signer, 40);
    let (_ledger, view) = f.ledger();
    let reply = action(
        &signer,
        &replacement,
        2,
        Body::Create {
            text: "a new reply".into(),
            reply_to: Some(parent.id()),
        },
        vec![parent.id()],
    );
    let draft = ActionDraft::new(reply.spec().clone(), *reply.signer()).unwrap();
    assert!(f
        .verifier
        .provisional_action(
            &reply,
            &replacement,
            &[V2OrdinaryDependency::new(&parent, &first)],
            &view,
        )
        .is_err());
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(
            &draft,
            &replacement,
            &[V2OrdinaryDependency::new(&parent, &first)],
            &view,
        )
        .is_err());
    assert!(f
        .verifier
        .local_archive_reply_parent(archived(), &[])
        .is_err());
    let foreign = Fixture::new();
    assert!(foreign
        .verifier
        .local_archive_reply_parent(archived(), &retirements)
        .is_err());
    let old = f
        .verifier
        .local_archive_reply_parent(archived(), &retirements)
        .unwrap();
    let plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &replacement, &[old], &view)
        .unwrap();
    assert_eq!(
        plan.admission_id_data(),
        replacement.admission_data().admission_id()
    );

    let edit = action(
        &signer,
        &replacement,
        3,
        Body::Edit {
            create: parent.id(),
            text: "old incarnation".into(),
        },
        vec![parent.id()],
    );
    assert!(f
        .verifier
        .provisional_action(
            &edit,
            &replacement,
            &[V2OrdinaryDependency::new(&parent, &first)],
            &view,
        )
        .is_err());
    let draft = ActionDraft::new(edit.spec().clone(), *edit.signer()).unwrap();
    let old = f
        .verifier
        .local_archive_reply_parent(archived(), &retirements)
        .unwrap();
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &replacement, &[old], &view)
        .is_err());
}

#[test]
fn incoming_reply_requires_a_selected_archive_receipt_even_for_a_hot_valid_parent() {
    let mut f = Fixture::new();
    let author = DeviceKeypair::from_seed(&[116; 32]);
    let current_key = DeviceKeypair::from_seed(&[117; 32]);
    let original = f.add(&author, 41);
    let current = f.add(&current_key, 42);
    let parent = create(&author, &original, 1);
    let (mut ledger, view) = f.ledger();
    retain(&f, &mut ledger, &parent, &original, &[]);
    let late = create(&author, &original, 2);
    let mut removed = f.remove(&original);
    assert!(removed.take_close_completion().unwrap().try_complete());
    let frozen = removed.take_frozen_data().unwrap();
    drop(removed);
    let retired = [&frozen];
    let reply = action(
        &current_key,
        &current,
        3,
        Body::Create {
            text: "reply after removal".into(),
            reply_to: Some(parent.id()),
        },
        vec![parent.id()],
    );
    assert_eq!(ledger.state(parent.id()), Some(ActionState::Valid));
    assert!(!f
        .verifier
        .locally_usable_ordinary_dependency(&ledger, parent.id()));
    assert_eq!(
        f.verifier
            .apply_with_locally_selected_ancestors(
                &mut ledger,
                &view,
                &reply,
                &current,
                &retired,
                (128, true)
            )
            .unwrap(),
        V2OrdinaryReceiveStatus::WaitingForDependencies(vec![parent.id()])
    );
    assert!(ledger.action(reply.id()).is_none());
    let admission = original.admission_data().admission_id();
    assert!(f
        .verifier
        .replay_current_outbox_ancestor(&mut ledger, &view, (&parent, admission), &retired, 128,)
        .is_err());
    let raw = parent.encode();
    let receipt =
        V2LocalArchiveOriginal::from_trusted_local_selection(&parent, admission, &raw).unwrap();
    assert_eq!(
        f.verifier
            .replay_locally_selected_ancestor(&mut ledger, &view, receipt, &retired, 128)
            .unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert!(f
        .verifier
        .locally_usable_ordinary_dependency(&ledger, parent.id()));
    assert!(ledger
        .guarded_projection(parent.id(), 128)
        .unwrap()
        .with_publication(|projection| matches!(projection, GuardedProjection::Held { .. }))
        .unwrap());
    assert!(f
        .verifier
        .apply_with_locally_selected_ancestors(
            &mut ledger,
            &view,
            &late,
            &original,
            &retired,
            (128, true),
        )
        .is_err());
    assert_eq!(
        f.verifier
            .apply_with_locally_selected_ancestors(
                &mut ledger,
                &view,
                &reply,
                &current,
                &retired,
                (128, true),
            )
            .unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert!(ledger.guarded_projection(reply.id(), 128).unwrap().with_publication(
        |projection| matches!(projection, GuardedProjection::Visible { text, .. } if text == "reply after removal"),
    ).unwrap());
    let edited = action(
        &current_key,
        &current,
        4,
        Body::Edit {
            create: reply.id(),
            text: "edit of my reply".into(),
        },
        vec![reply.id()],
    );
    assert_eq!(
        f.verifier
            .apply_with_locally_selected_ancestors(
                &mut ledger,
                &view,
                &edited,
                &current,
                &retired,
                (128, true),
            )
            .unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert!(ledger.guarded_projection(reply.id(), 128).unwrap().with_publication(
        |projection| matches!(projection, GuardedProjection::Visible { text, .. } if text == "edit of my reply"),
    ).unwrap());
}

#[test]
fn actual_remove_rejoin_closes_a1_open_but_keeps_verified_inclusion_and_historical_rights() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[92; 32]);
    let a1 = f.add(&key, 20);
    let proven = create(&key, &a1, 1);
    let late = create(&key, &a1, 2);
    let (mut ledger, view) = f.ledger();
    retain(&f, &mut ledger, &proven, &a1, &[]);
    retain(&f, &mut ledger, &late, &a1, &[]);
    install(&f, &mut ledger, &view, &proven, &a1, &[]);
    install(&f, &mut ledger, &view, &late, &a1, &[]);
    let old_lease = ledger.guarded_projection(late.id(), 128).unwrap();
    let (record, append) = f.append_one(&a1, &proven);
    drop(append);
    let token = f
        .verifier
        .accepted_action(
            &proven,
            V2GeneralAcceptanceSource::Current(&a1),
            &[],
            &record,
            0,
            &[],
            &view,
        )
        .unwrap();
    ledger.admit_projection(&token, 128).unwrap();
    let mut removed = f.remove(&a1);
    let frozen = removed.take_frozen_data().unwrap();
    let mut closed = removed.take_close_completion().unwrap();
    assert!(closed.try_complete());
    assert!(old_lease.with_publication(|_| ()).is_err());
    assert!(f.verifier.verify_action_semantics(&late, &a1, &[]).is_ok());
    assert!(f
        .verifier
        .provisional_action(&late, &a1, &[], &view)
        .is_err());
    assert!(ledger
        .guarded_projection(proven.id(), 128)
        .unwrap()
        .with_publication(|p| matches!(
            p,
            GuardedProjection::Visible {
                certainty: ProjectionCertainty::Proven,
                ..
            }
        ))
        .unwrap());
    assert!(ledger
        .guarded_projection(late.id(), 128)
        .unwrap()
        .with_publication(|p| matches!(p, GuardedProjection::Held { .. }))
        .unwrap());
    let frozen_token = f
        .verifier
        .accepted_action(
            &proven,
            V2GeneralAcceptanceSource::Frozen(&frozen),
            &[],
            &record,
            0,
            &[],
            &view,
        )
        .unwrap();
    ledger.admit_projection(&frozen_token, 128).unwrap();
    let late_record = AcceptanceRecord::new(AcceptanceSpec {
        group: GROUP.to_vec(),
        context: record.spec().context,
        channel: 42,
        action: late.id(),
        subject: *late.signer(),
        grant: late.spec().grant,
        basis: late.spec().basis,
        create: None,
    })
    .unwrap();
    assert!(f
        .verifier
        .accepted_action(
            &late,
            V2GeneralAcceptanceSource::Frozen(&frozen),
            &[],
            &late_record,
            0,
            &[],
            &view
        )
        .is_err());
    drop(removed); // actual old math drains before another candidate can allocate
    let a2 = f.add(&key, 21);
    assert_ne!(
        a1.admission_data().admission_id(),
        a2.admission_data().admission_id()
    );
    assert!(f
        .verifier
        .provisional_action(&late, &a2, &[], &view)
        .is_err());
    let fresh = create(&key, &a2, 3);
    retain(&f, &mut ledger, &fresh, &a2, &[]);
    install(&f, &mut ledger, &view, &fresh, &a2, &[]);
    assert!(f
        .verifier
        .provisional_action(&late, &a1, &[], &view)
        .is_err());
}

#[test]
fn actual_minimal_dependency_admission_binding_and_held_target_never_open_implicitly() {
    let mut f = Fixture::new();
    let k1 = DeviceKeypair::from_seed(&[93; 32]);
    let a1 = f.add(&k1, 24);
    let k2 = DeviceKeypair::from_seed(&[94; 32]);
    let a2 = f.add(&k2, 25);
    let c = create(&k1, &a1, 1);
    let e = action(
        &k1,
        &a1,
        2,
        Body::Edit {
            create: c.id(),
            text: "held edit".into(),
        },
        vec![c.id()],
    );
    let (mut ledger, view) = f.ledger();
    retain(&f, &mut ledger, &c, &a1, &[]);
    assert!(f.verifier.verify_action_semantics(&e, &a1, &[]).is_err());
    assert!(f
        .verifier
        .verify_action_semantics(&e, &a1, &[V2OrdinaryDependency::new(&c, &a2)])
        .is_err());
    let deps = [V2OrdinaryDependency::new(&c, &a1)];
    retain(&f, &mut ledger, &e, &a1, &deps);
    install(&f, &mut ledger, &view, &e, &a1, &deps);
    assert!(ledger
        .guarded_projection(c.id(), 128)
        .unwrap()
        .with_publication(|p| matches!(p, GuardedProjection::Held { .. }))
        .unwrap());
    assert!(
        f.verifier.verify_action_semantics(&c, &a1, &deps).is_err(),
        "caller extra coverage refuses"
    );
    let theft = action(&k2, &a2, 3, Body::Delete { create: c.id() }, vec![c.id()]);
    assert_eq!(
        f.verifier
            .verify_action_semantics(&theft, &a2, &deps)
            .unwrap_err(),
        ChatError::Permission
    );
    install(&f, &mut ledger, &view, &c, &a1, &[]);
    let lease = ledger.guarded_projection(c.id(), 128).unwrap();
    f.negative.0.store(false, Ordering::Release);
    assert!(lease.with_publication(|_| ()).is_err());
    assert!(f.verifier.provisional_action(&c, &a1, &[], &view).is_err());
}

#[test]
fn actual_known_parent_equivocation_fences_all_channels_before_body_and_preserves_both_headers() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[95; 32]);
    let admission = f.add(&key, 26);
    let c = create(&key, &admission, 1);
    let (mut ledger, view) = f.ledger();
    retain(&f, &mut ledger, &c, &admission, &[]);
    install(&f, &mut ledger, &view, &c, &admission, &[]);
    let lease = ledger.guarded_projection(c.id(), 128).unwrap();
    let base = f.verifier.replay_data();
    let mut spec = base.header().spec().clone();
    spec.parent = base.header().id();
    spec.sequence += 1;
    spec.body = OwnerBodyId::from_bytes([31; 32]);
    let first = SignedOwnerHeader::sign_using(spec.clone(), |b| Ok(f.owner.key.sign(b))).unwrap();
    f.verifier.observe_current_header(&first).unwrap();
    spec.body = OwnerBodyId::from_bytes([32; 32]);
    let second = SignedOwnerHeader::sign_using(spec, |b| Ok(f.owner.key.sign(b))).unwrap();
    assert_eq!(
        f.verifier.observe_current_header(&second).unwrap_err(),
        ChatError::AuthorityConflict
    );
    let (a, b) = f.verifier.current_contradiction_data().unwrap();
    assert_eq!(a.id(), first.id());
    assert_eq!(b.id(), second.id());
    assert!(lease.with_publication(|_| ()).is_err());
    assert_eq!(ledger.state(c.id()), Some(ActionState::Valid));
    assert!(f
        .verifier
        .fork_close_completion_mut()
        .unwrap()
        .try_complete());
    // An equal-label replacement lifetime cannot renew tokens retaining the original one.
    let mut replacement = f._group.begin_view();
    assert!(replacement.try_ready());
    assert!(lease.with_publication(|_| ()).is_err());
}

#[test]
fn actual_removed_provisional_heads_remain_historical_knowledge_without_hiding_other_members() {
    let mut f = Fixture::new();
    let removed_key = DeviceKeypair::from_seed(&[96; 32]);
    let removed = f.add(&removed_key, 27);
    let current_key = DeviceKeypair::from_seed(&[97; 32]);
    let current = f.add(&current_key, 28);
    let (mut ledger, view) = f.ledger();
    let held = create(&removed_key, &removed, 1);
    retain(&f, &mut ledger, &held, &removed, &[]);
    install(&f, &mut ledger, &view, &held, &removed, &[]);
    let live = create(&current_key, &current, 2);
    retain(&f, &mut ledger, &live, &current, &[]);
    install(&f, &mut ledger, &view, &live, &current, &[]);
    // A removed member's old Join on SOMEONE ELSE's still-open Create is historical
    // knowledge. Its projection permission is never needed to traverse its causal edges.
    let old_join = action(
        &removed_key,
        &removed,
        3,
        Body::Join { create: live.id() },
        vec![live.id()],
    );
    let old_deps = [V2OrdinaryDependency::new(&live, &current)];
    retain(&f, &mut ledger, &old_join, &removed, &old_deps);
    install(&f, &mut ledger, &view, &old_join, &removed, &old_deps);
    let mut retirement = f.remove(&removed);
    let mut close = retirement.take_close_completion().unwrap();
    assert!(close.try_complete());
    assert!(ledger
        .guarded_projection(held.id(), 128)
        .unwrap()
        .with_publication(|p| matches!(p, GuardedProjection::Held { .. }))
        .unwrap());
    let edit = action(
        &current_key,
        &current,
        4,
        Body::Edit {
            create: live.id(),
            text: "current member still writes".into(),
        },
        vec![old_join.id()],
    );
    let dependencies = ordered_dependencies(vec![(&live, &current), (&old_join, &removed)]);
    retain(&f, &mut ledger, &edit, &current, &dependencies);
    assert!(f
        .verifier
        .provisional_action(&edit, &current, &dependencies, &view)
        .is_err());
    let frozen = retirement.take_frozen_data().unwrap();
    let old_bytes = old_join.encode();
    let receipt = V2LocalArchiveOriginal::from_trusted_local_selection(
        &old_join,
        removed.admission_data().admission_id(),
        &old_bytes,
    )
    .unwrap();
    assert_eq!(
        f.verifier
            .replay_locally_selected_ancestor(&mut ledger, &view, receipt, &[&frozen], 128)
            .unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert_eq!(
        f.verifier
            .apply_with_locally_selected_ancestors(
                &mut ledger,
                &view,
                &edit,
                &current,
                &[&frozen],
                (128, true)
            )
            .unwrap(),
        V2OrdinaryReceiveStatus::Retained(ActionState::Valid)
    );
    assert!(ledger.guarded_projection(live.id(),128).unwrap().with_publication(|p|matches!(p,GuardedProjection::Visible {selected,text,..} if *selected==edit.id() && text=="current member still writes")).unwrap());
    // A Join may remain as historical knowledge. A current reply can be
    // visible with an exact local ancestor receipt without opening its parent.
    let join = action(
        &current_key,
        &current,
        5,
        Body::Join { create: held.id() },
        vec![held.id()],
    );
    let deps = [V2OrdinaryDependency::new(&held, &removed)];
    retain(&f, &mut ledger, &join, &current, &deps);
    assert!(f
        .verifier
        .provisional_action(&join, &current, &deps, &view)
        .is_err());
    assert!(ledger
        .projectable_causal_heads(held.id(), 128)
        .unwrap()
        .with_publication(|heads| heads.is_empty())
        .unwrap());
    let reply = action(
        &current_key,
        &current,
        6,
        Body::Create {
            text: "independent reply".into(),
            reply_to: Some(held.id()),
        },
        vec![held.id()],
    );
    retain(&f, &mut ledger, &reply, &current, &deps);
    let held_bytes = held.encode();
    let receipt = V2LocalArchiveOriginal::from_trusted_local_selection(
        &held,
        removed.admission_data().admission_id(),
        &held_bytes,
    )
    .unwrap();
    let retired = [frozen];
    let loan = f
        .verifier
        .local_archive_reply_parent(receipt, &retired)
        .unwrap();
    install(&f, &mut ledger, &view, &reply, &current, &[loan]);
    assert!(ledger
        .guarded_projection(reply.id(), 128)
        .unwrap()
        .with_publication(
            |p| matches!(p,GuardedProjection::Visible {text,..} if text=="independent reply")
        )
        .unwrap());
    assert!(ledger
        .guarded_projection(held.id(), 128)
        .unwrap()
        .with_publication(|p| matches!(p, GuardedProjection::Held { .. }))
        .unwrap());
}

#[test]
fn actual_removal_drain_waits_for_admitted_original_publication_and_is_monotone() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[98; 32]);
    let admission = f.add(&key, 29);
    let c = create(&key, &admission, 1);
    let (mut ledger, view) = f.ledger();
    retain(&f, &mut ledger, &c, &admission, &[]);
    install(&f, &mut ledger, &view, &c, &admission, &[]);
    let lease = ledger.guarded_projection(c.id(), 128).unwrap();
    let mut completion = None;
    lease
        .with_publication(|_| {
            let mut removed = f.remove(&admission);
            let mut closed = removed.take_close_completion().unwrap();
            assert!(
                !closed.try_complete(),
                "actual original group reader is still admitted"
            );
            completion = Some(closed);
        })
        .unwrap();
    let mut completion = completion.unwrap();
    assert!(completion.try_complete());
    assert!(completion.try_complete());
    assert!(lease.with_publication(|_| ()).is_err());
    assert_eq!(ledger.state(c.id()), Some(ActionState::Valid));
}

#[test]
fn actual_foreign_bad_signature_unknown_parent_refuse_and_old_known_parent_fork_closes() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[99; 32]);
    let admission = f.add(&key, 30);
    let c = create(&key, &admission, 1);
    let base = f.verifier.replay_data();
    let mut spec = base.header().spec().clone();
    spec.parent = base.header().id();
    spec.sequence += 1;
    spec.body = OwnerBodyId::from_bytes([41; 32]);
    let foreign_key = DeviceKeypair::from_seed(&[100; 32]);
    let mut foreign = spec.clone();
    foreign.issuer = foreign_key.verifying_key().to_bytes();
    let foreign = SignedOwnerHeader::sign_using(foreign, |b| Ok(foreign_key.sign(b))).unwrap();
    assert_eq!(
        f.verifier.observe_current_header(&foreign).unwrap_err(),
        ChatError::Scope
    );
    let mut unknown = spec.clone();
    unknown.parent = OwnerHeaderId::from_bytes([42; 32]);
    let unknown = SignedOwnerHeader::sign_using(unknown, |b| Ok(f.owner.key.sign(b))).unwrap();
    assert_eq!(
        f.verifier.observe_current_header(&unknown).unwrap_err(),
        ChatError::Dependency
    );
    let valid = SignedOwnerHeader::sign_using(spec, |b| Ok(f.owner.key.sign(b))).unwrap();
    let mut encoded = valid.encode();
    *encoded.last_mut().unwrap() ^= 1;
    let bad = SignedOwnerHeader::decode(&encoded).unwrap();
    assert_eq!(bad.id(), valid.id());
    assert_eq!(
        f.verifier.observe_current_header(&bad).unwrap_err(),
        ChatError::Signature
    );
    assert!(f.verifier.known.first.is_none());
    assert!(f.verifier.fork_close.is_none());
    // No invalid input displaced an active selected Admission or granted semantic authority.
    assert!(f
        .verifier
        .verify_action_semantics(&c, &admission, &[])
        .is_ok());
    let (record, mut retired) = f.append_one(&admission, &c);
    let mut archived = retired.take_known_parent_data().unwrap();
    drop(retired); // opaque archive escrow does not retain the old whole model
    let mut old = archived.parent_data().spec().clone();
    old.parent = archived.parent_data().id();
    old.sequence += 1;
    old.body = OwnerBodyId::from_bytes([43; 32]);
    let sibling = SignedOwnerHeader::sign_using(old, |b| Ok(f.owner.key.sign(b))).unwrap();
    assert_eq!(
        f.verifier
            .observe_archived_header(&mut archived, &sibling)
            .unwrap_err(),
        ChatError::AuthorityConflict
    );
    assert!(archived.contradiction_data().is_some());
    assert!(f
        .verifier
        .verify_action_semantics(&c, &admission, &[])
        .is_err());
    assert_eq!(record.spec().action, c.id());
}

#[test]
fn actual_candidate_is_single_and_wrong_selected_expectation_preserves_same_whole_owner() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[101; 32]);
    let body = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: f.verifier.replay_data().profile().id(),
        subject: key.verifying_key().to_bytes(),
        origin: AdmissionOrigin::Add {
            commit: [51; 32],
            before_epoch: 0,
            after_epoch: 1,
            key_package: [52; 32],
            credential: [53; 32],
        },
    })
    .unwrap();
    let context = TreeContext {
        profile: body.spec().profile,
        admission: body.admission_id(),
    };
    let mut slots: Vec<_> = f.verifier.replay_data().slots().map(|s| s.image).collect();
    slots.push(CurrentAdmissionSlot {
        admission: body.id(),
        subject: body.spec().subject,
        tip_body: body.id(),
        accepted_count: 0,
        accepted_root: context.empty_hash(),
    });
    let image = f.image(HeaderKind::Admission, body.id(), 1, slots);
    let header = f.owner.header(&image);
    let mut candidate = Some(
        f.verifier
            .derive_successor(&header, ReplayBody::Admission(&body), &image)
            .unwrap(),
    );
    assert!(matches!(
        f.verifier
            .derive_successor(&header, ReplayBody::Admission(&body), &image),
        Err(ChatError::Capacity)
    ));
    let old = f.verifier.replay_data().header().id();
    let wrong = V2HostSelectionExpectation::from_trusted_host(old, header.spec().state);
    assert!(f
        .verifier
        .advance_host_selected(&wrong, &mut candidate)
        .is_err());
    assert!(candidate.is_some());
    assert_eq!(f.verifier.replay_data().header().id(), old);
    let right = V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    let retired = f
        .verifier
        .advance_host_selected(&right, &mut candidate)
        .unwrap();
    assert!(candidate.is_none());
    assert_eq!(f.verifier.replay_data().header().id(), header.id());
    assert_eq!(f.verifier.admissions.len(), 2);
    let mut successor = header.spec().clone();
    successor.parent = header.id();
    successor.sequence += 1;
    let successor = SignedOwnerHeader::sign_using(successor, |b| Ok(f.owner.key.sign(b))).unwrap();
    assert!(
        matches!(
            f.verifier
                .derive_successor(&successor, ReplayBody::Admission(&body), &image),
            Err(ChatError::Capacity)
        ),
        "old actual math must drain before a third model could be derived"
    );
    assert_eq!(retired.parent_data().header().id(), old);
    drop(retired);
    assert!(f
        .verifier
        .derive_successor(&successor, ReplayBody::Admission(&body), &image)
        .is_err());
    assert!(
        !f.verifier.candidate_live.load(Ordering::Acquire),
        "failed body verification releases the same empty candidate claim"
    );
}

#[test]
fn actual_selected_remove_returns_whole_original_cut_for_independent_archive_custody() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[91; 32]);
    let admission = f.add(&key, 19);
    let original = create(&key, &admission, 7);
    let (_, appended) = f.append_one(&admission, &original);
    drop(appended);
    let slot = f
        .verifier
        .replay_data()
        .slot(admission.core.body.id())
        .unwrap();
    let expected_tip = slot.frontier.tip();
    let expected_header = slot.tip_header;
    let expected_hashes: Vec<_> = slot.frontier.hashes().collect();
    let mut retired = f.remove(&admission);
    let frozen = retired.take_frozen_data().unwrap();
    assert!(retired.frozen_data().is_none());
    let mut drain = retired.take_close_completion().unwrap();
    assert!(drain.try_complete());
    // Actual old math and then original governor can drain independently. The
    // retained cut is historical DATA, never renewed OPEN or a new live issuer.
    drop(retired);
    drop(f);
    assert_eq!(frozen.cut.admission, admission.core.body.id());
    assert_eq!(frozen.cut.subject, key.verifying_key().to_bytes());
    assert_eq!(frozen.cut.tip_header, expected_header);
    assert_eq!(frozen.cut.tip, expected_tip);
    assert_eq!(frozen.cut.frontier.tip(), expected_tip);
    assert_eq!(
        frozen.cut.frontier.hashes().collect::<Vec<_>>(),
        expected_hashes
    );
    drop(frozen); // whole archive owner is disposed outside any publication writer
}

#[test]
fn actual_concurrent_ordinary_forks_merge_additively_without_governing_conflict_or_owner() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[90; 32]);
    let admission = f.add(&key, 18);
    let owner_calls = f.owner.calls.get();
    let c = create(&key, &admission, 1);
    // Both edits acknowledge only Create. Presentation clocks point backwards in
    // the helper; neither wall time nor delivery order may erase either branch.
    let ea = action(
        &key,
        &admission,
        2,
        Body::Edit {
            create: c.id(),
            text: "left bubble edit".into(),
        },
        vec![c.id()],
    );
    let eb = action(
        &key,
        &admission,
        3,
        Body::Edit {
            create: c.id(),
            text: "right bubble edit".into(),
        },
        vec![c.id()],
    );
    let delete = action(
        &key,
        &admission,
        4,
        Body::Delete { create: c.id() },
        vec![ea.id()],
    );
    let join = action(
        &key,
        &admission,
        5,
        Body::Join { create: c.id() },
        vec![eb.id(), delete.id()],
    );
    let all = [&c, &ea, &eb, &delete, &join];
    let deliver = |ledger: &mut ChatLedger, view: &ProjectionView, incoming: &[&SignedAction]| {
        for a in incoming {
            let deps: Vec<_> = a
                .dependencies()
                .iter()
                .map(|id| {
                    V2OrdinaryDependency::new(
                        all.iter().copied().find(|a| a.id() == *id).unwrap(),
                        &admission,
                    )
                })
                .collect();
            ledger.import((*a).clone()).unwrap();
            ledger
                .authorize(
                    a.id(),
                    f.verifier
                        .verify_action_semantics(a, &admission, &deps)
                        .unwrap(),
                )
                .unwrap();
            // Quiesce even when this delivery still lacks a dependency. Later
            // arrivals must requeue that pending action without another witness.
            while ledger.advance(1).remaining {}
        }
        while ledger.advance(1).remaining {}
        for a in incoming {
            let deps: Vec<_> = a
                .dependencies()
                .iter()
                .map(|id| {
                    V2OrdinaryDependency::new(
                        all.iter().copied().find(|a| a.id() == *id).unwrap(),
                        &admission,
                    )
                })
                .collect();
            assert_eq!(ledger.state(a.id()), Some(ActionState::Valid));
            install(&f, ledger, view, a, &admission, &deps);
        }
    };
    let (mut left, lv) = f.ledger();
    let (mut right, rv) = f.ledger();
    deliver(&mut left, &lv, &[&c, &ea]);
    deliver(&mut right, &rv, &[&c, &eb]);
    deliver(&mut left, &lv, &[&eb, &ea]);
    deliver(&mut right, &rv, &[&ea, &eb]);
    let first = left.projection(c.id(), 512).unwrap().unwrap();
    assert_eq!(first, right.projection(c.id(), 512).unwrap().unwrap());
    assert_eq!(first.alternatives.len(), 2);
    assert_eq!(
        left.causal_heads(c.id(), 512).unwrap(),
        right.causal_heads(c.id(), 512).unwrap()
    );
    assert_eq!(left.causal_heads(c.id(), 512).unwrap().len(), 2);
    assert_eq!(
        left.guarded_projection(c.id(), 512)
            .unwrap()
            .with_publication(Clone::clone)
            .unwrap(),
        right
            .guarded_projection(c.id(), 512)
            .unwrap()
            .with_publication(Clone::clone)
            .unwrap()
    );
    // On the left, Join arrives before its Delete dependency. On the right,
    // Delete arrives first. Duplicates and one-unit scheduling cannot lose edits,
    // reopen deleted text or turn ordinary same-signer forks into governor forks.
    deliver(&mut left, &lv, &[&join, &delete, &join]);
    deliver(&mut right, &rv, &[&delete, &join, &delete]);
    assert_eq!(
        left.action_ids().collect::<Vec<_>>(),
        right.action_ids().collect::<Vec<_>>()
    );
    assert_eq!(left.action_ids().count(), 5);
    assert_eq!(left.causal_heads(c.id(), 512).unwrap(), vec![join.id()]);
    assert_eq!(
        left.causal_heads(c.id(), 512).unwrap(),
        right.causal_heads(c.id(), 512).unwrap()
    );
    let audit = left.projection(c.id(), 512).unwrap().unwrap();
    assert_eq!(audit, right.projection(c.id(), 512).unwrap().unwrap());
    assert_eq!(audit.alternatives.len(), 2);
    assert_eq!(audit.deleted_by, vec![delete.id()]);
    assert!(audit.visible_text.is_none());
    let projected = left
        .guarded_projection(c.id(), 512)
        .unwrap()
        .with_publication(Clone::clone)
        .unwrap();
    assert_eq!(
        projected,
        right
            .guarded_projection(c.id(), 512)
            .unwrap()
            .with_publication(Clone::clone)
            .unwrap()
    );
    assert!(matches!(
        projected,
        GuardedProjection::Deleted {
            certainty: ProjectionCertainty::Provisional,
            ..
        }
    ));
    for a in all {
        assert_eq!(left.action(a.id()), Some(a));
        assert_eq!(right.action(a.id()), Some(a));
    }
    assert!(f.verifier.current_contradiction_data().is_none());
    assert!(f.verifier.fork_close_completion_mut().is_none());
    assert_eq!(f.owner.calls.get(), owner_calls);
}

#[test]
fn removed_create_evidence_names_the_selected_remove_and_keeps_signature_bytes() {
    use crate::chat::cold_actions::OriginalActionDigest;
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[109; 32]);
    let first = f.add(&key, 20);
    let old = create(&key, &first, 1);
    let mut removed = f.remove(&first);
    assert!(removed.take_close_completion().unwrap().try_complete());
    let frozen = removed.take_frozen_data().unwrap();
    drop(removed);
    let selected = f.verifier.replay_data();
    let (ledger, view) = f.ledger();
    assert_eq!(
        frozen.admission_id_data(),
        first.admission_data().admission_id()
    );
    let original = old.encode();
    let proof = f
        .verifier
        .verify_removed_create_for_review(&original, &frozen, &ledger, &view, &[])
        .unwrap();
    assert_eq!(
        proof.removal_data(),
        (selected.header().id(), selected.mls_epoch_data())
    );
    assert_ne!(proof.removal_data().0, frozen.cut.tip_header);
    assert_eq!(
        proof.target_data().spec().context.admission,
        first.admission_data().admission_id()
    );
    assert_eq!(proof.target_data().spec().action, old.id());
    assert_eq!(
        proof.original_digest_data(),
        OriginalActionDigest::for_original_bytes(&original)
    );
    assert!(ledger.action(old.id()).is_none());
    assert!(f.verifier.require_current_removed_create(&proof).is_ok());
    let frame = proof.encode_record_data(&original).unwrap();
    let record = V2RemovedCreateRecord::decode(&frame).unwrap();
    assert_eq!(record.original_bytes_data(), original);
    assert_eq!(record.target_data(), proof.target_data());
    assert_eq!(record.removal_data(), proof.removal_data());
    assert!(record.require_evidence(&proof).is_ok());
    for end in 0..frame.len() {
        assert!(V2RemovedCreateRecord::decode(&frame[..end]).is_err());
    }
    let mut trailing = frame.to_vec();
    trailing.push(0);
    assert!(V2RemovedCreateRecord::decode(&trailing).is_err());
    let oversize = vec![0; MAX_REMOVED_CREATE_RECORD_BYTES + 1];
    assert!(matches!(
        V2RemovedCreateRecord::decode(&oversize),
        Err(ChatError::Bound)
    ));
    let mut changed_remove = frame.to_vec();
    let mut fields = catcoms_wire::Decoder::new(&frame);
    for _ in 0..3 {
        fields.get_bytes().unwrap();
    }
    let epoch_at = frame.len() - fields.remaining();
    changed_remove[epoch_at + 7] ^= 1;
    let foreign = V2RemovedCreateRecord::decode(&changed_remove).unwrap();
    assert!(matches!(
        foreign.require_evidence(&proof),
        Err(ChatError::Scope)
    ));

    let mut corrupt = original.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(proof.require_original_data(&original).is_ok());
    assert!(proof.require_original_data(&[]).is_err());
    assert!(proof.require_original_data(&corrupt).is_err());
    assert!(proof.encode_record_data(&corrupt).is_err());
    let mut corrupt_frame = frame.to_vec();
    *corrupt_frame.last_mut().unwrap() ^= 1;
    assert!(V2RemovedCreateRecord::decode(&corrupt_frame).is_err());
    fields.get_u64().unwrap();
    let digest_at = frame.len() - fields.remaining() + 4;
    corrupt_frame[digest_at..digest_at + 32]
        .copy_from_slice(OriginalActionDigest::for_original_bytes(&corrupt).as_bytes());
    assert!(matches!(
        V2RemovedCreateRecord::decode(&corrupt_frame),
        Err(ChatError::Signature)
    ));
    assert!(matches!(
        f.verifier
            .verify_removed_create_for_review(&corrupt, &frozen, &ledger, &view, &[]),
        Err(ChatError::Signature)
    ));
    let other = Fixture::new();
    let (other_ledger, other_view) = other.ledger();
    assert!(matches!(
        other.verifier.verify_removed_create_for_review(
            &original,
            &frozen,
            &other_ledger,
            &other_view,
            &[]
        ),
        Err(ChatError::Scope)
    ));

    let replacement = f.add(&key, 21);
    let changed = create(&key, &replacement, 2).encode();
    assert!(f.verifier.require_current_removed_create(&proof).is_err());
    let (ledger, view) = f.ledger();
    assert!(matches!(
        f.verifier
            .verify_removed_create_for_review(&changed, &frozen, &ledger, &view, &[]),
        Err(ChatError::Authority)
    ));
    let fresh = f
        .verifier
        .verify_removed_create_for_review(&original, &frozen, &ledger, &view, &[])
        .unwrap();
    assert_eq!(fresh.case_data(), proof.case_data());
    assert_eq!(fresh.removal_data(), proof.removal_data());
}

#[test]
fn removed_reply_requires_verified_dependencies_and_never_admits_a_delete() {
    use crate::chat::owner_bodies::{
        ExactReviewDecisionBody, ExactReviewOriginal, ReviewDecision, ReviewDecisionBody,
        ReviewSpec, ReviewStreamEntry, ReviewStreamPosition, ReviewStreamRecord,
        UnsignedReviewStreamEntry,
    };
    use crate::chat::owner_facts::OwnerFactId;
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[109; 32]);
    let other = DeviceKeypair::from_seed(&[110; 32]);
    let first = f.add(&key, 20);
    let parent_author = f.add(&other, 22);
    let parent = create(&other, &parent_author, 1);
    let reply = action(
        &key,
        &first,
        2,
        Body::Create {
            text: "A withheld reply".into(),
            reply_to: Some(parent.id()),
        },
        vec![parent.id()],
    );
    let delete = action(
        &key,
        &first,
        3,
        Body::Delete {
            create: parent.id(),
        },
        vec![parent.id()],
    );
    let mut removed = f.remove(&first);
    assert!(removed.take_close_completion().unwrap().try_complete());
    let frozen = removed.take_frozen_data().unwrap();
    drop(removed);
    let (mut ledger, view) = f.ledger();
    let original = reply.encode();
    let mut forged = original.clone();
    *forged.last_mut().unwrap() ^= 1;
    assert!(matches!(
        f.verifier
            .verify_removed_create_for_review(&forged, &frozen, &ledger, &view, &[]),
        Err(ChatError::Signature)
    ));
    let waiting = f
        .verifier
        .verify_removed_create_for_review(&original, &frozen, &ledger, &view, &[])
        .unwrap();
    assert_eq!(waiting.waiting_for_reply_parent_data(), Some(parent.id()));
    let held_bytes = waiting.encode_record_data(&original).unwrap();
    let held = V2RemovedCreateRecord::decode(&held_bytes).unwrap();
    assert_eq!(
        f.verifier
            .verify_removed_create_record_for_audit(&held, &frozen)
            .unwrap(),
        reply
    );
    let incorrect_remove = held.removal_data().0;
    let mut wrong_held = held_bytes.to_vec();
    let offset = wrong_held
        .windows(incorrect_remove.as_bytes().len())
        .position(|window| window == incorrect_remove.as_bytes())
        .unwrap();
    wrong_held[offset] ^= 1;
    let wrong_held = V2RemovedCreateRecord::decode(&wrong_held).unwrap();
    assert!(matches!(
        f.verifier
            .verify_removed_create_record_for_audit(&wrong_held, &frozen),
        Err(ChatError::Scope)
    ));
    let (remove_header, remove_epoch) = waiting.removal_data();
    let exact = ExactReviewOriginal {
        digest: waiting.original_digest_data(),
        remove_header,
        remove_epoch,
    };
    let decide = |decision| {
        ExactReviewDecisionBody::new(
            ReviewDecisionBody::new(ReviewSpec {
                target: waiting.target_data().clone(),
                intent: [76; 16],
                decision,
                previous: None,
            })
            .unwrap(),
            exact,
        )
        .unwrap()
    };
    let approve = decide(ReviewDecision::Approve);
    let reject = decide(ReviewDecision::Reject);
    assert_eq!(
        approve.require_evidence_data(&waiting),
        Err(ChatError::Dependency)
    );
    reject.require_evidence_data(&waiting).unwrap();
    ledger.import(parent.clone()).unwrap();
    let unverified = f
        .verifier
        .verify_removed_create_for_review(&original, &frozen, &ledger, &view, &[])
        .unwrap();
    assert_eq!(unverified.case_data(), waiting.case_data());
    assert_eq!(
        unverified.waiting_for_reply_parent_data(),
        Some(parent.id())
    );
    assert_eq!(
        approve.require_evidence_data(&unverified),
        Err(ChatError::Dependency)
    );
    retain(&f, &mut ledger, &parent, &parent_author, &[]);
    let ready = f
        .verifier
        .verify_removed_create_for_review(&original, &frozen, &ledger, &view, &[])
        .unwrap();
    assert_eq!(ready.waiting_for_reply_parent_data(), None);
    approve.require_evidence_data(&ready).unwrap();
    let record = ready.encode_record_data(&original).unwrap();
    approve
        .require_record_data(&V2RemovedCreateRecord::decode(&record).unwrap())
        .unwrap();
    let alternate_action = reviewed_create::alternate_withheld_reply(&reply);
    let alternate = alternate_action.encode();
    let alternate_evidence = f
        .verifier
        .verify_removed_create_for_review(&alternate, &frozen, &ledger, &view, &[])
        .unwrap();
    assert_eq!(alternate_evidence.case_data(), ready.case_data());
    assert_eq!(alternate_evidence.waiting_for_reply_parent_data(), None);
    assert_eq!(
        approve.require_evidence_data(&alternate_evidence),
        Err(ChatError::Scope)
    );
    let other_record = alternate_evidence.encode_record_data(&alternate).unwrap();
    assert_eq!(
        approve.require_record_data(&V2RemovedCreateRecord::decode(&other_record).unwrap()),
        Err(ChatError::Scope)
    );
    // The journal frame pins the same exact representation. The actual
    // founder/selection custody is separate from this data format test.
    let profile = f.verifier.replay_data().profile();
    let trusted = FounderGenesisAnchorData {
        group: profile.spec().group.clone(),
        founder: profile.spec().founder,
        profile: profile.id(),
        admission: OwnerFactId::from_bytes([43; 32]),
        tenure: profile.tenure(),
        header: OwnerHeaderId::from_bytes([44; 32]),
        state: f.verifier.replay_data().header().spec().state,
    };
    let entry = ReviewStreamEntry::sign_using(
        &trusted,
        ReviewStreamPosition {
            sequence: 1,
            previous: None,
            membership_header: f.verifier.replay_data().header().id(),
            membership_epoch: remove_epoch,
        },
        &approve,
        |bytes| Ok(f.owner.key.sign(bytes)),
    )
    .unwrap();
    let packed = ReviewStreamRecord::encode(&trusted, &entry, &approve, &record).unwrap();
    let reopened = ReviewStreamRecord::decode(&trusted, &packed).unwrap();
    assert_eq!(reopened.entry_data(), &entry);
    assert_eq!(reopened.decision_data(), &approve);
    assert_eq!(reopened.held_data().original_bytes_data(), &original);
    assert!(matches!(
        ReviewStreamRecord::decode_public_approval(&trusted, entry.id(), &packed),
        Err(ChatError::Dependency)
    ));
    let position = entry.position_data();
    let public_signed = |body| {
        let prepared = UnsignedReviewStreamEntry::prepare_public(&trusted, position, body).unwrap();
        let signature = f.owner.key.sign(prepared.signing_bytes());
        prepared.attach_signature(&trusted, signature).unwrap()
    };
    let public_approval = public_signed(&approve);
    let public_frame =
        ReviewStreamRecord::encode(&trusted, &public_approval, &approve, &record).unwrap();
    let public =
        ReviewStreamRecord::decode_public_approval(&trusted, public_approval.id(), &public_frame)
            .unwrap();
    assert_eq!(public.held_data().original_bytes_data(), &original);
    assert!(matches!(
        ReviewStreamRecord::decode_public_approval(&trusted, entry.id(), &public_frame),
        Err(ChatError::Scope)
    ));
    let public_rejection = public_signed(&reject);
    let rejected_frame =
        ReviewStreamRecord::encode(&trusted, &public_rejection, &reject, &record).unwrap();
    assert!(matches!(
        ReviewStreamRecord::decode_public_approval(
            &trusted,
            public_rejection.id(),
            &rejected_frame
        ),
        Err(ChatError::Scope)
    ));
    assert!(matches!(
        ReviewStreamRecord::encode(&trusted, &entry, &approve, &other_record),
        Err(ChatError::Scope)
    ));
    // Splice an alternate genuine record on the storage boundary. Do not
    // use the checked encoder to construct the cold reader's bad input.
    let prefix = packed.len() - record.len() - 4;
    let mut crossed = packed[..prefix].to_vec();
    crossed.extend_from_slice(&(other_record.len() as u32).to_be_bytes());
    crossed.extend_from_slice(&other_record);
    assert!(matches!(
        ReviewStreamRecord::decode(&trusted, &crossed),
        Err(ChatError::Scope)
    ));
    let wrong_pin = FounderGenesisAnchorData {
        group: trusted.group.clone(),
        header: OwnerHeaderId::from_bytes([45; 32]),
        ..trusted
    };
    assert!(ReviewStreamRecord::decode(&wrong_pin, &packed).is_err());
    let mut extra = packed.to_vec();
    extra.push(0);
    assert!(ReviewStreamRecord::decode(&trusted, &extra).is_err());
    assert_eq!(ready.case_data(), waiting.case_data());
    assert_eq!(ready.original_digest_data(), waiting.original_digest_data());
    assert_eq!(
        *ready.encode_record_data(&original).unwrap(),
        *waiting.encode_record_data(&original).unwrap(),
        "input order must not alter the stored frame"
    );
    assert!(matches!(
        f.verifier
            .verify_removed_create_for_review(&delete.encode(), &frozen, &ledger, &view, &[]),
        Err(ChatError::Permission)
    ));
    assert_eq!(ledger.action_ids().collect::<Vec<_>>(), vec![parent.id()]);

    let edit = action(
        &other,
        &parent_author,
        4,
        Body::Edit {
            create: parent.id(),
            text: "a valid edit, never a reply parent".into(),
        },
        vec![parent.id()],
    );
    let reply_to_edit = action(
        &key,
        &first,
        5,
        Body::Create {
            text: "a reply to the wrong action kind".into(),
            reply_to: Some(edit.id()),
        },
        vec![edit.id()],
    );
    retain(
        &f,
        &mut ledger,
        &edit,
        &parent_author,
        &[V2OrdinaryDependency::new(&parent, &parent_author)],
    );
    assert!(matches!(
        f.verifier.verify_removed_create_for_review(
            &reply_to_edit.encode(),
            &frozen,
            &ledger,
            &view,
            &[]
        ),
        Err(ChatError::Dependency)
    ));
    assert!(ledger.action(reply_to_edit.id()).is_none());
}
