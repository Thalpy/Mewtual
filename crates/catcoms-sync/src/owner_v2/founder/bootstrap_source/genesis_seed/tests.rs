//! Real creation -> sealed source -> actual closed SDK loader. Authored UNRUN under the
//! user compilation/test/build pause; resource fixtures are not Saved/Current authorities.
use super::*;
use crate::local_chat_scope::ManagedChatScopeRevocation;
use catcoms_mls::persist::{OwnerSdkGraphReservation, OwnerSdkRestoreLimits};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId, RequestCancellation};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use std::{
    cell::Cell,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Weak,
    },
};
use tokio::sync::watch;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
const KEY: [u8; 32] = [193; 32];
const DOMAIN: [u8; 32] = [194; 32];
const NONCE: [u8; 32] = [195; 32];
struct Reservation {
    group: Vec<u8>,
    signer: [u8; 32],
    identity: Arc<()>,
    owner: Arc<watch::Sender<bool>>,
    domain: [u8; 32],
    nonce: [u8; 32],
    revision: Option<u64>,
    reads: Cell<usize>,
}
struct Revoker(Weak<watch::Sender<bool>>);
impl ManagedChatScopeRevocation for Revoker {
    fn revoke(&self) {
        if let Some(owner) = self.0.upgrade() {
            owner.send_replace(true);
        }
    }
}
impl ManagedChatScopeReservation for Reservation {
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
    fn planned_revision_data(&self) -> Option<u64> {
        self.reads.set(self.reads.get() + 1);
        self.revision
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
impl Reservation {
    fn replacement(&self) -> Self {
        Self {
            group: self.group.clone(),
            signer: self.signer,
            identity: Arc::new(()),
            owner: Arc::new(watch::channel(false).0),
            domain: self.domain,
            nonce: self.nonce,
            revision: self.revision,
            reads: Cell::new(0),
        }
    }
}
struct Fixture {
    node: Node,
    reservation: Reservation,
    source: FounderBootstrapPreparation,
    opened: OpenedBootstrapSourceData,
}
fn fixture(id: u64) -> Fixture {
    let mut node = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(id)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(id),
        Box::new(ManualClock::new(1000)),
    )
    .unwrap();
    node.rendezvous_nodes = vec![
        ("/original/one".into(), vec![1, 2, 3]),
        ("/original/two".into(), vec![4, 5]),
    ];
    node.initialize_new_group_policy().unwrap();
    let reservation = Reservation {
        group: node.group_id(),
        signer: node.my_public_key().try_into().unwrap(),
        identity: Arc::new(()),
        owner: Arc::new(watch::channel(false).0),
        domain: DOMAIN,
        nonce: NONCE,
        revision: Some(37),
        reads: Cell::new(0),
    };
    node.stage_managed_chat_scope(DOMAIN, NONCE).unwrap();
    node.attach_managed_chat_reservation(&reservation).unwrap();
    let mut frozen = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    frozen
        .cache_private_snapshot(
            &mut node,
            FounderSnapshotLimits::new(1024 * 1024, 512 * 1024, 512 * 1024).unwrap(),
        )
        .unwrap();
    let mut archive = FounderArchivePreparation::from_cached_source(frozen, &node, DOMAIN).unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(id + 1);
    archive.prepare(&node, &KEY, &mut rng).unwrap();
    let mut source = FounderBootstrapPreparation::from_archive(archive, &node).unwrap();
    source.prepare(&node, &KEY, &mut rng).unwrap();
    let detached = source.take_prepared_for_handoff(&node, &KEY).unwrap();
    let opened = open_bootstrap_source_data(
        &KEY,
        detached.context_data(),
        detached.id(),
        detached.as_bytes(),
    )
    .unwrap();
    Fixture {
        node,
        reservation,
        source,
        opened,
    }
}
struct Fee(Arc<AtomicUsize>);
impl OwnerSdkGraphReservation for Fee {
    fn limits(&self) -> OwnerSdkRestoreLimits {
        OwnerSdkRestoreLimits {
            max_typed_bytes: 128 * 1024,
            max_work: 512 * 1024 * 1024,
        }
    }
}
impl Drop for Fee {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
fn admission(count: &Arc<AtomicUsize>) -> OwnerSdkRestoreAdmission {
    count.fetch_add(1, Ordering::SeqCst);
    OwnerSdkRestoreAdmission::new(Fee(count.clone())).unwrap()
}
fn limits() -> V2GenesisSeedLimits {
    V2GenesisSeedLimits::new(1024 * 1024, 512 * 1024, 512 * 1024).unwrap()
}
fn seed(opened: &OpenedBootstrapSourceData, count: &Arc<AtomicUsize>) -> UnboundV2GenesisSeed {
    prepare_v2_genesis_seed(opened, limits(), admission(count)).unwrap()
}

#[test]
fn actual_opener_retains_exact_genesis_and_seed_preserves_complete_allowed_metadata() {
    let f = fixture(0x61a0);
    let count = Arc::new(AtomicUsize::new(0));
    let seed = seed(&f.opened, &count);
    let data = seed.data();
    let spec = f.opened.view_data().context.spec_data();
    assert_eq!(data.source, f.opened.source_id_data());
    assert_eq!(data.profile.id(), spec.profile);
    assert_eq!(data.admission.id(), spec.admission);
    assert_eq!(data.header.id(), spec.header);
    assert_eq!(data.state, f.opened.genesis.image.as_slice());
    assert_eq!(data.profile, &f.opened.genesis.profile);
    assert_eq!(data.header, &f.opened.genesis.header);
    assert_eq!(data.tip, f.opened.genesis.frontier.tip());
    assert_eq!(data.tip.count, 0);
    assert_eq!(data.scope.domain(), DOMAIN);
    assert_eq!(data.scope.nonce(), NONCE);
    assert!(!data.scope.retired() && !data.scope.initializing());
    assert_eq!(data.routing_label, f.node.routing_label);
    assert_eq!(data.routing_slots, f.node.routing_secrets.len());
    assert_eq!(data.rendezvous_rows, 2);
    for (loaded, original) in seed
        .body
        .metadata
        .rendezvous
        .iter()
        .zip(&f.node.rendezvous_nodes)
    {
        assert_eq!(&loaded.0, &original.0);
        assert_eq!(loaded.1.as_slice(), original.1.as_slice());
    }
    assert_eq!(*seed.body.metadata._file_wrap_key, *f.node.file_wrap_key);
    assert!(data.has_policy);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    drop(seed);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn every_configured_allowance_one_byte_short_refuses_before_sdk_and_preserves_source() {
    let f = fixture(0x61a1);
    let original = f.opened.view_data().snapshot.to_vec();
    let count = Arc::new(AtomicUsize::new(0));
    for (snapshot, mls, backing) in [
        (1024 * 1024 - 1, 512 * 1024, 512 * 1024),
        (1024 * 1024, 512 * 1024 - 1, 512 * 1024),
        (1024 * 1024, 512 * 1024, 512 * 1024 - 1),
    ] {
        SDK_ENTRIES.with(|n| n.set(0));
        let limits = V2GenesisSeedLimits::new(snapshot, mls, backing).unwrap();
        assert!(prepare_v2_genesis_seed(&f.opened, limits, admission(&count)).is_err());
        assert_eq!(SDK_ENTRIES.with(Cell::get), 0);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(f.opened.view_data().snapshot, original);
    }
    assert!(V2GenesisSeedLimits::new(0, 0, 0).is_err());
}

#[test]
fn sdk_payment_survives_each_independent_half_and_two_separate_loads_need_two_fees() {
    let f = fixture(0x61a2);
    let count = Arc::new(AtomicUsize::new(0));
    let a = seed(&f.opened, &count);
    let b = seed(&f.opened, &count);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let SeedBody { device, group, .. } = *a.body;
    drop(device);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    drop(group);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let SeedBody { device, group, .. } = *b.body;
    drop(group);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    drop(device);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn warm_move_preserves_actual_runtime_identity_rng_and_original_fresh_owner() {
    let f = fixture(0x61a3);
    let count = Arc::new(AtomicUsize::new(0));
    let mut seed = seed(&f.opened, &count);
    seed.prepare_scope(&f.reservation).unwrap();
    assert_eq!(
        f.reservation.reads.get(),
        2,
        "once at original attachment and once at new attachment"
    );
    let transport = f.node.transport.clone();
    let clock = f.node.clock.clone();
    let mut expected = f.node.rng.clone();
    let source = f.opened.source_id_data();
    let parts = f.source.into_genesis_runtime_parts(f.node, source).unwrap();
    assert!(Arc::ptr_eq(&transport, &parts.body._transport));
    assert!(Arc::ptr_eq(&clock, &parts.body._clock));
    let mut actual = parts.body._rng.clone();
    assert_eq!(actual.next_u64(), expected.next_u64());
    assert!(parts
        .body
        .warm
        .as_ref()
        .unwrap()
        .fresh
        .matches_genesis_restored(
            parts.body.fresh.as_ref().unwrap(),
            &seed.body.group,
            &seed.body.context.spec_data().signer,
        ));
    let prepared = seed.prepare_runtime(parts).unwrap();
    let transport_count = Arc::strong_count(&transport);
    let node = prepared.into_node();
    assert_eq!(Arc::strong_count(&transport), transport_count);
    assert_eq!(node.data().source, source);
    assert!(node._parts.body.fresh.is_some());
    drop(node);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(Arc::strong_count(&transport), 1);
}

#[test]
fn warm_wrong_id_refusal_returns_same_original_sync_and_source_without_rng_change() {
    let f = fixture(0x61a4);
    let transport = f.node.transport.clone();
    let mut expected = f.node.rng.clone();
    let original_id = f.opened.source_id_data();
    let (source, node) = f
        .source
        .into_genesis_runtime_parts(f.node, BootstrapSourceId::from_bytes([0; 32]))
        .unwrap_err()
        .into_originals();
    assert!(Arc::ptr_eq(&transport, &node.transport));
    let mut actual = node.rng.clone();
    assert_eq!(actual.next_u64(), expected.next_u64());
    assert_eq!(source.transferred, Some(original_id));
    assert!(source.into_genesis_runtime_parts(node, original_id).is_ok());
}

#[test]
fn cancellation_before_warm_extraction_returns_the_originals_without_renewal() {
    let f = fixture(0x61ad);
    let transport = f.node.transport.clone();
    let id = f.opened.source_id_data();
    f.reservation.owner.send_replace(true);
    let (source, node) = f
        .source
        .into_genesis_runtime_parts(f.node, id)
        .unwrap_err()
        .into_originals();
    assert!(Arc::ptr_eq(&transport, &node.transport));
    assert_eq!(source.transferred, Some(id));
    assert!(source.into_genesis_runtime_parts(node, id).is_err());
}

#[test]
fn foreign_seed_pairing_returns_same_preallocated_inputs_then_original_seed_can_pair() {
    let a = fixture(0x61a5);
    let b = fixture(0x61a6);
    let count = Arc::new(AtomicUsize::new(0));
    let mut seed_a = seed(&a.opened, &count);
    seed_a.prepare_scope(&a.reservation).unwrap();
    let mut seed_b = seed(&b.opened, &count);
    seed_b.prepare_scope(&b.reservation).unwrap();
    let id = a.opened.source_id_data();
    let parts = a.source.into_genesis_runtime_parts(a.node, id).unwrap();
    let seed_address = &*seed_b.body as *const SeedBody;
    let parts_address = &*parts.body as *const RuntimeBody<MemNetwork, ChaCha20Rng>;
    let (returned_seed, parts, _) = seed_b.prepare_runtime(parts).unwrap_err().into_originals();
    assert_eq!(&*returned_seed.body as *const SeedBody, seed_address);
    assert_eq!(
        &*parts.body as *const RuntimeBody<MemNetwork, ChaCha20Rng>,
        parts_address
    );
    let node = seed_a.prepare_runtime(parts).unwrap().into_node();
    assert_eq!(node.data().source, id);
}

#[test]
fn same_labels_different_scope_owner_cannot_adopt_warm_runtime_or_renew_attachment() {
    let f = fixture(0x61a7);
    let count = Arc::new(AtomicUsize::new(0));
    let mut seed = seed(&f.opened, &count);
    let replacement = f.reservation.replacement();
    seed.prepare_scope(&replacement).unwrap();
    let id = f.opened.source_id_data();
    let parts = f.source.into_genesis_runtime_parts(f.node, id).unwrap();
    let (mut seed, parts, _) = seed.prepare_runtime(parts).unwrap_err().into_originals();
    assert!(seed.prepare_scope(&f.reservation).is_err());
    assert!(seed.prepare_runtime(parts).is_err());
}

#[test]
fn cancelled_original_after_parts_extraction_never_becomes_a_replacement_scope() {
    let f = fixture(0x61a8);
    let count = Arc::new(AtomicUsize::new(0));
    let mut seed = seed(&f.opened, &count);
    seed.prepare_scope(&f.reservation).unwrap();
    let id = f.opened.source_id_data();
    let parts = f.source.into_genesis_runtime_parts(f.node, id).unwrap();
    f.reservation.owner.send_replace(true);
    let (mut seed, parts, _) = seed.prepare_runtime(parts).unwrap_err().into_originals();
    assert!(seed.prepare_scope(&f.reservation.replacement()).is_err());
    assert!(seed.prepare_runtime(parts).is_err());
}

#[test]
fn cold_parts_are_fresh_free_require_attachment_and_keep_sdk_fee_until_node_drop() {
    let f = fixture(0x61a9);
    let count = Arc::new(AtomicUsize::new(0));
    let seed = seed(&f.opened, &count);
    let parts = V2GenesisRuntimeParts::cold_data(
        f.node.transport.clone(),
        f.node.rng.clone(),
        f.node.clock.clone(),
    );
    assert!(parts.body.fresh.is_none() && parts.body.warm.is_none());
    let (mut seed, parts, _) = seed.prepare_runtime(parts).unwrap_err().into_originals();
    seed.prepare_scope(&f.reservation).unwrap();
    seed.prepare_scope(&f.reservation).unwrap();
    let node = seed.prepare_runtime(parts).unwrap().into_node();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(node._parts.body.fresh.is_none());
    drop(node);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn every_wrong_scope_field_and_dead_observer_refuses_without_attaching() {
    let f = fixture(0x61aa);
    let count = Arc::new(AtomicUsize::new(0));
    for variant in 0..7 {
        let mut seed = seed(&f.opened, &count);
        let mut wrong = f.reservation.replacement();
        match variant {
            0 => wrong.group[0] ^= 1,
            1 => wrong.signer[0] ^= 1,
            2 => wrong.domain[0] ^= 1,
            3 => wrong.nonce[0] ^= 1,
            4 => wrong.revision = Some(38),
            5 => wrong.revision = None,
            _ => {
                wrong.owner.send_replace(true);
            }
        }
        assert!(seed.prepare_scope(&wrong).is_err());
        assert!(!seed.body.metadata.scope.genesis_attachment_live());
        seed.prepare_scope(&f.reservation).unwrap();
    }
}

/// Test-only authenticated source rewriting. The caller deliberately chooses a new expected
/// source/context, so this is DATA, not a reproduction of actual saved-source authority.
/// It pins that even a fully sealed/signed-consistent source cannot widen the initial SDK gate.
fn replace_snapshot(f: &Fixture, snapshot: &[u8]) -> PreparedBootstrapSource {
    let mut raw =
        codec::encode_original_body(&f.source.original, &f.node, &f.source.context).unwrap();
    let mut d = catcoms_wire::Decoder::new(&raw);
    d.get_bytes().unwrap();
    d.get_u16().unwrap();
    for _ in 0..6 {
        d.get_bytes().unwrap();
    }
    d.get_u64().unwrap();
    for _ in 0..5 {
        d.get_bytes().unwrap();
    }
    for _ in 0..3 {
        d.get_u64().unwrap();
    }
    let digest_at = raw.len() - d.remaining() + 4;
    d.get_bytes().unwrap();
    d.get_u32().unwrap();
    let frame_at = raw.len() - d.remaining();
    d.get_bytes().unwrap();
    let suffix = raw.len() - d.remaining();
    let mut spec = f.source.context.spec_data();
    spec.snapshot_digest = *snapshot::digest(spec.profile, snapshot).as_bytes();
    let context = BootstrapSourceContextData::from_data(spec).unwrap();
    raw[digest_at..digest_at + 32].copy_from_slice(&spec.snapshot_digest);
    let mut updated = Zeroizing::new(Vec::with_capacity(raw.len() + snapshot.len()));
    updated.extend_from_slice(&raw[..frame_at]);
    updated.extend_from_slice(&(snapshot.len() as u32).to_be_bytes());
    updated.extend_from_slice(snapshot);
    updated.extend_from_slice(&raw[suffix..]);
    codec::seal_original_body(
        &KEY,
        &updated,
        context,
        &mut ChaCha20Rng::seed_from_u64(199),
    )
    .unwrap()
}

#[test]
fn late_unknown_tail_bad_rendezvous_and_scope_refuse_in_opener_before_any_sdk_entry() {
    let f = fixture(0x61ab);
    let count = Arc::new(AtomicUsize::new(0));
    let original = f.opened.view_data().snapshot;
    let fields = s0::fields(original, f.opened.view_data().context.spec_data()).unwrap();
    let scope_at = fields.scope.as_ptr() as usize - original.as_ptr() as usize;
    let mut cases = vec![[original, &[0, 0, 0, 0]].concat()];
    let mut changed = original.to_vec();
    // Mandatory scope tag is late, after the full provider and every other metadata field.
    changed[scope_at + 4] ^= 1;
    cases.push(changed);
    let mut changed = original.to_vec();
    let rz = fields.rendezvous[0].unwrap().0;
    let rz_at = rz.as_ptr() as usize - original.as_ptr() as usize;
    changed[rz_at] = 0xff;
    cases.push(changed);
    SDK_ENTRIES.with(|n| n.set(0));
    for snapshot in cases {
        let replacement = replace_snapshot(&f, &snapshot);
        assert!(open_bootstrap_source_data(
            &KEY,
            replacement.context_data(),
            replacement.id(),
            replacement.as_bytes()
        )
        .is_err());
        assert_eq!(SDK_ENTRIES.with(Cell::get), 0);
    }
    assert!(prepare_v2_genesis_seed(&f.opened, limits(), admission(&count)).is_ok());
    assert_eq!(f.opened.view_data().snapshot, original);
}

#[test]
fn real_first_add_sdk_shape_is_supported_but_rejected_as_initial_v2_genesis() {
    let f = fixture(0x61ac);
    // A separate real graph copy permits constructing an epoch1 candidate without changing
    // the original source. Default legacy restoration is a DATA producer in this test only.
    let mls = s0::fields(
        f.opened.view_data().snapshot,
        f.opened.view_data().context.spec_data(),
    )
    .unwrap()
    .mls;
    let (device, mut group) = catcoms_mls::persist::restore_server(mls).unwrap();
    let member = MlsDevice::generate().unwrap();
    group
        .add_member(&device, member.key_package().unwrap())
        .unwrap();
    let added = catcoms_mls::persist::snapshot_server(&device, &group).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let (paid_device, paid_group) = catcoms_mls::persist::restore_server_owner_checkpoint_bounded(
        &added,
        512 * 1024,
        512 * 1024,
        admission(&count),
    )
    .unwrap();
    assert_eq!(paid_group.epoch(), 1);
    assert_eq!(paid_group.member_count(), 2);
    drop(paid_group);
    drop(paid_device);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let original = f.opened.view_data().snapshot;
    let mut d = catcoms_wire::Decoder::new(original);
    d.get_bytes().unwrap();
    let suffix = original.len() - d.remaining();
    let mut snapshot = Zeroizing::new(Vec::new());
    snapshot.extend_from_slice(&(added.len() as u32).to_be_bytes());
    snapshot.extend_from_slice(&added);
    snapshot.extend_from_slice(&original[suffix..]);
    let replacement = replace_snapshot(&f, &snapshot);
    let opened = open_bootstrap_source_data(
        &KEY,
        replacement.context_data(),
        replacement.id(),
        replacement.as_bytes(),
    )
    .unwrap();
    SDK_ENTRIES.with(|n| n.set(0));
    assert!(prepare_v2_genesis_seed(&opened, limits(), admission(&count)).is_err());
    assert_eq!(
        SDK_ENTRIES.with(Cell::get),
        1,
        "refusal is after the real supported SDK load"
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn opened_source_check_is_borrowed_data_only_and_refuses_a_different_actual_source() {
    let a = fixture(0x61ae);
    let b = fixture(0x61af);
    let count = Arc::new(AtomicUsize::new(0));
    let seed = seed(&a.opened, &count);
    let original = seed.body.genesis.clone();
    SDK_ENTRIES.with(|n| n.set(0));
    seed.check_opened_source_data(&a.opened).unwrap();
    assert!(seed.check_opened_source_data(&b.opened).is_err());
    assert_eq!(seed.body.genesis.profile, original.profile);
    assert_eq!(seed.body.genesis.header, original.header);
    assert_eq!(seed.body.genesis.image, original.image);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(SDK_ENTRIES.with(Cell::get), 0);
    a.reservation.owner.send_replace(true);
    // Byte consistency cannot renew or certify that cancelled original reservation. It is
    // deliberately distinct from Root's actual source/row/lifetime admission.
    seed.check_opened_source_data(&a.opened).unwrap();
}

#[test]
fn opened_source_check_compares_representatives_image_frontier_and_full_context_not_only_ids() {
    let a = fixture(0x61b0);
    let b = fixture(0x61b1);
    let count = Arc::new(AtomicUsize::new(0));
    let mut seed = seed(&a.opened, &count);
    let original = seed.body.genesis.clone();
    let context = seed.body.context.clone();
    let source = seed.body.source;
    for variant in 0..7 {
        seed.body.genesis = original.clone();
        seed.body.context = context.clone();
        seed.body.source = source;
        match variant {
            0 => seed.body.source = BootstrapSourceId::from_bytes([0; 32]),
            1 => seed.body.context = b.opened.view_data().context.clone(),
            2 => {
                // Private-corruption fixture only: public construction rejects this signature.
                // Same semantic CID must not hide a changed retained representative here.
                let mut encoded = original.profile.encode();
                *encoded.last_mut().unwrap() ^= 1;
                seed.body.genesis.profile = SignedOwnerProfile::decode(&encoded).unwrap();
                assert_eq!(seed.body.genesis.profile.id(), original.profile.id());
                assert!(!seed.body.genesis.profile.verify());
            }
            3 => seed.body.genesis.admission = b.opened.genesis.admission.clone(),
            4 => {
                let mut encoded = original.header.encode();
                *encoded.last_mut().unwrap() ^= 1;
                seed.body.genesis.header = SignedOwnerHeader::decode(&encoded).unwrap();
                assert_eq!(seed.body.genesis.header.id(), original.header.id());
                assert!(!seed.body.genesis.header.verify());
            }
            5 => seed.body.genesis.image[0] ^= 1,
            _ => {
                seed.body.genesis.frontier =
                    catcoms_replication::chat::acceptance_tree::Frontier::empty(
                        b.opened.genesis.frontier.tip().context,
                    );
            }
        }
        SDK_ENTRIES.with(|n| n.set(0));
        assert!(seed.check_opened_source_data(&a.opened).is_err());
        assert_eq!(SDK_ENTRIES.with(Cell::get), 0);
        assert!(a.opened.genesis.profile.verify() && a.opened.genesis.header.verify());
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
