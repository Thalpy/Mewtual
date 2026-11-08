//! Allocation-free framing for the first PRIVATE text Add input family.
//!
//! These are negative grammar checks, not a validated KeyPackage or an admission.
//! SDK signature/init-key/extension/lifetime validation MUST still run afterward.
//! In particular, the membership nonce below does not prove that an invite was
//! accepted or reserved. The Root governance journal owns that independent cut.
use catcoms_crypto::DeviceId;
use catcoms_wire::Decoder;

pub(super) const MAX_PACKAGE_BYTES: usize = 32 * 1024;
// Matches the unchanged CLOSED text-current output/cold grammar. The exact
// membership tuple below is smaller still; catalog's unrelated4096 cap does not
// authorize a larger package here.
pub(super) const MAX_CREDENTIAL_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Invalid;

/// All references borrow the exact original frame. No SDK object or cloned key
/// is created by this scan. The owning consumer pays the complete typed decoder
/// and all verification copies before it constructs either one.
#[derive(Debug)]
pub(super) struct Facts<'a> {
    pub subject: &'a [u8; 32],
    pub credential: &'a [u8],
    pub not_before: u64,
    pub not_after: u64,
}

struct Cursor<'a> {
    rest: &'a [u8],
}
impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], Invalid> {
        let (head, rest) = self.rest.split_at_checked(count).ok_or(Invalid)?;
        self.rest = rest;
        Ok(head)
    }
    fn byte(&mut self) -> Result<u8, Invalid> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Invalid> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().map_err(|_| Invalid)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, Invalid> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().map_err(|_| Invalid)?,
        ))
    }
    fn vector(&mut self, max: usize) -> Result<&'a [u8], Invalid> {
        let first = self.byte()?;
        let width = 1usize << (first >> 6);
        // MLS permits only the first three QUIC widths. Refuse non-minimal
        // framing too, matching pinned tls_codec's canonical MLS vectors.
        if width == 8 {
            return Err(Invalid);
        }
        let mut length = usize::from(first & 0x3f);
        for byte in self.take(width - 1)? {
            length = length
                .checked_mul(256)
                .and_then(|n| n.checked_add(usize::from(*byte)))
                .ok_or(Invalid)?;
        }
        if (width == 2 && length < 64) || (width == 4 && length < 16_384) || length > max {
            return Err(Invalid);
        }
        self.take(length)
    }
    fn exact_vector(&mut self, expected: &[u8]) -> Result<(), Invalid> {
        if self.vector(expected.len())? != expected {
            return Err(Invalid);
        }
        Ok(())
    }
    fn finish(self) -> Result<(), Invalid> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(Invalid)
        }
    }
}

/// The host debits this deterministic upper bound BEFORE the scan. The framing
/// cursor visits each field once; the bounded credential decoder and the fixed
/// identity hash add fewer than eight complete input passes. This does not pay
/// later TLS/SDK parsing, verification or cloning.
pub(super) fn scan_work(bytes: usize) -> Option<usize> {
    bytes.checked_mul(8)?.checked_add(4096)
}

/// Exact first-family grammar: MLS1.0/suite3, fixed X25519/Ed25519 widths,
/// Basic membership credential, the product's exact capability lists, and empty
/// leaf/package extensions. Unknown compatible-looking features stay closed;
/// this entry is intentionally narrower than the legacy KeyPackage parser.
pub(super) fn scan<'a>(
    bytes: &'a [u8],
    group: &[u8; 16],
    expected_subject: &[u8; 32],
    nonce: &[u8; 16],
) -> Result<Facts<'a>, Invalid> {
    if bytes.is_empty() || bytes.len() > MAX_PACKAGE_BYTES {
        return Err(Invalid);
    }
    let mut c = Cursor { rest: bytes };
    if c.u16()? != 1 || c.u16()? != 5 || c.u16()? != 1 || c.u16()? != 3 {
        return Err(Invalid);
    }
    let init = c.vector(32)?;
    let encryption = c.vector(32)?;
    let subject: &[u8; 32] = c.vector(32)?.try_into().map_err(|_| Invalid)?;
    if init.len() != 32
        || encryption.len() != 32
        || init == encryption
        || subject != expected_subject
    {
        return Err(Invalid);
    }
    if c.u16()? != 1 {
        return Err(Invalid);
    }
    let credential = c.vector(MAX_CREDENTIAL_BYTES)?;
    let mut membership = Decoder::new(credential);
    if membership.get_str().map_err(|_| Invalid)? != "catcoms/membership/v1"
        || membership.get_bytes().map_err(|_| Invalid)?
            != DeviceId::from_public_key_bytes(subject).as_bytes()
        || membership.get_bytes().map_err(|_| Invalid)? != group
        || membership.get_bytes().map_err(|_| Invalid)? != nonce
    {
        return Err(Invalid);
    }
    membership.finish().map_err(|_| Invalid)?;
    c.exact_vector(&[0, 1])?; // capabilities.versions: exactly MLS1.0.
    c.exact_vector(&[0, 3])?; // capabilities.ciphersuites: exactly suite3.
    c.exact_vector(&[])?; // capabilities.extensions.
    c.exact_vector(&[])?; // capabilities.proposals.
    c.exact_vector(&[0, 1])?; // capabilities.credentials: Basic only.
    if c.byte()? != 1 {
        // LeafNodeSource::KeyPackage.
        return Err(Invalid);
    }
    let not_before = c.u64()?;
    let not_after = c.u64()?;
    c.exact_vector(&[])?; // LeafNode extensions.
    if c.vector(64)?.len() != 64 {
        return Err(Invalid);
    }
    c.exact_vector(&[])?; // KeyPackage extensions.
    if c.vector(64)?.len() != 64 {
        return Err(Invalid);
    }
    c.finish()?;
    Ok(Facts {
        subject,
        credential,
        not_before,
        not_after,
    })
}

#[cfg(test)]
mod tests;
