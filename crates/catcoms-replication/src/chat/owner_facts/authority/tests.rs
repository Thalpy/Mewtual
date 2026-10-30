//! Genuine signed owner facts exercise the verifier. Synthetic Add references here test
//! binding only; actual MLS producer/source coverage lives in the sync owner-evidence tests.
use super::*;
use crate::chat::{
    ActionSpec, ActionState, Body, ChatLedger, ChatSnapshot, LedgerLimits, PendingReason,
    SignedAction,
};
use catcoms_crypto::DeviceKeypair;

struct Chain {
    admission: SignedOwnerFact,
    grant: SignedOwnerFact,
    basis: SignedOwnerFact,
}
impl Chain {
    fn refs(&self) -> OwnerBasisFacts<'_> {
        OwnerBasisFacts {
            admission: &self.admission,
            grant: &self.grant,
            basis: &self.basis,
        }
    }
}
fn key(n: u8) -> DeviceKeypair {
    DeviceKeypair::from_seed(&[n; 32])
}
fn profile(root: &DeviceKeypair, group: &[u8], creation: [u8; 32]) -> SignedOwnerFact {
    SignedOwnerFact::sign_using(
        OwnerFactSpec {
            group: group.to_vec(),
            profile: None,
            body: OwnerFactBody::Profile {
                creation,
                founder: root.verifying_key().to_bytes(),
            },
        },
        root.verifying_key().to_bytes(),
        |bytes| Ok(root.sign(bytes)),
    )
    .unwrap()
}
fn sign(root: &DeviceKeypair, p: &SignedOwnerFact, body: OwnerFactBody) -> SignedOwnerFact {
    SignedOwnerFact::sign_using(
        OwnerFactSpec {
            group: p.spec().group.clone(),
            profile: Some(p.id()),
            body,
        },
        root.verifying_key().to_bytes(),
        |bytes| Ok(root.sign(bytes)),
    )
    .unwrap()
}
fn chain(
    root: &DeviceKeypair,
    p: &SignedOwnerFact,
    subject: [u8; 32],
    channel: u128,
    incarnation: u8,
    rights: Rights,
    mut covered: Vec<BasisId>,
) -> Chain {
    let admission = sign(
        root,
        p,
        OwnerFactBody::Admission {
            subject,
            predecessor: p.id(),
            origin: AdmissionOrigin::Add {
                commit: [incarnation; 32],
                before_epoch: u64::from(incarnation),
                after_epoch: u64::from(incarnation) + 1,
                key_package: [incarnation + 1; 32],
                credential: [incarnation + 2; 32],
            },
        },
    );
    let grant = sign(
        root,
        p,
        OwnerFactBody::RealmGrant {
            channel,
            admission: admission.id(),
            subject,
            rights,
        },
    );
    covered.sort();
    let basis = sign(
        root,
        p,
        OwnerFactBody::Basis {
            channel,
            admission: admission.id(),
            subject,
            grant: grant.grant_id().unwrap(),
            covered,
        },
    );
    Chain {
        admission,
        grant,
        basis,
    }
}
fn expectation(p: &SignedOwnerFact) -> OwnerProfileExpectation {
    OwnerProfileExpectation::from_trusted_host(&p.spec().group, *p.issuer(), p.id()).unwrap()
}
fn action(
    p: &SignedOwnerFact,
    c: &Chain,
    signer: &DeviceKeypair,
    body: Body,
    parents: Vec<crate::chat::ActionId>,
) -> SignedAction {
    let OwnerFactBody::Basis { channel, .. } = c.basis.spec().body else {
        panic!("basis")
    };
    SignedAction::sign(
        signer,
        ActionSpec {
            realm: Realm::new(&p.spec().group, channel).unwrap(),
            basis: c.basis.basis_id().unwrap(),
            grant: c.grant.grant_id().unwrap(),
            nonce: [1; 16],
            presented_ms: 1_000,
            parents,
            body,
        },
    )
    .unwrap()
}
fn limits() -> LedgerLimits {
    LedgerLimits {
        max_actions: 64,
        max_action_bytes: 1024 * 1024,
    }
}
fn drain(ledger: &mut ChatLedger) {
    for _ in 0..1024 {
        if !ledger.advance(16).remaining {
            return;
        }
    }
    panic!("finite semantic work did not settle")
}

