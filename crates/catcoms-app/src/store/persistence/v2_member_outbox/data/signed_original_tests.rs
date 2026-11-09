//! `MemberSignedOriginal::from_completed_plan` over the REAL Core general verifier and plan
//! issuer, with an independent device key. The HOST expectation below is an explicit test
//! trust selection; no fixture-authority constructor issues the Plan.
use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::acceptance_tree::TreeContext;
use catcoms_replication::chat::owner_header::{HeaderKind, HeaderSpec, SignedOwnerHeader};
use catcoms_replication::chat::owner_profile::{
    AdmissionBody, AdmissionOrigin, AdmissionSpec, ProfileSpec, SignedOwnerProfile,
};
use catcoms_replication::chat::owner_state::{
    encode_state, virtual_genesis_state, CurrentAdmissionSlot, StateFrame, StateMeta,
};
use catcoms_replication::chat::{
    ChatLedger, FounderGenesisAnchorData, LedgerLimits, ProjectionGroup, ProjectionGroupView,
    ProjectionView, Realm, V2AuthenticatedAdmission, V2GeneralExpectation, V2GeneralPrefixVerifier,
    V2OriginNegative, V2ProjectionOrigin, MAX_ACTION_BYTES,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const GROUP: &[u8] = b"member-signed-original-constructor";

#[derive(Debug)]
struct Live(AtomicBool);
impl V2OriginNegative for Live {
    fn is_live(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

// Drop order: view/ledger, verifier (closes its group view), then the group owners.
struct Fixture {
    view: ProjectionView,
    _ledger: ChatLedger,
    verifier: V2GeneralPrefixVerifier,
    founder: V2AuthenticatedAdmission,
    live: Arc<Live>,
    _group_view: ProjectionGroupView,
    _group: ProjectionGroup,
    key: DeviceKeypair,
}

fn fixture() -> Fixture {
    let key = DeviceKeypair::from_seed(&[61; 32]);
    let founder_key = key.verifying_key().to_bytes();
    let creation = [62; 32];
    let profile = SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: GROUP.to_vec(),
            founder: founder_key,
            creation,
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    let admission = AdmissionBody::new(AdmissionSpec {
        group: GROUP.to_vec(),
        profile: profile.id(),
        subject: founder_key,
        origin: AdmissionOrigin::Founder { creation },
    })
    .unwrap();
    let context = TreeContext {
        profile: profile.id(),
        admission: admission.admission_id(),
    };
    let image = encode_state(
        StateMeta {
            group: GROUP,
            profile: profile.id(),
            issuer: founder_key,
            tenure: profile.tenure(),
            parent: profile.genesis_parent(),
            previous: virtual_genesis_state(&profile).unwrap(),
            sequence: 1,
            kind: HeaderKind::Admission,
            body: admission.id(),
            mls_epoch: 0,
        },
        &[CurrentAdmissionSlot {
            admission: admission.id(),
            subject: founder_key,
            tip_body: admission.id(),
            accepted_count: 0,
            accepted_root: context.empty_hash(),
        }],
    )
    .unwrap();
    let state = StateFrame::decode(&image).unwrap().commitment();
    let header = SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: GROUP.to_vec(),
            profile: profile.id(),
            issuer: founder_key,
            tenure: profile.tenure(),
            parent: profile.genesis_parent(),
            sequence: 1,
            kind: HeaderKind::Admission,
            body: admission.id(),
            state,
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    let expectation = V2GeneralExpectation::from_trusted_host(FounderGenesisAnchorData {
        group: GROUP.to_vec(),
        founder: founder_key,
        profile: profile.id(),
        admission: admission.admission_id(),
        tenure: profile.tenure(),
        header: header.id(),
        state,
    })
    .unwrap();
    let mut group = ProjectionGroup::new(GROUP).unwrap();
    let mut group_view = group.begin_view();
    assert!(group_view.try_ready());
    let live = Arc::new(Live(AtomicBool::new(true)));
    let verifier = V2GeneralPrefixVerifier::from_genesis(
        expectation,
        &profile,
        &admission,
        &header,
        &image,
        &group_view,
        V2ProjectionOrigin::new(live.clone()),
    )
    .unwrap();
    let founder = verifier.admission(admission.admission_id()).unwrap();
    let mut ledger = ChatLedger::new(
        Realm::new(GROUP, 7).unwrap(),
        LedgerLimits {
            max_actions: 4,
            max_action_bytes: 4 * MAX_ACTION_BYTES,
        },
    )
    .unwrap();
    let mut view = ledger.begin_projection_view_in(&group_view).unwrap();
    assert!(view.try_ready());
    Fixture {
        view,
        _ledger: ledger,
        verifier,
        founder,
        live,
        _group_view: group_view,
        _group: group,
        key,
    }
}

fn draft(f: &Fixture, nonce: u8) -> ActionDraft {
    let realm = Realm::new(GROUP, 7).unwrap();
    let labels = f.founder.admission_data().ordinary_labels(&realm).unwrap();
    ActionDraft::new(
        ActionSpec {
            realm,
            basis: labels.basis,
            grant: labels.grant,
            nonce: [nonce; 16],
            presented_ms: 1_000 + nonce as u64,
            parents: Vec::new(),
            body: Body::Create {
                text: format!("constructor original {nonce}"),
                reply_to: None,
            },
        },
        f.key.verifying_key().to_bytes(),
    )
    .unwrap()
}

fn signed_plan(
    f: &Fixture,
    draft: &ActionDraft,
) -> (
    V2OrdinarySigningPlan,
    OriginalV2OrdinarySigningPlanObservation,
) {
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(draft, &f.founder, &[], &f.view)
        .unwrap();
    let original = plan.observe_original();
    let call = plan
        .admit_signing(draft, &f.key.verifying_key().to_bytes())
        .unwrap()
        .begin_key()
        .unwrap();
    drop(call.cache_raw64(f.key.sign(draft.signing_bytes_data())));
    (plan, original)
}

#[test]
fn completed_plan_becomes_the_exact_signed_original_under_its_historical_admission() {
    let f = fixture();
    let draft = draft(&f, 1);
    let (plan, original) = signed_plan(&f, &draft);
    let expected = plan.complete_cached_raw64_data(&original, &draft).unwrap();
    let custody = MemberSignedOriginal::from_completed_plan(&plan, &original, &draft).unwrap();
    assert!(custody.action().verify());
    assert!(custody.action().id() == expected.id());
    assert_eq!(custody.action().encode(), expected.encode());
    assert!(custody.admission() == plan.admission_id_data());
    assert!(custody.admission() == f.founder.admission_data().admission_id());
}

#[test]
fn another_plans_observation_or_draft_is_refused() {
    let f = fixture();
    let first = draft(&f, 2);
    let second = draft(&f, 3);
    let (plan, original) = signed_plan(&f, &first);
    let (other_plan, other_original) = signed_plan(&f, &second);
    assert!(MemberSignedOriginal::from_completed_plan(&plan, &other_original, &first).is_err());
    assert!(MemberSignedOriginal::from_completed_plan(&plan, &original, &second).is_err());
    assert!(MemberSignedOriginal::from_completed_plan(&other_plan, &original, &first).is_err());
    // The matching triples still complete independently.
    assert!(MemberSignedOriginal::from_completed_plan(&plan, &original, &first).is_ok());
    assert!(
        MemberSignedOriginal::from_completed_plan(&other_plan, &other_original, &second).is_ok()
    );
}

#[test]
fn a_plan_without_a_cached_raw_is_a_dependency_refusal() {
    let f = fixture();
    let draft = draft(&f, 4);
    let plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &f.founder, &[], &f.view)
        .unwrap();
    let original = plan.observe_original();
    assert!(matches!(
        MemberSignedOriginal::from_completed_plan(&plan, &original, &draft),
        Err(ChatError::Dependency)
    ));
}

#[test]
fn after_stop_completion_still_returns_the_same_original_as_data() {
    let f = fixture();
    let draft = draft(&f, 5);
    let (mut plan, original) = signed_plan(&f, &draft);
    let before = MemberSignedOriginal::from_completed_plan(&plan, &original, &draft).unwrap();
    f.live.0.store(false, Ordering::Release);
    // Stop refuses every fresh admission...
    assert!(plan
        .admit_signing(&draft, &f.key.verifying_key().to_bytes())
        .is_err());
    // ...while the cached original is still exact DATA under its historical Admission.
    let after = MemberSignedOriginal::from_completed_plan(&plan, &original, &draft).unwrap();
    assert_eq!(after.action().encode(), before.action().encode());
    assert!(after.admission() == before.admission());
}
