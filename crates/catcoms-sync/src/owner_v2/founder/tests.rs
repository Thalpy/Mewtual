//! Actual creation/MLS tests. TEST reservation observers model only negative source custody;
//! none of these fixtures claims a Store fee, saved/current head or real publication barrier.
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use tokio::sync::watch;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
mod snapshot_tests;
pub(super) fn found(seed: u64) -> Node {
    ChannelSync::found(
        Hub::new().join(PeerId::from_u64(seed)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap()
}

#[test]
fn actual_v2_founder_produces_exact_signed_genesis_with_empty_context_and_no_runtime_effect() {
    let mut node = found(0x2f01);
    let epoch = node.group.epoch_authenticator_id();
    let mut original_rng = node.rng.clone(); // Test comparison only; NEVER installed into node.
    let prepared = node.prepare_owner_v2_founder().unwrap();
    let profile_id = prepared.profile_id();
    let frozen = prepared.freeze(&node).unwrap();
    let view = frozen.borrow_for_covering_save(&node).unwrap();
    assert_eq!(format!("{view:?}"), "FounderDataView { .. }");
    let profile = SignedOwnerProfile::decode(view.profile).unwrap();
    let admission = AdmissionBody::decode(view.admission).unwrap();
    let header = SignedOwnerHeader::decode(view.header).unwrap();
    let image = StateFrame::decode(view.state).unwrap();
    assert!(profile.verify() && header.verify());
    image
        .verify_genesis_binding(&profile, &admission, &header)
        .unwrap();
    assert_eq!(profile.id(), profile_id);
    assert_eq!(
        profile.spec().founder.as_slice(),
        node.device.public_key_bytes()
    );
    assert_eq!(profile.spec().group, node.group_id());
    assert!(
        matches!(admission.spec().origin, AdmissionOrigin::Founder { creation } if creation == profile.spec().creation)
    );
    assert_eq!(view.initial_tip_header, header.id());
    assert_eq!(view.frontier.tip().count, 0);
    assert_eq!(
        view.frontier.tip().context,
        TreeContext {
            profile: profile.id(),
            admission: admission.admission_id()
        }
    );
    assert_eq!(
        view.frontier.tip().root,
        image.slots().next().unwrap().accepted_root
    );
    assert_eq!(view.frontier.hashes().count(), 0);
    let original = [
        view.profile.to_vec(),
        view.admission.to_vec(),
        view.state.to_vec(),
        view.header.to_vec(),
    ];
    let view = frozen.borrow_for_covering_save(&node).unwrap();
    assert_eq!(
        [
            view.profile.to_vec(),
            view.admission.to_vec(),
            view.state.to_vec(),
            view.header.to_vec()
        ],
        original
    );
    assert!(view.profile.len() <= catcoms_replication::chat::owner_profile::MAX_PROFILE_BYTES);
    assert!(view.admission.len() <= catcoms_replication::chat::owner_profile::MAX_ADMISSION_BYTES);
    assert!(view.header.len() <= catcoms_replication::chat::owner_header::MAX_HEADER_BYTES);
    assert!(view.state.len() <= owner_state::MAX_STATE_BYTES);
    assert_eq!(node.group.epoch_authenticator_id(), epoch);
    assert_eq!(node.group.epoch(), 0);
    assert_eq!(node.group.member_count(), 1);
    assert_eq!(node.rng.next_u64(), original_rng.next_u64());
    assert!(node.outbox.is_empty() && node.commit_log.is_empty() && node.ledger.is_empty());
    assert!(node.initial_owner_evidence.is_none() && node.owner_operation.is_none());
    assert!(node.chat_scope_label().is_none());
}

#[test]
fn byte_identical_restored_mls_cannot_create_or_adopt_actual_v2_founder_source() {
    let mut node = found(0x2f02);
    let frozen = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    let snapshot = node.snapshot().unwrap();
    let restored = ChannelSync::restore(
        &snapshot,
        Hub::new().join(PeerId::from_u64(0x2f03)),
        ChaCha20Rng::seed_from_u64(0x2f03),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert_eq!(
        restored.group.epoch_authenticator_id(),
        node.group.epoch_authenticator_id()
    );
    assert_eq!(restored.group_id(), node.group_id());
    assert_eq!(restored.my_public_key(), node.my_public_key());
    assert!(restored.fresh_chat_admission().is_none());
    assert!(restored.prepare_owner_v2_founder().is_err());
    assert!(frozen.borrow_for_covering_save(&restored).is_err());
    assert!(frozen.borrow_for_covering_save(&node).is_ok());
    drop(node); // Frozen observations keep neither actual creation owner nor its sender alive.
    assert!(frozen.borrow_for_covering_save(&restored).is_err());
}

#[test]
fn taken_original_token_actual_add_wrong_signer_and_retained_work_refuse_old_source() {
    let mut node = found(0x2f04);
    let prepared = node.prepare_owner_v2_founder().unwrap();
    let token = node.take_fresh_chat_admission().unwrap();
    assert!(
        prepared.validate_original(&node).is_err(),
        "even a still-owned token must remain present in this Sync"
    );
    drop(token);
    assert!(node.prepare_owner_v2_founder().is_err());
    let mut node = found(0x2f05);
    let frozen = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    let member = MlsDevice::generate().unwrap();
    let package = member
        .key_package_for_invite(&node.group_id(), [5; 16])
        .unwrap();
    node.with_observed_mls_transition(|node| node.group.add_member(&node.device, package))
        .unwrap();
    assert_eq!(node.group.member_count(), 2);
    assert!(frozen.borrow_for_covering_save(&node).is_err());
    assert!(
        matches!(
            node.prepare_owner_v2_founder(),
            Err(SyncError::ChatScopeUnavailable)
        ),
        "advanced original source must refuse before SDK roster/untouched-state observations"
    );
    let mut node = found(0x2f06);
    let prepared = node.prepare_owner_v2_founder().unwrap();
    node.device = MlsDevice::generate().unwrap();
    assert!(prepared.freeze(&node).is_err());
    let mut node = found(0x2f07);
    let prepared = node.prepare_owner_v2_founder().unwrap();
    node.ledger.consume([7; 16]).unwrap();
    assert!(prepared.freeze(&node).is_err());
    assert!(
        node.ledger.is_consumed(&[7; 16]),
        "unsupported work is preserved, never truncated"
    );
}

pub(super) struct Reservation {
    group: Vec<u8>,
    signer: [u8; 32],
    identity: Arc<()>,
    owner: Arc<watch::Sender<bool>>,
}
struct Revoker(std::sync::Weak<watch::Sender<bool>>);
impl local_chat_scope::ManagedChatScopeRevocation for Revoker {
    fn revoke(&self) {
        if let Some(owner) = self.0.upgrade() {
            owner.send_replace(true);
        }
    }
}
impl local_chat_scope::ManagedChatScopeReservation for Reservation {
    fn group(&self) -> &[u8] {
        &self.group
    }
    fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
    fn domain(&self) -> [u8; 32] {
        [41; 32]
    }
    fn nonce(&self) -> [u8; 32] {
        [42; 32]
    }
    fn owner_identity(&self) -> Arc<()> {
        self.identity.clone()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        RequestCancellation::new(self.owner.subscribe(), None)
    }
    fn revocation(&self) -> Arc<dyn local_chat_scope::ManagedChatScopeRevocation> {
        Arc::new(Revoker(Arc::downgrade(&self.owner)))
    }
}
pub(super) fn reserve(node: &mut Node) -> Reservation {
    let reservation = Reservation {
        group: node.group_id(),
        signer: node.my_public_key().try_into().unwrap(),
        identity: Arc::new(()),
        owner: Arc::new(watch::channel(false).0),
    };
    node.stage_managed_chat_scope([41; 32], [42; 32]).unwrap();
    node.attach_managed_chat_reservation(&reservation).unwrap();
    reservation
}

#[test]
fn original_private_reservation_survives_exact_freeze_but_cannot_be_renewed_or_replaced() {
    let mut node = found(0x2f08);
    let original = reserve(&mut node); // Actual host must reserve FIRST, not rebind an unmanaged preparation.
    let frozen = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    assert!(frozen.borrow_for_covering_save(&node).is_ok());
    original.owner.send_replace(true);
    assert!(frozen.borrow_for_covering_save(&node).is_err());
    assert!(node.prepare_owner_v2_founder().is_err());
    let mut dropped = found(0x2f14);
    let reservation = reserve(&mut dropped);
    let frozen_drop = dropped
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&dropped)
        .unwrap();
    drop(reservation);
    assert!(
        frozen_drop.borrow_for_covering_save(&dropped).is_err(),
        "frozen observations must not keep the original reservation sender alive"
    );
    let mut node = found(0x2f09);
    let _original = reserve(&mut node);
    let frozen = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    let original_label = node.chat_scope_label().cloned();
    let original_revision = node.chat_scope_revision();
    node.local_chat_scope = Default::default(); // Internal adversarial replacement; no production reset API.
    let _replacement = reserve(&mut node);
    assert_eq!(node.chat_scope_label(), original_label.as_ref());
    assert_eq!(node.chat_scope_revision(), original_revision);
    assert!(
        frozen.borrow_for_covering_save(&node).is_err(),
        "equal labels/revision cannot replace physical reservation ownership"
    );
}

#[test]
fn changed_scope_required_missing_scope_retirement_and_v1_tail_never_reinterpret_genesis() {
    let mut node = found(0x2f10);
    let prepared = node.prepare_owner_v2_founder().unwrap();
    node.stage_managed_chat_scope([1; 32], [2; 32]).unwrap();
    assert!(prepared.freeze(&node).is_err());
    let prepared = node.prepare_owner_v2_founder().unwrap();
    node.local_chat_scope.retire();
    assert!(prepared.freeze(&node).is_err());
    let mut node = found(0x2f11);
    node.require_managed_chat_scope();
    assert!(node.prepare_owner_v2_founder().is_err());
    let mut node = found(0x2f12);
    let v1 = node.prepare_owner_evidence_checkpoint(9).unwrap();
    node.stage_managed_chat_scope([1; 32], [2; 32]).unwrap();
    node.stage_owner_evidence_checkpoint(&v1).unwrap();
    assert!(node.prepare_owner_v2_founder().is_err());
    assert!(node.initial_owner_evidence.is_some());
}

#[test]
fn frozen_public_signature_representative_and_state_substitution_refuse_before_host_borrow() {
    let node = found(0x2f13);
    let mut frozen = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    *frozen.profile.last_mut().unwrap() ^= 1; // Same semantic CID, invalid signature representation.
    let invalid = SignedOwnerProfile::decode(&frozen.profile).unwrap();
    assert_eq!(invalid.id(), frozen.original.profile.id());
    assert!(!invalid.verify());
    assert!(frozen.borrow_for_covering_save(&node).is_err());
    *frozen.profile.last_mut().unwrap() ^= 1;
    assert!(frozen.borrow_for_covering_save(&node).is_ok());
    *frozen.original.state.last_mut().unwrap() ^= 1;
    assert!(frozen.borrow_for_covering_save(&node).is_err());
}

#[test]
fn producer_observation_types_are_send_sync_without_clone_or_owner_escape() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PreparedFounder>();
    send_sync::<FrozenFounder>();
}
