//! Test-only signing. No production generic signing or unsigned packet factory
//! is exported by this inactive codec leaf.
use super::*;

pub(super) fn identities() -> (DeviceKeypair, DeviceKeypair) {
    let first = DeviceKeypair::from_seed(&[1; 32]);
    let second = DeviceKeypair::from_seed(&[2; 32]);
    if first.device_id() < second.device_id() {
        (first, second)
    } else {
        (second, first)
    }
}
pub(super) fn scope(policy: bool) -> Scope {
    Scope {
        group: [3; 32],
        policy: policy.then_some([0; 32]),
        room: u128::MAX,
    }
}
pub(super) fn signed(
    kind: u8,
    signer: &DeviceKeypair,
    scope: Scope,
    id: Nonce,
    issued: u64,
    expires: u64,
    body: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::with_capacity(242 + body.len() + 4);
    e.put_str(OBJECT_DOMAIN).unwrap();
    e.put_u8(VERSION);
    e.put_u8(kind);
    scope.put(&mut e);
    fixed(&mut e, signer.device_id().as_bytes());
    fixed(&mut e, &signer.verifying_key().to_bytes());
    fixed(&mut e, &id);
    e.put_u64(issued);
    e.put_u64(expires);
    let unsigned = if kind == 7 {
        fixed(&mut e, body);
        e.finish()
    } else {
        let mut bytes = e.finish();
        bytes.extend_from_slice(body);
        bytes
    };
    let signature = signer.sign(&unsigned);
    let mut e = Encoder::with_capacity(unsigned.len() + 72);
    fixed(&mut e, &unsigned);
    fixed(&mut e, &signature);
    e.finish()
}
pub(super) fn payload(op: u8, epoch: u64, tail: &[u8]) -> Vec<u8> {
    let mut bytes = vec![1, op, ADMISSION_PROFILE];
    bytes.extend(epoch.to_be_bytes());
    bytes.extend(tail);
    bytes
}
pub(super) fn control_payload(bytes: &[u8]) -> Vec<u8> {
    let mut outer = Decoder::new(bytes);
    let unsigned = outer.get_bytes().unwrap();
    let _: [u8; 64] = array(&mut outer).unwrap();
    outer.finish().unwrap();
    let mut d = Decoder::new(unsigned);
    assert_eq!(d.get_str().unwrap(), OBJECT_DOMAIN);
    assert_eq!(d.get_u8().unwrap(), VERSION);
    assert_eq!(d.get_u8().unwrap(), 7);
    Scope::get(&mut d).unwrap();
    for _ in 0..3 {
        let _: [u8; 32] = array(&mut d).unwrap();
    }
    d.get_u64().unwrap();
    d.get_u64().unwrap();
    let payload = d.get_bytes().unwrap().to_vec();
    d.finish().unwrap();
    payload
}
pub(super) fn availability(scope: Scope, available: bool) -> Vec<u8> {
    let (low, high) = identities();
    let mut tail = Vec::new();
    tail.extend(low.device_id().as_bytes());
    for field in [[21; 32], [31; 32], [32; 32], [0; 32]] {
        tail.extend(field);
    }
    tail.extend(9u64.to_be_bytes());
    tail.extend(u64::MAX.to_be_bytes());
    tail.extend([u8::from(available), u8::from(available)]);
    signed(
        7,
        &high,
        scope,
        [22; 32],
        1000,
        16000,
        &payload(2, 7, &tail),
    )
}
pub(super) fn request(scope: Scope, capability: &[u8]) -> Vec<u8> {
    let (low, high) = identities();
    let mut tail = Vec::new();
    tail.extend(low.device_id().as_bytes());
    tail.extend(high.device_id().as_bytes());
    for field in [[41; 32], [31; 32], [42; 32], [32; 32], [0; 32], [0; 32]] {
        tail.extend(field);
    }
    tail.extend(3u64.to_be_bytes());
    tail.extend(9u64.to_be_bytes());
    tail.extend([0; 32]);
    tail.extend((capability.len() as u16).to_be_bytes());
    tail.extend(capability);
    signed(7, &low, scope, [23; 32], 2000, 17000, &payload(3, 7, &tail))
}
pub(super) fn grant(scope: Scope, request: Nonce, commit_by: u64) -> Vec<u8> {
    let (low, high) = identities();
    let mut tail = Vec::new();
    tail.extend(low.device_id().as_bytes());
    tail.extend(request);
    tail.extend([0; 32]);
    tail.extend(commit_by.to_be_bytes());
    tail.extend(1u16.to_be_bytes());
    signed(
        7,
        &high,
        scope,
        [24; 32],
        3000,
        18000,
        &payload(4, 7, &tail),
    )
}

pub(super) fn terminal_payload(scope: Scope, unknown: bool, observed: u8) -> Vec<u8> {
    let (low, high) = identities();
    let present = !unknown;
    let decision = if unknown { 0 } else { 1 };
    let at_wall = if unknown { 0 } else { u64::MAX };
    let mut record = vec![1, 1];
    record.extend(scope.group);
    record.push(u8::from(scope.policy.is_some()));
    record.extend(scope.policy.unwrap_or([0; 32]));
    record.extend(scope.room.to_be_bytes());
    record.extend(7u64.to_be_bytes());
    record.extend(high.device_id().as_bytes());
    record.extend(low.device_id().as_bytes());
    record.extend([55; 32]);
    for _ in 0..2 {
        record.push(u8::from(present));
        record.extend([0; 32]);
    }
    record.extend([decision, 0]);
    record.extend(at_wall.to_be_bytes());
    assert_eq!(record.len(), 263);
    let mut preimage = b"mewtual/voice/admission/terminal/v1\0".to_vec();
    preimage.extend(record);
    let digest = if unknown {
        [0; 32]
    } else {
        *blake3::hash(&preimage).as_bytes()
    };
    let mut tail = low.device_id().as_bytes().to_vec();
    tail.extend([55; 32]);
    for _ in 0..2 {
        tail.push(u8::from(present));
        tail.extend([0; 32]);
    }
    tail.extend([decision, 0]);
    tail.extend(at_wall.to_be_bytes());
    tail.extend(digest);
    tail.push(observed);
    payload(8, 7, &tail)
}
