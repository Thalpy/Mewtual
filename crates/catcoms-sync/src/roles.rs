//! Member-roles authority; the owner-signed admin roster published in the `MemberRoles` doc.
//!
//! This lives in the **sync** layer (not the app layer) on purpose. The membership-admission
//! gate (`ChannelSync::inviter_is_authorized`) is the security-critical reader, and in the
//! single-committer (Option C) model **only the owner runs admission**. So the authoritative
//! admin set is kept as owner-**local** state (`ChannelSync::admin_roster`, persisted in the
//! snapshot), which a malicious member cannot write; closing the demoted-admin grant-replay
//! residual (THREAT-MODEL item 3): replay, deletion, or forgery against the shared CRDT cannot
//! promote anyone, because the gate never reads the CRDT.
//!
//! What lives *here* is the **published copy** of that set: a single owner-signed `roster` value
//! in the `MemberRoles` doc, so honest non-owner clients can show trustworthy role badges. It is
//! **display / propagation and reject-only relay preflight**; `read_published_roster` verifies the owner's signature so a
//! tampering member's UI edits are rejected by every reader, but a stale-replay or deletion of
//! the published copy can affect relay liveness, but never grants admission. See
//! `docs/design-grant-revocation.md`.

use std::collections::HashSet;

use automerge::{AutoCommit, ObjId, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::{verify_with_public_bytes, DeviceId};

/// The reserved document id for the per-server member-roles document.
pub const ROLES_DOC: u128 = 0;
/// Domain separator for the owner's signed admin-roster. Bumped to `v2` of the roster format so a
/// stray old per-fingerprint grant blob can never be reinterpreted as a roster (domain + format
/// separation across versions).
pub const ROLE_ROSTER_DOMAIN: &[u8] = b"catcoms/role-roster/v2";
/// The single doc key under which the owner-signed roster is published.
pub const ROSTER_KEY: &str = "roster";

/// Short display label only. Never use these four bytes for role authority or target selection.
pub fn fingerprint(id: &DeviceId) -> String {
    id.as_bytes()[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Pure owner-side authority decision, shared with the live admission gate. Membership
/// must come from MLS, never a peer descriptor; fingerprints are deliberately absent.
pub(crate) fn authorizes_device(
    owner: DeviceId,
    admins: &std::collections::BTreeSet<DeviceId>,
    device: DeviceId,
    current_member: bool,
    durable: bool,
) -> bool {
    current_member && (device == owner || (durable && admins.contains(&device)))
}

/// The bytes the owner signs to publish the admin roster at generation `gen`. Domain-separated,
/// group-bound (the `group_id` is **length-prefixed** so `… ‖ group_id ‖ gen ‖ …` can never
/// reparse as a different split), generation-bound, and the full device identities are length-counted +
/// expected **sorted** so the signed bytes are canonical (one admin set ⇒ one signature).
pub fn roster_payload(group_id: &[u8], gen: u64, fps: &[DeviceId]) -> Vec<u8> {
    let mut p = Vec::with_capacity(ROLE_ROSTER_DOMAIN.len() + 12 + group_id.len() + fps.len() * 32);
    p.extend_from_slice(ROLE_ROSTER_DOMAIN);
    p.extend_from_slice(&(group_id.len() as u16).to_be_bytes());
    p.extend_from_slice(group_id);
    p.extend_from_slice(&gen.to_be_bytes());
    p.extend_from_slice(&(fps.len() as u16).to_be_bytes());
    for fp in fps {
        p.extend_from_slice(fp.as_bytes()); // each identity is exactly 32 bytes
    }
    p
}

/// Encode the stored roster value: `gen ‖ owner_pk ‖ n ‖ fps ‖ sig`. `owner_pk` is the 32-byte
/// owner signing key; `sig` is the owner's signature over [`roster_payload`]. `fps` contains sorted, distinct full device identities.
pub fn encode_roster(gen: u64, owner_pk: &[u8], fps: &[DeviceId], sig: &[u8; 64]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + 32 + 2 + fps.len() * 32 + 64);
    v.extend_from_slice(&gen.to_be_bytes());
    v.extend_from_slice(owner_pk);
    v.extend_from_slice(&(fps.len() as u16).to_be_bytes());
    for fp in fps {
        v.extend_from_slice(fp.as_bytes());
    }
    v.extend_from_slice(sig);
    v
}

/// Read a `Bytes` scalar field (empty if absent or another type).
fn bytes_field(doc: &AutoCommit, obj: &ObjId, key: &str) -> Vec<u8> {
    match doc.get(obj, key) {
        Ok(Some((Value::Scalar(s), _))) => match s.as_ref() {
            ScalarValue::Bytes(b) => b.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Materialize the admin device identities from the **published** roster in `doc`. Returns `Some(set)`
/// iff the `roster` value parses, was signed by the **current owner's** key (the signing key's
/// full device id must equal `owner_id`; comparing the 32-byte id, not the 4-byte display fp,
/// keeps a forged-roster attack at a full preimage), and the signature verifies; otherwise `None`
/// (fail-closed). **Display only**; the admission gate uses the owner's local `admin_roster`, so
/// a stale replay may deny a relay; it can never authorize an MLS Add.
pub fn read_published_roster(
    doc: &AutoCommit,
    group_id: &[u8],
    owner_id: &DeviceId,
) -> Option<HashSet<DeviceId>> {
    let v = bytes_field(doc, &ROOT, ROSTER_KEY);
    // layout: gen(8) ‖ owner_pk(32) ‖ n(2) ‖ ids(n*32) ‖ sig(64)
    const HEAD: usize = 8 + 32 + 2;
    if v.len() < HEAD + 64 {
        return None;
    }
    let gen = u64::from_be_bytes(v[0..8].try_into().ok()?);
    let owner_pk = &v[8..40];
    let n = u16::from_be_bytes(v[40..42].try_into().ok()?) as usize;
    let fps_len = n.checked_mul(32)?;
    if v.len() != HEAD + fps_len + 64 {
        return None; // exact-length: rejects trailing/short padding + count/blob mismatch
    }
    let mut fps = Vec::with_capacity(n);
    for i in 0..n {
        let s = HEAD + i * 32;
        fps.push(DeviceId::from_bytes(v[s..s + 32].try_into().ok()?));
    }
    // Canonical set: reject duplicate or unsorted identities even if signed.
    if fps.windows(2).any(|pair| pair[0] >= pair[1]) {
        return None;
    }
    let sig: [u8; 64] = v[HEAD + fps_len..HEAD + fps_len + 64].try_into().ok()?;
    // The roster must be signed by the *current* owner's device key (full id, not the fp).
    if DeviceId::from_public_key_bytes(owner_pk) != *owner_id {
        return None;
    }
    // Verify over the parsed (stored) order; a malicious reorder/dup changes the payload and
    // fails here, so the returned set always reflects exactly the owner-signed bytes.
    if !verify_with_public_bytes(owner_pk, &roster_payload(group_id, gen, &fps), &sig) {
        return None;
    }
    Some(fps.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::transaction::Transactable;
    use catcoms_mls::MlsDevice;

    #[test]
    fn full_identity_roster_distinguishes_collision_and_rejects_noncanonical_sets() {
        let owner = MlsDevice::generate().unwrap();
        let a = DeviceId::from_bytes([7; 32]);
        let mut raw = *a.as_bytes();
        raw[31] = 8;
        let twin = DeviceId::from_bytes(raw);
        assert_eq!(fingerprint(&a), fingerprint(&twin));
        let grants = std::collections::BTreeSet::from([a]);
        assert!(authorizes_device(owner.device_id(), &grants, a, true, true));
        assert!(!authorizes_device(
            owner.device_id(),
            &grants,
            twin,
            true,
            true
        ));
        // A removed admin's sole remaining twin never inherits its exact grant.
        assert!(!authorizes_device(
            owner.device_id(),
            &grants,
            a,
            false,
            true
        ));
        assert!(!authorizes_device(
            a,
            &std::collections::BTreeSet::new(),
            twin,
            true,
            true
        ));
        let read = |ids: &[DeviceId]| {
            let sig = owner.sign(&roster_payload(b"g", 1, ids)).unwrap();
            let mut doc = AutoCommit::new();
            doc.put(
                ROOT,
                ROSTER_KEY,
                ScalarValue::Bytes(encode_roster(1, &owner.public_key_bytes(), ids, &sig)),
            )
            .unwrap();
            read_published_roster(&doc, b"g", &owner.device_id())
        };
        let set = read(&[a]).unwrap();
        assert!(set.contains(&a));
        assert!(!set.contains(&twin));
        assert!(read(&[a, a]).is_none());
        assert!(read(&[twin, a]).is_none());
    }
}
