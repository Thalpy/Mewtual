//! Real SDK founding/S0 and real public/page cryptography; no Store/Fresh reconstruction claim.
//! All regressions are authored UNRUN while compiler/tests/builds are paused.
use super::*;
use crate::local_chat_scope::{ManagedChatScopeReservation, ManagedChatScopeRevocation};
use catcoms_crypto::{unseal, SealedBlob};
use catcoms_mls::MlsDevice;
use catcoms_replication::chat::{
    cold_manifest::ManifestPage,
    cold_records::{HeaderRow, RecordBatch, RecordKind},
};
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId, RequestCancellation};
use catcoms_storage::private_chat_page::open_private_chat_page;
use catcoms_wire::Decoder;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, Error, RngCore, SeedableRng};
use std::{
    cell::Cell,
    ops::Range,
    sync::{Arc, Weak},
};
use tokio::sync::watch;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
const VAULT: [u8; 32] = [81; 32];
const NONCE: [u8; 32] = [82; 32];
const KEY: [u8; 32] = [83; 32];

struct Reservation {
    group: Vec<u8>,
    signer: [u8; 32],
    identity: Arc<()>,
    owner: Arc<watch::Sender<bool>>,
    revision: Cell<Option<u64>>,
    revision_reads: Cell<usize>,
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
        VAULT
    }
    fn nonce(&self) -> [u8; 32] {
        NONCE
    }
    fn planned_revision_data(&self) -> Option<u64> {
        self.revision_reads.set(self.revision_reads.get() + 1);
        self.revision.get()
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
fn found(seed: u64) -> Node {
    ChannelSync::found(
        Hub::new().join(PeerId::from_u64(seed)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap()
}
fn reserve(node: &mut Node, revision: Option<u64>) -> Reservation {
    let reservation = Reservation {
        group: node.group_id(),
        signer: node.my_public_key().try_into().unwrap(),
        identity: Arc::new(()),
        owner: Arc::new(watch::channel(false).0),
        revision: Cell::new(revision),
        revision_reads: Cell::new(0),
    };
    node.stage_managed_chat_scope(VAULT, NONCE).unwrap();
    node.attach_managed_chat_reservation(&reservation).unwrap();
    reservation
}
fn archive(node: &mut Node) -> FounderArchivePreparation {
    let mut source = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(node)
        .unwrap();
    source
        .cache_private_snapshot(
            node,
            FounderSnapshotLimits::new(1024 * 1024, 512 * 1024, 512 * 1024).unwrap(),
        )
        .unwrap();
    FounderArchivePreparation::from_cached_source(source, node, VAULT).unwrap()
}
fn prepared(node: &mut Node) -> FounderBootstrapPreparation {
    let mut original = archive(node);
    original
        .prepare(node, &KEY, &mut CountingRng::new())
        .unwrap();
    FounderBootstrapPreparation::from_archive(original, node).unwrap()
}
struct CountingRng {
    inner: ChaCha20Rng,
    bytes: usize,
    first: Option<Box<dyn FnOnce()>>,
}
impl CountingRng {
    fn new() -> Self {
        Self {
            inner: ChaCha20Rng::from_seed([84; 32]),
            bytes: 0,
            first: None,
        }
    }
    fn enter(&mut self, bytes: usize) {
        if let Some(hook) = self.first.take() {
            hook();
        }
        self.bytes += bytes;
    }
}
impl RngCore for CountingRng {
    fn next_u32(&mut self) -> u32 {
        self.enter(4);
        self.inner.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.enter(8);
        self.inner.next_u64()
    }
    fn fill_bytes(&mut self, out: &mut [u8]) {
        self.enter(out.len());
        self.inner.fill_bytes(out);
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), Error> {
        self.enter(out.len());
        self.inner.try_fill_bytes(out)
    }
}
impl CryptoRng for CountingRng {}

fn plaintext(source: &PreparedBootstrapSource) -> Zeroizing<Vec<u8>> {
    let start = 4 + b"catcoms/private-chat-bootstrap-source-sealed/v1\0".len() + 2;
    let sealed = SealedBlob {
        nonce: source.as_bytes()[start..start + 24].try_into().unwrap(),
        ciphertext: source.as_bytes()[start + 28..].to_vec(),
    };
    let key = Zeroizing::new(blake3::derive_key(
        "catcoms/private-chat-bootstrap-source-key/v1",
        &KEY,
    ));
    Zeroizing::new(unseal(&key, &sealed).unwrap())
}
fn payload_ranges(raw: &[u8]) -> [Range<usize>; 4] {
    let mut d = Decoder::new(raw);
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
    d.get_bytes().unwrap();
    d.get_u32().unwrap();
    std::array::from_fn(|_| {
        let start = raw.len() - d.remaining() + 4;
        let bytes = d.get_bytes().unwrap();
        start..start + bytes.len()
    })
}
fn reseal(body: &[u8], context: &BootstrapSourceContextData) -> PreparedBootstrapSource {
    codec::seal_original_body(&KEY, body, context.clone(), &mut CountingRng::new()).unwrap()
}

#[test]
fn original_source_context_exact_cache_single_handoff_and_offline_after_creator_drop() {
    let mut node = found(0x5f01);
    let reservation = reserve(&mut node, Some(37));
    let mut source = prepared(&mut node);
    let mut rng = CountingRng::new();
    let output = source.prepare(&node, &KEY, &mut rng).unwrap();
    let original = output.as_bytes().to_vec();
    let original_id = output.id();
    let context = output.context_data().clone();
    let spec = context.spec_data();
    assert_eq!(spec.group, node.group_id_ref());
    assert_eq!(spec.signer.as_slice(), node.my_public_key());
    assert_eq!(
        spec.fresh_attempt,
        node.fresh_chat_admission().unwrap().attempt_id()
    );
    assert_eq!(spec.planned_revision, 37);
    assert_ne!(spec.planned_revision, node.chat_scope_revision());
    assert_eq!(
        reservation.revision_reads.get(),
        1,
        "not sampled again during source checks"
    );
    assert_eq!(
        source.prepare(&node, &KEY, &mut rng).unwrap().as_bytes(),
        original
    );
    assert_eq!(rng.bytes, 24);
    let detached = source.take_prepared_for_handoff(&node, &KEY).unwrap();
    assert_eq!(detached.id(), original_id);
    assert!(source.prepare(&node, &KEY, &mut rng).is_err());
    assert!(source.borrow_prepared_for_handoff(&node, &KEY).is_err());
    assert!(source.take_prepared_for_handoff(&node, &KEY).is_err());
    assert_eq!(rng.bytes, 24);
    drop(source);
    drop(reservation);
    drop(node);
    assert_eq!(detached.as_bytes(), original);
    let opened =
        open_bootstrap_source_data(&KEY, &context, original_id, detached.as_bytes()).unwrap();
    let view = opened.view_data();
    assert_eq!(
        snapshot::digest(spec.profile, view.snapshot).as_bytes(),
        &spec.snapshot_digest
    );
    assert_eq!(view.context, &context);
    // This independent DATA parser does not need or synthesize a live creator/Fresh/SDK instance.
    assert!(
        !view.profile.is_empty()
            && !view.records_frame.is_empty()
            && !view.manifest_frame.is_empty()
    );
}

#[test]
fn missing_package_refusal_returns_same_owner_and_none_revision_cannot_be_refreshed() {
    let mut node = found(0x5f02);
    let reservation = reserve(&mut node, Some(9));
    let original = archive(&mut node);
    let mut original = FounderBootstrapPreparation::from_archive(original, &node)
        .unwrap_err()
        .into_original();
    original
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    assert!(FounderBootstrapPreparation::from_archive(original, &node).is_ok());
    drop(reservation);

    let mut node = found(0x5f03);
    let reservation = reserve(&mut node, None);
    let mut original = archive(&mut node);
    original
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    let original = FounderBootstrapPreparation::from_archive(original, &node)
        .unwrap_err()
        .into_original();
    reservation.revision.set(Some(9));
    assert!(node.attach_managed_chat_reservation(&reservation).is_err());
    assert!(FounderBootstrapPreparation::from_archive(original, &node).is_err());
}

#[test]
fn changed_durable_revision_same_labels_identity_and_local_revision_refuses_reattach() {
    let mut node = found(0x5f04);
    let reservation = reserve(&mut node, Some(12));
    let mut source = prepared(&mut node);
    let local = node.chat_scope_revision();
    reservation.revision.set(Some(13));
    assert!(node.attach_managed_chat_reservation(&reservation).is_err());
    assert_eq!(local, node.chat_scope_revision());
    let output = source
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    assert_eq!(
        output.context_data().spec_data().planned_revision,
        12,
        "original capture stays fixed"
    );
    // Simulate an internal replacement of the actual stored original observation, not just a
    // caller changing its claimed method: the already captured source must refuse that too.
    node.local_chat_scope
        .reservation
        .as_mut()
        .unwrap()
        .planned_revision = Some(13);
    assert!(source.borrow_prepared_for_handoff(&node, &KEY).is_err());
}

#[test]
fn source_mismatch_equal_restore_and_original_negative_refuse_before_nonce() {
    let mut node = found(0x5f05);
    let reservation = reserve(&mut node, Some(17));
    let mut source = prepared(&mut node);
    let mut rng = CountingRng::new();
    assert!(source.prepare(&node, &[99; 32], &mut rng).is_err());
    assert_eq!(rng.bytes, 0);
    let old = node.routing_label;
    node.routing_label = old.wrapping_add(1);
    assert!(source.prepare(&node, &KEY, &mut rng).is_err());
    node.routing_label = old; // current byte-exact restoration is allowed, not mutation-history tracking
    let raw = source
        .original
        .borrow_private_snapshot_for_covering_save(&node)
        .unwrap()
        .snapshot;
    let restored = Node::restore(
        raw,
        Hub::new().join(PeerId::from_u64(0x5f06)),
        ChaCha20Rng::seed_from_u64(0x5f06),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert!(source.prepare(&restored, &KEY, &mut rng).is_err());
    reservation.owner.send_replace(true);
    assert!(source.prepare(&node, &KEY, &mut rng).is_err());
    assert_eq!(rng.bytes, 0);
}

#[test]
fn cancellation_during_seal_retains_unexposed_original_ciphertext_without_retry_resample() {
    let mut node = found(0x5f07);
    let reservation = reserve(&mut node, Some(19));
    let mut source = prepared(&mut node);
    let owner = reservation.owner.clone();
    let mut rng = CountingRng::new();
    rng.first = Some(Box::new(move || {
        owner.send_replace(true);
    }));
    assert!(source.prepare(&node, &KEY, &mut rng).is_err());
    let candidate = source.prepared.as_ref().unwrap();
    let id = candidate.id();
    let bytes = candidate.as_bytes().to_vec();
    assert_eq!(rng.bytes, 24);
    assert!(source.prepare(&node, &KEY, &mut rng).is_err());
    assert!(source.borrow_prepared_for_handoff(&node, &KEY).is_err());
    assert!(source.take_prepared_for_handoff(&node, &KEY).is_err());
    assert_eq!(source.prepared.as_ref().unwrap().id(), id);
    assert_eq!(source.prepared.as_ref().unwrap().as_bytes(), bytes);
    assert_eq!(rng.bytes, 24);
}

#[test]
fn independent_expected_context_all_fields_and_wrong_key_or_source_id_refuse() {
    let mut node = found(0x5f08);
    let _reservation = reserve(&mut node, Some(23));
    let mut source = prepared(&mut node);
    let output = source
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    let base = output.context_data().spec_data();
    let mut cases = Vec::new();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut spec = base;
            spec.$field = $value;
            cases.push(BootstrapSourceContextData::from_data(spec).unwrap());
        }};
    }
    changed!(signer, [1; 32]);
    changed!(fresh_attempt, [2; 32]);
    changed!(scope_nonce, [3; 32]);
    changed!(planned_revision, 24);
    changed!(profile, OwnerFactId::from_bytes([4; 32]));
    changed!(admission, OwnerBodyId::from_bytes([5; 32]));
    changed!(header, OwnerHeaderId::from_bytes([6; 32]));
    changed!(records, PrivateChatPageId::from_bytes([7; 32]));
    changed!(manifest, PrivateChatPageId::from_bytes([8; 32]));
    changed!(snapshot_digest, [9; 32]);
    changed!(records_plaintext_bytes, base.records_plaintext_bytes + 1);
    changed!(
        limits,
        FounderSnapshotLimits::new(2 * 1024 * 1024, 512 * 1024, 512 * 1024).unwrap()
    );
    let mut group = base.group.to_vec();
    group[0] ^= 1;
    changed!(group, &group);
    let mut vault = base;
    vault.vault = [10; 32];
    vault.scope_domain = [10; 32];
    cases.push(BootstrapSourceContextData::from_data(vault).unwrap());
    for context in cases {
        assert!(
            open_bootstrap_source_data(&KEY, &context, output.id(), output.as_bytes()).is_err()
        );
    }
    assert!(open_bootstrap_source_data(
        &[1; 32],
        output.context_data(),
        output.id(),
        output.as_bytes()
    )
    .is_err());
    assert!(open_bootstrap_source_data(
        &KEY,
        output.context_data(),
        BootstrapSourceId::from_bytes([1; 32]),
        output.as_bytes()
    )
    .is_err());
}

#[test]
fn strict_outer_and_inner_framing_caps_domains_versions_and_limits_refuse() {
    let mut node = found(0x5f09);
    let _reservation = reserve(&mut node, Some(29));
    let mut source = prepared(&mut node);
    let output = source
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    let context = output.context_data();
    for at in [
        0,
        4,
        4 + b"catcoms/private-chat-bootstrap-source-sealed/v1\0".len(),
    ] {
        let mut frame = output.as_bytes().to_vec();
        frame[at] ^= 1;
        assert!(
            open_bootstrap_source_data(&KEY, context, BootstrapSourceId::of(&frame), &frame)
                .is_err()
        );
    }
    for frame in [
        output.as_bytes()[..output.as_bytes().len() - 1].to_vec(),
        [output.as_bytes(), &[0]].concat(),
    ] {
        assert!(
            open_bootstrap_source_data(&KEY, context, BootstrapSourceId::of(&frame), &frame)
                .is_err()
        );
    }
    let raw = plaintext(output);
    let ranges = payload_ranges(&raw);
    let mut cases = Vec::new();
    let mut changed = raw.to_vec();
    changed[4] ^= 1;
    cases.push(changed);
    let mut changed = raw.to_vec();
    changed[4 + b"catcoms/private-chat-bootstrap-source/v1\0".len()] ^= 1;
    cases.push(changed);
    let mut changed = raw.to_vec();
    changed.extend_from_slice(&[0]);
    cases.push(changed);
    let mut changed = raw.to_vec();
    // Limits precede framed digest32, records length and the first payload's length word.
    let limits_at = ranges[0].start - 4 - 4 - 36 - 24;
    changed[limits_at..limits_at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    cases.push(changed);
    for range in &ranges {
        let mut changed = raw.to_vec();
        changed[range.start - 4..range.start].copy_from_slice(&u32::MAX.to_be_bytes());
        cases.push(changed);
    }
    for body in cases {
        let candidate = reseal(&body, context);
        assert!(
            open_bootstrap_source_data(&KEY, context, candidate.id(), candidate.as_bytes())
                .is_err()
        );
    }
    let mut rng = CountingRng::new();
    assert!(codec::seal_original_body(
        &KEY,
        &vec![0; MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES + 1],
        context.clone(),
        &mut rng
    )
    .is_err());
    assert_eq!(rng.bytes, 0);
}

#[test]
fn raw_s0_digest_provider_labels_scope_and_unknown_tail_cannot_be_relabelled_as_valid() {
    let mut node = found(0x5f10);
    let _reservation = reserve(&mut node, Some(31));
    let mut source = prepared(&mut node);
    let output = source
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    let raw = plaintext(output);
    let ranges = payload_ranges(&raw);
    let s0 = &raw[ranges[0].clone()];
    let mut mls = Decoder::new(s0);
    let mls_raw = mls.get_bytes().unwrap();
    let mut inner = Decoder::new(mls_raw);
    inner.get_bytes().unwrap();
    let signer_at = 4 + mls_raw.len() - inner.remaining() + 4;
    let scope_nonce_at = s0.len() - (32 + 1 + 4); // nonce CONTENTS + Active + aliases0
    for at in [signer_at, scope_nonce_at] {
        let mut body = raw.to_vec();
        body[ranges[0].start + at] ^= 1;
        let mut spec = output.context_data().spec_data();
        spec.snapshot_digest = *snapshot::digest(spec.profile, &body[ranges[0].clone()]).as_bytes();
        let context = BootstrapSourceContextData::from_data(spec).unwrap();
        // Update the body digest too: the rejection must be the actual S0 label parser.
        let digest_at = ranges[0].start - 4 - 4 - 36;
        body[digest_at + 4..digest_at + 36].copy_from_slice(&context.spec_data().snapshot_digest);
        let candidate = reseal(&body, &context);
        assert!(
            open_bootstrap_source_data(&KEY, &context, candidate.id(), candidate.as_bytes())
                .is_err()
        );
    }
    let mut changed = raw.to_vec();
    changed[ranges[0].start] ^= 1;
    let candidate = reseal(&changed, output.context_data());
    assert!(open_bootstrap_source_data(
        &KEY,
        output.context_data(),
        candidate.id(),
        candidate.as_bytes()
    )
    .is_err());
    let mut trailing_s0 = s0.to_vec();
    trailing_s0.extend_from_slice(&[0; 4]);
    assert!(s0::preflight(&trailing_s0, output.context_data().spec_data()).is_err());
}

#[test]
fn original_nested_record_manifest_and_profile_frames_are_required_not_only_their_labels() {
    let mut node = found(0x5f11);
    let _reservation = reserve(&mut node, Some(41));
    let mut source = prepared(&mut node);
    let output = source
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    let raw = plaintext(output);
    let ranges = payload_ranges(&raw);
    for range in &ranges[1..] {
        let mut body = raw.to_vec();
        body[range.end - 1] ^= 1;
        let candidate = reseal(&body, output.context_data());
        assert!(open_bootstrap_source_data(
            &KEY,
            output.context_data(),
            candidate.id(),
            candidate.as_bytes()
        )
        .is_err());
    }
    assert!(open_private_chat_page(
        &KEY,
        PrivateChatPageId::from_bytes(*output.id().as_bytes()),
        output.as_bytes()
    )
    .is_err());
    let generic =
        PreparedPrivateChatPage::prepare(&KEY, b"not a bootstrap source", &mut CountingRng::new())
            .unwrap();
    assert!(open_bootstrap_source_data(
        &KEY,
        output.context_data(),
        BootstrapSourceId::of(generic.as_bytes()),
        generic.as_bytes()
    )
    .is_err());
}

#[test]
fn source_and_plaintext_debug_omit_private_context_and_ciphertext() {
    let mut node = found(0x5f12);
    let _reservation = reserve(&mut node, Some(43));
    let mut source = prepared(&mut node);
    let output = source
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    let opened =
        open_bootstrap_source_data(&KEY, output.context_data(), output.id(), output.as_bytes())
            .unwrap();
    assert_eq!(format!("{:?}", output.id()), "BootstrapSourceId { .. }");
    assert_eq!(
        format!("{:?}", output.context_data()),
        "BootstrapSourceContextData { .. }"
    );
    assert_eq!(format!("{:?}", opened), "OpenedBootstrapSourceData { .. }");
    assert_eq!(
        format!("{:?}", opened.view_data()),
        "BootstrapSourceDataView { .. }"
    );
    assert!(!format!("{:?}", output).contains("digest"));
}

#[test]
fn transferred_source_observation_checks_original_and_never_reopens_output_or_rng() {
    let mut node = found(0x5f14);
    let reservation = reserve(&mut node, Some(53));
    let mut source = prepared(&mut node);
    let mut rng = CountingRng::new();
    let output = source.prepare(&node, &KEY, &mut rng).unwrap();
    let context = output.context_data().clone();
    let id = output.id();
    assert!(source
        .validate_transferred_source(&node, &KEY, &context, id)
        .is_err());
    let detached = source.take_prepared_for_handoff(&node, &KEY).unwrap();
    source
        .validate_transferred_source(&node, &KEY, detached.context_data(), detached.id())
        .unwrap();
    assert!(source
        .validate_transferred_source(&node, &[99; 32], &context, id)
        .is_err());
    assert!(source
        .validate_transferred_source(
            &node,
            &KEY,
            &context,
            BootstrapSourceId::from_bytes([0; 32])
        )
        .is_err());
    let mut spec = context.spec_data();
    spec.scope_nonce[0] ^= 1;
    let foreign = BootstrapSourceContextData::from_data(spec).unwrap();
    assert!(source
        .validate_transferred_source(&node, &KEY, &foreign, id)
        .is_err());
    let raw = source
        .original
        .borrow_private_snapshot_for_covering_save(&node)
        .unwrap()
        .snapshot;
    let restored = Node::restore(
        raw,
        Hub::new().join(PeerId::from_u64(0x5f15)),
        ChaCha20Rng::seed_from_u64(0x5f15),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    assert!(source
        .validate_transferred_source(&restored, &KEY, &context, id)
        .is_err());
    let old = node.routing_label;
    node.routing_label = old.wrapping_add(1);
    assert!(source
        .validate_transferred_source(&node, &KEY, &context, id)
        .is_err());
    node.routing_label = old;
    source
        .validate_transferred_source(&node, &KEY, &context, id)
        .unwrap();
    assert!(source.prepare(&node, &KEY, &mut rng).is_err());
    assert!(source.borrow_prepared_for_handoff(&node, &KEY).is_err());
    assert!(source.take_prepared_for_handoff(&node, &KEY).is_err());
    assert_eq!(rng.bytes, 24);
    reservation.owner.send_replace(true);
    assert!(source
        .validate_transferred_source(&node, &KEY, &context, id)
        .is_err());
    open_bootstrap_source_data(&KEY, &context, id, detached.as_bytes()).unwrap();
}

#[test]
fn freshly_encrypted_nested_frames_still_require_real_header_signature_and_manifest_semantics() {
    let mut node = found(0x5f13);
    let _reservation = reserve(&mut node, Some(47));
    let mut source = prepared(&mut node);
    let output = source
        .prepare(&node, &KEY, &mut CountingRng::new())
        .unwrap();
    let context = output.context_data();
    let raw = plaintext(output);
    let ranges = payload_ranges(&raw);
    let spec = context.spec_data();
    let profile = SignedOwnerProfile::decode(&raw[ranges[1].clone()]).unwrap();
    let operation = ArchiveOperation::Genesis {
        body: spec.admission,
    };
    let mut records = open_private_chat_page(&KEY, spec.records, &raw[ranges[2].clone()]).unwrap();
    let manifest = open_private_chat_page(&KEY, spec.manifest, &raw[ranges[3].clone()]).unwrap();
    let (payload_start, payload_len, signature_end) = {
        let batch = RecordBatch::decode(
            &records,
            BatchBinding {
                vault: &VAULT,
                profile: &profile,
                role: PageRole::GenesisRecords,
                operation: &operation,
                append: None,
            },
        )
        .unwrap();
        let row = batch
            .rows()
            .find(|row| row.key.kind == RecordKind::Header)
            .unwrap();
        HeaderRow::decode(row.payload).unwrap();
        let mut header_row = Decoder::new(row.payload);
        header_row.get_bytes().unwrap();
        header_row.get_u16().unwrap();
        let mut header = Decoder::new(header_row.get_bytes().unwrap());
        header.get_bytes().unwrap();
        header.get_bytes().unwrap();
        let signature = header.get_bytes().unwrap();
        let payload_start = row.payload.as_ptr() as usize - records.as_ptr() as usize;
        (
            payload_start,
            row.payload.len(),
            signature.as_ptr() as usize - records.as_ptr() as usize + signature.len(),
        )
    };
    records[signature_end - 1] ^= 1; // SAME semantic CID, invalid actual signed representative.
    let mut original = blake3::Hasher::new();
    original.update(b"catcoms/private-chat-original-record/v1\0");
    original.update(&[2]); // closed Header record kind
    original.update(&records[payload_start..payload_start + payload_len]);
    // Keep the original-representation digest correct, so signature verification must reject.
    records[payload_start - 36..payload_start - 4].copy_from_slice(original.finalize().as_bytes());
    let new_records =
        PreparedPrivateChatPage::prepare(&KEY, &records, &mut CountingRng::new()).unwrap();
    let new_manifest_plain = encode_manifest(&ManifestSpec {
        vault: VAULT,
        group: spec.group,
        profile: spec.profile,
        sequence: 1,
        previous: None,
        source_head: None,
        before_header: None,
        after_header: spec.header,
        operation,
        pages: &[PageDescriptor {
            role: PageRole::GenesisRecords,
            page: new_records.id(),
            plaintext_bytes: records.len(),
            entries: 3,
        }],
    })
    .unwrap();
    ManifestPage::decode(&new_manifest_plain).unwrap();
    let new_manifest =
        PreparedPrivateChatPage::prepare(&KEY, &new_manifest_plain, &mut CountingRng::new())
            .unwrap();
    let mut changed_spec = spec;
    changed_spec.records = new_records.id();
    changed_spec.manifest = new_manifest.id();
    let changed_context = BootstrapSourceContextData::from_data(changed_spec).unwrap();
    let mut body = codec::encode_original_body(&source.original, &node, &changed_context).unwrap();
    let body_ranges = payload_ranges(&body);
    body[body_ranges[2].clone()].copy_from_slice(new_records.as_bytes());
    body[body_ranges[3].clone()].copy_from_slice(new_manifest.as_bytes());
    let candidate = reseal(&body, &changed_context);
    assert!(open_bootstrap_source_data(
        &KEY,
        &changed_context,
        candidate.id(),
        candidate.as_bytes()
    )
    .is_err());

    // A separately well-encrypted manifest can still lie about its sole record descriptor.
    let wrong_manifest_plain = encode_manifest(&ManifestSpec {
        vault: VAULT,
        group: spec.group,
        profile: spec.profile,
        sequence: 1,
        previous: None,
        source_head: None,
        before_header: None,
        after_header: spec.header,
        operation,
        pages: &[PageDescriptor {
            role: PageRole::GenesisRecords,
            page: spec.records,
            plaintext_bytes: spec.records_plaintext_bytes + 1,
            entries: 3,
        }],
    })
    .unwrap();
    assert_ne!(wrong_manifest_plain.as_slice(), manifest.as_slice());
    let wrong_manifest =
        PreparedPrivateChatPage::prepare(&KEY, &wrong_manifest_plain, &mut CountingRng::new())
            .unwrap();
    let mut changed_spec = spec;
    changed_spec.manifest = wrong_manifest.id();
    let changed_context = BootstrapSourceContextData::from_data(changed_spec).unwrap();
    let mut body = codec::encode_original_body(&source.original, &node, &changed_context).unwrap();
    let body_ranges = payload_ranges(&body);
    body[body_ranges[3].clone()].copy_from_slice(wrong_manifest.as_bytes());
    let candidate = reseal(&body, &changed_context);
    assert!(open_bootstrap_source_data(
        &KEY,
        &changed_context,
        candidate.id(),
        candidate.as_bytes()
    )
    .is_err());
}
