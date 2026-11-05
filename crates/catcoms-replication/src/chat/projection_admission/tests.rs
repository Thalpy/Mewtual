//! Private fixtures verify real signatures under an explicitly pinned test issuer. They are
//! NOT the absent production current-governance/profile/admission verifier or a saved status.
use super::*;
use catcoms_crypto::{verify_with_public_bytes, DeviceKeypair};
use catcoms_wire::Encoder;
use std::collections::BTreeSet;

struct Fixture {
    owner: DeviceKeypair,
    member: DeviceKeypair,
    realm: Realm,
    context: EvidenceContext,
}
impl Fixture {
    fn new() -> Self {
        Self {
            owner: DeviceKeypair::from_seed(&[250; 32]),
            member: DeviceKeypair::from_seed(&[1; 32]),
            realm: Realm::new(b"projection-real-signed-fixture", 9).unwrap(),
            context: EvidenceContext {
                profile: owner_facts::OwnerFactId::from_bytes([2; 32]),
                admission: owner_facts::OwnerFactId::from_bytes([3; 32]),
            },
        }
    }
    fn ledger(&self) -> ChatLedger {
        ChatLedger::new(
            self.realm.clone(),
            LedgerLimits {
                max_actions: 128,
                max_action_bytes: 1024 * 1024,
            },
        )
        .unwrap()
    }
    fn witness(&self) -> VerifiedAuthorityBasis {
        let realm = self.realm.id();
        let basis = BasisId::from_bytes([4; 32]);
        let subject = self.member.verifying_key().to_bytes();
        let issuer = self.owner.verifying_key().to_bytes();
        let bytes = super::super::authority::fixture_grant_bytes(
            realm,
            basis,
            subject,
            Rights::MEMBER,
            &issuer,
        );
        let grant = GrantId::from_bytes(*blake3::hash(&bytes).as_bytes());
        let covered = BTreeSet::new();
        let basis_bytes =
            super::super::authority::fixture_basis_bytes(realm, basis, grant, &covered);
        super::super::authority::verified_fixture(
            &issuer,
            realm,
            basis,
            subject,
            Rights::MEMBER,
            covered,
            self.owner.sign(&bytes),
            self.owner.sign(&basis_bytes),
        )
        .unwrap()
    }
    fn action(&self, nonce: u8, body: Body, mut parents: Vec<ActionId>) -> SignedAction {
        parents.sort();
        parents.dedup();
        let witness = self.witness();
        SignedAction::sign(
            &self.member,
            ActionSpec {
                realm: self.realm.clone(),
                basis: witness.basis(),
                grant: witness.grant(),
                nonce: [nonce; 16],
                presented_ms: u64::from(nonce),
                parents,
                body,
            },
        )
        .unwrap()
    }
    fn create(&self, nonce: u8, text: &str) -> SignedAction {
        self.action(
            nonce,
            Body::Create {
                text: text.into(),
                reply_to: None,
            },
            vec![],
        )
    }
    fn retain(&self, ledger: &mut ChatLedger, action: &SignedAction) {
        ledger.import(action.clone()).unwrap();
        ledger.authorize(action.id(), self.witness()).unwrap();
        for _ in 0..1024 {
            if !ledger.advance(32).remaining {
                break;
            }
        }
        assert_eq!(ledger.state(action.id()), Some(ActionState::Valid));
    }
    fn token(
        &self,
        action: &SignedAction,
        view: &ProjectionView,
        open: Option<&ProjectionOpenAdmission>,
    ) -> ProjectionAdmissionToken {
        assert!(action.verify());
        assert!(self.witness().matches(action));
        let context = open.map_or(self.context, |open| EvidenceContext {
            profile: open.life.profile,
            admission: open.life.admission,
        });
        if let Some(open) = open {
            assert!(Arc::ptr_eq(&open.life.view, &view.life));
            assert_eq!(open.life.subject, *action.signer());
        }
        // This test-only signed statement checks exact action/context/class binding, while
        // deliberately making no claim to resolve production owner forks or complete closure.
        let mut bytes = Encoder::new();
        bytes
            .put_bytes(b"mewtual/test-only/current-projection/v1\0")
            .unwrap();
        bytes.put_bytes(&action.semantic_bytes()).unwrap();
        bytes.put_bytes(context.profile.as_bytes()).unwrap();
        bytes.put_bytes(context.admission.as_bytes()).unwrap();
        bytes.put_u8(u8::from(open.is_some()));
        let signature = self.owner.sign(bytes.as_bytes());
        assert!(verify_with_public_bytes(
            &self.owner.verifying_key().to_bytes(),
            bytes.as_bytes(),
            &signature
        ));
        bytes.put_bytes(&signature).unwrap();
        ProjectionAdmissionToken {
            binding: ActionBinding::of(action.id(), action),
            context,
            view: view.life.clone(),
            open: open.map(|scope| scope.life.clone()),
            proof: owner_facts::OwnerFactId::from_bytes(*blake3::hash(bytes.as_bytes()).as_bytes()),
            source: Arc::new(()),
        }
    }
    fn open(&self, view: &ProjectionView) -> ProjectionOpenAdmission {
        view.open_admission(
            self.context.profile,
            self.context.admission,
            self.member.verifying_key().to_bytes(),
        )
        .unwrap()
    }
    fn resolution(
        &self,
        ledger: &ChatLedger,
        delete: ActionId,
        view: &ProjectionView,
    ) -> CompleteProjectionResolution {
        let capsule = ledger.entries[&delete].prior_delete.clone().unwrap();
        let revision = view.life.revision.load(Ordering::Acquire);
        // Private signed fixture of a complete-current-closure assertion. This exercises
        // capability binding, not the still-absent real common-prefix/governance verifier.
        let mut bytes = Encoder::new();
        bytes
            .put_bytes(b"mewtual/test-only/complete-effects/v1\0")
            .unwrap();
        bytes.put_bytes(capsule.delete.action.as_bytes()).unwrap();
        bytes.put_bytes(capsule.create.action.as_bytes()).unwrap();
        bytes.put_bytes(capsule.proof.as_bytes()).unwrap();
        bytes.put_u64(revision);
        for (id, entry) in &ledger.entries {
            if entry
                .projection
                .as_ref()
                .is_some_and(|token| token.live_for(&view.life, *id, &entry.action))
            {
                bytes.put_bytes(id.as_bytes()).unwrap();
            }
        }
        let signature = self.owner.sign(bytes.as_bytes());
        assert!(verify_with_public_bytes(
            &self.owner.verifying_key().to_bytes(),
            bytes.as_bytes(),
            &signature
        ));
        CompleteProjectionResolution {
            capsule,
            view: view.life.clone(),
            revision,
        }
    }
}
fn result(ledger: &ChatLedger, create: ActionId) -> GuardedProjection {
    ledger
        .guarded_projection(create, 10_000)
        .unwrap()
        .with_publication(Clone::clone)
        .unwrap()
}
fn visible(ledger: &ChatLedger, create: ActionId, expected: &str, certainty: ProjectionCertainty) {
    assert!(
        matches!(result(ledger, create), GuardedProjection::Visible { text, certainty: actual, .. } if text == expected && actual == certainty)
    );
}

