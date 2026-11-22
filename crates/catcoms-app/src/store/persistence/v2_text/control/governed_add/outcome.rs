//! Exact private Add outcome, linked to the original SDK-entry ciphertext.
//! A decoded record is only data. Recovery still verifies the original
//! governing chain and the restored SDK state before selecting anything.
use super::sdk_entry::ENTRY_BYTES;
use super::*;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::ServerStore;
use catcoms_mls::group::text_current_add::MAX_TEXT_ADD_ROUTING_PLAIN_BYTES;
use catcoms_mls::persist::text_current_add::TextCurrentAddLimits;
use catcoms_replication::chat::{
    join_bindings::MAX_JOIN_PAYLOAD_BYTES, owner_header::MAX_HEADER_SEMANTIC_BYTES,
    owner_profile::MAX_ADMISSION_BYTES, owner_state::MAX_STATE_BYTES,
};
use catcoms_sync::owner_v2::V2FounderAddOutcomeData;

pub(in crate::store::persistence::v2_text) const OUTCOME_NAME: &str = "text.add.outcome.v2";
const TAG: &[u8] = b"catcoms/private-chat-text-governed-add-outcome/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-outcome-key/v2";
const ENTRY_DOMAIN: &str = "catcoms/private-chat-text-governed-add-outcome-parent/v2";
const VERSION: u16 = 2;
// Provider snapshots have their own bounded family. None of these limits
// extends the original Head1, Archive1, or WAL grammars.
const MAX_SNAPSHOT: usize = TextCurrentAddLimits::governed_text_v2().max_snapshot_bytes;
const BODY_CAPS: [usize; 8] = [
    MAX_JOIN_PAYLOAD_BYTES, // Commit
    MAX_JOIN_PAYLOAD_BYTES, // Welcome
    MAX_JOIN_PAYLOAD_BYTES, // GroupInfo
    MAX_TEXT_ADD_ROUTING_PLAIN_BYTES + 48,
    MAX_ADMISSION_BYTES,
    MAX_HEADER_SEMANTIC_BYTES,
    MAX_STATE_BYTES,
    MAX_SNAPSHOT,
];
const HEADER_BYTES: usize = 4 + TAG.len() + 2 + 2 * (4 + 32) + 3 * 8 + 4 * BODY_CAPS.len();
const MAX_PLAIN: usize = HEADER_BYTES
    + 3 * MAX_JOIN_PAYLOAD_BYTES
    + MAX_TEXT_ADD_ROUTING_PLAIN_BYTES
    + 48
    + MAX_ADMISSION_BYTES
    + MAX_HEADER_SEMANTIC_BYTES
    + MAX_STATE_BYTES
    + MAX_SNAPSHOT;
pub(super) const MAX_FRAME: usize = MAX_PLAIN + 40;
pub(super) const MIN_FRAME: usize = HEADER_BYTES + BODY_CAPS.len() + 40;

struct OutcomeFields<'a> {
    before: u64,
    after: u64,
    admitted_at_ms: u64,
    authenticator: &'a [u8; 32],
    bodies: [&'a [u8]; 8],
}

/// One presealed outcome for the original SDK completion. Its independent
/// payment outlives the bytes, including across a late Current refusal.
pub(super) struct SealedGovernedAddOutcome {
    parent: [u8; 32],
    pub(super) raw: Option<Zeroizing<Vec<u8>>>,
    _fee: CatalogReadReservation,
}

impl SealedGovernedAddOutcome {
    /// Outside writers, after the real SDK entry and Header intent are known.
    /// The caller supplies the original covered entry ciphertext and the joint
    /// outcome loan. No second snapshot encoding is generated here.
    pub(super) fn prepare(
        store: &ServerStore,
        entry: &[u8],
        outcome: &V2FounderAddOutcomeData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        let parent = entry_digest(entry)?;
        let result = outcome.result_data();
        let unsigned = outcome.unsigned_data();
        let mut fields = OutcomeFields {
            before: result.original_parent_data().epoch_data(),
            after: result.after_epoch_data(),
            admitted_at_ms: result.request_data().validation_time_ms_data(),
            authenticator: result.after_authenticator_data(),
            bodies: [
                result.commit_data(),
                result.welcome_data(),
                result.group_info_data(),
                result
                    .sealed_routing_transfer_data()
                    .map_err(|_| refused())?,
                &[], // Admission encoding is allocated only after payment.
                unsigned.signing_bytes_data(),
                unsigned.image_data(),
                result.secret_snapshot_data(),
            ],
        };
        let maximum = fields.bodies.iter().zip(BODY_CAPS).enumerate().try_fold(
            HEADER_BYTES,
            |length, (index, (body, cap))| -> Result<usize, AppError> {
                if index == 4 {
                    Ok(length + cap)
                } else {
                    require_body(body, cap)?;
                    Ok(length + body.len())
                }
            },
        )?;
        let bytes = 4 * (maximum + 40) + 4 * std::mem::size_of::<Self>() + 4096;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(bytes)?;
        fee.validate(pool, bytes)?;
        let admission = outcome.admission_data().encode();
        fields.bodies[4] = &admission;
        let key = Zeroizing::new(store.keys.db_key()?);
        let raw = seal_fields(&key, parent, &fields, rng)?;
        Ok(Self {
            parent,
            raw: Some(raw),
            _fee: fee,
        })
    }

