//! Genuine production verifier/issuer tests with independent device keys. HOST trust and
//! selected save expectations are explicit test selections, not proof of App/MLS/network
//! integration. No cfg(test) grant/witness issuer supplies any positive evidence here.
use super::*;

mod admission_prepare;
mod read_accessors;
mod review_regressions;
mod selected_history;
mod signing;
use crate::chat::owner_bodies::{AcceptanceSpec, AppendBody, AppendSpec, RemovalBody, RemovalSpec};
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
        let body = RemovalBody::new(RemovalSpec {
            group: GROUP.to_vec(),
            subject: admission.core.body.spec().subject,
            commit: [199; 32],
            before_epoch: self.epoch(),
            after_epoch: self.epoch() + 1,
            tip_header: slot.tip_header,
            frozen_tip: slot.frontier.tip(),
        })
        .unwrap();
        let slots = base
            .slots()
            .filter(|s| s.image.admission != admission.core.body.id())
            .map(|s| s.image)
            .collect();
        let image = self.image(HeaderKind::Removal, body.id(), self.epoch() + 1, slots);
        let header = self.owner.header(&image);
        self.advance(&header, ReplayBody::Removal(&body), &image)
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
    install(&f, &mut ledger, &view, &edit, &current, &dependencies);
    assert!(ledger.guarded_projection(live.id(),128).unwrap().with_publication(|p|matches!(p,GuardedProjection::Visible {selected,text,..} if *selected==edit.id() && text=="current member still writes")).unwrap());
    // A fresh current Join/reply may acknowledge a held Create as causal history, but its
    // token cannot open that Create. The reply is its own independently admitted message.
    let join = action(
        &current_key,
        &current,
        5,
        Body::Join { create: held.id() },
        vec![held.id()],
    );
    let deps = [V2OrdinaryDependency::new(&held, &removed)];
    retain(&f, &mut ledger, &join, &current, &deps);
    install(&f, &mut ledger, &view, &join, &current, &deps);
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
    install(&f, &mut ledger, &view, &reply, &current, &deps);
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
