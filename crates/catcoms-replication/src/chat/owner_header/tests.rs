//! Real signature/strict framing checks; no test key or raw header label is owner provenance.
use super::*;
use catcoms_crypto::DeviceKeypair;
use std::cell::Cell;

fn key(seed: u8) -> DeviceKeypair {
    DeviceKeypair::from_seed(&[seed; 32])
}
fn spec() -> HeaderSpec {
    HeaderSpec {
        group: vec![7; MAX_GROUP_ID_BYTES],
        profile: OwnerFactId::from_bytes([11; 32]),
        issuer: key(47).verifying_key().to_bytes(),
        tenure: OwnerTenureId::from_bytes([12; 32]),
        parent: OwnerHeaderId::from_bytes([13; 32]),
        sequence: 0x0102_0304_0506_0708,
        kind: HeaderKind::AcceptanceAppend,
        body: OwnerBodyId::from_bytes([14; 32]),
        state: OwnerStateCommitment::from_bytes([15; 32]),
    }
}
fn sign(spec: HeaderSpec) -> SignedOwnerHeader {
    SignedOwnerHeader::sign_using(spec, |bytes| Ok(key(47).sign(bytes))).unwrap()
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn raw_semantic(spec: &HeaderSpec, version: u16, kind: u8) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, b"mewtual/chat/owner-header/v2\0");
    out.extend_from_slice(&version.to_be_bytes());
    field(&mut out, &spec.group);
    field(&mut out, spec.profile.as_bytes());
    field(&mut out, &spec.issuer);
    field(&mut out, spec.tenure.as_bytes());
    field(&mut out, spec.parent.as_bytes());
    out.extend_from_slice(&spec.sequence.to_be_bytes());
    out.push(kind);
    field(&mut out, spec.body.as_bytes());
    field(&mut out, spec.state.as_bytes());
    out
}
fn wrap(semantic: &[u8], signature: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, b"mewtual/chat/owner-header-envelope/v2\0");
    field(&mut out, semantic);
    field(&mut out, signature);
    out
}
fn unhex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn all_closed_header_kinds_round_trip_real_signatures_at_exact_maximum() {
    for kind in [
        HeaderKind::Admission,
        HeaderKind::AcceptanceAppend,
        HeaderKind::Removal,
        HeaderKind::ReviewDecision,
    ] {
        let mut input = spec();
        input.kind = kind;
        let header = sign(input);
        assert!(header.verify());
        assert_eq!(header.semantic_bytes().len(), 520);
        let encoded = header.encode();
        assert_eq!(encoded.len(), 634);
        let decoded = SignedOwnerHeader::decode(&encoded).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(decoded.encode(), encoded);
        assert!(decoded.verify());
    }
    assert_eq!(MAX_HEADER_SEMANTIC_BYTES, 520);
    assert_eq!(MAX_HEADER_BYTES, 634);
}

#[test]
fn exact_domains_length_framing_and_big_endian_sequence_match_independent_bytes() {
    let input = spec();
    let header = sign(input.clone());
    let semantic = raw_semantic(&input, 2, 2);
    assert_eq!(header.semantic_bytes(), semantic);
    assert_eq!(header.encode(), wrap(&semantic, header.signature()));
    let mut id_input = b"mewtual/chat/owner-header-id/v2\0".to_vec();
    id_input.extend_from_slice(&semantic);
    assert_eq!(header.id().as_bytes(), blake3::hash(&id_input).as_bytes());
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("signature-variants.json")).unwrap();
    assert_eq!(semantic, unhex(fixture["semantic"].as_str().unwrap()));
}

#[test]
fn genuine_alternate_signing_nonces_normalize_but_each_representation_must_verify() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("signature-variants.json")).unwrap();
    let headers: Vec<_> = fixture["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| SignedOwnerHeader::decode(&unhex(frame.as_str().unwrap())).unwrap())
        .collect();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[0], sign(spec()), "fixture seed47 normal signer");
    assert!(headers.iter().all(SignedOwnerHeader::verify));
    assert_ne!(headers[0].signature(), headers[1].signature());
    assert_ne!(headers[0], headers[1]);
    assert_eq!(headers[0].id(), headers[1].id());
    assert_eq!(headers[0].spec(), headers[1].spec());
    assert_eq!(headers[0].semantic_bytes(), headers[1].semantic_bytes());

    let mut corrupted = *headers[0].signature();
    corrupted[0] ^= 1;
    let bad = SignedOwnerHeader::decode(&wrap(&headers[0].semantic_bytes(), &corrupted)).unwrap();
    assert_eq!(bad.id(), headers[0].id());
    assert!(!bad.verify());
    assert!(
        headers[1].verify(),
        "invalid-first identity must not poison the good representation"
    );
}