#[test]
fn owner_basis_original_signed_chain_restores_semantics_without_saved_authority() {
    let root = key(90);
    let member = key(91);
    let p = profile(&root, b"owner-basis-chain", [1; 32]);
    let c = chain(
        &root,
        &p,
        member.verifying_key().to_bytes(),
        9,
        1,
        Rights::MEMBER,
        vec![],
    );
    let expected = expectation(&p);
    let witness = expected.verify_basis(&p, c.refs(), &[]).unwrap();
    let a = action(
        &p,
        &c,
        &member,
        Body::Create {
            text: "original".into(),
            reply_to: None,
        },
        vec![],
    );
    let mut ledger = ChatLedger::new(a.spec().realm.clone(), limits()).unwrap();
    ledger.import(a.clone()).unwrap();
    ledger.authorize(a.id(), witness).unwrap();
    drain(&mut ledger);
    assert_eq!(ledger.state(a.id()), Some(ActionState::Valid));
    let bytes = ledger.snapshot().unwrap().encode();
    let snapshot = ChatSnapshot::decode(&bytes, limits()).unwrap();
    assert_eq!(snapshot.encode(), bytes);
    let mut restored = ChatLedger::restore(&bytes, limits(), &[]).unwrap();
    drain(&mut restored);
    assert_eq!(
        restored.state(a.id()),
        Some(ActionState::Pending(PendingReason::Authority))
    );
    restored
        .authorize(a.id(), expected.verify_basis(&p, c.refs(), &[]).unwrap())
        .unwrap();
    drain(&mut restored);
    assert_eq!(restored.state(a.id()), Some(ActionState::Valid));
    assert_eq!(restored.snapshot().unwrap().encode(), bytes);
}

#[test]
fn owner_basis_requires_exact_root_subject_channel_grant_rights_and_kp_incarnation() {
    let root = key(92);
    let member = key(93);
    let p = profile(&root, b"owner-basis-binding", [1; 32]);
    let a1 = chain(
        &root,
        &p,
        member.verifying_key().to_bytes(),
        9,
        1,
        Rights::MEMBER,
        vec![],
    );
    let a2 = chain(
        &root,
        &p,
        member.verifying_key().to_bytes(),
        9,
        2,
        Rights::MEMBER,
        vec![],
    );
    let expected = expectation(&p);
    assert_ne!(a1.admission.id(), a2.admission.id());
    assert_ne!(a1.grant.grant_id(), a2.grant.grant_id());
    assert_ne!(a1.basis.basis_id(), a2.basis.basis_id());
    assert_eq!(
        expected.verify_basis(
            &p,
            OwnerBasisFacts {
                admission: &a2.admission,
                ..a1.refs()
            },
            &[]
        ),
        Err(ChatError::Authority)
    );
    let old = action(
        &p,
        &a1,
        &member,
        Body::Create {
            text: "A1 original".into(),
            reply_to: None,
        },
        vec![],
    );
    let mut ledger = ChatLedger::new(old.spec().realm.clone(), limits()).unwrap();
    ledger.import(old.clone()).unwrap();
    assert_eq!(
        ledger.authorize(old.id(), expected.verify_basis(&p, a2.refs(), &[]).unwrap()),
        Err(ChatError::Authority)
    );
    let new_rights = sign(
        &root,
        &p,
        OwnerFactBody::RealmGrant {
            channel: 9,
            admission: a1.admission.id(),
            subject: member.verifying_key().to_bytes(),
            rights: Rights::MODERATOR,
        },
    );
    assert_eq!(
        expected.verify_basis(
            &p,
            OwnerBasisFacts {
                grant: &new_rights,
                ..a1.refs()
            },
            &[]
        ),
        Err(ChatError::Authority)
    );
    let wrong_subject = sign(
        &root,
        &p,
        OwnerFactBody::RealmGrant {
            channel: 9,
            admission: a1.admission.id(),
            subject: key(94).verifying_key().to_bytes(),
            rights: Rights::MEMBER,
        },
    );
    assert_eq!(
        expected.verify_basis(
            &p,
            OwnerBasisFacts {
                grant: &wrong_subject,
                ..a1.refs()
            },
            &[]
        ),
        Err(ChatError::Authority)
    );
    let wrong_channel = sign(
        &root,
        &p,
        OwnerFactBody::Basis {
            channel: 10,
            admission: a1.admission.id(),
            subject: member.verifying_key().to_bytes(),
            grant: a1.grant.grant_id().unwrap(),
            covered: vec![],
        },
    );
    assert_eq!(
        expected.verify_basis(
            &p,
            OwnerBasisFacts {
                basis: &wrong_channel,
                ..a1.refs()
            },
            &[]
        ),
        Err(ChatError::Authority)
    );
    let other_profile = profile(&root, b"owner-basis-binding", [2; 32]);
    assert_eq!(
        expected.verify_basis(&other_profile, a1.refs(), &[]),
        Err(ChatError::Scope)
    );
    let other_group = profile(&root, b"foreign-owner-basis", [1; 32]);
    assert_eq!(
        expected.verify_basis(&other_group, a1.refs(), &[]),
        Err(ChatError::Scope)
    );
    let bad_pin = OwnerProfileExpectation::from_trusted_host(
        &p.spec().group,
        key(95).verifying_key().to_bytes(),
        p.id(),
    )
    .unwrap();
    assert_eq!(
        bad_pin.verify_basis(&p, a1.refs(), &[]),
        Err(ChatError::Scope)
    );
    let mut broken = a1.basis.encode();
    *broken.last_mut().unwrap() ^= 1;
    let broken = SignedOwnerFact::decode(&broken).unwrap();
    assert_eq!(
        expected.verify_basis(
            &p,
            OwnerBasisFacts {
                basis: &broken,
                ..a1.refs()
            },
            &[]
        ),
        Err(ChatError::Signature)
    );
    // A failed candidate does not poison this immutable expected root or a later good proof.
    assert!(expected.verify_basis(&p, a1.refs(), &[]).is_ok());
}

