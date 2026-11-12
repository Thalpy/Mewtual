//! Actual Core crypto/issuer path. The test HOST selects explicit trusted fixtures; these are
//! not evidence that App SavedB, native ingress or transport are integrated.
use super::*;
use crate::chat::acceptance_tree::Frontier;
use crate::chat::owner_bodies::{AcceptanceSpec, AppendSpec};
use crate::chat::owner_header::{HeaderKind, HeaderSpec};
use crate::chat::owner_profile::{AdmissionOrigin, AdmissionSpec, ProfileSpec};
use crate::chat::owner_state::{
    encode_state, virtual_genesis_state, CurrentAdmissionSlot, StateMeta,
};
use catcoms_crypto::DeviceKeypair;
use std::sync::atomic::AtomicUsize;

mod cold;

const GROUP: &[u8] = b"actual-v2-founder-projection-issuer";

#[derive(Debug)]
struct Negative {
    live: AtomicBool,
    // Set only immediately before publication to distinguish the preliminary check from
    // the check inside the actual group/view readers, without time or scheduling guesses.
    checks_left: AtomicUsize,
}
impl Negative {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            live: AtomicBool::new(true),
            checks_left: AtomicUsize::new(usize::MAX),
        })
    }
}
impl V2OriginNegative for Negative {
    fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
            && self
                .checks_left
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    if n == usize::MAX {
                        Some(n)
                    } else {
                        n.checked_sub(1)
                    }
                })
                .is_ok()
    }
}

