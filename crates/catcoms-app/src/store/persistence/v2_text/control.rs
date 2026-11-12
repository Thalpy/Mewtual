//! Kind-separated immutable private-control envelopes. No file or permission is created.
//!
//! This private codec layer accepts only the exact closed tag/version for its kind. Its
//! caller must also use the kind's full typed parser and bind original selected source. A
//! valid MAC or a complete random candidate is neither Saved nor an admitted work item.
use super::common::{field, refused, VERSION};
use crate::store::{frame, unframe};
use crate::AppError;
use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::Decoder;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum TextControlKindData {
    Head,
    Archive,
    Retain,
    Intent,
    Signature,
    Publication,
    Seed,
    Wal,
}
impl TextControlKindData {
    pub(super) fn plain_cap(self) -> usize {
        match self {
            Self::Head | Self::Archive => 2048,
            Self::Retain => 1024,
            Self::Intent => 4096,
            Self::Signature | Self::Publication => 512,
            Self::Seed => 32768,
            Self::Wal => 2 * 1024 * 1024,
        }
    }
    fn domains(self) -> (&'static [u8], &'static str, &'static [u8]) {
        macro_rules! names {
            ($name:literal) => {
                (
                    concat!("catcoms/private-chat-text-", $name, "/v1\0").as_bytes(),
                    concat!("catcoms/private-chat-text-", $name, "-key/v1"),
                    concat!("catcoms/private-chat-text-", $name, "-id/v1\0").as_bytes(),
                )
            };
        }
        match self {
            Self::Head => names!("head"),
            Self::Archive => names!("archive"),
            Self::Retain => names!("retain"),
            Self::Intent => names!("intent"),
            Self::Signature => names!("signature"),
            Self::Publication => names!("publication"),
            Self::Seed => names!("seed"),
            Self::Wal => names!("wal"),
        }
    }
    fn check_plain(self, plain: &[u8]) -> Result<(), AppError> {
        if plain.is_empty() || plain.len() > self.plain_cap() {
            return Err(refused());
        }
        let mut d = Decoder::new(plain);
        if field(&mut d, self.domains().0.len())? != self.domains().0
            || d.get_u16().map_err(|_| refused())? != VERSION
        {
            return Err(refused());
        }
        // This is only the common envelope boundary. Full kind-specific EOF/inner checks
        // remain mandatory and cannot be replaced by this private tag check.
        Ok(())
    }
    pub(super) fn id_data(self, raw: &[u8]) -> TextControlIdData {
        let mut h = blake3::Hasher::new();
        h.update(self.domains().2);
        h.update(raw);
        TextControlIdData {
            kind: self,
            digest: *h.finalize().as_bytes(),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct TextControlIdData {
    kind: TextControlKindData,
    digest: [u8; 32],
}
impl std::fmt::Debug for TextControlIdData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextControlIdData").finish_non_exhaustive()
    }
}
impl TextControlIdData {
    pub(in crate::store) fn kind_data(self) -> TextControlKindData {
        self.kind
    }
    pub(in crate::store) fn as_bytes(&self) -> &[u8; 32] {
        &self.digest
    }
    pub(super) fn from_data(kind: TextControlKindData, digest: [u8; 32]) -> Self {
        Self { kind, digest }
    }
}

/// Non-Clone exact candidate. Caller pays plaintext+AEAD growth+frame overlaps BEFORE this
/// constructor, retains the same owner through every retry, and moves its real fee with it.
pub(in crate::store) struct PreparedTextControlFrameData {
    raw: Zeroizing<Vec<u8>>,
    id: TextControlIdData,
}
impl std::fmt::Debug for PreparedTextControlFrameData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedTextControlFrameData")
            .finish_non_exhaustive()
    }
}
impl PreparedTextControlFrameData {
    pub(super) fn prepare_data(
        kind: TextControlKindData,
        key: &[u8; 32],
        plain: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        kind.check_plain(plain)?; // cap/tag/version precede key/RNG/seal, not just final frame
        let derived = Zeroizing::new(blake3::derive_key(kind.domains().1, key));
        let sealed = seal(&derived, plain, rng)?;
        let raw = Zeroizing::new(frame(&sealed));
        if raw.len() != plain.len() + 40 || raw.capacity() > kind.plain_cap() + 40 {
            return Err(refused());
        }
        Ok(Self {
            id: kind.id_data(&raw),
            raw,
        })
    }
    pub(in crate::store) fn id_data(&self) -> TextControlIdData {
        self.id
    }
    pub(in crate::store) fn frame_data(&self) -> &[u8] {
        &self.raw
    }
}

pub(super) fn open_control_data(
    key: &[u8; 32],
    expected: TextControlIdData,
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if raw.len() <= 40
        || raw.len() > expected.kind.plain_cap() + 40
        || expected.kind.id_data(raw) != expected
    {
        return Err(refused());
    }
    // Original raw + copied ciphertext + plaintext EACH overlap. In particular the old WAL
    // open buffers MUST drain before a new WAL Encoder/seal; do not reuse index-page fees.
    let derived = Zeroizing::new(blake3::derive_key(expected.kind.domains().1, key));
    let plain = Zeroizing::new(unseal(&derived, &unframe(raw)?)?);
    expected.kind.check_plain(&plain)?;
    Ok(plain)
}

/// A bounded MAC observation only, NEVER absence, scratch permission or parsed validity.
pub(super) enum TextControlMacInspectionData {
    Unauthenticated,
    Authenticated,
}

/// Inspect the RESERVED first-seed-stage purpose/version, under the caller's independent
///4MiB recovery invoice. Preserve ANY known-control MAC success before interpreting kind,
/// plaintext cap, tag, version or group. Plaintexts drain before trying another key; this
/// is eight bounded sequential attempts, not eight simultaneously retained buffers.
/// Unknown future KDF authenticity is not detectable: a future writer MUST select an old-
/// reader-refusal version before changing the meaning of `text.seed.stage.v1`.
pub(super) fn inspect_seed_stage_mac_data(
    key: &[u8; 32],
    raw: &[u8],
) -> Result<TextControlMacInspectionData, AppError> {
    if raw.len() > TextControlKindData::Seed.plain_cap() + 40 {
        return Err(refused());
    }
    // Recognize this OTHER public envelope only to refuse/preserve it. No private-page
    // decrypt/adoption or root trust is inferred from its public prefix.
    const PAGE: &[u8] = b"catcoms/private-chat-sealed-page/v1\0";
    if raw.len() >= 4 + PAGE.len()
        && raw[..4] == (PAGE.len() as u32).to_be_bytes()
        && &raw[4..4 + PAGE.len()] == PAGE
    {
        return Err(refused());
    }
    if raw.len() < 40 {
        return Ok(TextControlMacInspectionData::Unauthenticated);
    }
    let sealed = unframe(raw)?;
    for kind in [
        TextControlKindData::Head,
        TextControlKindData::Archive,
        TextControlKindData::Retain,
        TextControlKindData::Intent,
        TextControlKindData::Signature,
        TextControlKindData::Publication,
        TextControlKindData::Seed,
        TextControlKindData::Wal,
    ] {
        let derived = Zeroizing::new(blake3::derive_key(kind.domains().1, key));
        if let Ok(plain) = unseal(&derived, &sealed) {
            drop(Zeroizing::new(plain));
            return Ok(TextControlMacInspectionData::Authenticated);
        }
    }
    Ok(TextControlMacInspectionData::Unauthenticated)
}

#[cfg(test)]
mod tests;
