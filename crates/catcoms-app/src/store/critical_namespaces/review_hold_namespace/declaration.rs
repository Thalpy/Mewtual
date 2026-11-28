//! Exact recovery data for the founder-only catalog declaration. A seed
//! can restore the same selected bytes after interruption. Its contents
//! never replace the caller's own row, held files or directory witness.
use super::*;
use crate::store::{frame, unframe};
use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const FRAME_HEADER: usize = 8 + 4;
const OVERHEAD: usize = 4 + 2 + 6 * 36 + 2 * 4;
pub(in crate::store) const MAX_SEED_FRAME: usize = FRAME_HEADER
    + OVERHEAD
    + ReviewDeclarationKind::Decision.domain().len()
    + 2 * catalog::MAX_CATALOG_FRAMED
    + 40;

/// Two closed declaration formats with independent seeds and transition
/// checks. Keep the published hold preimage and marker unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum ReviewDeclarationKind {
    Hold,
    Decision,
}

impl ReviewDeclarationKind {
    const fn domain(self) -> &'static [u8] {
        match self {
            Self::Hold => b"catcoms/private-chat-v2-founder-review-declaration/v1\0",
            Self::Decision => b"catcoms/private-chat-v2-founder-review-decision-declaration/v1\0",
        }
    }

    fn key_domain(self) -> &'static str {
        match self {
            Self::Hold => "catcoms/private-chat-v2-founder-review-declaration-key/v1",
            Self::Decision => "catcoms/private-chat-v2-founder-review-decision-declaration-key/v1",
        }
    }

    fn frame_tag(self) -> &'static [u8; 8] {
        match self {
            Self::Hold => b"CTV2RVW\x01",
            Self::Decision => b"CTV2RVD\x01",
        }
    }

    fn prepare_catalog(
        self,
        old: &[u8],
        domain: &[u8; 32],
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        match self {
            Self::Hold => prepare_declaration(old, domain),
            Self::Decision => {
                super::super::review_decision_namespace::prepare_declaration(old, domain)
            }
        }
    }

    fn verify_catalog(self, old: &[u8], new: &[u8], domain: &[u8; 32]) -> Result<(), AppError> {
        match self {
            Self::Hold => verify_declaration(old, new, domain),
            Self::Decision => {
                super::super::review_decision_namespace::verify_declaration(old, new, domain)
            }
        }
    }

    fn selected_marker(
        self,
        key: &[u8; 32],
        layout: CurrentLayoutData,
        candidate: &[u8],
    ) -> Result<[u8; MARKER_LEN], AppError> {
        let (parsed, descriptor) = match self {
            Self::Hold => {
                let parsed = catalog::parse_review_hold_family(candidate, &layout.vault, true)?;
                (parsed.current_layout, parsed.review_hold_payload)
            }
            Self::Decision => {
                let parsed = catalog::parse_review_decision_family(candidate, &layout.vault, true)?;
                (parsed.current_layout, parsed.review_decision_payload)
            }
        };
        if parsed != Some(layout) {
            return Err(refused());
        }
        let descriptor = descriptor.ok_or_else(refused)?;
        match self {
            Self::Hold => marker_data(key, descriptor),
            Self::Decision => super::super::review_decision_namespace::marker_data(key, descriptor),
        }
    }
}

/// Exact labels derived from the independently verified native inventory.
/// The physical caller checks each original leaf even on cold recovery.
#[derive(Clone, Copy)]
pub(in crate::store) struct FounderLeaves {
    pub(in crate::store) row: [u8; 32],
    pub(in crate::store) index: [u8; 32],
    pub(in crate::store) pins: [u8; 32],
    pub(in crate::store) history: [u8; 32],
}

impl FounderLeaves {
    fn fields(self, layout: CurrentLayoutData) -> [[u8; 32]; 6] {
        [
            layout.vault,
            layout.root,
            self.row,
            self.index,
            self.pins,
            self.history,
        ]
    }
}

pub(in crate::store) struct PreparedDeclaration {
    pub(in crate::store) candidate: Zeroizing<Vec<u8>>,
    pub(in crate::store) marker: [u8; MARKER_LEN],
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
}