struct Fixture {
    key: DeviceKeypair,
    founder: AdmissionBody,
    header: SignedOwnerHeader,
    image: Vec<u8>,
    profile: SignedOwnerProfile,
    group: ProjectionGroup,
    view: ProjectionGroupView,
    negative: Arc<Negative>,
    origin: V2ProjectionOrigin,
}
impl Fixture {
    fn new() -> Self {
        let key = DeviceKeypair::from_seed(&[67; 32]);
        let profile = SignedOwnerProfile::sign_using(
            ProfileSpec {
                group: GROUP.to_vec(),
                founder: key.verifying_key().to_bytes(),
                creation: [68; 32],
            },
            |bytes| Ok(key.sign(bytes)),
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
        let header = SignedOwnerHeader::sign_using(
            HeaderSpec {
                group: GROUP.to_vec(),
                profile: profile.id(),
                issuer: profile.spec().founder,
                tenure: profile.tenure(),
                parent: profile.genesis_parent(),
                sequence: 1,
                kind: HeaderKind::Admission,
                body: founder.id(),
                state: crate::chat::owner_state::StateFrame::decode(&image)
                    .unwrap()
                    .commitment(),
            },
            |bytes| Ok(key.sign(bytes)),
        )
        .unwrap();
        let mut group = ProjectionGroup::new(GROUP).unwrap();
        let mut view = group.begin_view();
        assert!(view.try_ready());
        let negative = Negative::new();
        let origin = V2ProjectionOrigin::new(negative.clone());
        Self {
            key,
            founder,
            header,
            image,
            profile,
            group,
            view,
            negative,
            origin,
        }
    }
    fn anchor(&self) -> FounderGenesisAnchorData {
        FounderGenesisAnchorData {
            group: GROUP.to_vec(),
            founder: self.profile.spec().founder,
            profile: self.profile.id(),
            admission: self.founder.admission_id(),
            tenure: self.profile.tenure(),
            header: self.header.id(),
            state: self.header.spec().state,
        }
    }
    fn verifier(&self) -> V2FounderPrefixVerifier {
        V2FounderPrefixVerifier::from_genesis(
            V2FounderExpectation::from_trusted_host(self.anchor()).unwrap(),
            &self.profile,
            &self.founder,
            &self.header,
            &self.image,
            &self.view,
            self.origin.clone(),
        )
        .unwrap()
    }
    fn action(&self, nonce: u8, body: Body, parents: Vec<ActionId>) -> SignedAction {
        let realm = Realm::new(GROUP, 9).unwrap();
        let labels = self.founder.ordinary_labels(&realm).unwrap();
        SignedAction::sign(
            &self.key,
            ActionSpec {
                realm,
                basis: labels.basis,
                grant: labels.grant,
                nonce: [nonce; 16],
                presented_ms: u64::from(nonce),
                parents,
                body,
            },
        )
        .unwrap()
    }
    fn create(&self, nonce: u8) -> SignedAction {
        self.action(
            nonce,
            Body::Create {
                text: format!("body {nonce}"),
                reply_to: None,
            },
            vec![],
        )
    }
    fn record(&self, action: &SignedAction) -> AcceptanceRecord {
        AcceptanceRecord::new(AcceptanceSpec {
            group: GROUP.to_vec(),
            context: TreeContext {
                profile: self.profile.id(),
                admission: self.founder.admission_id(),
            },
            channel: action.spec().realm.channel(),
            action: action.id(),
            subject: *action.signer(),
            grant: action.spec().grant,
            basis: action.spec().basis,
            create: action.spec().body.create(),
        })
        .unwrap()
    }
    fn append(
        &self,
        verifier: &V2FounderPrefixVerifier,
        records: &[AcceptanceRecord],
    ) -> V2VerifiedAppendCandidate {
        let base = verifier.replay_data();
        let slot = base.slot(self.founder.id()).unwrap();
        let mut leaves: Vec<_> = records.iter().map(AcceptanceRecord::leaf).collect();
        leaves.sort();
        let appended = slot.frontier.prepare_append(&leaves).unwrap();
        let body = AppendBody::new(AppendSpec {
            group: GROUP.to_vec(),
            previous_tip: slot.tip_header,
            old_tip: slot.frontier.tip(),
            new_tip: appended.new_tip(),
            leaves,
        })
        .unwrap();
        let plan = base.prepare_append_data(&body).unwrap();
        let header = SignedOwnerHeader::sign_using(plan.header_spec_data().clone(), |b| {
            Ok(self.key.sign(b))
        })
        .unwrap();
        verifier
            .derive_verified_append(&header, &body, plan.image_data())
            .unwrap()
    }
    fn select(&self, verifier: &mut V2FounderPrefixVerifier, candidate: V2VerifiedAppendCandidate) {
        let selected = V2HostSelectionExpectation::from_trusted_host(
            candidate.header_data().id(),
            candidate.header_data().spec().state,
        );
        let mut candidate = Some(candidate);
        let residual = verifier
            .advance_host_selected(&selected, &mut candidate)
            .unwrap();
        assert!(candidate.is_none());
        assert!(residual.append_data().is_some());
        drop(residual);
    }
    fn ledger(&self) -> (ChatLedger, ProjectionView) {
        let mut ledger = ChatLedger::new(
            Realm::new(GROUP, 9).unwrap(),
            LedgerLimits {
                max_actions: 128,
                max_action_bytes: 1024 * 1024,
            },
        )
        .unwrap();
        let mut view = ledger.begin_projection_view_in(&self.view).unwrap();
        assert!(view.try_ready());
        (ledger, view)
    }
    fn retain(
        &self,
        ledger: &mut ChatLedger,
        verifier: &V2FounderPrefixVerifier,
        action: &SignedAction,
    ) {
        ledger.import(action.clone()).unwrap();
        ledger
            .authorize(
                action.id(),
                verifier.verify_create_semantics(action).unwrap(),
            )
            .unwrap();
        while ledger.advance(128).remaining {}
        assert_eq!(ledger.state(action.id()), Some(ActionState::Valid));
    }
}

#[test]
fn trusted_genesis_is_complete_and_each_actual_signature_is_checked() {
    let f = Fixture::new();
    for field in 0..7 {
        let mut a = f.anchor();
        match field {
            0 => a.founder = [0; 32],
            1 => a.profile = OwnerFactId::from_bytes([0; 32]),
            2 => a.admission = OwnerFactId::from_bytes([0; 32]),
            3 => a.tenure = OwnerTenureId::from_bytes([0; 32]),
            4 => a.header = OwnerHeaderId::from_bytes([0; 32]),
            5 => a.state = OwnerStateCommitment::from_bytes([0; 32]),
            _ => a.group = b"another group".to_vec(),
        }
        assert!(V2FounderPrefixVerifier::from_genesis(
            V2FounderExpectation::from_trusted_host(a).unwrap(),
            &f.profile,
            &f.founder,
            &f.header,
            &f.image,
            &f.view,
            f.origin.clone()
        )
        .is_err());
    }
    let mut raw = f.header.encode();
    *raw.last_mut().unwrap() ^= 1;
    let invalid = SignedOwnerHeader::decode(&raw).unwrap();
    assert_eq!(invalid.id(), f.header.id());
    assert!(V2FounderPrefixVerifier::from_genesis(
        V2FounderExpectation::from_trusted_host(f.anchor()).unwrap(),
        &f.profile,
        &f.founder,
        &invalid,
        &f.image,
        &f.view,
        f.origin.clone()
    )
    .is_err());
    let mut image = f.image.clone();
    *image.last_mut().unwrap() ^= 1;
    assert!(V2FounderPrefixVerifier::from_genesis(
        V2FounderExpectation::from_trusted_host(f.anchor()).unwrap(),
        &f.profile,
        &f.founder,
        &f.header,
        &image,
        &f.view,
        f.origin.clone()
    )
    .is_err());
}

#[test]
fn semantic_witness_is_exact_create_only_and_does_not_unhold_projection() {
    let f = Fixture::new();
    let v = f.verifier();
    let a = f.create(1);
    let witness = v.verify_create_semantics(&a).unwrap();
    assert_eq!(witness.rights(), Rights::CREATE);
    assert!(!witness.matches(&f.create(2)));
    let (mut ledger, _view) = f.ledger();
    f.retain(&mut ledger, &v, &a);
    assert!(matches!(
        ledger
            .guarded_projection(a.id(), 100)
            .unwrap()
            .with_publication(Clone::clone)
            .unwrap(),
        GuardedProjection::Held { .. }
    ));
    for action in [
        f.action(
            3,
            Body::Edit {
                create: a.id(),
                text: "edit".into(),
            },
            vec![a.id()],
        ),
        f.action(4, Body::Delete { create: a.id() }, vec![a.id()]),
        f.action(
            5,
            Body::Create {
                text: "reply".into(),
                reply_to: Some(a.id()),
            },
            vec![a.id()],
        ),
        f.action(6, Body::Join { create: a.id() }, vec![a.id()]),
    ] {
        assert!(v.verify_create_semantics(&action).is_err());
    }
}

#[test]
fn unselected_candidate_does_not_grant_acceptance_and_wrong_selection_preserves_it() {
    let f = Fixture::new();
    let mut v = f.verifier();
    let a = f.create(1);
    let r = f.record(&a);
    let (_ledger, view) = f.ledger();
    let mut candidate = Some(f.append(&v, std::slice::from_ref(&r)));
    assert!(v.accepted_create(&a, &r, 0, &[], &view).is_err());
    let original = candidate.as_ref().unwrap().header_data().id();
    let wrong = V2HostSelectionExpectation::from_trusted_host(
        OwnerHeaderId::from_bytes([0; 32]),
        candidate.as_ref().unwrap().header_data().spec().state,
    );
    assert!(v.advance_host_selected(&wrong, &mut candidate).is_err());
    assert_eq!(candidate.as_ref().unwrap().header_data().id(), original);
    f.select(&mut v, candidate.take().unwrap());
    assert!(v.accepted_create(&a, &r, 0, &[], &view).is_ok());
}

#[test]
fn candidates_cannot_cross_verifiers_or_replay_over_an_advanced_parent() {
    let f = Fixture::new();
    let mut a = f.verifier();
    let mut b = f.verifier();
    let r = f.record(&f.create(1));
    let mut candidate = Some(f.append(&a, std::slice::from_ref(&r)));
    let selected = V2HostSelectionExpectation::from_trusted_host(
        candidate.as_ref().unwrap().header_data().id(),
        candidate.as_ref().unwrap().header_data().spec().state,
    );
    assert!(b.advance_host_selected(&selected, &mut candidate).is_err());
    assert!(candidate.is_some());
    let stale = f.append(&a, std::slice::from_ref(&r));
    f.select(&mut a, candidate.take().unwrap());
    candidate = Some(stale);
    assert!(a.advance_host_selected(&selected, &mut candidate).is_err());
    assert!(candidate.is_some());
}

#[test]
fn acceptance_checks_real_context_exact_ordinal_signature_and_original_group() {
    let f = Fixture::new();
    let mut v = f.verifier();
    let a = f.create(1);
    let r = f.record(&a);
    let (_ledger, view) = f.ledger();
    let candidate = f.append(&v, std::slice::from_ref(&r));
    f.select(&mut v, candidate);
    assert!(v.accepted_create(&a, &r, 0, &[], &view).is_ok());
    assert!(v.accepted_create(&a, &r, 1, &[], &view).is_err());
    assert!(v
        .accepted_create(&a, &r, 0, &[TreeHash::from_bytes([0; 32])], &view)
        .is_err());
    let mut wrong_context = r.spec().clone();
    wrong_context.context.admission = OwnerFactId::from_bytes([0; 32]);
    let wrong_context = AcceptanceRecord::new(wrong_context).unwrap();
    assert!(
        wrong_context.matches_action(&a).is_ok(),
        "action binding alone intentionally cannot authenticate tree context"
    );
    assert!(v
        .accepted_create(&a, &wrong_context, 0, &[], &view)
        .is_err());
    let mut raw = a.encode();
    *raw.last_mut().unwrap() ^= 1;
    let invalid = SignedAction::decode(&raw).unwrap();
    assert_eq!(invalid.id(), a.id());
    assert!(v.accepted_create(&invalid, &r, 0, &[], &view).is_err());
    let mut other_group = ProjectionGroup::new(GROUP).unwrap();
    let other = other_group.begin_view();
    let mut ledger = ChatLedger::new(
        a.spec().realm.clone(),
        LedgerLimits {
            max_actions: 8,
            max_action_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    let other_channel = ledger.begin_projection_view_in(&other).unwrap();
    assert!(v.accepted_create(&a, &r, 0, &[], &other_channel).is_err());
    let unbound = ledger.begin_projection_view();
    assert!(v.accepted_create(&a, &r, 0, &[], &unbound).is_err());
}

#[test]
fn source_negative_revokes_both_uninstalled_token_and_retained_plaintext_result() {
    let f = Fixture::new();
    let mut v = f.verifier();
    let a = f.create(1);
    let r = f.record(&a);
    let (mut ledger, view) = f.ledger();
    f.retain(&mut ledger, &v, &a);
    let candidate = f.append(&v, std::slice::from_ref(&r));
    f.select(&mut v, candidate);
    let token = v.accepted_create(&a, &r, 0, &[], &view).unwrap();
    ledger.admit_projection(&token, 100).unwrap();
    let result = ledger.guarded_projection(a.id(), 100).unwrap();
    assert!(matches!(
        result.with_publication(Clone::clone).unwrap(),
        GuardedProjection::Visible {
            certainty: ProjectionCertainty::Proven,
            ..
        }
    ));
    f.negative.live.store(false, Ordering::Release);
    assert!(
        view.life.live(),
        "source cancellation is independent of channel labels"
    );
    assert!(result
        .with_publication(|_| panic!("stale plaintext escaped"))
        .is_err());
    assert!(ledger.admit_projection(&token, 100).is_err());
    f.negative.live.store(true, Ordering::Release);
    assert!(result
        .with_publication(|_| panic!("a later observation revived the original source"))
        .is_err());
    assert!(ledger.admit_projection(&token, 100).is_err());
    assert_eq!(
        ledger.state(a.id()),
        Some(ActionState::Valid),
        "historical semantics remain additive"
    );
}

#[test]
fn source_negative_is_checked_inside_actual_reader_not_only_before_it() {
    let f = Fixture::new();
    let mut v = f.verifier();
    let a = f.create(1);
    let r = f.record(&a);
    let (mut ledger, view) = f.ledger();
    f.retain(&mut ledger, &v, &a);
    let candidate = f.append(&v, std::slice::from_ref(&r));
    f.select(&mut v, candidate);
    ledger
        .admit_projection(&v.accepted_create(&a, &r, 0, &[], &view).unwrap(), 100)
        .unwrap();
    let result = ledger.guarded_projection(a.id(), 100).unwrap();
    f.negative.checks_left.store(1, Ordering::Release);
    assert!(result
        .with_publication(|_| panic!("negative won between outer and inner checks"))
        .is_err());
}

#[test]
fn normal_append_keeps_original_proven_history_supported_without_new_source_identity() {
    let f = Fixture::new();
    let mut v = f.verifier();
    let a = f.create(1);
    let r = f.record(&a);
    let (mut ledger, view) = f.ledger();
    f.retain(&mut ledger, &v, &a);
    let candidate = f.append(&v, std::slice::from_ref(&r));
    f.select(&mut v, candidate);
    ledger
        .admit_projection(&v.accepted_create(&a, &r, 0, &[], &view).unwrap(), 100)
        .unwrap();
    let result = ledger.guarded_projection(a.id(), 100).unwrap();
    let b = f.create(2);
    let second = f.append(&v, &[f.record(&b)]);
    f.select(&mut v, second);
    assert!(result.with_publication(|_| ()).is_ok());
    let sibling = v.context().leaf_hash(f.record(&b).leaf());
    let refreshed = v.accepted_create(&a, &r, 0, &[sibling], &view).unwrap();
    ledger.admit_projection(&refreshed, 100).unwrap();
    assert!(result.with_publication(|_| ()).is_ok(), "an unchanged accepted effect must not invalidate the original read just because its supporting tip advanced");
    f.origin.cancel();
    assert!(result
        .with_publication(|_| panic!("cancelled original source escaped"))
        .is_err());
}

#[test]
fn admitted_callback_finishes_but_group_close_waits_for_its_actual_reader() {
    use std::sync::mpsc;
    use std::time::Duration;
    let f = Fixture::new();
    let mut v = f.verifier();
    let a = f.create(1);
    let r = f.record(&a);
    let (mut ledger, view) = f.ledger();
    f.retain(&mut ledger, &v, &a);
    let candidate = f.append(&v, std::slice::from_ref(&r));
    f.select(&mut v, candidate);
    ledger
        .admit_projection(&v.accepted_create(&a, &r, 0, &[], &view).unwrap(), 100)
        .unwrap();
    let result = ledger.guarded_projection(a.id(), 100).unwrap();
    let (reached, receiver) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        result.with_publication(|_| {
            reached.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            42
        })
    });
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    f.origin.cancel();
    let mut close = f.view.revoker().close();
    assert!(
        !close.try_complete(),
        "cancel is not a completed publication drain"
    );
    release.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), Ok(42));
    assert!(close.try_complete());
}

