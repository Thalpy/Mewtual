//! A linked ID12 descriptor understood only during installed A2 replay.
//! General catalog admission and ordinary ID9 writers retain their grammar.
use super::*;

const LINKED_REVIEW_VERSION: u32 = 2;
const LINKED_REVIEW_TAG: &[u8] = b"catcoms/chat-v2/review-receiver-rejoin-parent/v1\0";

pub(in crate::store) enum InstalledCatalogBindingData {
    Direct([u8; 32]),
    ReviewSuccessor(ReviewPredecessorData),
}

/// Descriptor data only. Reopen the exact original ciphertext and check its
/// single typed insertion before deriving the historical rejoin digest.
#[derive(PartialEq, Eq)]
pub(in crate::store) struct ReviewPredecessorData {
    generation: u64,
    framed: [u8; 32],
    descriptor: [u8; 32],
}

impl ReviewPredecessorData {
    /// Reopen the exact installed predecessor. The caller retains its actual
    /// file and Current independently throughout the covered replay.
    pub(in crate::store) fn verify_installed_parent(
        &self,
        key: &[u8; 32],
        older_frame: &[u8],
        selected_frame: &[u8],
        domain: &[u8; 32],
    ) -> Result<[u8; 32], AppError> {
        if !(41..=catalog::MAX_CATALOG_FRAMED).contains(&older_frame.len())
            || blake3::hash(older_frame).as_bytes() != &self.framed
        {
            return Err(refused());
        }
        let older = open_framed_catalog(key, older_frame)?;
        let selected = open_framed_catalog(key, selected_frame)?;
        let original = parse_joined_history(&older, domain, true)?;
        let (generation, link) = inspect_link(&selected, domain)?;
        if link.as_ref() != Some(self) || generation.checked_sub(1) != Some(self.generation) {
            return Err(refused());
        }
        catalog_declaration::validate_insert_pair_data(
            &older,
            &selected,
            domain,
            super::super::review_receive_namespace::ID,
            LINKED_REVIEW_VERSION,
        )?;
        super::super::review_receive_namespace::require_descriptor(
            &self.descriptor,
            original.current_layout.ok_or_else(refused)?,
            (
                original.joined_pin_payload.ok_or_else(refused)?,
                original.native_registry_payload.ok_or_else(refused)?,
                original.shared_history_payload.ok_or_else(refused)?,
            ),
        )?;
        Ok(self.framed)
    }
}

pub(in crate::store) fn inspect_installed_catalog_binding(
    key: &[u8; 32],
    frame: &[u8],
    domain: &[u8; 32],
) -> Result<InstalledCatalogBindingData, AppError> {
    let plain = open_framed_catalog(key, frame)?;
    let (generation, link) = inspect_link(&plain, domain)?;
    match link {
        Some(parent) if generation.checked_sub(1) == Some(parent.generation) => {
            Ok(InstalledCatalogBindingData::ReviewSuccessor(parent))
        }
        Some(_) => Err(refused()),
        None => {
            parse_joined_history(&plain, domain, true)?;
            Ok(InstalledCatalogBindingData::Direct(
                *blake3::hash(frame).as_bytes(),
            ))
        }
    }
}

fn inspect_link(
    plain: &[u8],
    domain: &[u8; 32],
) -> Result<(u64, Option<ReviewPredecessorData>), AppError> {
    use super::super::catalog_envelope::{parse, MAX_ENVELOPE_PLAIN};
    let envelope = parse(plain, domain, MAX_ENVELOPE_PLAIN)?;
    for n in 0..envelope.count {
        let entry = envelope.entry(n)?;
        if entry.id != super::super::review_receive_namespace::ID
            || entry.version != LINKED_REVIEW_VERSION
        {
            continue;
        }
        let mut input = Decoder::new(entry.payload);
        let bad = |_| refused();
        if input.get_bytes().map_err(bad)? != LINKED_REVIEW_TAG {
            return Err(refused());
        }
        let descriptor = input
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| refused())?;
        let generation = input.get_u64().map_err(bad)?;
        let framed = input
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| refused())?;
        input.finish().map_err(bad)?;
        return Ok((
            envelope.generation,
            Some(ReviewPredecessorData {
                generation,
                framed,
                descriptor,
            }),
        ));
    }
    Ok((envelope.generation, None))
}