    pub(super) fn require_entry(&self, entry: &[u8]) -> Result<(), AppError> {
        if self.parent != entry_digest(entry)? {
            return Err(refused());
        }
        Ok(())
    }
}

fn entry_digest(entry: &[u8]) -> Result<[u8; 32], AppError> {
    if entry.len() != ENTRY_BYTES {
        return Err(refused());
    }
    Ok(blake3::derive_key(ENTRY_DOMAIN, entry))
}

fn require_body(body: &[u8], cap: usize) -> Result<(), AppError> {
    if body.is_empty() || body.len() > cap {
        return Err(refused());
    }
    Ok(())
}

fn seal_fields(
    key: &[u8; 32],
    parent: [u8; 32],
    fields: &OutcomeFields<'_>,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if fields.before.checked_add(1) != Some(fields.after) {
        return Err(refused());
    }
    for (body, cap) in fields.bodies.iter().zip(BODY_CAPS) {
        require_body(body, cap)?;
    }
    let length = HEADER_BYTES + fields.bodies.iter().map(|body| body.len()).sum::<usize>();
    let mut e = Encoder::with_capacity(length);
    put(&mut e, TAG)?;
    e.put_u16(VERSION);
    put(&mut e, &parent)?;
    put(&mut e, fields.authenticator)?;
    e.put_u64(fields.before)
        .put_u64(fields.after)
        .put_u64(fields.admitted_at_ms);
    for body in &fields.bodies[..7] {
        put(&mut e, body)?;
    }
    // Arm zeroization before adding the exact noncanonical provider snapshot.
    let snapshot = fields.bodies[7];
    let mut plain = Zeroizing::new(e.finish());
    plain.extend_from_slice(&(snapshot.len() as u32).to_be_bytes());
    plain.extend_from_slice(snapshot);
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let raw = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
    if raw.len() != length + 40 {
        return Err(refused());
    }
    Ok(raw)
}

/// Caller prepays the bounded ciphertext copy and plaintext before this read.
pub(in crate::store::persistence::v2_text) fn open_outcome_data(
    key: &[u8; 32],
    entry: &[u8],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(MIN_FRAME..=MAX_FRAME).contains(&raw.len()) {
        return Err(refused());
    }
    let parent = entry_digest(entry)?;
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&key, &unframe(raw)?)?);
    decode_outcome_fields(&plain, parent)?;
    Ok(plain)
}

/// Call only on the plaintext of the authenticated exact held Outcome.
pub(super) fn header_signing_bytes<'a>(
    entry: &[u8],
    plain: &'a [u8],
) -> Result<&'a [u8], AppError> {
    Ok(decode_outcome_fields(plain, entry_digest(entry)?)?.bodies[5])
}

fn decode_outcome_fields(plain: &[u8], parent: [u8; 32]) -> Result<OutcomeFields<'_>, AppError> {
    let mut d = Decoder::new(plain);
    if field(&mut d, TAG.len())? != TAG
        || d.get_u16().map_err(|_| refused())? != VERSION
        || b32(&mut d)? != parent
    {
        return Err(refused());
    }
    let authenticator = field(&mut d, 32)?.try_into().map_err(|_| refused())?;
    let before = d.get_u64().map_err(|_| refused())?;
    let after = d.get_u64().map_err(|_| refused())?;
    let admitted_at_ms = d.get_u64().map_err(|_| refused())?;
    if before.checked_add(1) != Some(after) {
        return Err(refused());
    }
    let mut bodies = [&[][..]; BODY_CAPS.len()];
    for (body, cap) in bodies.iter_mut().zip(BODY_CAPS) {
        *body = field(&mut d, cap)?;
        require_body(body, cap)?;
    }
    d.finish().map_err(|_| refused())?;
    Ok(OutcomeFields {
        before,
        after,
        admitted_at_ms,
        authenticator,
        bodies,
    })
}

#[cfg(test)]
mod tests;