#[test]
fn every_signed_scope_and_commitment_field_changes_semantic_identity() {
    let original = sign(spec());
    let mut variants = Vec::new();
    let mut changed = spec();
    changed.group[0] ^= 1;
    variants.push(changed);
    let mut changed = spec();
    changed.profile = OwnerFactId::from_bytes([21; 32]);
    variants.push(changed);
    let mut changed = spec();
    changed.tenure = OwnerTenureId::from_bytes([22; 32]);
    variants.push(changed);
    let mut changed = spec();
    changed.parent = OwnerHeaderId::from_bytes([23; 32]);
    variants.push(changed);
    let mut changed = spec();
    changed.sequence += 1;
    variants.push(changed);
    let mut changed = spec();
    changed.kind = HeaderKind::Removal;
    variants.push(changed);
    let mut changed = spec();
    changed.body = OwnerBodyId::from_bytes([24; 32]);
    variants.push(changed);
    let mut changed = spec();
    changed.state = OwnerStateCommitment::from_bytes([25; 32]);
    variants.push(changed);
    for changed in variants {
        let header = sign(changed);
        assert!(header.verify());
        assert_ne!(header.id(), original.id());
        assert_ne!(header.semantic_bytes(), original.semantic_bytes());
        let stale_signature =
            SignedOwnerHeader::decode(&wrap(&header.semantic_bytes(), original.signature()))
                .unwrap();
        assert!(!stale_signature.verify());
    }
    let mut other_issuer = spec();
    other_issuer.issuer = key(48).verifying_key().to_bytes();
    let header =
        SignedOwnerHeader::sign_using(other_issuer, |bytes| Ok(key(48).sign(bytes))).unwrap();
    assert_ne!(header.id(), original.id());
}

#[test]
fn malformed_outer_inner_fixed_fields_and_all_truncated_prefixes_refuse() {
    let header = sign(spec());
    let encoded = header.encode();
    for end in 0..encoded.len() {
        assert!(
            SignedOwnerHeader::decode(&encoded[..end]).is_err(),
            "prefix {end}"
        );
    }
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert_eq!(SignedOwnerHeader::decode(&trailing), Err(ChatError::Bound));
    for length in [0, MAX_GROUP_ID_BYTES + 1] {
        let mut invalid = spec();
        invalid.group = vec![7; length];
        assert!(SignedOwnerHeader::decode(&wrap(&raw_semantic(&invalid, 2, 2), &[0; 64])).is_err());
    }
    for version in [0, 1, 3, u16::MAX] {
        assert_eq!(
            SignedOwnerHeader::decode(&wrap(&raw_semantic(&spec(), version, 2), &[0; 64])),
            Err(ChatError::Scope)
        );
    }
    for kind in [0, 5, 255] {
        assert_eq!(
            SignedOwnerHeader::decode(&wrap(&raw_semantic(&spec(), 2, kind), &[0; 64])),
            Err(ChatError::Malformed)
        );
    }
    let mut zero_sequence = spec();
    zero_sequence.sequence = 0;
    assert_eq!(
        SignedOwnerHeader::decode(&wrap(&raw_semantic(&zero_sequence, 2, 2), &[0; 64])),
        Err(ChatError::Dependency)
    );

    let first_digest = 4 + HEADER_DOMAIN.len() + 2 + 4 + MAX_GROUP_ID_BYTES;
    let sequence = first_digest + 4 * 36;
    for position in [
        first_digest,
        first_digest + 36,
        first_digest + 72,
        first_digest + 108,
        sequence + 9,
        sequence + 9 + 36,
    ] {
        for wrong in [0u32, 31, 33, u32::MAX] {
            let mut malformed = header.semantic_bytes();
            malformed[position..position + 4].copy_from_slice(&wrong.to_be_bytes());
            assert!(SignedOwnerHeader::decode(&wrap(&malformed, header.signature())).is_err());
        }
    }
    for length in [0, 63, 65] {
        assert!(
            SignedOwnerHeader::decode(&wrap(&header.semantic_bytes(), &vec![0; length])).is_err()
        );
    }
    // Use a shorter group so the malformed extra byte stays below the envelope cap.
    let mut short = spec();
    short.group = vec![7];
    let mut inner_trailing = raw_semantic(&short, 2, 2);
    inner_trailing.push(0);
    assert!(SignedOwnerHeader::decode(&wrap(&inner_trailing, &[0; 64])).is_err());
    let mut wrong_domain = header.semantic_bytes();
    wrong_domain[4] ^= 1;
    assert!(SignedOwnerHeader::decode(&wrap(&wrong_domain, header.signature())).is_err());
}

fn chain() -> (SignedOwnerHeader, SignedOwnerHeader) {
    let mut parent = spec();
    parent.sequence = 1;
    let parent = sign(parent);
    let mut child = spec();
    child.parent = parent.id();
    child.sequence = 2;
    (parent, sign(child))
}

