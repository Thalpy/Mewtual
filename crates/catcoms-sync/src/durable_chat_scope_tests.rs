//! Explicit trusted-host fixtures for the dormant sync boundary. These do not claim that native
//! mounted-store activation or saved-before-exposure integration has already been implemented.
use super::*;
use automerge::{transaction::Transactable, ROOT};
use catcoms_rt::{Hub, ManualClock, MemNetwork, RequestCancellation};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use tokio::sync::watch;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;

struct Binding {
    group: Vec<u8>,
    signer: [u8; 32],
    domain: [u8; 32],
    nonce: [u8; 32],
    owner: watch::Sender<bool>,
}
impl Binding {
    fn fixture(node: &Node, nonce: u8) -> Self {
        Self {
            group: node.group_id(),
            signer: node.my_public_key().try_into().unwrap(),
            domain: [0x92; 32],
            nonce: [nonce; 32],
            owner: watch::channel(false).0,
        }
    }
    fn stage_and_bind(&self, node: &mut Node) {
        node.stage_managed_chat_scope(self.domain, self.nonce)
            .unwrap();
        node.bind_managed_chat_scope(self).unwrap();
    }
}
impl local_chat_scope::ManagedChatScopeBinding for Binding {
    fn group(&self) -> &[u8] {
        &self.group
    }
    fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
    fn domain(&self) -> [u8; 32] {
        self.domain
    }
    fn nonce(&self) -> [u8; 32] {
        self.nonce
    }
    fn publication_authority(&self) -> RequestCancellation {
        RequestCancellation::new(self.owner.subscribe(), None)
    }
}
fn node(hub: &Arc<Hub>, id: u64) -> Node {
    ChannelSync::found(
        hub.join(PeerId::from_u64(id)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(id),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap()
}
fn restore(snapshot: &[u8], hub: &Arc<Hub>, id: u64) -> Node {
    ChannelSync::restore(
        snapshot,
        hub.join(PeerId::from_u64(id)),
        ChaCha20Rng::seed_from_u64(id),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap()
}
fn prepare(node: &mut Node, token: u8) -> PreparedChat {
    let context = node.durable_send_context().unwrap();
    node.prepare_durable_chat(
        [token; 16],
        [token; 32],
        context,
        5,
        format!("m{token}"),
        |doc| doc.put(ROOT, format!("m{token}"), "original"),
    )
    .unwrap()
}
fn advance(node: &mut Node, count: u8) {
    for _ in 0..count {
        let member = MlsDevice::generate().unwrap();
        let package = member.key_package().unwrap();
        node.snapshot_epoch_keys();
        node.with_observed_mls_transition(|node| node.group.add_member(&node.device, package))
            .unwrap();
        node.evict_past_keys();
    }
}

#[tokio::test]
async fn stable_preparation_keeps_original_across_churn_restart_and_expired_key_window() {
    let hub = Hub::new();
    let mut n = node(&hub, 901);
    n.open_channel(DocType::Channel, 5).await.unwrap();
    let binding = Binding::fixture(&n, 1);
    binding.stage_and_bind(&mut n);
    let context = n.durable_send_context().unwrap();
    let first = prepare(&mut n, 1);
    let sealed = n.durable_chat.records[&[1; 16]].sealed.clone();
    assert!(
        n.durable_chat.v2,
        "stable Sends retain their original in the existing v2 format"
    );
    advance(&mut n, 10);
    assert_eq!(context, n.durable_send_context().unwrap());
    let snapshot = n.snapshot().unwrap();
    let mut restarted = restore(&snapshot, &hub, 902);
    assert!(restarted.durable_send_context().is_err());
    assert!(matches!(
        restarted
            .post(DocType::Channel, 5, |_| panic!("unbound must not edit"))
            .await,
        Err(SyncError::ChatScopeUnavailable)
    ));
    restarted.bind_managed_chat_scope(&binding).unwrap();
    assert_eq!(context, restarted.durable_send_context().unwrap());
    assert_eq!(first.change, prepare(&mut restarted, 1).change);
    assert_eq!(sealed, restarted.durable_chat.records[&[1; 16]].sealed);
    restarted
        .commit_durable_chat([1; 16], |_, _| Ok(()))
        .unwrap();
    assert_eq!(restarted.doc(DocType::Channel, 5).unwrap().op_count(), 1);
    assert_eq!(sealed, restarted.durable_chat.records[&[1; 16]].sealed);
    let sealed = SealedOp::decode(&sealed).unwrap();
    assert!(
        EncryptedDoc::capture_local_chat_op(
            &sealed,
            &restarted.group,
            &restarted.device,
            first.change.unwrap()
        )
        .is_err(),
        "commit did not recover by pretending the old epoch is current"
    );
}

#[tokio::test]
async fn first_scope_initialization_after_unrelated_epochs_keeps_attached_admission_provenance() {
    let hub = Hub::new();
    let mut n = node(&hub, 930);
    let attempt = n.fresh_chat_admission().unwrap().attempt_id();
    n.open_channel(DocType::Channel, 5).await.unwrap();
    advance(&mut n, 10);
    assert_eq!(n.fresh_chat_admission().unwrap().attempt_id(), attempt);
    assert!(n.chat_scope_label().is_none());
    let binding = Binding::fixture(&n, 10);
    binding.stage_and_bind(&mut n);
    let context = n.durable_send_context().unwrap();
    assert_eq!(n.fresh_chat_admission().unwrap().attempt_id(), attempt);
    prepare(&mut n, 1);
    n.commit_durable_chat([1; 16], |_, _| Ok(())).unwrap();
    assert_eq!(context, n.durable_send_context().unwrap());
}

#[tokio::test]
async fn accepted_v1_migrates_exact_publication_but_unaccepted_and_stopped_requests_do_not() {
    let hub = Hub::new();
    let mut n = node(&hub, 903);
    n.open_channel(DocType::Channel, 5).await.unwrap();
    n.open_channel(DocType::Channel, 6).await.unwrap();
    let prior_context = n.durable_send_context().unwrap();
    n.prepare_durable_chat([3; 16], [3; 32], prior_context, 6, "m3".into(), |doc| {
        doc.put(ROOT, "m3", "prior accepted")
    })
    .unwrap();
    n.commit_durable_chat([3; 16], |_, _| Ok(())).unwrap();
    advance(&mut n, 1);
    let next_context = n.durable_send_context().unwrap();
    n.durable_chat.stop_obsolete(Some(&next_context));
    let old_context = n.durable_send_context().unwrap();
    prepare(&mut n, 1);
    n.commit_durable_chat([1; 16], |_, _| Ok(())).unwrap();
    prepare(&mut n, 2);
    let original_v1 = n.durable_chat.encode().unwrap();
    assert_eq!(original_v1[0], 1);
    let snapshot = n.snapshot().unwrap();
    let mut n = restore(&snapshot, &hub, 904);
    assert!(
        matches!(
            n.durable_chat.records[&[1; 16]].effect,
            ChatEffect::Signed { original: None, .. }
        ),
        "fixture really lost private originals in v1"
    );
    let receipt = n.durable_chat.records[&[1; 16]].clone();
    advance(&mut n, 10);
    let binding = Binding::fixture(&n, 2);
    n.require_managed_chat_scope();
    n.stage_managed_chat_scope(binding.domain, binding.nonce)
        .unwrap();
    assert_eq!(
        original_v1,
        n.durable_chat.encode().unwrap(),
        "migration leaves exact v1 receipt bytes unchanged"
    );
    let snapshot = n.snapshot().unwrap();
    let mut n = restore(&snapshot, &hub, 905);
    n.drain_durable_chat().await;
    assert_eq!(receipt.sealed, n.durable_chat.records[&[1; 16]].sealed);
    assert!(
        !n.durable_chat.records[&[1; 16]].stopped,
        "unbound restore retains accepted publication"
    );
    n.bind_managed_chat_scope(&binding).unwrap();
    assert_ne!(old_context, n.durable_send_context().unwrap());
    let replay = n
        .durable_chat_lookup([1; 16], [1; 32], old_context, 5)
        .unwrap()
        .unwrap();
    assert!(replay.durable && replay.replayed);
    assert!(n
        .prepare_durable_chat([2; 16], [2; 32], old_context, 5, "m2".into(), |_| panic!(
            "unaccepted legacy intent cannot be upgraded"
        ))
        .is_err());
    assert!(n.durable_chat.records[&[2; 16]].stopped);
    assert!(n.durable_chat.records[&[3; 16]].stopped);
    let peer = hub.join(PeerId::from_u64(906));
    let topic = n
        .channel_topic_for(DocType::Channel, 5, n.routing_label)
        .unwrap();
    peer.subscribe(topic).await.unwrap();
    n.drain_durable_chat().await;
    let Some(TransportEvent::Gossip { data, .. }) = peer.next_event().await else {
        panic!("original publication");
    };
    assert_eq!(receipt.sealed, data.as_ref());
    let migrated = &n.durable_chat.records[&[1; 16]];
    assert_eq!(receipt.binding, migrated.binding);
    assert_eq!(receipt.context, migrated.context);
    assert_eq!(receipt.message_id, migrated.message_id);
    assert!(migrated.sealed.is_empty());
    assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 1);
}

#[tokio::test]
async fn managed_no_tail_classification_and_retired_or_revoked_runtime_refuse_lower_authoring() {
    let hub = Hub::new();
    let mut n = node(&hub, 907);
    n.open_channel(DocType::Channel, 5).await.unwrap();
    let legacy = n.snapshot().unwrap();
    let mut restored = restore(&legacy, &hub, 908);
    restored.require_managed_chat_scope();
    assert!(restored.chat_scope_label().is_none());
    assert!(restored.durable_send_context().is_err());
    assert!(matches!(
        restored
            .post(DocType::Channel, 5, |_| panic!(
                "no-tail managed restore is unbound"
            ))
            .await,
        Err(SyncError::ChatScopeUnavailable)
    ));
    let binding = Binding::fixture(&restored, 3);
    binding.stage_and_bind(&mut restored);
    let context = restored.durable_send_context().unwrap();
    let authority = restored.local_chat_scope.authority().unwrap();
    binding.owner.send_replace(true);
    assert!(authority.is_cancelled());
    assert!(restored.durable_send_context().is_err());
    assert!(restored.bind_managed_chat_scope(&binding).is_err());
    let replacement = Binding::fixture(&restored, 3);
    assert!(
        restored.bind_managed_chat_scope(&replacement).is_err(),
        "same Actor cannot remint revoked custody"
    );
    let mut new_owner = restore(&restored.snapshot().unwrap(), &hub, 909);
    new_owner.bind_managed_chat_scope(&replacement).unwrap();
    assert_eq!(context, new_owner.durable_send_context().unwrap());
    new_owner.retire_managed_chat_scope();
    assert!(new_owner.chat_scope_label().unwrap().retired());
    assert!(new_owner.bind_managed_chat_scope(&replacement).is_err());
    let mut retired = restore(&new_owner.snapshot().unwrap(), &hub, 910);
    assert!(retired.bind_managed_chat_scope(&replacement).is_err());
    assert!(retired.durable_send_context().is_err());
}

#[tokio::test]
async fn scope_tail_is_strict_and_changes_context_only_for_actual_scope_identity() {
    let hub = Hub::new();
    let mut n = node(&hub, 911);
    let legacy = n.snapshot().unwrap();
    assert_eq!(legacy.as_slice(), n.snapshot().unwrap().as_slice());
    let binding = Binding::fixture(&n, 4);
    binding.stage_and_bind(&mut n);
    let context = n.durable_send_context().unwrap();
    let snapshot = n.snapshot().unwrap();
    assert!(
        snapshot.starts_with(&legacy),
        "legacy prefix and v1 empty receipt bytes are unchanged"
    );
    let old_reader = Decoder::new(&snapshot[legacy.len()..]);
    assert!(
        old_reader.finish().is_err(),
        "old strict framing refuses even an empty scope tail"
    );
    assert!(ChannelSync::restore(
        &snapshot[..snapshot.len() - 1],
        hub.join(PeerId::from_u64(912)),
        ChaCha20Rng::seed_from_u64(912),
        Box::new(ManualClock::new(1))
    )
    .is_err());
    let mut unknown = snapshot.to_vec();
    let marker = unknown
        .windows(b"catcoms/local-chat-scope/v1".len())
        .position(|bytes| bytes == b"catcoms/local-chat-scope/v1")
        .unwrap();
    unknown[marker + b"catcoms/local-chat-scope/v".len()] = b'9';
    assert!(ChannelSync::restore(
        &unknown,
        hub.join(PeerId::from_u64(913)),
        ChaCha20Rng::seed_from_u64(913),
        Box::new(ManualClock::new(1))
    )
    .is_err());
    let mut same = restore(&legacy, &hub, 914);
    let newer = Binding::fixture(&same, 5);
    newer.stage_and_bind(&mut same);
    assert_ne!(
        context,
        same.durable_send_context().unwrap(),
        "durable lineage nonce reset changes context even with repeated entropy"
    );
    let mut different = restore(&legacy, &hub, 915);
    let mut other_vault = Binding::fixture(&different, 4);
    other_vault.domain = [0x93; 32];
    other_vault.stage_and_bind(&mut different);
    assert_ne!(context, different.durable_send_context().unwrap());
}

#[tokio::test]
async fn pending_stop_blocks_migration_until_original_negative_repair_covers_it() {
    let hub = Hub::new();
    let mut n = node(&hub, 916);
    n.open_channel(DocType::Channel, 5).await.unwrap();
    let context = n.durable_send_context().unwrap();
    prepare(&mut n, 1);
    n.commit_durable_chat([1; 16], |_, _| Ok(())).unwrap();
    assert!(n
        .stop_durable_chat(
            [1; 16],
            [1; 32],
            context,
            5,
            "m1".into(),
            ChatAction::Send,
            None,
            |_, _| Err(ChatSaveError::ReplacedNotDurable("held parent sync".into()))
        )
        .is_err());
    assert!(n.durable_chat_needs_repair());
    n.require_managed_chat_scope();
    let binding = Binding::fixture(&n, 6);
    assert!(matches!(
        n.stage_managed_chat_scope(binding.domain, binding.nonce),
        Err(SyncError::ChatPreparationPending)
    ));
    assert!(n.chat_scope_label().unwrap().initializing());
    n.drain_durable_chat().await;
    n.repair_durable_chat(|_, _| Ok(())).unwrap();
    binding.stage_and_bind(&mut n);
    let replay = n
        .durable_chat_lookup([1; 16], [1; 32], context, 5)
        .unwrap()
        .unwrap();
    assert_eq!(replay.disposition, ChatDisposition::StoppedAfterCommit);
    assert!(n.durable_chat.records[&[1; 16]].sealed.is_empty());
    assert_eq!(n.doc(DocType::Channel, 5).unwrap().op_count(), 1);
    let snapshot = n.snapshot().unwrap();
    let restarted = restore(&snapshot, &hub, 917);
    assert_eq!(
        restarted
            .durable_chat_lookup([1; 16], [1; 32], context, 5)
            .unwrap()
            .unwrap()
            .disposition,
        ChatDisposition::StoppedAfterCommit
    );
}

#[tokio::test]
async fn scope_tail_cannot_omit_an_accepted_original_or_bind_wrong_full_identity() {
    let hub = Hub::new();
    let mut n = node(&hub, 918);
    n.open_channel(DocType::Channel, 5).await.unwrap();
    prepare(&mut n, 1);
    n.commit_durable_chat([1; 16], |_, _| Ok(())).unwrap();
    let binding = Binding::fixture(&n, 7);
    n.stage_managed_chat_scope(binding.domain, binding.nonce)
        .unwrap();
    let mut wrong = Binding::fixture(&n, 7);
    wrong.signer[0] ^= 1;
    assert!(n.bind_managed_chat_scope(&wrong).is_err());
    wrong.signer = binding.signer;
    wrong.group.push(0);
    assert!(n.bind_managed_chat_scope(&wrong).is_err());
    let encoded_scope = n
        .local_chat_scope
        .encode(&n.group_id(), &n.my_public_key(), &n.durable_chat)
        .unwrap()
        .unwrap();
    let snapshot = n.snapshot().unwrap();
    let prefix = &snapshot[..snapshot.len() - encoded_scope.len() - 4];
    let mut tail = Encoder::new();
    tail.put_str("catcoms/local-chat-scope/v1").unwrap();
    tail.put_bytes(&n.group_id()).unwrap();
    tail.put_bytes(&n.my_public_key()).unwrap();
    tail.put_bytes(&binding.domain).unwrap();
    tail.put_bytes(&binding.nonce).unwrap();
    tail.put_u8(0);
    tail.put_u32(0);
    let mut omitted = prefix.to_vec();
    let mut framed = Encoder::new();
    framed.put_bytes(&tail.finish()).unwrap();
    omitted.extend(framed.finish());
    assert!(
        ChannelSync::restore(
            &omitted,
            hub.join(PeerId::from_u64(919)),
            ChaCha20Rng::seed_from_u64(919),
            Box::new(ManualClock::new(1))
        )
        .is_err(),
        "missing alias must refuse whole promotion, never silently stop accepted v1 work"
    );
    assert!(n.durable_chat.records[&[1; 16]].durable);
    assert!(!n.durable_chat.records[&[1; 16]].stopped);
}

#[tokio::test]
async fn actual_applied_removal_revokes_original_publication_even_when_transition_unwinds() {
    let hub = Hub::new();
    let mut owner = node(&hub, 920);
    let invite = owner.mint_invite([0xa1; 16], 10_000, Vec::new()).unwrap();
    let device = MlsDevice::generate().unwrap();
    let package = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let expected = device.key_package_reference(&package).unwrap();
    let (welcome, routing, signature) = owner
        .admit_now(&invite, &serialize_key_package(&package).unwrap(), 1_000)
        .unwrap();
    let (group, routing) =
        finish_join(&device, &invite, &welcome, &signature, &routing, &expected).unwrap();
    let mut member = ChannelSync::new_joined(
        hub.join(PeerId::from_u64(921)),
        group,
        device,
        ChaCha20Rng::seed_from_u64(921),
        Box::new(ManualClock::new(1_000)),
        routing,
    );
    let binding = Binding::fixture(&member, 8);
    binding.stage_and_bind(&mut member);
    let snapshot = member.snapshot().unwrap();
    let authority = member.local_chat_scope.authority().unwrap();
    // remove() only stages the signed contest entry. Close the injected-clock window and
    // run its real completion seam before selecting the retained removal record.
    owner.config.stage_decision_window_ms = 0;
    let before = owner.epoch();
    owner.remove(&member.device_id()).await.unwrap();
    let staged = owner.pending.as_ref().unwrap().best.clone();
    assert_eq!(staged.commit_epoch, member.epoch());
    assert!(owner.resolve_pending_if_expired());
    assert_eq!(owner.epoch(), before + 1);
    assert!(!owner.group.contains_device(&member.device_id()));
    let record = owner.commit_log.back().unwrap().clone();
    assert_eq!(record.commit_id(), staged.commit_id());
    assert!(member.apply_commit_in_order(&record));
    assert!(authority.is_cancelled());
    assert!(member.chat_scope_label().unwrap().retired());
    assert!(member.fresh_chat_admission().is_none());
    assert!(member.durable_send_context().is_err());

    let mut panicking = restore(&snapshot, &hub, 922);
    panicking.bind_managed_chat_scope(&binding).unwrap();
    let authority = panicking.local_chat_scope.authority().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        panicking.with_observed_mls_transition(|node| {
            node.group
                .process_incoming(&node.device, &record.mls_commit)
                .unwrap();
            panic!("after actual MLS removal");
        });
    }));
    assert!(result.is_err());
    assert!(
        authority.is_cancelled(),
        "panic cannot retain a queued original permit"
    );
    assert!(panicking.chat_scope_label().unwrap().retired());
}

