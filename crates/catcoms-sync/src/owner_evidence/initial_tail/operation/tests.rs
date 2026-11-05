//! Real-key/actual-Add crypto and serializer tests under an explicit TEST trusted-host binding.
//! These do not prove Current, fsync, mount custody or a native save. App tests use actual Stores.
use super::*;
use crate::local_chat_scope::{
    ManagedChatScopeBinding, ManagedChatScopeReservation, ManagedChatScopeRevocation,
};
use crate::owner_evidence::membership_candidate::PreparedOwnerMembership;
use crate::owner_evidence::operation_tail::OwnerOperationPredecessor;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use tokio::sync::watch;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
struct Binding {
    group: Vec<u8>,
    signer: [u8; 32],
    profile: OwnerFactId,
    owner: Arc<watch::Sender<bool>>,
    identity: Arc<()>,
}
struct Revoker(std::sync::Weak<watch::Sender<bool>>);
impl ManagedChatScopeRevocation for Revoker {
    fn revoke(&self) {
        if let Some(owner) = self.0.upgrade() {
            owner.send_replace(true);
        }
    }
}
impl ManagedChatScopeReservation for Binding {
    fn group(&self) -> &[u8] {
        &self.group
    }
    fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
    fn domain(&self) -> [u8; 32] {
        [31; 32]
    }
    fn nonce(&self) -> [u8; 32] {
        [32; 32]
    }
    fn owner_identity(&self) -> Arc<()> {
        self.identity.clone()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        RequestCancellation::new(self.owner.subscribe(), None)
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        Arc::new(Revoker(Arc::downgrade(&self.owner)))
    }
}
impl ManagedChatScopeBinding for Binding {
    fn publication_authority(&self) -> RequestCancellation {
        self.reservation_observer()
    }
}
impl ManagedOwnerEvidenceBinding for Binding {
    fn profile_id(&self) -> OwnerFactId {
        self.profile
    }
    fn owner_founder(&self) -> &[u8; 32] {
        &self.signer
    }
}
fn fixture() -> (Node, Binding, PreparedOwnerMembership) {
    let mut node = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(0xae01)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0xae01),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    node.initialize_new_group_policy().unwrap();
    let profile = node.prepare_owner_evidence_checkpoint(9).unwrap();
    let binding = Binding {
        group: node.group_id(),
        signer: node.my_public_key().try_into().unwrap(),
        profile: profile.profile_id(),
        owner: Arc::new(watch::channel(false).0),
        identity: Arc::new(()),
    };
    node.stage_managed_chat_scope(binding.domain(), binding.nonce())
        .unwrap();
    node.stage_owner_evidence_checkpoint(&profile).unwrap();
    node.bind_managed_chat_scope(&binding).unwrap();
    node.bind_managed_owner_evidence(&binding).unwrap();
    let invite = node.mint_invite([33; 16], 100_000, vec![]).unwrap();
    let member = MlsDevice::generate().unwrap();
    let package = catcoms_mls::serialize_key_package(
        &member
            .key_package_for_invite(&node.group_id(), invite.invite_nonce)
            .unwrap(),
    )
    .unwrap();
    let predecessor = OwnerOperationPredecessor {
        domain: binding.domain(),
        nonce: binding.nonce(),
        head: [34; 32],
        generation: 1,
        snapshot: [35; 32],
    };
    let candidate = node
        .prepare_owner_direct_add_checkpoint(
            &invite,
            &package,
            9,
            predecessor,
            512 * 1024,
            512 * 1024,
        )
        .unwrap();
    (node, binding, candidate)
}
fn freeze(
    node: &mut Node,
    candidate: &PreparedOwnerMembership,
) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    node.snapshot_owner_direct_add_checkpoint(candidate, 24 * 1024 * 1024, 512 * 1024, 512 * 1024)
}

#[test]
fn first_add_foreign_profile_bootstrap_is_refused_before_snapshot_and_source_mutation() {
    let (mut node, _binding, mut candidate) = fixture();
    let bootstrap = candidate
        .evidence
        .facts
        .values()
        .find(|fact| matches!(fact.spec().body, OwnerFactBody::JoinBootstrap { .. }))
        .unwrap()
        .clone();
    let mut foreign_spec = bootstrap.spec().clone();
    foreign_spec.profile = Some(OwnerFactId::from_bytes([99; 32]));
    let foreign = SignedOwnerFact::sign_using(foreign_spec, candidate.operation.owner, |bytes| {
        node.device.sign(bytes).map_err(|_| ChatError::Signature)
    })
    .unwrap();
    assert!(
        foreign.verify(),
        "a valid founder signature alone cannot choose another profile"
    );
    candidate.evidence.facts.remove(&bootstrap.id());
    candidate.evidence.facts.insert(foreign.id(), foreign);
    assert!(freeze(&mut node, &candidate).is_err());
    assert_eq!(node.epoch(), 0);
    assert_eq!(node.member_count(), 1);
    assert!(!node.ledger.is_consumed(&candidate.operation.nonce));
    assert!(node.owner_operation.is_none());
}

