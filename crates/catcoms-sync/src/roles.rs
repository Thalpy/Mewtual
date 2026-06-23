//! Member-roles authority — reading the owner-signed admin grants from the `MemberRoles` doc.
//!
//! This lives in the **sync** layer (not the app layer) on purpose: the membership-admission
//! gate (`ChannelSync::inviter_is_authorized`) must consult the *live* roles doc at admission
//! time, with zero staleness. Keeping `read_admins` here — next to the gate that enforces it —
//! means the verdict is computed from the doc as it exists when a join is served, not from a
//! cached snapshot some other code path has to remember to refresh. The product/UI layer
//! (`catcoms-app`) re-exports these so the roster/role display reuses the same canonical logic.

use std::collections::HashSet;

use automerge::{AutoCommit, ObjId, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::{verify_with_public_bytes, DeviceId};

/// The reserved document id for the per-server member-roles document.
pub const ROLES_DOC: u128 = 0;
/// Domain separator for an owner's admin-grant signature.
pub const ROLE_GRANT_DOMAIN: &[u8] = b"catcoms/role-grant/v1";

/// Short 4-byte hex fingerprint of a device id — the role-doc map key + the UI display id.
pub fn fingerprint(id: &DeviceId) -> String {
    id.as_bytes()[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The bytes the owner signs to grant `target_fp` admin in this group (domain-separated +
/// group- and target-bound, so a grant can't be replayed to another group or member). The
/// `group_id` is **length-prefixed** so `… ‖ group_id ‖ target_fp` can never reparse as a
/// different `(group_id', target_fp')` pair (no concatenation ambiguity), regardless of length.
pub fn grant_payload(group_id: &[u8], target_fp: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(ROLE_GRANT_DOMAIN.len() + 2 + group_id.len() + target_fp.len());
    p.extend_from_slice(ROLE_GRANT_DOMAIN);
    p.extend_from_slice(&(group_id.len() as u16).to_be_bytes());
    p.extend_from_slice(group_id);
    p.extend_from_slice(target_fp.as_bytes());
    p
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

/// Materialize the set of fingerprints with a **valid owner-signed** admin grant: the stored
/// `owner_pubkey ‖ sig` must verify over `grant_payload(group_id, fp)` AND the signing key's
/// **full device id** must equal the current owner's. Comparing the full 32-byte device id
/// (not the 4-byte display fingerprint) keeps a forged-grant attack at a full preimage (2^256)
/// rather than a feasible 2^32 fingerprint grind. Forged/foreign grants are ignored.
pub fn read_admins(doc: &AutoCommit, group_id: &[u8], owner_id: &DeviceId) -> HashSet<String> {
    let mut out = HashSet::new();
    for key in doc.keys(ROOT) {
        let grant = bytes_field(doc, &ROOT, &key);
        if grant.len() != 96 {
            continue; // 32-byte pubkey + 64-byte signature
        }
        let (pubkey, sig_bytes) = grant.split_at(32);
        let Ok(sig) = <[u8; 64]>::try_from(sig_bytes) else {
            continue;
        };
        // The grant must be signed by the *current* owner's device key (full id, not the fp).
        if DeviceId::from_public_key_bytes(pubkey) != *owner_id {
            continue;
        }
        if verify_with_public_bytes(pubkey, &grant_payload(group_id, &key), &sig) {
            out.insert(key);
        }
    }
    out
}