#[test]
fn independent_genesis_replay_has_new_cursor_identity_and_observes_original_group_drop() {
    let f = Fixture::new();
    let mut hot = f.verifier();
    let a = f.create(1);
    let r = f.record(&a);
    let candidate = f.append(&hot, std::slice::from_ref(&r));
    let header = candidate.header_data().clone();
    let image = candidate.image_data().to_vec();
    f.select(&mut hot, candidate);
    let slot = Frontier::empty(hot.context());
    let prepared = slot.prepare_append(&[r.leaf()]).unwrap();
    let body = AppendBody::new(AppendSpec {
        group: GROUP.to_vec(),
        previous_tip: f.header.id(),
        old_tip: slot.tip(),
        new_tip: prepared.new_tip(),
        leaves: vec![r.leaf()],
    })
    .unwrap();
    let mut cold = f.verifier();
    let mut candidate = Some(cold.derive_verified_append(&header, &body, &image).unwrap());
    let selected = V2HostSelectionExpectation::from_trusted_host(header.id(), header.spec().state);
    cold.advance_host_selected(&selected, &mut candidate)
        .unwrap();
    assert_eq!(hot.replay_data().image(), cold.replay_data().image());
    assert!(!Arc::ptr_eq(&hot.identity, &cold.identity));
    assert_eq!(format!("{cold:?}"), "V2FounderPrefixVerifier { .. }");
    // Retaining this domain owner is intentional: dropping it, not equality of labels,
    // controls actual group lifetime even when both verified replay cursors still exist.
    drop(f.group);
    assert!(cold.verify_create_semantics(&a).is_err());
}
