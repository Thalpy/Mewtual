//! Immutable critical namespace5/version1 descriptor: the ordinary-member outbox layout.
//!
//! Namespace5 declares that this vault keeps independently durable ordinary-member outbox
//! namespaces (`persistence::v2_member_outbox`) under the fixed parent directory owned by the
//! concrete configured handler in `member_outbox_layout`. It is deliberately independent of
//! governance namespace4: ordinary member storage never requires a governance declaration,
//! founder certification or governance recovery, and either namespace may be declared first
//! (the shared `catalog_declaration` helper inserts at the sorted position).
//!
//! Descriptor bytes: tag `catcoms/private-chat-v2-member-outbox-layout/v1\0`, u16 version1,
//! then three length-framed b32 fields: vault, root and a commitment to the exact canonical
//! namespace3 descriptor (BLAKE3 of `catcoms/private-chat-v2-member-outbox-current-descriptor/v1\0`
//! followed by the big-endian u64 descriptor length and the complete descriptor bytes). That
//! commitment is neither a whole-catalog plaintext digest nor a ciphertext identity, and its
//! domain differs from governance's, so neither descriptor can stand in for the other.
//! Counters, quotas, rows and outbox contexts never belong in this immutable descriptor.
//!
//! DATA only: decoding grants no mount, outbox owner, save admission, Source or governance
//! authority. Only the configured physical handler interprets the fixed directory.
use super::{archive_current_namespace_codec as current, catalog_declaration};
use super::{pending_join_codec as prior, refused};
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(super) const NAMESPACE_ID: u32 = 5;
pub(super) const NAMESPACE_VERSION: u32 = 1;
const TAG: &[u8] = b"catcoms/private-chat-v2-member-outbox-layout/v1\0";
const VERSION: u16 = 1;
/// Tag frame (u32 length + bytes), u16 version, three length-framed 32-byte fields.
pub(super) const DESCRIPTOR_BYTES: usize = 4 + TAG.len() + 2 + 3 * 36;

/// Immutable capability labels. `Copy` DATA: equality is negative identity comparison only.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct MemberOutboxCapabilityData {
    vault: [u8; 32],
    root: [u8; 32],
    current_descriptor: [u8; 32],
}
impl std::fmt::Debug for MemberOutboxCapabilityData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemberOutboxCapabilityData")
    }
}

fn field32(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    d.get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())
}

/// Commitment to the exact canonical ID3 descriptor bytes, domain-separated from governance.
fn current_descriptor_digest(raw: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"catcoms/private-chat-v2-member-outbox-current-descriptor/v1\0");
    h.update(&(raw.len() as u64).to_be_bytes());
    h.update(raw);
    *h.finalize().as_bytes()
}

impl MemberOutboxCapabilityData {
    pub(in crate::store) fn current_layout_data(self) -> current::CurrentLayoutData {
        current::CurrentLayoutData {
            vault: self.vault,
            root: self.root,
        }
    }

    pub(in crate::store) fn encode_data(self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(DESCRIPTOR_BYTES);
        e.put_bytes(TAG).map_err(|_| refused())?;
        e.put_u16(VERSION);
        for field in [self.vault, self.root, self.current_descriptor] {
            e.put_bytes(&field).map_err(|_| refused())?;
        }
        if e.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        Ok(Zeroizing::new(e.finish()))
    }

    fn decode_exact(raw: &[u8], expected: Self) -> Result<Self, AppError> {
        if raw.len() != DESCRIPTOR_BYTES {
            return Err(refused());
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes().map_err(|_| refused())? != TAG
            || d.get_u16().map_err(|_| refused())? != VERSION
        {
            return Err(refused());
        }
        let value = Self {
            vault: field32(&mut d)?,
            root: field32(&mut d)?,
            current_descriptor: field32(&mut d)?,
        };
        d.finish().map_err(|_| refused())?;
        if value != expected {
            return Err(refused());
        }
        Ok(value)
    }

    /// Decode a parsed ID5 payload against the SAME catalog's already-validated ID3 layout
    /// and exact ID3 descriptor bytes. Any other vault, root or descriptor refuses.
    pub(in crate::store) fn decode_data(
        payload: &[u8],
        layout: current::CurrentLayoutData,
        original_current_descriptor: &[u8],
    ) -> Result<Self, AppError> {
        current::CurrentLayoutData::decode_data(original_current_descriptor, layout)?;
        Self::decode_exact(
            payload,
            Self {
                vault: layout.vault,
                root: layout.root,
                current_descriptor: current_descriptor_digest(original_current_descriptor),
            },
        )
    }

    /// The capability a declaration over this complete original catalog must add.
    pub(in crate::store) fn from_original_catalog_data(
        original: &[u8],
        domain: &[u8; 32],
    ) -> Result<Self, AppError> {
        original_data(original, domain)
    }
}

/// The complete original must be a supported settled Current3 catalog (known JOIN v1/v2 DATA,
/// ID2, ID3, optionally governance ID4) WITHOUT ID5. This is grammar validation of DATA, not
/// selection of a runtime executor: the configured opener's own parser governs support.
fn original_data(raw: &[u8], domain: &[u8; 32]) -> Result<MemberOutboxCapabilityData, AppError> {
    let parsed = prior::parse_member_outbox_declaration_original(raw, domain)?;
    let layout = parsed.current_layout.ok_or_else(refused)?;
    let archive = parsed.archive.as_ref().ok_or_else(refused)?;
    let descriptor = parsed.current_payload.ok_or_else(refused)?;
    if parsed.member_outbox.is_some()
        || layout.vault != *domain
        || archive.vault != layout.vault
        || archive.root != layout.root
    {
        return Err(refused());
    }
    current::CurrentLayoutData::decode_data(descriptor, layout)?;
    Ok(MemberOutboxCapabilityData {
        vault: layout.vault,
        root: layout.root,
        current_descriptor: current_descriptor_digest(descriptor),
    })
}

/// Produce the complete successor DATA candidate with exactly one new ID5 at its sorted
/// position. The caller funds the original input, candidate buffer, parser metadata and later
/// sealing overlap independently BEFORE calling; there is no payer here. A declaration seed
/// must retain BOTH whole original and candidate frames.
pub(super) fn prepare_declaration_data(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let capability = original_data(original, domain)?;
    let descriptor = capability.encode_data()?;
    let candidate = catalog_declaration::insert_entry_data(
        original,
        domain,
        NAMESPACE_ID,
        NAMESPACE_VERSION,
        &descriptor,
    )?;
    validate_declaration_pair_data(original, &candidate, domain)?;
    Ok(candidate)
}

/// Validate one exact original -> declaration pair: one inserted ID5 whose descriptor binds
/// the original's own vault/root/ID3 descriptor, every other entry byte-identical (an
/// existing governance ID4 included). Not an admitted write or a mounted capability.
pub(super) fn validate_declaration_pair_data(
    original: &[u8],
    candidate: &[u8],
    domain: &[u8; 32],
) -> Result<MemberOutboxCapabilityData, AppError> {
    let expected = original_data(original, domain)?;
    let added = catalog_declaration::validate_insert_pair_data(
        original,
        candidate,
        domain,
        NAMESPACE_ID,
        NAMESPACE_VERSION,
    )?;
    MemberOutboxCapabilityData::decode_exact(added, expected)
}

#[cfg(test)]
mod tests;