#[test]
fn semantic_history_and_resupplied_snapshot_witnesses_do_not_open_guarded_projection() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let create = f.create(1, "retained audit body");
    f.retain(&mut ledger, &create);
    assert_eq!(
        ledger
            .projection(create.id(), 100)
            .unwrap()
            .unwrap()
            .visible_text
            .as_deref(),
        Some("retained audit body")
    );
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Held { .. }
    ));
    let snapshot = ledger.snapshot().unwrap().encode();
    let mut restored =
        ChatLedger::restore(&snapshot, ledger.limits, &[(create.id(), f.witness())]).unwrap();
    while restored.advance(32).remaining {}
    let mut view = restored.begin_projection_view();
    assert!(view.try_ready());
    assert!(matches!(
        result(&restored, create.id()),
        GuardedProjection::Held { .. }
    ));
    assert_eq!(
        restored.snapshot().unwrap().encode(),
        snapshot,
        "projection metadata never changes raw snapshot bytes"
    );
}

#[test]
fn each_effect_needs_its_own_exact_token_and_only_chosen_text_is_exposed() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "original");
    let edit = f.action(
        2,
        Body::Edit {
            create: create.id(),
            text: "chosen".into(),
        },
        vec![create.id()],
    );
    let held = f.action(
        3,
        Body::Edit {
            create: create.id(),
            text: "held newer".into(),
        },
        vec![edit.id()],
    );
    let delete = f.action(
        4,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    for action in [&create, &edit, &held, &delete] {
        f.retain(&mut ledger, action);
    }
    ledger
        .admit_projection(&f.token(&edit, &view, None), 100)
        .unwrap();
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Held { .. }
    ));
    assert!(ledger
        .projectable_causal_heads(create.id(), 100)
        .unwrap()
        .with_publication(|heads| heads.is_empty())
        .unwrap());
    ledger
        .admit_projection(&f.token(&create, &view, None), 100)
        .unwrap();
    visible(&ledger, create.id(), "chosen", ProjectionCertainty::Proven);
    let output = result(&ledger, create.id());
    assert!(!format!("{output:?}").contains("held newer"));
    assert!(ledger.entries[&delete.id()].prior_delete.is_none());
}

#[test]
fn held_join_is_causal_knowledge_without_superseding_visible_edits_by_itself() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "base");
    let a = f.action(
        2,
        Body::Edit {
            create: create.id(),
            text: "a".into(),
        },
        vec![create.id()],
    );
    let join = f.action(
        3,
        Body::Join {
            create: create.id(),
        },
        vec![a.id()],
    );
    let b = f.action(
        4,
        Body::Edit {
            create: create.id(),
            text: "b".into(),
        },
        vec![join.id()],
    );
    for action in [&create, &a, &join, &b] {
        f.retain(&mut ledger, action);
    }
    for action in [&create, &a] {
        ledger
            .admit_projection(&f.token(action, &view, None), 100)
            .unwrap();
    }
    visible(&ledger, create.id(), "a", ProjectionCertainty::Proven);
    ledger
        .admit_projection(&f.token(&b, &view, None), 100)
        .unwrap();
    visible(&ledger, create.id(), "b", ProjectionCertainty::Proven);
    assert_eq!(
        ledger
            .projectable_causal_heads(create.id(), 1000)
            .unwrap()
            .with_publication(|heads| heads.to_vec())
            .unwrap(),
        vec![b.id()]
    );
}