#[test]
fn checked_successors_require_both_signatures_all_scope_fields_and_exact_parent_sequence() {
    let (parent, child) = chain();
    assert_eq!(child.verify_successor_of(&parent), Ok(()));
    for field in 0..4 {
        let mut changed = child.spec().clone();
        match field {
            0 => changed.group[0] ^= 1,
            1 => changed.profile = OwnerFactId::from_bytes([51; 32]),
            2 => changed.tenure = OwnerTenureId::from_bytes([52; 32]),
            _ => changed.issuer = key(48).verifying_key().to_bytes(),
        }
        let issuer = if field == 3 { 48 } else { 47 };
        let foreign =
            SignedOwnerHeader::sign_using(changed, |bytes| Ok(key(issuer).sign(bytes))).unwrap();
        assert_eq!(foreign.verify_successor_of(&parent), Err(ChatError::Scope));
    }
    let mut changed = child.spec().clone();
    changed.parent = OwnerHeaderId::from_bytes([53; 32]);
    assert_eq!(
        sign(changed).verify_successor_of(&parent),
        Err(ChatError::Dependency)
    );
    for sequence in [1, 3, u64::MAX] {
        let mut changed = child.spec().clone();
        changed.sequence = sequence;
        assert_eq!(
            sign(changed).verify_successor_of(&parent),
            Err(ChatError::Dependency)
        );
    }
    let mut bad_signature = *parent.signature();
    bad_signature[0] ^= 1;
    let bad_parent =
        SignedOwnerHeader::decode(&wrap(&parent.semantic_bytes(), &bad_signature)).unwrap();
    assert_eq!(bad_parent.id(), parent.id());
    assert_eq!(
        child.verify_successor_of(&bad_parent),
        Err(ChatError::Signature)
    );
    let bad_child =
        SignedOwnerHeader::decode(&wrap(&child.semantic_bytes(), &bad_signature)).unwrap();
    assert_eq!(
        bad_child.verify_successor_of(&parent),
        Err(ChatError::Signature)
    );
}

#[test]
fn sequence_exhaustion_does_not_wrap_and_body_presence_cannot_select_a_successor() {
    let (parent, child) = chain();
    let mut conflicting = child.spec().clone();
    conflicting.body = OwnerBodyId::from_bytes([61; 32]);
    let conflicting = sign(conflicting);
    assert_ne!(child.id(), conflicting.id());
    assert_eq!(child.verify_successor_of(&parent), Ok(()));
    assert_eq!(conflicting.verify_successor_of(&parent), Ok(()));
    // No body is supplied to this math-only API; neither success chooses a governing winner.
    let mut exhausted = parent.spec().clone();
    exhausted.sequence = u64::MAX;
    let exhausted = sign(exhausted);
    let mut wrapped = child.spec().clone();
    wrapped.parent = exhausted.id();
    wrapped.sequence = 1;
    assert_eq!(
        sign(wrapped).verify_successor_of(&exhausted),
        Err(ChatError::Dependency)
    );
}

#[test]
fn signing_callback_refuses_wrong_key_and_errors_and_invalid_grammar_before_custody() {
    let positive_calls = Cell::new(0);
    let positive = SignedOwnerHeader::sign_using(spec(), |bytes| {
        positive_calls.set(positive_calls.get() + 1);
        assert_eq!(bytes, raw_semantic(&spec(), 2, 2));
        Ok(key(47).sign(bytes))
    })
    .unwrap();
    assert!(positive.verify());
    assert_eq!(positive_calls.get(), 1);
    assert_eq!(
        SignedOwnerHeader::sign_using(spec(), |bytes| Ok(key(48).sign(bytes))),
        Err(ChatError::Signature)
    );
    assert_eq!(
        SignedOwnerHeader::sign_using(spec(), |_| Err(ChatError::Permission)),
        Err(ChatError::Permission)
    );
    let calls = Cell::new(0);
    let mut invalid = spec();
    invalid.sequence = 0;
    assert!(SignedOwnerHeader::sign_using(invalid, |_| {
        calls.set(calls.get() + 1);
        Ok([0; 64])
    })
    .is_err());
    for group in [Vec::new(), vec![7; MAX_GROUP_ID_BYTES + 1]] {
        let mut invalid = spec();
        invalid.group = group;
        assert!(SignedOwnerHeader::sign_using(invalid, |_| {
            calls.set(calls.get() + 1);
            Ok([0; 64])
        })
        .is_err());
    }
    assert_eq!(calls.get(), 0);
}

#[test]
fn legacy_owner_fact_and_v2_header_envelopes_refuse_each_other() {
    use crate::chat::owner_facts::{OwnerFactBody, OwnerFactSpec, SignedOwnerFact};
    let key = key(47);
    let legacy = SignedOwnerFact::sign_using(
        OwnerFactSpec {
            group: vec![7],
            profile: None,
            body: OwnerFactBody::Profile {
                creation: [1; 32],
                founder: key.verifying_key().to_bytes(),
            },
        },
        key.verifying_key().to_bytes(),
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    assert!(SignedOwnerHeader::decode(&legacy.encode()).is_err());
    assert!(SignedOwnerFact::decode(&sign(spec()).encode()).is_err());
}