#[test]
fn owner_basis_coverage_needs_exact_independent_same_profile_realm_chains() {
    let root = key(96);
    let p = profile(&root, b"owner-basis-coverage", [1; 32]);
    let a = chain(
        &root,
        &p,
        key(97).verifying_key().to_bytes(),
        9,
        1,
        Rights::MEMBER,
        vec![],
    );
    let b = chain(
        &root,
        &p,
        key(98).verifying_key().to_bytes(),
        9,
        2,
        Rights::MEMBER,
        vec![],
    );
    let main = chain(
        &root,
        &p,
        key(99).verifying_key().to_bytes(),
        9,
        3,
        Rights::MEMBER,
        vec![a.basis.basis_id().unwrap(), b.basis.basis_id().unwrap()],
    );
    let expected = expectation(&p);
    let left = expected
        .verify_basis(&p, main.refs(), &[a.refs(), b.refs()])
        .unwrap();
    let right = expected
        .verify_basis(&p, main.refs(), &[b.refs(), a.refs()])
        .unwrap();
    assert_eq!(left, right);
    assert!(left.covers(a.basis.basis_id().unwrap()));
    assert!(left.covers(b.basis.basis_id().unwrap()));
    assert_eq!(
        expected.verify_basis(&p, main.refs(), &[]),
        Err(ChatError::Authority)
    );
    assert_eq!(
        expected.verify_basis(&p, main.refs(), &[a.refs(), a.refs()]),
        Err(ChatError::Authority)
    );
    let foreign = profile(&root, b"owner-basis-coverage", [2; 32]);
    let foreign = chain(
        &root,
        &foreign,
        key(98).verifying_key().to_bytes(),
        9,
        2,
        Rights::MEMBER,
        vec![],
    );
    assert_eq!(
        expected.verify_basis(&p, main.refs(), &[a.refs(), foreign.refs()]),
        Err(ChatError::Scope)
    );
    let other_channel = chain(
        &root,
        &p,
        key(98).verifying_key().to_bytes(),
        10,
        2,
        Rights::MEMBER,
        vec![],
    );
    assert_eq!(
        expected.verify_basis(&p, main.refs(), &[a.refs(), other_channel.refs()]),
        Err(ChatError::Authority)
    );
    let rejoin = chain(
        &root,
        &p,
        key(98).verifying_key().to_bytes(),
        9,
        4,
        Rights::MEMBER,
        vec![],
    );
    assert_eq!(
        expected.verify_basis(&p, main.refs(), &[a.refs(), rejoin.refs()]),
        Err(ChatError::Authority)
    );
}