#[test]
fn admission_close_preserves_proven_a2_but_never_renews_original_a1_or_old_read_lease() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let a1 = f.open(&view);
    let provisional = f.create(1, "a1 provisional");
    let accepted = f.create(2, "a2 proven");
    for action in [&provisional, &accepted] {
        f.retain(&mut ledger, action);
    }
    let old_token = f.token(&provisional, &view, Some(&a1));
    ledger.admit_projection(&old_token, 100).unwrap();
    ledger
        .admit_projection(&f.token(&accepted, &view, None), 100)
        .unwrap();
    let old_result = ledger.guarded_projection(accepted.id(), 100).unwrap();
    let mut close = a1.close();
    assert!(close.try_complete());
    assert!(close.try_complete());
    assert!(old_result.with_publication(|_| ()).is_err());
    assert!(matches!(
        result(&ledger, provisional.id()),
        GuardedProjection::Held { .. }
    ));
    visible(
        &ledger,
        accepted.id(),
        "a2 proven",
        ProjectionCertainty::Proven,
    );
    let _a2 = f.open(&view); // deliberately identical raw labels; original A1 token stays closed
    assert!(ledger.admit_projection(&old_token, 100).is_err());
}

#[test]
fn provisional_delete_hides_without_proven_claim_and_never_mints_prior_custody() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let open = f.open(&view);
    let create = f.create(1, "body");
    let delete = f.action(
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    for action in [&create, &delete] {
        f.retain(&mut ledger, action);
    }
    ledger
        .admit_projection(&f.token(&create, &view, None), 100)
        .unwrap();
    ledger
        .admit_projection(&f.token(&delete, &view, Some(&open)), 100)
        .unwrap();
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Deleted {
            certainty: ProjectionCertainty::Provisional,
            ..
        }
    ));
    assert!(ledger.entries[&delete.id()].prior_delete.is_none());
    let mut close = open.close();
    assert!(close.try_complete());
    visible(&ledger, create.id(), "body", ProjectionCertainty::Proven);
}

#[test]
fn proven_delete_of_provisional_create_mints_without_render_and_survives_proof_loss() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let open = f.open(&view);
    let create = f.create(1, "never resurrect");
    let delete = f.action(
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    for action in [&create, &delete] {
        f.retain(&mut ledger, action);
    }
    // Delete arrives first. It cannot make the held Create visible or mint eligible effect.
    ledger
        .admit_projection(&f.token(&delete, &view, None), 100)
        .unwrap();
    assert!(ledger.entries[&delete.id()].prior_delete.is_none());
    ledger
        .admit_projection(&f.token(&create, &view, Some(&open)), 100)
        .unwrap();
    assert!(
        ledger.entries[&delete.id()].prior_delete.is_some(),
        "minted at installation, never by rendering"
    );
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Deleted {
            certainty: ProjectionCertainty::Provisional,
            ..
        }
    ));
    let mut next = ledger.begin_projection_view();
    assert!(next.try_ready());
    ledger
        .admit_projection(&f.token(&create, &next, None), 100)
        .unwrap();
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::IncompleteSuppressed { .. }
    ));
}

#[test]
fn each_independent_proven_delete_retains_its_own_negative_obligation() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "body");
    let d1 = f.action(
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    let d2 = f.action(
        3,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    for action in [&create, &d1, &d2] {
        f.retain(&mut ledger, action);
        ledger
            .admit_projection(&f.token(action, &view, None), 100)
            .unwrap();
    }
    assert!(
        ledger.entries[&d1.id()].prior_delete.is_some()
            && ledger.entries[&d2.id()].prior_delete.is_some()
    );
    let mut next = ledger.begin_projection_view();
    assert!(next.try_ready());
    ledger
        .admit_projection(&f.token(&create, &next, None), 100)
        .unwrap();
    for id in [d1.id(), d2.id()] {
        let resolution = f.resolution(&ledger, id, &next);
        ledger.resolve_projection_delete(&resolution).unwrap();
        if id == d1.id() {
            assert!(matches!(
                result(&ledger, create.id()),
                GuardedProjection::IncompleteSuppressed { .. }
            ));
        }
    }
    visible(&ledger, create.id(), "body", ProjectionCertainty::Proven);
}

#[test]
fn guarded_edit_preserves_original_creation_timestamp_and_refuses_partial_work() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "base");
    let edit = f.action(
        90,
        Body::Edit {
            create: create.id(),
            text: "edit".into(),
        },
        vec![create.id()],
    );
    for action in [&create, &edit] {
        f.retain(&mut ledger, action);
        ledger
            .admit_projection(&f.token(action, &view, None), 100)
            .unwrap();
    }
    assert!(
        matches!(result(&ledger, create.id()), GuardedProjection::Visible { presented_ms: 1, selected, text, .. } if selected == edit.id() && text == "edit")
    );
    assert!(matches!(
        ledger.guarded_projection(create.id(), 0),
        Err(ChatError::WorkLimit)
    ));
    assert!(matches!(
        ledger.projectable_causal_heads(create.id(), 0),
        Err(ChatError::WorkLimit)
    ));
}