#[tokio::test]
async fn initializing_tail_resumes_bounded_original_recovery_without_bootstrapping_another_nonce() {
    let hub = Hub::new();
    let mut n = node(&hub, 923);
    n.open_channel(DocType::Channel, 5).await.unwrap();
    prepare(&mut n, 1);
    n.commit_durable_chat([1; 16], |_, _| Ok(())).unwrap();
    for index in 0..catcoms_replication::doc::MAX_LOCAL_CHAT_RECOVERY_SCANNED_OPS + 2 {
        n.post(DocType::Channel, 5, |doc| {
            doc.put(ROOT, "later", index as u64)
        })
        .await
        .unwrap();
    }
    let original = n.durable_chat.records[&[1; 16]].sealed.clone();
    let snapshot = n.snapshot().unwrap();
    let mut n = restore(&snapshot, &hub, 924);
    let binding = Binding::fixture(&n, 9);
    n.require_managed_chat_scope();
    assert!(matches!(
        n.stage_managed_chat_scope(binding.domain, binding.nonce),
        Err(SyncError::ChatScopeMigrationPending)
    ));
    let label = n.chat_scope_label().unwrap();
    assert!(label.initializing());
    assert_eq!(label.nonce(), binding.nonce);
    assert!(n.bind_managed_chat_scope(&binding).is_err());
    let initializing = n.snapshot().unwrap();
    let mut n = restore(&initializing, &hub, 925);
    assert!(n.chat_scope_label().unwrap().initializing());
    assert_eq!(n.chat_scope_label().unwrap().nonce(), binding.nonce);
    n.drain_durable_chat().await;
    assert_eq!(n.durable_chat.records[&[1; 16]].sealed, original);
    assert!(!n.durable_chat.records[&[1; 16]].stopped);
    assert!(matches!(
        n.stage_managed_chat_scope(binding.domain, binding.nonce),
        Err(SyncError::ChatScopeMigrationPending)
    ));
    n.stage_managed_chat_scope(binding.domain, binding.nonce)
        .unwrap();
    assert!(!n.chat_scope_label().unwrap().initializing());
    n.bind_managed_chat_scope(&binding).unwrap();
    assert_eq!(
        n.doc(DocType::Channel, 5).unwrap().op_count(),
        catcoms_replication::doc::MAX_LOCAL_CHAT_RECOVERY_SCANNED_OPS + 3
    );
    assert_eq!(n.durable_chat.records[&[1; 16]].sealed, original);
}
