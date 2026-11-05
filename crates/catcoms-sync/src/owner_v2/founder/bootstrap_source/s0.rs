//! Borrow the CLOSED original founder S0 schema without loading a MemoryStorage/MLS group or
//! expanding JSON. This is framing/label DATA validation, NOT a runnable-SDK/Fresh proof. Keep
//! every field/order synchronized with lib.rs::snapshot_with_governance and snapshot.rs matcher.
use super::*;
use crate::{group_policy, local_chat_scope::LocalChatScope, member_finalization};
use catcoms_wire::Decoder;

fn bad() -> SyncError {
    SyncError::Malformed
}
fn field<'a>(d: &mut Decoder<'a>, max: usize) -> Result<&'a [u8], SyncError> {
    let bytes = d.get_bytes().map_err(|_| bad())?;
    if bytes.len() > max {
        return Err(bad());
    }
    Ok(bytes)
}
fn zero(d: &mut Decoder<'_>) -> Result<(), SyncError> {
    if d.get_u32().map_err(|_| bad())? != 0 {
        return Err(bad());
    }
    Ok(())
}
fn fixed<'a>(d: &mut Decoder<'a>, len: usize) -> Result<&'a [u8], SyncError> {
    let bytes = field(d, len)?;
    if bytes.len() != len {
        return Err(bad());
    }
    Ok(bytes)
}

pub(super) fn preflight(raw: &[u8], spec: BootstrapSourceContextSpec<'_>) -> Result<(), SyncError> {
    if raw.is_empty() || raw.len() > spec.limits.max_snapshot_bytes() {
        return Err(bad());
    }
    let mut d = Decoder::new(raw);
    mls(field(&mut d, spec.limits.max_mls_bytes())?, spec)?;
    routing(field(&mut d, 12 + crate::MAX_ROUTING_SECRETS * 44)?)?;
    {
        let mut ledger = Decoder::new(field(&mut d, 8)?);
        zero(&mut ledger)?;
        zero(&mut ledger)?;
        ledger.finish().map_err(|_| bad())?;
    }
    zero(&mut d)?;
    zero(&mut d)?;
    zero(&mut d)?; // docs/log/peers: never-run source only
    fixed(&mut d, 32)?; // opaque original file-wrap key
    if d.get_u64().map_err(|_| bad())? != 0 {
        return Err(bad());
    } // initial admin generation
    zero(&mut d)?; // obsolete grants
    let count = d.get_u32().map_err(|_| bad())? as usize;
    if count > crate::initialization_snapshot::MAX_INITIAL_RENDEZVOUS {
        return Err(bad());
    }
    for _ in 0..count {
        std::str::from_utf8(field(&mut d, 1024)?).map_err(|_| bad())?;
        field(&mut d, 128)?;
    }
    zero(&mut d)?; // direct results
    tenure(field(&mut d, 98)?, spec.signer)?;
    let policy = field(&mut d, 4101)?;
    // A bounded actual public policy decode is allowed; this does NOT expand SDK/provider data.
    if let Some(policy) = group_policy::decode_pin(policy)? {
        if policy.group_id() != spec.group {
            return Err(bad());
        }
        group_policy::require_supported(&policy)?;
    }
    {
        let mut durable = Decoder::new(field(&mut d, 5)?);
        if !matches!(durable.get_u8().map_err(|_| bad())?, 1 | 2) {
            return Err(bad());
        }
        zero(&mut durable)?;
        durable.finish().map_err(|_| bad())?;
    }
    member_finalization::preflight_owner_initialization_pending(field(&mut d, 5)?, 0)?;
    if field(&mut d, 64)? != b"catcoms/admin-authority/v2" {
        return Err(bad());
    }
    zero(&mut d)?;
    let scope = field(&mut d, 512)?;
    LocalChatScope::preflight_owner_initialization(scope, spec.group, &spec.signer)?;
    // Reuse the exact closed first-source parser above, then compare the actual scope labels.
    // The durable planned row revision is NOT encoded in S0; only external intent/context binds it.
    let mut scope = Decoder::new(scope);
    field(&mut scope, 64)?;
    field(&mut scope, MAX_GROUP_ID_BYTES)?;
    fixed(&mut scope, 32)?;
    if fixed(&mut scope, 32)? != spec.scope_domain || fixed(&mut scope, 32)? != spec.scope_nonce {
        return Err(bad());
    }
    d.finish().map_err(|_| bad()) // unknown/v1-owner/operation tails cannot be ignored
}

fn mls(raw: &[u8], spec: BootstrapSourceContextSpec<'_>) -> Result<(), SyncError> {
    let mut d = Decoder::new(raw);
    let storage = field(&mut d, spec.limits.max_mls_bytes())?;
    if fixed(&mut d, 32)? != spec.signer || field(&mut d, MAX_GROUP_ID_BYTES)? != spec.group {
        return Err(bad());
    }
    d.finish().map_err(|_| bad())?;
    let mut rows = Decoder::new(storage);
    let count = rows.get_u32().map_err(|_| bad())? as usize;
    // Two framing words per row are a strict physical minimum. No count-selected allocation,
    // key/value copy, JSON/serde decode, table probe or SDK map is made here. In particular this
    // does not establish duplicate-key semantics, provider table layout or typed MLS validity.
    if count > rows.remaining() / 8 {
        return Err(bad());
    }
    for _ in 0..count {
        field(&mut rows, spec.limits.max_mls_bytes())?;
        field(&mut rows, spec.limits.max_mls_bytes())?;
    }
    rows.finish().map_err(|_| bad())
}

fn routing(raw: &[u8]) -> Result<(), SyncError> {
    let mut d = Decoder::new(raw);
    d.get_u64().map_err(|_| bad())?;
    let count = d.get_u32().map_err(|_| bad())? as usize;
    if count > crate::MAX_ROUTING_SECRETS {
        return Err(bad());
    }
    let mut previous = None;
    for _ in 0..count {
        let slot = d.get_u64().map_err(|_| bad())?;
        if previous.is_some_and(|old| old >= slot) {
            return Err(bad());
        }
        previous = Some(slot);
        fixed(&mut d, 32)?;
    }
    d.finish().map_err(|_| bad())
}

fn tenure(raw: &[u8], signer: [u8; 32]) -> Result<(), SyncError> {
    let mut d = Decoder::new(raw);
    // An actual fresh founder emits exact v2 epoch0/start0/nonimported; version3's historical
    // archive is impossible here. Leaf digest remains opaque until separately paid typed restore.
    if d.get_u8().map_err(|_| bad())? != 2
        || d.get_u64().map_err(|_| bad())? != 0
        || fixed(&mut d, 32)? != catcoms_crypto::DeviceId::from_public_key_bytes(&signer).as_bytes()
        || fixed(&mut d, 8)? != 0u64.to_be_bytes()
    {
        return Err(bad());
    }
    let leaf = fixed(&mut d, 36)?;
    if leaf[..4] != 0u32.to_be_bytes() || d.get_u8().map_err(|_| bad())? != 0 {
        return Err(bad());
    }
    d.finish().map_err(|_| bad())
}