#[test]
fn pending_invalid_foreign_and_old_view_tokens_refuse_without_projection_cache() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "body");
    ledger.import(create.clone()).unwrap();
    let token = f.token(&create, &view, None);
    assert!(ledger.admit_projection(&token, 100).is_err());
    assert!(ledger.entries[&create.id()].projection.is_none());
    ledger.authorize(create.id(), f.witness()).unwrap();
    while ledger.advance(32).remaining {}
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Held { .. }
    ));
    let edit = f.action(
        8,
        Body::Edit {
            create: create.id(),
            text: "edit".into(),
        },
        vec![create.id()],
    );
    f.retain(&mut ledger, &edit);
    let invalid = f.action(
        9,
        Body::Edit {
            create: edit.id(),
            text: "not a Create target".into(),
        },
        vec![edit.id()],
    );
    ledger.import(invalid.clone()).unwrap();
    ledger.authorize(invalid.id(), f.witness()).unwrap();
    while ledger.advance(32).remaining {}
    assert_eq!(
        ledger.state(invalid.id()),
        Some(ActionState::Invalid(ChatError::Dependency))
    );
    assert!(ledger
        .admit_projection(&f.token(&invalid, &view, None), 100)
        .is_err());
    assert!(ledger.entries[&invalid.id()].projection.is_none());
    let mut other = f.ledger();
    let mut other_view = other.begin_projection_view();
    assert!(other_view.try_ready());
    f.retain(&mut other, &create);
    assert!(other.admit_projection(&token, 100).is_err());
    let mut next = ledger.begin_projection_view();
    assert!(next.try_ready());
    assert!(ledger.admit_projection(&token, 100).is_err());
    assert!(ledger.entries[&create.id()].projection.is_none());
}

#[test]
fn create_token_scan_work_refusal_is_atomic_and_does_not_mint_partial_delete_capsules() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "body");
    let d1 = f.action(
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    let d2 = f.action(
        3,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    for action in [&create, &d1, &d2] {
        f.retain(&mut ledger, action);
    }
    for action in [&d1, &d2] {
        ledger
            .admit_projection(&f.token(action, &view, None), 100)
            .unwrap();
    }
    let revision = view.life.revision.load(Ordering::Acquire);
    assert_eq!(
        ledger.admit_projection(&f.token(&create, &view, None), 1),
        Err(ChatError::WorkLimit)
    );
    assert_eq!(view.life.revision.load(Ordering::Acquire), revision);
    assert!(ledger.entries[&create.id()].projection.is_none());
    assert!(
        ledger.entries[&d1.id()].prior_delete.is_none()
            && ledger.entries[&d2.id()].prior_delete.is_none()
    );
    ledger
        .admit_projection(&f.token(&create, &view, None), 100)
        .unwrap();
    assert!(
        ledger.entries[&d1.id()].prior_delete.is_some()
            && ledger.entries[&d2.id()].prior_delete.is_some()
    );
}

use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
struct ReleaseOnDrop(Option<Sender<()>>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
fn hold_publication(
    message: GuardedMessageProjection,
) -> (
    Receiver<()>,
    ReleaseOnDrop,
    Receiver<bool>,
    std::thread::JoinHandle<()>,
) {
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (done_tx, done_rx) = channel();
    let worker = std::thread::spawn(move || {
        let result = message.with_publication(|_| {
            let _ = reached_tx.send(());
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        });
        let _ = done_tx.send(result.is_ok());
    });
    (reached_rx, ReleaseOnDrop(Some(release_tx)), done_rx, worker)
}

#[test]
fn actual_admitted_reply_delays_close_completion_and_pending_blocks_new_readers() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let open = f.open(&view);
    let create = f.create(1, "proven survives A1 close");
    f.retain(&mut ledger, &create);
    ledger
        .admit_projection(&f.token(&create, &view, None), 100)
        .unwrap();
    let (reached, release, done, worker) =
        hold_publication(ledger.guarded_projection(create.id(), 100).unwrap());
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    let revoker = open.revoker();
    let mut one = revoker.close();
    let mut two = revoker.close();
    assert!(!one.try_complete() && !two.try_complete());
    assert_eq!(
        view.life.pending.load(Ordering::Acquire),
        1,
        "same original close registers once"
    );
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Held { .. }
    ));
    drop(one); // another real completion retains the same original obligation
    assert!(!view.life.cancelled.load(Ordering::Acquire));
    drop(release);
    assert!(done.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap();
    assert!(two.try_complete() && two.try_complete());
    assert_eq!(view.life.pending.load(Ordering::Acquire), 0);
    visible(
        &ledger,
        create.id(),
        "proven survives A1 close",
        ProjectionCertainty::Proven,
    );
}

