//! Exact affine DATA custody, no worker/commit/saved/paid-quiescence receipt is fabricated.
use super::super::{
    common::*,
    records::{RawSignatureRecordData, TextIntentIdData},
};
use super::*;
use catcoms_replication::chat::{owner_header::OwnerHeaderId, ActionId};
use catcoms_wire::Decoder;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
fn signature(rng: &mut ChaCha20Rng) -> PreparedTextControlFrameData {
    RawSignatureRecordData {
        context: TextContextIdData::from_bytes([1; 32]),
        transaction: TextTransactionIdData::from_bytes([2; 32]),
        intent: TextIntentIdData::from_bytes([3; 32]),
        pre_intent_head: TextHeadIdData::from_bytes([4; 32]),
        public_parent: OwnerHeaderId::from_bytes([5; 32]),
        unsigned_commitment: [6; 32],
        action: ActionId::from_bytes([7; 32]),
        raw: [8; 64],
    }
    .prepare_frame_data(&[9; 32], rng)
    .unwrap()
}
#[test]
fn wrong_role_and_operation_return_same_materialized_original_without_new_crypto() {
    let mut rng = ChaCha20Rng::seed_from_u64(71);
    let frame = signature(&mut rng);
    let id = frame.id_data();
    let ptr = frame.frame_data().as_ptr();
    let (why, frame) =
        ImmutableTextArtifactData::from_control_data(TextArtifactRoleData::Retain, frame)
            .unwrap_err();
    assert_eq!(why, TextArtifactRefusalData::WrongKind);
    assert!(frame.id_data() == id);
    assert_eq!(frame.frame_data().as_ptr(), ptr);
    let object =
        ImmutableTextArtifactData::from_control_data(TextArtifactRoleData::Signature, frame)
            .unwrap();
    let refusal = PreparedTextArtifactSetData::assemble_data(
        TextArtifactOperationData::Unsigned,
        vec![object],
    )
    .unwrap_err();
    assert_eq!(refusal.reason, TextArtifactRefusalData::Geometry);
    assert_eq!(refusal.objects[0].frame_data().as_ptr(), ptr);
    let set = PreparedTextArtifactSetData::assemble_data(
        TextArtifactOperationData::RawSignature,
        refusal.objects,
    )
    .unwrap();
    assert_eq!(
        set.operation_data(),
        TextArtifactOperationData::RawSignature
    );
    assert_eq!(set.raw_bytes_data(), 405);
    assert_eq!(set.packed_bytes_data(), 447);
    let mut e = Encoder::with_capacity(453);
    set.encode_into_data(&mut e).unwrap();
    let raw = e.finish();
    let mut d = Decoder::new(&raw);
    assert_eq!(d.get_u16().unwrap(), 1);
    let packed = d.get_bytes().unwrap();
    d.finish().unwrap();
    assert_eq!(packed.len(), 447);
    let mut p = Decoder::new(packed);
    assert_eq!(p.get_u8().unwrap(), TextArtifactRoleData::Signature as u8);
    assert_eq!(p.get_u8().unwrap(), 2);
    assert_eq!(p.get_bytes().unwrap(), id.as_bytes());
    assert_eq!(p.get_bytes().unwrap(), set.objects_data()[0].frame_data());
    p.finish().unwrap();
    let objects = set.into_objects_data();
    assert_eq!(objects[0].role_data(), TextArtifactRoleData::Signature);
    assert_eq!(objects[0].frame_data().as_ptr(), ptr);
}
#[test]
fn page_control_kind_cap_and_empty_or_extra_geometry_refuse_without_adoption() {
    let mut rng = ChaCha20Rng::seed_from_u64(72);
    let page =
        PreparedPrivateChatPage::prepare(&[9; 32], b"inert inner page DATA", &mut rng).unwrap();
    let id = page.id();
    let ptr = page.as_bytes().as_ptr();
    let (why, page) =
        ImmutableTextArtifactData::from_page_data(TextArtifactRoleData::Signature, page)
            .unwrap_err();
    assert_eq!(why, TextArtifactRefusalData::WrongKind);
    assert_eq!(page.id(), id);
    assert_eq!(page.as_bytes().as_ptr(), ptr);
    let object =
        ImmutableTextArtifactData::from_page_data(TextArtifactRoleData::TokenIndex, page).unwrap();
    let refusal =
        PreparedTextArtifactSetData::assemble_data(TextArtifactOperationData::Retain, vec![object])
            .unwrap_err();
    assert_eq!(refusal.reason, TextArtifactRefusalData::Geometry);
    assert_eq!(refusal.objects[0].frame_data().as_ptr(), ptr);
    let empty =
        PreparedTextArtifactSetData::assemble_data(TextArtifactOperationData::RawSignature, vec![])
            .unwrap_err();
    assert_eq!(empty.reason, TextArtifactRefusalData::Bound);
    let a = ImmutableTextArtifactData::from_control_data(
        TextArtifactRoleData::Signature,
        signature(&mut rng),
    )
    .unwrap();
    let b = ImmutableTextArtifactData::from_control_data(
        TextArtifactRoleData::Signature,
        signature(&mut rng),
    )
    .unwrap();
    let refusal = PreparedTextArtifactSetData::assemble_data(
        TextArtifactOperationData::RawSignature,
        vec![a, b],
    )
    .unwrap_err();
    assert_eq!(refusal.reason, TextArtifactRefusalData::Bound);
    assert_eq!(refusal.objects.len(), 2);
}
#[test]
fn spare_vector_backing_is_bounded_and_preserved_on_owning_refusal() {
    let mut rng = ChaCha20Rng::seed_from_u64(73);
    let frame = signature(&mut rng);
    let id = frame.id_data();
    let ptr = frame.frame_data().as_ptr();
    let object =
        ImmutableTextArtifactData::from_control_data(TextArtifactRoleData::Signature, frame)
            .unwrap();
    let mut objects = Vec::with_capacity(71);
    objects.push(object);
    let capacity = objects.capacity();
    let refusal = PreparedTextArtifactSetData::assemble_data(
        TextArtifactOperationData::RawSignature,
        objects,
    )
    .unwrap_err();
    assert_eq!(refusal.reason, TextArtifactRefusalData::Bound);
    assert_eq!(refusal.objects.capacity(), capacity);
    assert_eq!(refusal.objects.len(), 1);
    assert_eq!(refusal.objects[0].id_data(), *id.as_bytes());
    assert_eq!(refusal.objects[0].frame_data().as_ptr(), ptr);
}
