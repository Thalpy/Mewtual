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
//! **display / propagation only**; `read_published_roster` verifies the owner's signature so a
//! tampering member's UI edits are rejected by every reader, but a stale-replay or deletion of
//! the published copy is at worst cosmetic (it never gates admission). See
//! `docs/design-grant-revocation.md`.

use std::collections::HashSet;

use automerge::{AutoCommit, ObjId, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::{verify_with_public_bytes, DeviceId};

/// The reserved document id for the per-server member-roles document.
pub const ROLES_DOC: u128 = 0;
/// Domain separator for the owner's signed admin-roster. Bumped to `v1` of the roster format so a
/// stray old per-fingerprint grant blob can never be reinterpreted as a roster (domain + format
/// separation across versions).
pub const ROLE_ROSTER_DOMAIN: &[u8] = b"catcoms/role-roster/v1";
/// The single doc key under which the owner-signed roster is published.
pub const ROSTER_KEY: &str = "roster";

/// Short 4-byte hex fingerprint of a device id; the roster entry + the UI display id. Always 8
/// ASCII hex characters, which the roster wire format relies on (fixed-width entries).
pub fn fingerprint(id: &DeviceId) -> String {
    id.as_bytes()[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The bytes the owner signs to publish the admin roster at generation `gen`. Domain-separated,
/// group-bound (the `group_id` is **length-prefixed** so `… ‖ group_id ‖ gen ‖ …` can never
/// reparse as a different split), generation-bound, and the fingerprints are length-counted +
/// expected **sorted** so the signed bytes are canonical (one admin set ⇒ one signature).
pub fn roster_payload(group_id: &[u8], gen: u64, fps: &[String]) -> Vec<u8> {
    let mut p = Vec::with_capacity(ROLE_ROSTER_DOMAIN.len() + 12 + group_id.len() + fps.len() * 8);
    p.extend_from_slice(ROLE_ROSTER_DOMAIN);
    p.extend_from_slice(&(group_id.len() as u16).to_be_bytes());
    p.extend_from_slice(group_id);
    p.extend_from_slice(&gen.to_be_bytes());
    p.extend_from_slice(&(fps.len() as u16).to_be_bytes());
    for fp in fps {
        p.extend_from_slice(fp.as_bytes()); // each fingerprint is exactly 8 ASCII hex chars
    }
    p
}

/// Encode the stored roster value: `gen ‖ owner_pk ‖ n ‖ fps ‖ sig`. `owner_pk` is the 32-byte
/// owner signing key; `sig` is the owner's signature over [`roster_payload`]. `fps` must be sorted.
pub fn encode_roster(gen: u64, owner_pk: &[u8], fps: &[String], sig: &[u8; 64]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + 32 + 2 + fps.len() * 8 + 64);
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

/// Materialize the admin fingerprints from the **published** roster in `doc`. Returns `Some(set)`
/// iff the `roster` value parses, was signed by the **current owner's** key (the signing key's
/// full device id must equal `owner_id`; comparing the 32-byte id, not the 4-byte display fp,
/// keeps a forged-roster attack at a full preimage), and the signature verifies; otherwise `None`
/// (fail-closed). **Display only**; the admission gate uses the owner's local `admin_roster`, so
/// a stale-replay or deletion of this published copy is cosmetic, never an admission bypass.
pub fn read_published_roster(
    doc: &AutoCommit,
    group_id: &[u8],
    owner_id: &DeviceId,
) -> Option<HashSet<String>> {
    let v = bytes_field(doc, &ROOT, ROSTER_KEY);
    // layout: gen(8) ‖ owner_pk(32) ‖ n(2) ‖ fps(n*8) ‖ sig(64)
    const HEAD: usize = 8 + 32 + 2;
    if v.len() < HEAD + 64 {
        return None;
    }
    let gen = u64::from_be_bytes(v[0..8].try_into().ok()?);
    let owner_pk = &v[8..40];
    let n = u16::from_be_bytes(v[40..42].try_into().ok()?) as usize;
    let fps_len = n.checked_mul(8)?;
    if v.len() != HEAD + fps_len + 64 {
        return None; // exact-length: rejects trailing/short padding + count/blob mismatch
    }
    let mut fps = Vec::with_capacity(n);
    for i in 0..n {
        let s = HEAD + i * 8;
        fps.push(std::str::from_utf8(&v[s..s + 8]).ok()?.to_string());
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
