//! Framing-only adversarial regressions. These intentionally do NOT claim that synthetic
//! signatures/MLS bytes below establish an accepted operation. Actual signed Add/root/Welcome
//! tests belong to the covering owner/member consumer and must run before activation.
use super::*;

#[test]
fn source_expectation_debug_omits_scope_and_predecessor_labels() {
    let (tail, _) = shape();
    assert_eq!(
        format!("{:?}", tail.predecessor),
        "OwnerOperationPredecessor { .. }"
    );
    let expected = super::super::membership_restore::OwnerOperationSourceExpectation {
        group: &tail.group,
        signer: &tail.owner,
        profile: tail.profile,
        predecessor: tail.predecessor,
        request: tail.binding,
    };
    assert_eq!(
        format!("{expected:?}"),
        "OwnerOperationSourceExpectation { .. }"
    );
}

fn shape() -> (OwnerOperationTail, CommitRecord) {
    let record = CommitRecord {
        group_id: vec![1; 16],
        commit_epoch: 0,
        committer_device: [2; 32],
        mls_commit: vec![3; 32],
        base_authenticator: [4; 32],
        committer_sig: [5; 64],
    };
    let mut routing = Encoder::new();
    routing.put_bytes(&[6; 24]).unwrap();
    routing.put_bytes(&[7; 16]).unwrap();
    let mut response = Encoder::new();
    response.put_u8(JOIN_READY);
    response.put_bytes(&[8; 24]).unwrap();
    response.put_bytes(&[9; 64]).unwrap();
    response.put_bytes(&routing.finish()).unwrap();
    let group = record.group_id.clone();
    let owner = [10; 32];
    let profile = OwnerFactId::from_bytes([11; 32]);
    let invite = vec![12; 16];
    let package = vec![13; 32];
    let channel = 14;
    let binding = request_binding(&group, &owner, profile, channel, &invite, &package);
    (
        OwnerOperationTail {
            group,
            owner,
            profile,
            predecessor: OwnerOperationPredecessor {
                domain: [15; 32],
                nonce: [16; 32],
                head: [17; 32],
                generation: 1,
                snapshot: [18; 32],
            },
            channel,
            nonce: [19; 16],
            subject: [20; 32],
            reference: [21; 32],
            binding,
            invite,
            package,
            response: response.finish(),
            topic: vec![22; 32],
            commit: record.commit_id(),
            before: [23; 32],
            after: [24; 32],
        },
        record,
    )
}

#[test]
fn owner_operation_complete_borrowed_frame_roundtrips_without_result_duplication() {
    let (original, commit) = shape();
    let bytes = original.encode(&commit).unwrap();
    let raw = preflight(&bytes).unwrap();
    assert_eq!(raw.invite, original.invite);
    assert_eq!(raw.package, original.package);
    assert_eq!(raw.response, original.response);
    let decoded = OwnerOperationTail::decode(&bytes, &commit).unwrap();
    assert_eq!(decoded.encode(&commit).unwrap(), bytes);
    assert!(decoded.exact_request(original.channel, &original.invite, &original.package));
    assert_eq!(response_view(&decoded.response).unwrap().welcome, [8; 24]);
}

#[test]
fn owner_operation_every_truncation_trailing_count_and_length_refuse_before_owned_decode() {
    let (original, commit) = shape();
    let bytes = original.encode(&commit).unwrap();
    let before = OWNED_DECODE_ENTRIES.with(std::cell::Cell::get);
    for end in 0..bytes.len() {
        assert!(OwnerOperationTail::decode(&bytes[..end], &commit).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(OwnerOperationTail::decode(&trailing, &commit).is_err());
    let mut keys = bytes.clone();
    let last = keys.len() - 1;
    keys[last] = 1;
    assert!(matches!(
        OwnerOperationTail::decode(&keys, &commit),
        Err(SyncError::OwnerInitializationUnsupported)
    ));
    let mut long_group = bytes.clone();
    let group_length = 4 + TAG.len();
    long_group[group_length..group_length + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(OwnerOperationTail::decode(&long_group, &commit).is_err());
    let mut unknown = bytes.clone();
    unknown[4] ^= 1;
    assert!(OwnerOperationTail::decode(&unknown, &commit).is_err());
    assert_eq!(OWNED_DECODE_ENTRIES.with(std::cell::Cell::get), before);
}

#[test]
fn owner_operation_exact_identity_includes_channel_and_all_original_bytes() {
    let (original, _) = shape();
    assert!(!original.exact_request(original.channel + 1, &original.invite, &original.package));
    let mut package = original.package.clone();
    package[0] ^= 1;
    assert!(!original.exact_request(original.channel, &original.invite, &package));
    let mut invite = original.invite.clone();
    invite[0] ^= 1;
    assert!(!original.exact_request(original.channel, &invite, &original.package));
    assert_ne!(
        original.binding,
        request_binding(
            &original.group,
            &original.owner,
            original.profile,
            original.channel + 1,
            &original.invite,
            &original.package,
        )
    );
}

#[test]
fn owner_operation_cannot_encode_another_commit_or_unsupported_generation() {
    let (mut original, mut commit) = shape();
    commit.mls_commit[0] ^= 1;
    assert!(original.encode(&commit).is_err());
    commit.mls_commit[0] ^= 1;
    original.predecessor.generation = 2;
    assert!(original.encode(&commit).is_err());
    original.predecessor.generation = 1;
    original.topic = vec![1; MAX_TOPIC + 1];
    assert!(original.encode(&commit).is_err());
}

#[test]
fn owner_operation_supplied_oversized_log_refuses_before_encoding_scratch() {
    let (original, mut record) = shape();
    let bytes = original.encode(&record).unwrap();
    record.mls_commit.resize(MAX_COMMIT + 1, 3);
    let before = OWNED_DECODE_ENTRIES.with(std::cell::Cell::get);
    assert!(OwnerOperationTail::decode(&bytes, &record).is_err());
    assert_eq!(OWNED_DECODE_ENTRIES.with(std::cell::Cell::get), before);
}
