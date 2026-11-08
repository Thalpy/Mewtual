//! Real unsigned SDK DATA/codec conformance. No SavedI or Append signing is fabricated.
use super::*;
use crate::store::persistence::v2_text::{
    encode_unsigned_intent_data, TextUnsignedIntentBindingsData, TextUnsignedIntentFrameData,
};
use catcoms_replication::chat::owner_state::transition::{ReplayMath, REPLAY_MATH_PRIVATE_INVOICE};
use catcoms_sync::owner_v2::{V2UnsignedAppendDataView, APPEND_DATA_BUFFER_BYTES};

fn commitment(signing: &[u8], image: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/v2-unsigned-append-intent/v1\0");
    hash.update(&(signing.len() as u64).to_be_bytes());
    hash.update(signing);
    hash.update(&(image.len() as u64).to_be_bytes());
    hash.update(image);
    *hash.finalize().as_bytes()
}

#[test]
fn actual_unsigned_plan_intent_is_strict_bounded_data_and_not_a_selected_signing_permit() {
    let mut f = super::super::promotion::promoted_text_fixture(0, 0x7f240);
    let selected = selected_parent(&f);
    let mut data = retain(&f, &selected, "one original unsigned child");
    let _create_fee = attach_original(&mut f, &mut data, "one original unsigned child");
    let store = f.layout.checked_store().unwrap();
    let owner = f.owner.borrow_current().unwrap();
    data.prepare_pages(
        store,
        owner.source(),
        &mut ChaCha20Rng::seed_from_u64(0x7f241),
    )
    .unwrap();
    let pool = &store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool;
    // Mathematical resident state and SDK append preparation have THEIR OWN real fees.
    let _model_fee = pool.try_reserve(REPLAY_MATH_PRIVATE_INVOICE).unwrap();
    let _append_fee = pool.try_reserve(APPEND_DATA_BUFFER_BYTES).unwrap();
    let source = owner.source().data();
    let parent = source.header.id();
    let parent_sequence = source.header.spec().sequence;
    let model = ReplayMath::from_genesis(
        source.profile,
        source.admission,
        source.header,
        source.state,
    )
    .unwrap();
    drop(owner);
    let mut append = None;
    f.owner
        .prepare_append_data_for_test(store, &model, data.action_data().unwrap(), &mut append)
        .unwrap();
    let view = f
        .owner
        .borrow_unsigned_append_for_test(store, &model, append.as_ref().unwrap())
        .unwrap();
    let bindings = TextUnsignedIntentBindingsData {
        context: data.context_data().id_data().unwrap(),
        transaction: data.transaction_data(),
        retain: data.retain_frame_data().unwrap().unwrap().1,
        token: data.token_data(),
        request: data.request_data(),
        // These are DATA fixtures. This test does not create/select O_A, I or a SavedI.
        // The real factory must bind all outer source fields to actual selected A and I.
        pre_intent_head: data.parent_head_data(),
        pre_intent_archive: data.parent_archive_data(),
        accepted_root: [7; 32],
        token_root: [8; 32],
        public_parent: parent,
        issuer: *view.action.signer(),
        unsigned_commitment: view.intent,
    };
    let raw =
        encode_unsigned_intent_data(data.context_data(), bindings, parent_sequence, &view).unwrap();
    assert!(raw.len() <= 2809);
    let parsed = TextUnsignedIntentFrameData::decode_data(&raw, data.context_data()).unwrap();
    assert!(parsed.matches_unsigned_data(&view));
    assert_eq!(parsed.bindings, bindings);
    for n in 0..raw.len() {
        assert!(TextUnsignedIntentFrameData::decode_data(&raw[..n], data.context_data()).is_err());
    }
    let mut trailing = raw.to_vec();
    trailing.push(0);
    assert!(TextUnsignedIntentFrameData::decode_data(&trailing, data.context_data()).is_err());
    assert!(TextUnsignedIntentFrameData::decode_data(&vec![0; 2810], data.context_data()).is_err());
    // Frozen42-byte tag/version prefix, followed by twelve canonical b32 fields.
    for field in [0, 1, 3, 4, 9, 10, 11] {
        let mut bad = raw.to_vec();
        bad[42 + field * 36 + 4] ^= 1;
        assert!(
            TextUnsignedIntentFrameData::decode_data(&bad, data.context_data()).is_err(),
            "field {field}"
        );
    }
    let mut overflow = raw.to_vec();
    overflow[474..482].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(TextUnsignedIntentFrameData::decode_data(&overflow, data.context_data()).is_err());
    let mut overlong = raw.to_vec();
    overlong[482..486].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(TextUnsignedIntentFrameData::decode_data(&overlong, data.context_data()).is_err());
    let mut empty = raw.to_vec();
    empty.drain(486..486 + view.signing_bytes.len());
    empty[482..486].copy_from_slice(&0u32.to_be_bytes());
    assert!(TextUnsignedIntentFrameData::decode_data(&empty, data.context_data()).is_err());

    // A consistent arbitrary signing preimage is STILL just untrusted DATA. The actual
    // SDK view comparison refuses it. There is deliberately no decoded-bytes signer API.
    let changed = V2UnsignedAppendDataView {
        action: view.action,
        record: view.record,
        body: view.body,
        signing_bytes: b"not the original canonical header preimage",
        image: view.image,
        intent: commitment(b"not the original canonical header preimage", view.image),
    };
    let mut changed_bindings = bindings;
    changed_bindings.unsigned_commitment = changed.intent;
    let changed_raw = encode_unsigned_intent_data(
        data.context_data(),
        changed_bindings,
        parent_sequence,
        &changed,
    )
    .unwrap();
    let changed_data =
        TextUnsignedIntentFrameData::decode_data(&changed_raw, data.context_data()).unwrap();
    assert!(!changed_data.matches_unsigned_data(&view));

    // Conversely, SDK-field equality alone does NOT attest an outer private-root label.
    // Pin this limitation so future selected-I factory code cannot treat this matcher
    // as the genuine Current/retained-A/transaction/original-plan proof it still needs.
    let mut other_outer = bindings;
    other_outer.accepted_root[0] ^= 1;
    let other_raw =
        encode_unsigned_intent_data(data.context_data(), other_outer, parent_sequence, &view)
            .unwrap();
    assert!(
        TextUnsignedIntentFrameData::decode_data(&other_raw, data.context_data())
            .unwrap()
            .matches_unsigned_data(&view)
    );
}