#[test]
fn first_add_wrong_topic_or_local_scope_refuses_before_covering_snapshot() {
    for field in 0..3 {
        let (mut node, _binding, mut candidate) = fixture();
        match field {
            0 => candidate.operation.topic[0] ^= 1,
            1 => candidate.operation.predecessor.domain[0] ^= 1,
            _ => candidate.operation.predecessor.nonce[0] ^= 1,
        }
        assert!(freeze(&mut node, &candidate).is_err());
        assert_eq!(node.epoch(), 0);
        assert_eq!(node.member_count(), 1);
    }
}

#[test]
fn first_add_source_governance_and_history_cannot_change_behind_private_candidate() {
    let (mut node, _binding, candidate) = fixture();
    node.file_wrap_key[0] ^= 1;
    assert!(freeze(&mut node, &candidate).is_err());
    node.file_wrap_key[0] ^= 1;
    node.ledger.consume([77; 16]).unwrap();
    assert!(freeze(&mut node, &candidate).is_err());
    assert!(node.ledger.is_consumed(&[77; 16]));
    assert!(!node.ledger.is_consumed(&candidate.operation.nonce));
}

#[test]
fn first_add_canonical_parent_expectation_is_checked_before_owner_restore() {
    let (mut node, binding, candidate) = fixture();
    let bytes = freeze(&mut node, &candidate).unwrap();
    let limits = OwnerInitializationRestoreLimits {
        max_snapshot_bytes: 24 * 1024 * 1024,
        max_mls_bytes: 512 * 1024,
        max_mls_backing_bytes: 512 * 1024,
    };
    let mut expected = candidate.operation.predecessor;
    for field in 0..3 {
        match field {
            0 => expected.head[0] ^= 1,
            1 => expected.snapshot[0] ^= 1,
            _ => expected.generation = 2,
        }
        assert!(crate::owner_evidence::validate_owner_operation_snapshot(
            &bytes,
            limits,
            crate::owner_evidence::OwnerOperationSourceExpectation {
                group: &binding.group,
                signer: &binding.signer,
                profile: binding.profile,
                predecessor: expected,
                request: candidate.operation.binding
            }
        )
        .is_err());
        expected = candidate.operation.predecessor;
    }
    let restored = ChannelSync::restore_owner_operation_checkpoint(
        &bytes,
        Hub::new().join(PeerId::from_u64(0xae02)),
        ChaCha20Rng::seed_from_u64(0xae02),
        Box::new(ManualClock::new(1_000_000)),
        limits,
    )
    .unwrap();
    restored
        .validate_owner_initialization_chain(binding.profile, &binding.signer, &binding.signer)
        .unwrap();
    assert!(
        restored.owner_evidence_unbound(),
        "data validation never binds a runtime"
    );
    assert!(restored.durable_send_context().is_err());
}

#[test]
fn first_add_same_invite_and_package_cannot_choose_another_channel() {
    let (mut node, _binding, candidate) = fixture();
    let invite = InviteToken::decode(&candidate.operation.invite).unwrap();
    assert!(node
        .prepare_owner_direct_add_checkpoint(
            &invite,
            &candidate.operation.package,
            10,
            candidate.operation.predecessor,
            512 * 1024,
            512 * 1024
        )
        .is_err());
    assert_eq!(node.epoch(), 0);
    assert!(candidate.operation.exact_request(
        9,
        &candidate.operation.invite,
        &candidate.operation.package
    ));
    assert!(!candidate.operation.exact_request(
        10,
        &candidate.operation.invite,
        &candidate.operation.package
    ));
}

#[test]
fn first_add_generic_restore_refuses_oversized_separate_log_before_sdk_header_or_tail_encoding() {
    let (mut initial, _binding, candidate) = fixture();
    let original = freeze(&mut initial, &candidate).unwrap();
    let mut oversized = CommitRecord::decode(&candidate.commit.encode()).unwrap();
    oversized
        .mls_commit
        .resize(operation_tail::MAX_COMMIT + 1, 0);
    let replacement = oversized.encode();
    let (start, end) = {
        let mut d = Decoder::new(&original);
        for _ in 0..3 {
            d.get_bytes().unwrap();
        }
        assert_eq!(d.get_u32().unwrap(), 0);
        assert_eq!(d.get_u32().unwrap(), 1);
        let record = d.get_bytes().unwrap();
        let start = record.as_ptr() as usize - original.as_ptr() as usize - 4;
        (start, start + 4 + record.len())
    };
    let mut altered = Vec::new();
    altered.extend_from_slice(&original[..start]);
    altered.extend_from_slice(&(replacement.len() as u32).to_be_bytes());
    altered.extend_from_slice(&replacement);
    altered.extend_from_slice(&original[end..]);
    assert!(matches!(
        ChannelSync::restore(
            &altered,
            Hub::new().join(PeerId::from_u64(0xae09)),
            ChaCha20Rng::seed_from_u64(0xae09),
            Box::new(ManualClock::new(1_000))
        ),
        Err(SyncError::OwnerInitializationUnsupported)
    ));
    // The typed unsupported-cap refusal occurs before ordinary header rejection of this
    // deliberately non-decryptable enlarged record; old behavior returned Malformed later.
}

