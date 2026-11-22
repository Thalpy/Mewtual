//! Namespace6 descriptor and whole-catalog declaration grammar.
use super::*;
use crate::store::critical_namespaces::{catalog_declaration, pending_join_codec as catalog};

pub(in crate::store::critical_namespaces) const ID: u32 = 6;
pub(in crate::store::critical_namespaces) const VERSION: u32 = 1;
const TAG: &[u8] = b"catcoms/private-chat-v2-joined-pin-namespace/v1\0";
const BYTES: usize = 4 + TAG.len() + 2 + 4 * 36;
const GOVERNANCE_DOMAIN: &str = "catcoms/private-chat-v2-joined-pin-governance/v1";
const OUTBOX_DOMAIN: &str = "catcoms/private-chat-v2-joined-pin-member-outbox/v1";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces) struct JoinedPinNamespace {
    layout: CurrentLayoutData,
    governance: [u8; 32],
    outbox: [u8; 32],
}

impl JoinedPinNamespace {
    fn for_current(layout: CurrentLayoutData, governance: &[u8], outbox: &[u8]) -> Self {
        Self {
            layout,
            governance: blake3::derive_key(GOVERNANCE_DOMAIN, governance),
            outbox: blake3::derive_key(OUTBOX_DOMAIN, outbox),
        }
    }

    fn encode_data(self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(BYTES);
        let failed = |_| refused();
        e.put_bytes(TAG).map_err(failed)?;
        e.put_u16(VERSION as u16);
        for field in [
            self.layout.vault,
            self.layout.root,
            self.governance,
            self.outbox,
        ] {
            e.put_bytes(&field).map_err(failed)?;
        }
        Ok(Zeroizing::new(e.finish()))
    }

    pub(in crate::store::critical_namespaces) fn decode_data(
        raw: &[u8],
        layout: CurrentLayoutData,
        governance: &[u8],
        outbox: &[u8],
    ) -> Result<Self, AppError> {
        if raw.len() != BYTES {
            return Err(refused());
        }
        let expected = Self::for_current(layout, governance, outbox);
        let mut d = Decoder::new(raw);
        let failed = |_| refused();
        if d.get_bytes().map_err(failed)? != TAG || d.get_u16().map_err(failed)? != VERSION as u16 {
            return Err(refused());
        }
        for field in [
            layout.vault,
            layout.root,
            expected.governance,
            expected.outbox,
        ] {
            if d.get_bytes().map_err(failed)? != field {
                return Err(refused());
            }
        }
        d.finish().map_err(failed)?;
        Ok(expected)
    }
}

pub(in crate::store::critical_namespaces) fn prepare_declaration(
    original: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let original_layout = catalog::parse_governed_native_registry_layout(original, domain, true)?;
    let descriptor = JoinedPinNamespace::for_current(
        original_layout.current_layout.ok_or_else(refused)?,
        original_layout.governance_payload.ok_or_else(refused)?,
        original_layout.member_outbox_payload.ok_or_else(refused)?,
    )
    .encode_data()?;
    let candidate =
        catalog_declaration::insert_entry_data(original, domain, ID, VERSION, &descriptor)?;
    verify_declaration(original, &candidate, domain)?;
    Ok(candidate)
}

pub(in crate::store::critical_namespaces) fn verify_declaration(
    original: &[u8],
    candidate: &[u8],
    domain: &[u8; 32],
) -> Result<(), AppError> {
    catalog::parse_governed_native_registry_layout(original, domain, true)?;
    catalog_declaration::validate_insert_pair_data(original, candidate, domain, ID, VERSION)?;
    catalog::parse_joined_native_registry_layout(candidate, domain, true)?;
    Ok(())
}
