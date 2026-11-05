//! Authenticated secret DATA only. None of these fixtures claims a durable mounted join,
//! actual catalog/source capture, Current publication or physical crash acceptance. UNRUN.
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

fn limits() -> DirectJoinLimits {
    DirectJoinLimits::new(512 * 1024, 512 * 1024, 1024 * 1024).unwrap()
}
fn prepare() -> PreparedDirectJoin {
    let hub = Hub::new();
    let mut owner = ChannelSync::found(
        hub.join(PeerId::from_u64(0x71)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0x71),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let invite = owner.mint_invite([0x71; 16], 10_000, Vec::new()).unwrap();
    PreparedDirectJoin::prepare(MlsDevice::generate().unwrap(), invite, limits()).unwrap()
}
fn encode(
    source: &PreparedDirectJoin,
    binding: &[u8; 32],
    master: &[u8],
    request: &[u8],
) -> Zeroizing<Vec<u8>> {
    let mut output = Zeroizing::new(Vec::new());
    append(&mut output, TAG.as_bytes());
    output.push(CHECKPOINT_DIRECT);
    output.push(ROOT_OWNER_FACT_V1);
    append(&mut output, binding);
    append(&mut output, source.signer());
    append(&mut output, &source.expected);
    append(&mut output, master);
    append(&mut output, request);
    output
}
fn encrypt(plain: &[u8]) -> SealedDirectJoinSource {
    SealedDirectJoinSource {
        blob: seal(&[0x31; 32], plain, &mut ChaCha20Rng::seed_from_u64(0x71)).unwrap(),
        local: None,
    }
}

#[test]
fn original_seal_is_exact_and_repeated_binding_uses_no_new_nonce() {
    let mut source = prepare();
    let key = [0x31; 32];
    let binding = [0x41; 32];
    let request = source.request.clone();
    let master = source.master.clone();
    let mut rng = ChaCha20Rng::seed_from_u64(0x71);
    let sealed = source.seal_source(&key, binding, &mut rng).unwrap();
    let (nonce, ciphertext) = sealed.encrypted_parts();
    let saved = SealedDirectJoinSource::from_encrypted_parts(*nonce, ciphertext).unwrap();
    let mut expected_rng = rng.clone();
    let sealed = source.seal_source(&key, binding, &mut rng).unwrap();
    assert_eq!(sealed.encrypted_parts(), saved.encrypted_parts());
    assert_eq!(rng.next_u64(), expected_rng.next_u64());
    assert!(source.seal_source(&key, [0x42; 32], &mut rng).is_err());
    assert!(source.seal_source(&[0x32; 32], binding, &mut rng).is_err());
    let mut recovered = RecoveredDirectJoinSource::open(&saved, &key, binding, limits()).unwrap();
    assert_eq!(recovered.original.master.as_slice(), master.as_slice());
    assert_eq!(recovered.original.request, request);
    assert_eq!(recovered.group(), source.group());
    assert_eq!(recovered.signer(), source.signer());
    assert_eq!(recovered.source_binding(), &binding);
    // No public adapter exposes `original`; keep the defense even if a later actual consumer
    // obtains the internal field. Recovery can never create a new seal/original identity.
    assert!(recovered
        .original
        .seal_source(&key, binding, &mut rng)
        .is_err());
    assert!(!format!("{recovered:?}").contains("master"));
}

#[test]
fn encrypted_frame_caps_wrong_key_binding_and_tampering_refuse() {
    assert!(SealedDirectJoinSource::from_encrypted_parts([0; 24], &[0; 15]).is_err());
    assert!(SealedDirectJoinSource::from_encrypted_parts(
        [0; 24],
        &vec![0; MAX_PLAIN + TAG_BYTES + 1]
    )
    .is_err());
    let mut source = prepare();
    let sealed = source
        .seal_source(
            &[0x31; 32],
            [0x41; 32],
            &mut ChaCha20Rng::seed_from_u64(0x71),
        )
        .unwrap();
    assert!(RecoveredDirectJoinSource::open(sealed, &[0x32; 32], [0x41; 32], limits()).is_err());
    assert!(RecoveredDirectJoinSource::open(sealed, &[0x31; 32], [0x42; 32], limits()).is_err());
    let (nonce, ciphertext) = sealed.encrypted_parts();
    let mut wrong_nonce = *nonce;
    wrong_nonce[0] ^= 1;
    let bad = SealedDirectJoinSource::from_encrypted_parts(wrong_nonce, ciphertext).unwrap();
    assert!(RecoveredDirectJoinSource::open(&bad, &[0x31; 32], [0x41; 32], limits()).is_err());
    let mut changed = ciphertext.to_vec();
    changed[0] ^= 1;
    let bad = SealedDirectJoinSource::from_encrypted_parts(*nonce, &changed).unwrap();
    assert!(RecoveredDirectJoinSource::open(&bad, &[0x31; 32], [0x41; 32], limits()).is_err());
}

#[test]
fn authenticated_nested_malformed_counts_and_unknown_flavor_refuse_before_owned_decode() {
    let source = prepare();
    let binding = [0x41; 32];
    let base = encode(&source, &binding, &source.master, &source.request);
    let before = OWNED_DECODES.with(std::cell::Cell::get);
    let mut cases = Vec::new();
    let mut trailing = base.to_vec();
    trailing.push(0);
    cases.push(Zeroizing::new(trailing));
    let mut flavor = base.to_vec();
    flavor[4 + TAG.len()] = 2; // future v2 join must not enter this legacy checkpoint reader
    cases.push(Zeroizing::new(flavor));
    let mut root_kind = base.to_vec();
    root_kind[4 + TAG.len() + 1] = 2; // a same-width public-owner v2 root is a different authority
    cases.push(Zeroizing::new(root_kind));
    for length in [0, 1, 4, base.len() - 1] {
        cases.push(Zeroizing::new(base[..length].to_vec()));
    }
    let mut request_decoder = Decoder::new(&source.request[1..]);
    let invite = request_decoder.get_bytes().unwrap();
    let package = request_decoder.get_bytes().unwrap();
    let mut invite_decoder = Decoder::new(invite);
    invite_decoder.get_str().unwrap();
    for _ in 0..4 {
        invite_decoder.get_bytes().unwrap();
    }
    invite_decoder.get_u64().unwrap();
    let count_at = invite.len() - invite_decoder.remaining();
    let mut bad_invite = invite.to_vec();
    bad_invite[count_at..count_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut request = vec![KIND_JOIN];
    append(&mut request, &bad_invite);
    append(&mut request, package);
    cases.push(encode(&source, &binding, &source.master, &request));
    let mut extra_request = source.request.to_vec();
    extra_request.push(0);
    cases.push(encode(&source, &binding, &source.master, &extra_request));
    for plain in cases {
        assert!(
            RecoveredDirectJoinSource::open(&encrypt(&plain), &[0x31; 32], binding, limits())
                .is_err()
        );
    }
    assert_eq!(OWNED_DECODES.with(std::cell::Cell::get), before);
}

#[test]
fn same_signer_master_without_selected_private_bundle_cannot_recover() {
    let hub = Hub::new();
    let mut owner = ChannelSync::found(
        hub.join(PeerId::from_u64(0x81)),
        MlsDevice::generate().unwrap(),
        ChaCha20Rng::seed_from_u64(0x81),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let invite = owner.mint_invite([0x81; 16], 10_000, Vec::new()).unwrap();
    let device = MlsDevice::generate().unwrap();
    let earlier =
        snapshot_join_device_bounded(&device, limits().provider, limits().backing).unwrap();
    let source = PreparedDirectJoin::prepare(device, invite, limits()).unwrap();
    let binding = [0x41; 32];
    let plain = encode(&source, &binding, &earlier, &source.request);
    assert!(
        RecoveredDirectJoinSource::open(&encrypt(&plain), &[0x31; 32], binding, limits()).is_err()
    );
    let actual = encode(&source, &binding, &source.master, &source.request);
    assert!(
        RecoveredDirectJoinSource::open(&encrypt(&actual), &[0x31; 32], binding, limits()).is_ok()
    );
}

#[test]
fn actual_negative_limits_refuse_saved_source_without_changing_original() {
    let mut source = prepare();
    let master = source.master.clone();
    let request = source.request.clone();
    let sealed = source
        .seal_source(
            &[0x31; 32],
            [0x41; 32],
            &mut ChaCha20Rng::seed_from_u64(0x71),
        )
        .unwrap();
    for limits in [
        DirectJoinLimits::new(1, limits().backing, limits().exchange).unwrap(),
        DirectJoinLimits::new(limits().provider, 1, limits().exchange).unwrap(),
        DirectJoinLimits::new(limits().provider, limits().backing, 1).unwrap(),
    ] {
        assert!(RecoveredDirectJoinSource::open(sealed, &[0x31; 32], [0x41; 32], limits).is_err());
    }
    assert_eq!(source.master.as_slice(), master.as_slice());
    assert_eq!(source.request, request);
}

#[tokio::test]
async fn dropping_unpolled_finish_does_not_send_but_first_request_poll_irrevocably_closes_sealing()
{
    let mut source = prepare();
    let hub = Hub::new();
    let owner = hub.join(PeerId::from_u64(0x91));
    let member = hub.join(PeerId::from_u64(0x92));
    let mut handoff = PreOwnerConnectionHandoff::default();
    // Async creation alone executes no body/transport call. There is no ambiguous send yet.
    drop(source.finish_tracking(&member, &mut handoff, PeerId::from_u64(0x91)));
    source
        .seal_source(
            &[0x31; 32],
            [0x41; 32],
            &mut ChaCha20Rng::seed_from_u64(0x71),
        )
        .unwrap();
    let mut future =
        Box::pin(source.finish_tracking(&member, &mut handoff, PeerId::from_u64(0x91)));
    assert!(futures::poll!(&mut future).is_pending());
    drop(future);
    assert!(source.request_started);
    assert!(source
        .seal_source(
            &[0x31; 32],
            [0x41; 32],
            &mut ChaCha20Rng::seed_from_u64(0x71)
        )
        .is_err());
    drop(owner);
}