#[test]
fn two_independent_admission_closes_and_last_abandoned_completion_are_conservative() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let a = f.open(&view);
    let b = f.open(&view);
    let mut ca = a.close();
    let cb = b.close();
    assert_eq!(view.life.pending.load(Ordering::Acquire), 2);
    assert!(ca.try_complete());
    assert_eq!(view.life.pending.load(Ordering::Acquire), 1);
    drop(cb);
    assert!(view.life.cancelled.load(Ordering::Acquire));
    assert!(
        !view.life.live(),
        "abandoning a pending drain cannot reopen unrelated publications"
    );
}

#[test]
fn nested_and_stale_replacement_views_wait_for_the_original_shared_reader() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let first = ledger.begin_projection_view();
    let create = f.create(1, "old");
    f.retain(&mut ledger, &create);
    ledger
        .admit_projection(&f.token(&create, &first, None), 100)
        .unwrap();
    let (reached, release, done, worker) =
        hold_publication(ledger.guarded_projection(create.id(), 100).unwrap());
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut second = ledger.begin_projection_view();
    assert!(!second.try_ready());
    let mut third = ledger.begin_projection_view();
    assert!(!third.try_ready());
    assert!(!second.try_ready(), "a replaced pending view cannot revive");
    drop(second);
    drop(release);
    assert!(done.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap();
    assert!(third.try_ready());
    assert!(ledger
        .admit_projection(&f.token(&create, &first, None), 100)
        .is_err());
    ledger
        .admit_projection(&f.token(&create, &third, None), 100)
        .unwrap();
    visible(&ledger, create.id(), "old", ProjectionCertainty::Proven);
}

#[test]
fn callback_self_close_and_busy_positive_install_never_wait_or_mutate_before_admission() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let open = f.open(&view);
    let create = f.create(1, "body");
    let edit = f.action(
        2,
        Body::Edit {
            create: create.id(),
            text: "later".into(),
        },
        vec![create.id()],
    );
    for action in [&create, &edit] {
        f.retain(&mut ledger, action);
    }
    ledger
        .admit_projection(&f.token(&create, &view, Some(&open)), 100)
        .unwrap();
    let message = ledger.guarded_projection(create.id(), 100).unwrap();
    let edit_token = f.token(&edit, &view, None);
    let revision = view.life.revision.load(Ordering::Acquire);
    let mut close = message
        .with_publication(|_| {
            assert!(ledger.admit_projection(&edit_token, 100).is_err());
            assert_eq!(view.life.revision.load(Ordering::Acquire), revision);
            assert!(ledger.entries[&edit.id()].projection.is_none());
            let mut close = open.close();
            assert!(!close.try_complete());
            close
        })
        .unwrap();
    assert!(close.try_complete());
    assert!(message.with_publication(|_| ()).is_err());
}

#[test]
fn callback_owned_view_and_ledger_drop_or_unwind_cannot_deadlock_its_reader() {
    for unwind in [false, true] {
        let f = Fixture::new();
        let mut ledger = f.ledger();
        let view = ledger.begin_projection_view();
        let create = f.create(1, "body");
        f.retain(&mut ledger, &create);
        ledger
            .admit_projection(&f.token(&create, &view, None), 100)
            .unwrap();
        let message = ledger.guarded_projection(create.id(), 100).unwrap();
        let life = view.life.clone();
        let (done_tx, done_rx) = channel();
        let worker = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                message.with_publication(move |_| {
                    drop(ledger);
                    drop(view);
                    assert!(!life.live());
                    if unwind {
                        panic!("test-only callback unwind");
                    }
                })
            }));
            let _ = done_tx.send(if unwind {
                result.is_err()
            } else {
                result.is_ok_and(|result| result.is_ok())
            });
        });
        // Harness timeout only. This catches a blocking owner Drop instead of hanging the
        // mandatory test suite; there is no scoped join on the failure/unwind path.
        assert!(done_rx.recv_timeout(Duration::from_secs(10)).unwrap());
        worker.join().unwrap();
    }
}

#[test]
fn preparing_close_unwind_cancels_without_a_false_complete_or_scope_renewal() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let open = f.open(&view);
    *open.life.preparing_hook.lock().unwrap() =
        Some(Box::new(|| panic!("test-only pre-registration unwind")));
    let revoker = open.revoker();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| revoker.close())).is_err());
    assert!(view.life.cancelled.load(Ordering::Acquire));
    assert_eq!(open.life.phase.load(Ordering::Acquire), UNCOUNTED);
    let mut duplicate = revoker.close();
    assert!(duplicate.try_complete());
    assert!(duplicate.try_complete());
    let mut replacement = ledger.begin_projection_view();
    assert!(replacement.try_ready());
    assert!(!open.life.live());
}