#[test]
fn owner_basis_semantic_witness_never_substitutes_for_exact_foreign_delete_approval() {
    let root = key(100);
    let author = key(101);
    let moderator = key(102);
    let p = profile(&root, b"owner-basis-delete", [1; 32]);
    let a = chain(
        &root,
        &p,
        author.verifying_key().to_bytes(),
        9,
        1,
        Rights::MEMBER,
        vec![],
    );
    let m = chain(
        &root,
        &p,
        moderator.verifying_key().to_bytes(),
        9,
        2,
        Rights::MODERATOR,
        vec![a.basis.basis_id().unwrap()],
    );
    let create = action(
        &p,
        &a,
        &author,
        Body::Create {
            text: "retained author".into(),
            reply_to: None,
        },
        vec![],
    );
    let delete = action(
        &p,
        &m,
        &moderator,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    let expected = expectation(&p);
    let mut ledger = ChatLedger::new(create.spec().realm.clone(), limits()).unwrap();
    ledger.import(delete.clone()).unwrap();
    ledger
        .authorize(
            delete.id(),
            expected.verify_basis(&p, m.refs(), &[a.refs()]).unwrap(),
        )
        .unwrap();
    ledger.import(create.clone()).unwrap();
    ledger
        .authorize(
            create.id(),
            expected.verify_basis(&p, a.refs(), &[]).unwrap(),
        )
        .unwrap();
    drain(&mut ledger);
    assert_eq!(
        ledger.state(delete.id()),
        Some(ActionState::Pending(PendingReason::ModerationApproval))
    );
    assert_eq!(
        ledger
            .projection(create.id(), 1000)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("retained author")
    );
}

#[test]
fn owner_basis_founder_creation_must_match_pinned_profile_and_types_are_send_sync() {
    let root = key(103);
    let p = profile(&root, b"owner-basis-founder", [1; 32]);
    let founder_chain = |creation| {
        let admission = sign(
            &root,
            &p,
            OwnerFactBody::Admission {
                subject: root.verifying_key().to_bytes(),
                predecessor: p.id(),
                origin: AdmissionOrigin::Founder { creation },
            },
        );
        let grant = sign(
            &root,
            &p,
            OwnerFactBody::RealmGrant {
                channel: 9,
                admission: admission.id(),
                subject: root.verifying_key().to_bytes(),
                rights: Rights::MEMBER,
            },
        );
        let basis = sign(
            &root,
            &p,
            OwnerFactBody::Basis {
                channel: 9,
                admission: admission.id(),
                subject: root.verifying_key().to_bytes(),
                grant: grant.grant_id().unwrap(),
                covered: vec![],
            },
        );
        Chain {
            admission,
            grant,
            basis,
        }
    };
    let good = founder_chain([1; 32]);
    assert!(expectation(&p).verify_basis(&p, good.refs(), &[]).is_ok());
    let bad = founder_chain([2; 32]);
    assert_eq!(
        expectation(&p).verify_basis(&p, bad.refs(), &[]),
        Err(ChatError::Authority),
        "a fully linked, genuinely root-signed chain cannot substitute another creation"
    );
    let wrong_kind = sign(
        &root,
        &p,
        OwnerFactBody::Removal {
            admission: good.admission.id(),
            subject: root.verifying_key().to_bytes(),
            commit: [3; 32],
            before_epoch: 1,
            after_epoch: 2,
            accepted: vec![],
        },
    );
    assert_eq!(
        expectation(&p).verify_basis(
            &p,
            OwnerBasisFacts {
                admission: &wrong_kind,
                ..good.refs()
            },
            &[]
        ),
        Err(ChatError::Authority)
    );
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OwnerProfileExpectation>();
    send_sync::<OwnerBasisFacts<'static>>();
}

#[test]
fn owner_basis_selected_proof_count_is_bounded_before_any_root_verification() {
    let root = key(104);
    let p = profile(&root, b"owner-basis-count", [1; 32]);
    let c = chain(
        &root,
        &p,
        key(105).verifying_key().to_bytes(),
        9,
        1,
        Rights::MEMBER,
        vec![],
    );
    // Corrupt the root too: Count refusal must precede any cryptographic/root work.
    let mut bytes = p.encode();
    *bytes.last_mut().unwrap() ^= 1;
    let bad = SignedOwnerFact::decode(&bytes).unwrap();
    assert_eq!(
        expectation(&p).verify_basis(&bad, c.refs(), &vec![c.refs(); MAX_BASIS_COVERAGE + 1]),
        Err(ChatError::Bound)
    );
}