fn restored_outcome(hub: &Arc<Hub>) -> (Node, Binding) {
    let (mut initial, binding, candidate) = fixture();
    let bytes = freeze(&mut initial, &candidate).unwrap();
    let mut node = ChannelSync::restore_owner_operation_checkpoint(
        &bytes,
        hub.join(PeerId::from_u64(0xae03)),
        ChaCha20Rng::seed_from_u64(0xae03),
        Box::new(ManualClock::new(1_000_000)),
        OwnerInitializationRestoreLimits {
            max_snapshot_bytes: 24 * 1024 * 1024,
            max_mls_bytes: 512 * 1024,
            max_mls_backing_bytes: 512 * 1024,
        },
    )
    .unwrap();
    // Explicit TEST trusted-host contract only; native genuine Saved tests live in App.
    node.bind_managed_chat_scope(&binding).unwrap();
    node.bind_managed_owner_evidence(&binding).unwrap();
    (node, binding)
}

#[tokio::test]
async fn first_add_commit_debt_survives_no_peers_and_submits_exact_original_over_memory_transport()
{
    let hub = Hub::new();
    let (mut node, binding) = restored_outcome(&hub);
    assert!(node.owner_commit_submission_owed());
    assert_eq!(
        node.publish_saved_owner_operation_checkpoint(
            binding.publication_authority(),
            Arc::new(())
        )
        .await,
        Err(catcoms_rt::PublishOnceError::NoPeers)
    );
    assert!(node.owner_commit_submission_owed());
    let operation = node.owner_operation.as_ref().unwrap();
    let request = operation.binding;
    let topic = Topic::new(Bytes::copy_from_slice(&operation.topic));
    let record = node.commit_log.front().unwrap().encode();
    let recipient = hub.join(PeerId::from_u64(0xae04));
    recipient.subscribe(topic.clone()).await.unwrap();
    assert_eq!(
        node.publish_saved_owner_operation_checkpoint(
            binding.publication_authority(),
            Arc::new(())
        )
        .await,
        Ok(true)
    );
    let Some(TransportEvent::Gossip {
        topic: actual,
        data,
        ..
    }) = recipient.next_event().await
    else {
        panic!("actual eligible subscriber must receive original commit");
    };
    assert_eq!(actual, topic);
    assert_eq!(data.first(), Some(&CTRL_COMMIT));
    assert_eq!(&data[1..], record.as_slice());
    assert!(!node.owner_commit_submission_owed());
    assert_eq!(
        node.publish_saved_owner_operation_checkpoint(
            binding.publication_authority(),
            Arc::new(())
        )
        .await,
        Ok(false)
    );
    assert_eq!(
        node.owner_operation_request_id(),
        Some(request),
        "local submission is not durable debt deletion"
    );
    assert_eq!(node.commit_log.len(), 1);
}

#[tokio::test]
async fn first_add_commit_debt_refuses_original_revocation_and_oversize_before_encoding() {
    let hub = Hub::new();
    let (mut node, binding) = restored_outcome(&hub);
    let effect = binding.publication_authority();
    binding.owner.send_replace(true);
    assert_eq!(
        node.publish_saved_owner_operation_checkpoint(effect, Arc::new(()))
            .await,
        Err(catcoms_rt::PublishOnceError::Revoked)
    );
    assert!(node.owner_commit_submission_owed());
    let (mut node, binding) = restored_outcome(&hub);
    node.commit_log
        .front_mut()
        .unwrap()
        .mls_commit
        .resize(catcoms_rt::MAX_PUBLISH_ONCE_BYTES, 0);
    let oversized_id = node.commit_log.front().unwrap().commit_id();
    Arc::get_mut(node.owner_operation.as_mut().unwrap())
        .unwrap()
        .commit = oversized_id;
    assert_eq!(
        node.publish_saved_owner_operation_checkpoint(
            binding.publication_authority(),
            Arc::new(())
        )
        .await,
        Err(catcoms_rt::PublishOnceError::TooLarge)
    );
    assert!(node.owner_commit_submission_owed());
}