#[test]
fn revision_pending_and_completion_overflow_cancel_without_wrapping() {
    for which in 0..3 {
        let f = Fixture::new();
        let mut ledger = f.ledger();
        let view = ledger.begin_projection_view();
        let open = f.open(&view);
        if which == 0 {
            view.life.revision.store(u64::MAX, Ordering::Release);
        }
        if which == 1 {
            view.life.pending.store(usize::MAX, Ordering::Release);
        }
        if which == 2 {
            open.life.completions.store(usize::MAX, Ordering::Release);
        }
        let mut close = open.revoker().close();
        assert!(close.try_complete());
        assert!(view.life.cancelled.load(Ordering::Acquire));
        if which == 0 {
            assert_eq!(view.life.revision.load(Ordering::Acquire), u64::MAX);
        }
        if which == 1 {
            assert_eq!(view.life.pending.load(Ordering::Acquire), usize::MAX);
        }
        if which == 2 {
            assert_eq!(open.life.completions.load(Ordering::Acquire), usize::MAX);
        }
    }
}

#[test]
fn complete_resolution_refuses_foreign_stale_and_replaced_capsules_without_clearing_others() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "body");
    let delete = f.action(
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    for action in [&create, &delete] {
        f.retain(&mut ledger, action);
        ledger
            .admit_projection(&f.token(action, &view, None), 100)
            .unwrap();
    }
    let stale = f.resolution(&ledger, delete.id(), &view);
    let mut next = ledger.begin_projection_view();
    assert!(next.try_ready());
    ledger
        .admit_projection(&f.token(&create, &next, None), 100)
        .unwrap();
    assert!(ledger.resolve_projection_delete(&stale).is_err());
    let mut wrong = f.resolution(&ledger, delete.id(), &next);
    wrong.capsule.source = Arc::new(());
    assert!(ledger.resolve_projection_delete(&wrong).is_err());
    assert!(ledger.entries[&delete.id()].prior_delete.is_some());
    let current = f.resolution(&ledger, delete.id(), &next);
    let unrelated = f.create(8, "other");
    f.retain(&mut ledger, &unrelated);
    ledger
        .admit_projection(&f.token(&unrelated, &next, None), 100)
        .unwrap();
    assert!(
        ledger.resolve_projection_delete(&current).is_err(),
        "a stale complete-closure revision cannot adopt a newer one"
    );
    ledger
        .resolve_projection_delete(&f.resolution(&ledger, delete.id(), &next))
        .unwrap();
    visible(&ledger, create.id(), "body", ProjectionCertainty::Proven);
}

#[test]
fn concurrent_duplicate_close_during_preparation_cannot_complete_or_double_register() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let open = f.open(&view);
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (done_tx, done_rx) = channel();
    let release = ReleaseOnDrop(Some(release_tx));
    *open.life.preparing_hook.lock().unwrap() = Some(Box::new(move || {
        let _ = reached_tx.send(());
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    }));
    let revoker = open.revoker();
    let worker_revoker = revoker.clone();
    let worker = std::thread::spawn(move || {
        let _ = done_tx.send(worker_revoker.close());
    });
    reached_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(open.life.phase.load(Ordering::Acquire), PREPARING);
    let mut duplicate = revoker.close();
    assert!(!duplicate.try_complete());
    drop(release);
    let mut original = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    worker.join().unwrap();
    assert_eq!(view.life.pending.load(Ordering::Acquire), 1);
    assert!(original.try_complete() && duplicate.try_complete());
    assert_eq!(view.life.pending.load(Ordering::Acquire), 0);
    assert!(!view.life.cancelled.load(Ordering::Acquire));
}

#[test]
fn moderation_pending_delete_cannot_preinstall_current_support_or_uncapsuled_later_effect() {
    let f = Fixture::new();
    let mut moderator = Fixture::new();
    moderator.member = DeviceKeypair::from_seed(&[2; 32]);
    moderator.context.admission = owner_facts::OwnerFactId::from_bytes([5; 32]);
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "retained");
    f.retain(&mut ledger, &create);
    ledger
        .admit_projection(&f.token(&create, &view, None), 100)
        .unwrap();
    let delete = moderator.action(
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    ledger.import(delete.clone()).unwrap();
    ledger.authorize(delete.id(), moderator.witness()).unwrap();
    while ledger.advance(32).remaining {}
    assert_eq!(
        ledger.state(delete.id()),
        Some(ActionState::Pending(PendingReason::ModerationApproval))
    );
    let token = moderator.token(&delete, &view, None);
    assert!(ledger.admit_projection(&token, 100).is_err());
    assert!(ledger.entries[&delete.id()].prior_delete.is_none());
    let issuer = moderator.owner.verifying_key().to_bytes();
    let bytes = super::super::authority::fixture_delete_approval_bytes(&issuer, &delete).unwrap();
    let approval = super::super::authority::verified_delete_fixture(
        &issuer,
        &delete,
        moderator.owner.sign(&bytes),
    )
    .unwrap();
    ledger.approve_delete(delete.id(), approval).unwrap();
    while ledger.advance(32).remaining {}
    visible(
        &ledger,
        create.id(),
        "retained",
        ProjectionCertainty::Proven,
    );
    assert!(
        ledger.entries[&delete.id()].projection.is_none(),
        "advance cannot revive the refused token"
    );
    ledger.admit_projection(&token, 100).unwrap();
    assert!(ledger.entries[&delete.id()].prior_delete.is_some());
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Deleted {
            certainty: ProjectionCertainty::Proven,
            ..
        }
    ));
}