impl PreparedDeclaration {
    pub(in crate::store) fn prepare(
        kind: ReviewDeclarationKind,
        key: &[u8; 32],
        original: &[u8],
        layout: CurrentLayoutData,
        leaves: FounderLeaves,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        let old = open_catalog(key, original)?;
        let new = kind.prepare_catalog(&old, &layout.vault)?;
        let marker = kind.selected_marker(key, layout, &new)?;
        let candidate = Zeroizing::new(frame(&seal(key, &new, rng)?));
        let domain = kind.domain();
        let mut seed =
            Encoder::with_capacity(OVERHEAD + domain.len() + original.len() + candidate.len());
        let malformed = |_| refused();
        seed.put_bytes(domain).map_err(malformed)?;
        seed.put_u16(VERSION as u16);
        for field in leaves.fields(layout) {
            seed.put_bytes(&field).map_err(malformed)?;
        }
        seed.put_bytes(original).map_err(malformed)?;
        seed.put_bytes(&candidate).map_err(malformed)?;
        let seed_key = Zeroizing::new(blake3::derive_key(kind.key_domain(), key));
        let sealed = seal(&seed_key, &Zeroizing::new(seed.finish()), rng)?;
        let raw = Zeroizing::new(frame(&sealed));
        let length = u32::try_from(FRAME_HEADER + raw.len()).map_err(|_| refused())?;
        let mut seed = Zeroizing::new(Vec::with_capacity(length as usize));
        seed.extend_from_slice(kind.frame_tag());
        seed.extend_from_slice(&length.to_le_bytes());
        seed.extend_from_slice(&raw);
        Ok(Self {
            candidate,
            marker,
            seed,
        })
    }
}

pub(in crate::store) struct AuthenticatedSeed {
    kind: ReviewDeclarationKind,
    plain: Zeroizing<Vec<u8>>,
}

pub(in crate::store) struct OpenedDeclaration<'a> {
    pub(in crate::store) original: &'a [u8],
    pub(in crate::store) candidate: &'a [u8],
    pub(in crate::store) marker: [u8; MARKER_LEN],
}

impl AuthenticatedSeed {
    pub(in crate::store) fn open(
        kind: ReviewDeclarationKind,
        key: &[u8; 32],
        raw: &[u8],
    ) -> Result<Self, AppError> {
        if Self::declared_length(kind, raw) != Some(raw.len()) {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(kind.key_domain(), key));
        Ok(Self {
            kind,
            plain: Zeroizing::new(unseal(&key, &unframe(&raw[FRAME_HEADER..])?)?),
        })
    }

    pub(in crate::store) fn recognizes_frame(kind: ReviewDeclarationKind, raw: &[u8]) -> bool {
        // Leave future versions of this claim to a reader that knows them.
        let tag = kind.frame_tag();
        raw.starts_with(&tag[..tag.len() - 1])
    }

    /// A power cut can leave only a prefix in the private scratch role.
    /// A complete corrupt frame or another format remains on disk.
    pub(in crate::store) fn is_incomplete_scratch(kind: ReviewDeclarationKind, raw: &[u8]) -> bool {
        let tag = kind.frame_tag();
        if raw.len() < tag.len() {
            return tag.starts_with(raw);
        }
        if raw.len() < FRAME_HEADER {
            return raw.starts_with(tag);
        }
        Self::declared_length(kind, raw).is_some_and(|length| raw.len() < length)
    }

    fn declared_length(kind: ReviewDeclarationKind, raw: &[u8]) -> Option<usize> {
        let bytes = raw.strip_prefix(kind.frame_tag())?.get(..4)?;
        let length = u32::from_le_bytes(bytes.try_into().ok()?) as usize;
        let overhead = FRAME_HEADER + OVERHEAD + kind.domain().len() + 40;
        (overhead + 2 * 41..=overhead + 2 * catalog::MAX_CATALOG_FRAMED)
            .contains(&length)
            .then_some(length)
    }

    pub(in crate::store) fn validate(
        &self,
        key: &[u8; 32],
        layout: CurrentLayoutData,
        leaves: FounderLeaves,
    ) -> Result<OpenedDeclaration<'_>, AppError> {
        let mut input = Decoder::new(&self.plain);
        let malformed = |_| refused();
        if input.get_bytes().map_err(malformed)? != self.kind.domain()
            || input.get_u16().map_err(malformed)? != VERSION as u16
        {
            return Err(refused());
        }
        for field in leaves.fields(layout) {
            if input.get_bytes().map_err(malformed)? != field {
                return Err(refused());
            }
        }
        let original = input.get_bytes().map_err(malformed)?;
        let candidate = input.get_bytes().map_err(malformed)?;
        input.finish().map_err(malformed)?;
        let old = open_catalog(key, original)?;
        let new = open_catalog(key, candidate)?;
        self.kind.verify_catalog(&old, &new, &layout.vault)?;
        let marker = self.kind.selected_marker(key, layout, &new)?;
        Ok(OpenedDeclaration {
            original,
            candidate,
            marker,
        })
    }
}

fn open_catalog(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(41..=catalog::MAX_CATALOG_FRAMED).contains(&raw.len()) {
        return Err(refused());
    }
    Ok(Zeroizing::new(unseal(key, &unframe(raw)?)?))
}
