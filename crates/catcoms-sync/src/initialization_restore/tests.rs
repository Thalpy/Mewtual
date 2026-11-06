//! Real initializer/legacy bytes plus adversarial framing. Only the test input editor copies
//! data; the production preflight must finish before the actual common decoder's first entry.
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::ops::Range;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
fn limits() -> OwnerInitializationRestoreLimits {
    OwnerInitializationRestoreLimits {
        max_snapshot_bytes: 1024 * 1024,
        max_mls_bytes: 512 * 1024,
        max_mls_backing_bytes: 512 * 1024,
    }
}
fn founder() -> Node {
    let mut node = ChannelSync::found(
        Hub::new().join(PeerId::from_u64(0xf001)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0xf001),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    node.initialize_new_group_policy().unwrap();
    node
}
fn initial() -> (Node, Zeroizing<Vec<u8>>) {
    let mut node = founder();
    let evidence = node.prepare_owner_evidence_checkpoint(9).unwrap();
    node.stage_managed_chat_scope([1; 32], [2; 32]).unwrap();
    node.stage_owner_evidence_checkpoint(&evidence).unwrap();
    let cap = limits();
    let bytes = node
        .snapshot_owner_initialization(
            cap.max_snapshot_bytes,
            cap.max_mls_bytes,
            cap.max_mls_backing_bytes,
        )
        .unwrap();
    (node, bytes)
}
fn restore(bytes: &[u8], limits: OwnerInitializationRestoreLimits) -> Result<Node, SyncError> {
    ChannelSync::restore_owner_initialization(
        bytes,
        Hub::new().join(PeerId::from_u64(0xf002)),
        ChaCha20Rng::seed_from_u64(0xf002),
        Box::new(ManualClock::new(2_000)),
        limits,
    )
}
fn before_common(bytes: &[u8]) -> SyncError {
    let before = DECODER_ENTRIES.with(std::cell::Cell::get);
    let error = match restore(bytes, limits()) {
        Err(error) => error,
        Ok(_) => panic!("unsupported input unexpectedly materialized"),
    };
    assert_eq!(
        DECODER_ENTRIES.with(std::cell::Cell::get),
        before,
        "refusal must precede actual common decoder, clones and SDK"
    );
    error
}

/// Locate actual existing framing; mutations preserve every original byte outside the chosen
/// field. This never generates a substitute MLS group, owner witness or source admission.
struct Locations {
    mls: Range<usize>,
    routing: Range<usize>,
    ledger: Range<usize>,
    counts: [usize; 7], // documents, commits, peers, old grants, rendezvous, direct, admins
    rendezvous_end: usize,
    tenure: Range<usize>,
    policy: Range<usize>,
    durable: Range<usize>,
    pending: Range<usize>,
    scope: Range<usize>,
    owner: Range<usize>,
}
fn located_frame(decoder: &mut Decoder<'_>, total: usize) -> Range<usize> {
    let start = total - decoder.remaining() + 4;
    let bytes = decoder.get_bytes().unwrap();
    start..start + bytes.len()
}
fn zero_count(decoder: &mut Decoder<'_>, total: usize) -> usize {
    let position = total - decoder.remaining();
    assert_eq!(decoder.get_u32().unwrap(), 0);
    position
}
fn locations(bytes: &[u8]) -> Locations {
    let total = bytes.len();
    let mut decoder = Decoder::new(bytes);
    let mls = located_frame(&mut decoder, total);
    let routing = located_frame(&mut decoder, total);
    let ledger = located_frame(&mut decoder, total);
    let documents = zero_count(&mut decoder, total);
    let commits = zero_count(&mut decoder, total);
    let peers = zero_count(&mut decoder, total);
    decoder.get_bytes().unwrap(); // stable wrap key
    decoder.get_u64().unwrap();
    let old_grants = zero_count(&mut decoder, total);
    let rendezvous = zero_count(&mut decoder, total);
    let rendezvous_end = total - decoder.remaining();
    let direct = zero_count(&mut decoder, total);
    let tenure = located_frame(&mut decoder, total);
    let policy = located_frame(&mut decoder, total);
    let durable = located_frame(&mut decoder, total);
    let pending = located_frame(&mut decoder, total);
    assert_eq!(decoder.get_str().unwrap(), "catcoms/admin-authority/v2");
    let admins = zero_count(&mut decoder, total);
    let scope = located_frame(&mut decoder, total);
    let owner = located_frame(&mut decoder, total);
    decoder.finish().unwrap();
    Locations {
        mls,
        routing,
        ledger,
        counts: [
            documents, commits, peers, old_grants, rendezvous, direct, admins,
        ],
        rendezvous_end,
        tenure,
        policy,
        durable,
        pending,
        scope,
        owner,
    }
}
fn replaced(bytes: &[u8], range: Range<usize>, body: &[u8]) -> Vec<u8> {
    let mut output = bytes[..range.start - 4].to_vec();
    output.extend_from_slice(&(body.len() as u32).to_be_bytes());
    output.extend_from_slice(body);
    output.extend_from_slice(&bytes[range.end..]);
    output
}

fn signed_originals(node: &mut Node) -> Vec<Vec<u8>> {
    let snapshot = node
        .docs
        .get_mut(&(DocType::Channel, 9))
        .unwrap()
        .snapshot()
        .unwrap();
    let mut decoder = Decoder::new(&snapshot);
    assert_eq!(decoder.get_u16().unwrap(), DocType::Channel.tag());
    assert_eq!(decoder.get_u128().unwrap(), 9);
    decoder.get_bytes().unwrap(); // materialized graph is separate from original signed evidence
    let count = decoder.get_u32().unwrap();
    let originals = (0..count)
        .map(|_| {
            let bytes = decoder.get_bytes().unwrap().to_vec();
            assert!(catcoms_replication::op::SignedOp::decode(&bytes)
                .unwrap()
                .verify());
            bytes
        })
        .collect();
    decoder.finish().unwrap();
    originals
}

#[test]
fn actual_founder_initializer_restores_exact_identity_and_remains_unbound_at_exact_caps() {
    let (node, bytes) = initial();
    let locations = locations(&bytes);
    let exact = OwnerInitializationRestoreLimits {
        max_snapshot_bytes: bytes.len(),
        max_mls_bytes: locations.mls.len(),
        ..limits()
    };
    let restored = restore(&bytes, exact).unwrap();
    assert_eq!(restored.group_id(), node.group_id());
    assert_eq!(restored.my_public_key(), node.my_public_key());
    assert_eq!(
        restored.group.epoch_authenticator_id(),
        node.group.epoch_authenticator_id()
    );
    assert_eq!(
        restored.owner_evidence_profile_id(),
        node.owner_evidence_profile_id()
    );
    assert!(restored.fresh_chat_admission().is_none());
    assert!(
        restored.durable_send_context().is_err(),
        "decoding does not mint Current/canonical runtime"
    );
    for too_small in [
        OwnerInitializationRestoreLimits {
            max_snapshot_bytes: exact.max_snapshot_bytes - 1,
            ..exact
        },
        OwnerInitializationRestoreLimits {
            max_mls_bytes: exact.max_mls_bytes - 1,
            ..exact
        },
    ] {
        let before = DECODER_ENTRIES.with(std::cell::Cell::get);
        assert!(matches!(
            restore(&bytes, too_small),
            Err(SyncError::OwnerInitializationUnsupported)
        ));
        assert_eq!(DECODER_ENTRIES.with(std::cell::Cell::get), before);
    }
}

#[test]
fn all_declared_inventory_count_bombs_and_unknown_or_missing_critical_tail_refuse_before_common_decode(
) {
    let (_node, bytes) = initial();
    let at = locations(&bytes);
    for offset in at.counts {
        let mut hostile = bytes.to_vec();
        hostile[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(before_common(&hostile), SyncError::Malformed));
    }
    for end in [
        at.tenure.start - 4,
        at.policy.start - 4,
        at.durable.start - 4,
        at.pending.start - 4,
        at.scope.start - 4,
        at.owner.start - 4,
    ] {
        assert!(matches!(
            before_common(&bytes[..end]),
            SyncError::OwnerInitializationUnsupported
        ));
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(matches!(before_common(&trailing), SyncError::Malformed));
    let mut bad_scope = bytes[at.scope.clone()].to_vec();
    bad_scope[4] ^= 1;
    assert!(matches!(
        before_common(&replaced(&bytes, at.scope, &bad_scope)),
        SyncError::Malformed
    ));
}

#[test]
fn closed_field_lengths_counts_and_retained_obligations_are_not_truncated_before_restore() {
    let (_node, bytes) = initial();
    let at = locations(&bytes);
    let mut routing = Encoder::new();
    routing.put_u64(0).put_u32((MAX_ROUTING_SECRETS + 1) as u32);
    for index in 0..=MAX_ROUTING_SECRETS {
        routing.put_u64(index as u64);
        routing.put_bytes(&[1; 32]).unwrap();
    }
    let mut ledger = Encoder::new();
    ledger.put_u32(1);
    ledger.put_bytes(&[2; 16]).unwrap();
    ledger.put_u32(0);
    let mut pending = Encoder::new();
    pending
        .put_u8(1)
        .put_u32((MAX_INITIAL_FINALIZATIONS + 1) as u32);
    for index in 0..=MAX_INITIAL_FINALIZATIONS {
        pending.put_bytes(&[index as u8; 32]).unwrap();
        pending.put_bytes(&[1; 32]).unwrap();
    }
    let mut durable = Encoder::new();
    durable.put_u8(2).put_u32(1).put_u8(0);
    let mut scope = bytes[at.scope.clone()].to_vec();
    let alias_count = scope.len() - 4;
    scope[alias_count..].copy_from_slice(&1u32.to_be_bytes());
    scope.extend_from_slice(&[0; 201]);
    for (range, body) in [
        (at.routing, routing.finish()),
        (at.ledger, ledger.finish()),
        (at.tenure, vec![0; 257]),
        (at.policy, vec![0; 4102]),
        (at.pending, pending.finish()),
        (at.durable, durable.finish()),
        (at.scope, scope),
    ] {
        assert!(matches!(
            before_common(&replaced(&bytes, range, &body)),
            SyncError::OwnerInitializationUnsupported
        ));
    }
}

#[test]
fn rendezvous_and_admin_bounds_precede_owned_strings_or_sets_and_metadata_ceiling_precedes_all_tails(
) {
    let (_node, bytes) = initial();
    let at = locations(&bytes);
    let mut nodes = Encoder::new();
    nodes.put_u32((MAX_INITIAL_RENDEZVOUS + 1) as u32);
    for _ in 0..=MAX_INITIAL_RENDEZVOUS {
        nodes.put_str("").unwrap();
        nodes.put_bytes(&[]).unwrap();
    }
    let mut changed = bytes[..at.counts[4]].to_vec();
    changed.extend_from_slice(&nodes.finish());
    changed.extend_from_slice(&bytes[at.rendezvous_end..]);
    assert!(matches!(
        before_common(&changed),
        SyncError::OwnerInitializationUnsupported
    ));
    let mut long_address = Encoder::new();
    long_address.put_u32(1);
    long_address.put_str(&"x".repeat(1025)).unwrap();
    long_address.put_bytes(&[]).unwrap();
    let mut changed = bytes[..at.counts[4]].to_vec();
    changed.extend_from_slice(&long_address.finish());
    changed.extend_from_slice(&bytes[at.rendezvous_end..]);
    assert!(matches!(
        before_common(&changed),
        SyncError::OwnerInitializationUnsupported
    ));
    let mut admins = Encoder::new();
    admins.put_u32((MAX_INITIAL_ADMINS + 1) as u32);
    for index in 0..=MAX_INITIAL_ADMINS {
        admins.put_bytes(&[index as u8; 32]).unwrap();
    }
    let mut changed = bytes[..at.counts[6]].to_vec();
    changed.extend_from_slice(&admins.finish());
    changed.extend_from_slice(&bytes[at.counts[6] + 4..]);
    assert!(matches!(
        before_common(&changed),
        SyncError::OwnerInitializationUnsupported
    ));
    let mut too_much_metadata = bytes.to_vec();
    too_much_metadata.resize(at.mls.len() + 4 + METADATA_FEE + 1, 0);
    assert!(matches!(
        before_common(&too_much_metadata),
        SyncError::OwnerInitializationUnsupported
    ));
}

#[test]
fn counted_basis_fields_inside_original_owner_facts_refuse_before_common_decoder_or_sdk() {
    let (_node, bytes) = initial();
    let at = locations(&bytes);
    let mut owner = Decoder::new(&bytes[at.owner.clone()]);
    let tag = owner.get_str().unwrap();
    let profile = owner.get_bytes().unwrap();
    let count = owner.get_u32().unwrap();
    let original: Vec<_> = (0..count).map(|_| owner.get_bytes().unwrap()).collect();
    owner.finish().unwrap();
    for bad_count in [
        1u16,
        (catcoms_replication::chat::owner_facts::MAX_BASIS_COVERAGE + 1) as u16,
    ] {
        let mut outer = Encoder::new();
        outer.put_str(tag).unwrap();
        outer.put_bytes(profile).unwrap();
        outer.put_u32(count);
        let mut edited = false;
        for raw in &original {
            let mut envelope = Decoder::new(raw);
            let envelope_tag = envelope.get_bytes().unwrap();
            let semantic = envelope.get_bytes().unwrap();
            let signature = envelope.get_bytes().unwrap();
            envelope.finish().unwrap();
            let mut body = Decoder::new(semantic);
            body.get_bytes().unwrap();
            let kind = body.get_u8().unwrap();
            if kind == 4 {
                body.get_bytes().unwrap();
                body.get_u16().unwrap();
                body.get_bytes().unwrap();
                body.get_bytes().unwrap();
                body.get_u128().unwrap();
                for _ in 0..3 {
                    body.get_bytes().unwrap();
                }
                let offset = semantic.len() - body.remaining();
                assert_eq!(
                    body.get_u16().unwrap(),
                    0,
                    "actual founder starts with empty causal coverage"
                );
                let mut hostile = semantic.to_vec();
                hostile[offset..offset + 2].copy_from_slice(&bad_count.to_be_bytes());
                let mut replacement = Encoder::new();
                replacement.put_bytes(envelope_tag).unwrap();
                replacement.put_bytes(&hostile).unwrap();
                replacement.put_bytes(signature).unwrap();
                outer.put_bytes(&replacement.finish()).unwrap();
                edited = true;
            } else {
                outer.put_bytes(raw).unwrap();
            }
        }
        assert!(edited);
        let error = before_common(&replaced(&bytes, at.owner.clone(), &outer.finish()));
        if bad_count == 1 {
            assert!(matches!(error, SyncError::Malformed));
        } else {
            assert!(matches!(error, SyncError::OwnerInitializationUnsupported));
        }
    }
}

#[tokio::test]
async fn legacy_actual_signed_history_still_restores_while_closed_checkpoint_refuses_it_without_losing_originals(
) {
    use automerge::transaction::Transactable;
    let mut node = founder();
    node.open_channel(DocType::Channel, 9).await.unwrap();
    node.post(DocType::Channel, 9, |document| {
        document.put(
            automerge::ROOT,
            "original",
            "retained exact legacy operation",
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let expected = signed_originals(&mut node);
    let bytes = node.snapshot().unwrap();
    assert!(matches!(
        before_common(&bytes),
        SyncError::OwnerInitializationUnsupported
    ));
    let mut restored = ChannelSync::restore(
        &bytes,
        Hub::new().join(PeerId::from_u64(0xf003)),
        ChaCha20Rng::seed_from_u64(0xf003),
        Box::new(ManualClock::new(2_000)),
    )
    .unwrap();
    let actual = signed_originals(&mut restored);
    assert_eq!(actual, expected);
    assert_eq!(signed_originals(&mut node), expected);
}