#[test]
fn resolving_current_proven_support_keeps_its_capsule_for_a_later_proof_loss() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "body");
    let delete = f.action(
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    for action in [&create, &delete] {
        f.retain(&mut ledger, action);
        ledger
            .admit_projection(&f.token(action, &view, None), 100)
            .unwrap();
    }
    let resolution = f.resolution(&ledger, delete.id(), &view);
    ledger.resolve_projection_delete(&resolution).unwrap();
    assert!(ledger.entries[&delete.id()].prior_delete.is_some());
    let mut next = ledger.begin_projection_view();
    assert!(next.try_ready());
    ledger
        .admit_projection(&f.token(&create, &next, None), 100)
        .unwrap();
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::IncompleteSuppressed { .. }
    ));
}

#[test]
fn full_entry_quota_does_not_block_negative_close_and_unknown_tokens_allocate_no_entries() {
    let f = Fixture::new();
    let mut ledger = ChatLedger::new(
        f.realm.clone(),
        LedgerLimits {
            max_actions: 1,
            max_action_bytes: MAX_ACTION_BYTES,
        },
    )
    .unwrap();
    let view = ledger.begin_projection_view();
    let open = f.open(&view);
    let create = f.create(1, "full");
    f.retain(&mut ledger, &create);
    ledger
        .admit_projection(&f.token(&create, &view, Some(&open)), 100)
        .unwrap();
    let unknown = f.create(2, "unknown");
    let before = ledger.retained_action_bytes();
    let revision = view.life.revision.load(Ordering::Acquire);
    assert_eq!(
        ledger.admit_projection(&f.token(&unknown, &view, None), 100),
        Err(ChatError::Unknown)
    );
    assert_eq!(ledger.entries.len(), 1);
    assert_eq!(ledger.retained_action_bytes(), before);
    assert_eq!(view.life.revision.load(Ordering::Acquire), revision);
    let mut close = open.close();
    assert!(close.try_complete());
    assert_eq!(ledger.entries.len(), 1);
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Held { .. }
    ));
}

#[test]
fn genuine_signature_variants_remain_one_retained_action_and_cannot_open_projection() {
    fn unhex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../tests/signature-variants.json")).unwrap();
    let actions: Vec<_> = vectors["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| SignedAction::decode(&unhex(frame.as_str().unwrap())).unwrap())
        .collect();
    assert_eq!(actions[0].id(), actions[1].id());
    assert_ne!(actions[0].signature(), actions[1].signature());
    assert!(actions.iter().all(SignedAction::verify));
    for order in [[0, 1], [1, 0]] {
        let mut ledger = ChatLedger::new(
            actions[0].spec().realm.clone(),
            LedgerLimits {
                max_actions: 1,
                max_action_bytes: MAX_ACTION_BYTES,
            },
        )
        .unwrap();
        let view = ledger.begin_projection_view();
        ledger.import(actions[order[0]].clone()).unwrap();
        ledger.import(actions[order[1]].clone()).unwrap();
        while ledger.advance(32).remaining {}
        assert_eq!(ledger.entries.len(), 1);
        assert!(view.life.live());
        assert!(matches!(
            result(&ledger, actions[0].id()),
            GuardedProjection::Held { .. }
        ));
    }
}

#[test]
fn equivalent_current_proof_retries_preserve_original_support_and_existing_read_revision() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let create = f.create(1, "body");
    f.retain(&mut ledger, &create);
    let token = f.token(&create, &view, None);
    ledger.admit_projection(&token, 100).unwrap();
    let message = ledger.guarded_projection(create.id(), 100).unwrap();
    let revision = view.life.revision.load(Ordering::Acquire);
    ledger
        .admit_projection(&f.token(&create, &view, None), 0)
        .unwrap();
    assert_eq!(view.life.revision.load(Ordering::Acquire), revision);
    assert!(Arc::ptr_eq(
        &ledger.entries[&create.id()]
            .projection
            .as_ref()
            .unwrap()
            .source,
        &token.source
    ));
    assert!(message.with_publication(|_| ()).is_ok());
}

#[test]
fn poisoned_actual_barrier_cannot_make_replacement_ready_or_publish_body() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let mut view = ledger.begin_projection_view();
    let create = f.create(1, "body");
    f.retain(&mut ledger, &create);
    ledger
        .admit_projection(&f.token(&create, &view, None), 100)
        .unwrap();
    let gate = view.life.gate.clone();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _writer = gate.write().unwrap();
        panic!("test-only positive-writer unwind");
    }))
    .is_err());
    assert!(!view.try_ready());
    assert!(matches!(
        result(&ledger, create.id()),
        GuardedProjection::Held { .. }
    ));
    let mut next = ledger.begin_projection_view();
    assert!(!next.try_ready());
    let mut close = view.close();
    assert!(
        close.try_complete(),
        "exclusive poisoned guard can still prove old publication drain"
    );
}

#[test]
fn retained_negative_view_revoker_never_keeps_or_renews_a_dropped_original_view() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let view = ledger.begin_projection_view();
    let revoker = view.revoker();
    let create = f.create(1, "body");
    f.retain(&mut ledger, &create);
    let token = f.token(&create, &view, None);
    ledger.admit_projection(&token, 100).unwrap();
    let message = ledger.guarded_projection(create.id(), 100).unwrap();
    drop(view);
    assert!(message.with_publication(|_| ()).is_err());
    let mut first = revoker.close();
    let mut second = revoker.close();
    assert!(first.try_complete() && second.try_complete());
    let mut next = ledger.begin_projection_view();
    assert!(next.try_ready());
    assert!(ledger.admit_projection(&token, 100).is_err());
    ledger
        .admit_projection(&f.token(&create, &next, None), 100)
        .unwrap();
    visible(&ledger, create.id(), "body", ProjectionCertainty::Proven);
    assert!(
        first.try_complete(),
        "old closed result stays idempotent without closing the new view"
    );
}

#[test]
fn equal_complete_current_evidence_converges_after_different_prior_delete_support() {
    let f = Fixture::new();
    let mut left = f.ledger();
    let mut right = f.ledger();
    let old_left = left.begin_projection_view();
    let old_right = right.begin_projection_view();
    let create = f.create(1, "retained");
    let d1 = f.action(
        2,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    let d2 = f.action(
        3,
        Body::Delete {
            create: create.id(),
        },
        vec![create.id()],
    );
    for action in [&create, &d1, &d2] {
        f.retain(&mut left, action);
        f.retain(&mut right, action);
    }
    for (ledger, view, prior) in [(&mut left, &old_left, &d1), (&mut right, &old_right, &d2)] {
        ledger
            .admit_projection(&f.token(&create, view, None), 100)
            .unwrap();
        ledger
            .admit_projection(&f.token(prior, view, None), 100)
            .unwrap();
    }
    // No rendering minted either prior capsule. The replicas retain different original
    // obligations, but neither is allowed to grandfather its local branch into visibility.
    let mut lv = left.begin_projection_view();
    let mut rv = right.begin_projection_view();
    assert!(lv.try_ready() && rv.try_ready());
    left.admit_projection(&f.token(&create, &lv, None), 100)
        .unwrap();
    right
        .admit_projection(&f.token(&create, &rv, None), 100)
        .unwrap();
    assert!(matches!(
        result(&left, create.id()),
        GuardedProjection::IncompleteSuppressed { .. }
    ));
    assert!(matches!(
        result(&right, create.id()),
        GuardedProjection::IncompleteSuppressed { .. }
    ));
    for (ledger, view) in [(&mut left, &lv), (&mut right, &rv)] {
        ledger
            .admit_projection(&f.token(&d1, view, None), 100)
            .unwrap();
    }
    let resolution = f.resolution(&right, d2.id(), &rv);
    right.resolve_projection_delete(&resolution).unwrap();
    assert_eq!(result(&left, create.id()), result(&right, create.id()));
    assert!(
        matches!(result(&right, create.id()), GuardedProjection::Deleted { delete, certainty: ProjectionCertainty::Proven, .. } if delete == d1.id())
    );
}

#[test]
fn completed_original_view_covers_its_old_scope_without_waiting_on_new_view_readers() {
    let f = Fixture::new();
    let mut ledger = f.ledger();
    let first = ledger.begin_projection_view();
    let open = f.open(&first);
    let create = f.create(1, "body");
    f.retain(&mut ledger, &create);
    ledger
        .admit_projection(&f.token(&create, &first, None), 100)
        .unwrap();
    let (reached, release, done, worker) =
        hold_publication(ledger.guarded_projection(create.id(), 100).unwrap());
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut old_scope = open.close();
    assert!(!old_scope.try_complete());
    let mut unrelated = f.ledger();
    let unrelated_view = unrelated.begin_projection_view();
    let mut unrelated_close = unrelated_view.close();
    assert!(unrelated_close.try_complete());
    assert!(
        !old_scope.try_complete(),
        "another ledger's real closed barrier cannot settle this original scope"
    );
    let mut view_close = first.revoker().close();
    assert!(!view_close.try_complete());
    drop(release);
    assert!(done.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap();
    assert!(view_close.try_complete());
    assert_eq!(first.life.pending.load(Ordering::Acquire), 1);
    let mut next = ledger.begin_projection_view();
    assert!(next.try_ready());
    ledger
        .admit_projection(&f.token(&create, &next, None), 100)
        .unwrap();
    let (reached, release, done, worker) =
        hold_publication(ledger.guarded_projection(create.id(), 100).unwrap());
    reached.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        old_scope.try_complete() && old_scope.try_complete(),
        "cached SAME original view drain must not wait for unrelated later publications"
    );
    assert_eq!(first.life.pending.load(Ordering::Acquire), 0);
    drop(release);
    assert!(done.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap();
    visible(&ledger, create.id(), "body", ProjectionCertainty::Proven);
}
