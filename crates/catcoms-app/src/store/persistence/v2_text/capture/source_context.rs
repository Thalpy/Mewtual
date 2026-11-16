//! Frozen FORMAT source-context commitment DATA, shared by every real C derivation.
//!
//! This is the literal domain followed by catcoms_wire bytes(canonical Source context),
//! including its u32 big-endian length. Raw BLAKE3(context) is a DIFFERENT namespace and
//! is intentionally not accepted as a prototype/migration alias. Hashing proves no source.
use super::refused;
use crate::AppError;

const DOMAIN: &[u8] = b"catcoms/private-chat-text-source-context/v1\0";
const MAX_CANONICAL_SOURCE_CONTEXT: usize = 692;

pub(in crate::store::persistence::v2_text) fn canonical_source_context_digest_data(
    canonical: &[u8],
) -> Result<[u8; 32], AppError> {
    if canonical.is_empty() || canonical.len() > MAX_CANONICAL_SOURCE_CONTEXT {
        return Err(refused());
    }
    let mut hash = blake3::Hasher::new();
    hash.update(DOMAIN);
    hash.update(&(canonical.len() as u32).to_be_bytes());
    hash.update(canonical);
    Ok(*hash.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_context_commitment_matches_independent_frozen_domain_and_bytes_vector() {
        // Hash-only DATA fixture, not a claim these32 bytes encode an authorized Source.
        // Independent one-chunk BLAKE3 reference, checked against official empty/abc
        // vectors, yields this digest for this exact frozen FORMAT preimage:
        // 636174636f6d732f707269766174652d636861742d746578742d736f757263652d
        // 636f6e746578742f76310000000020
        // 000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
        let canonical: [u8; 32] = std::array::from_fn(|i| i as u8);
        let expected = [
            0xb7, 0x87, 0x44, 0xc6, 0x83, 0x9e, 0x8c, 0xbb, 0xca, 0x5c, 0x89, 0x57, 0x3b, 0x80,
            0x90, 0xfb, 0x28, 0x7e, 0xd7, 0x1d, 0x51, 0x6a, 0xf2, 0x8f, 0x36, 0x7c, 0xb3, 0x20,
            0xc6, 0x7b, 0x2a, 0xf3,
        ];
        assert_eq!(
            canonical_source_context_digest_data(&canonical).unwrap(),
            expected
        );
        assert_ne!(*blake3::hash(&canonical).as_bytes(), expected);
        let mut unframed = blake3::Hasher::new();
        unframed.update(DOMAIN);
        unframed.update(&canonical);
        assert_ne!(*unframed.finalize().as_bytes(), expected);
        let mut changed = canonical;
        changed[17] ^= 1;
        assert_ne!(
            canonical_source_context_digest_data(&changed).unwrap(),
            expected
        );
        assert_ne!(
            canonical_source_context_digest_data(&canonical[..31]).unwrap(),
            expected
        );
    }

    #[test]
    fn source_context_commitment_bounds_input_before_hashing() {
        assert!(canonical_source_context_digest_data(&[]).is_err());
        assert!(canonical_source_context_digest_data(&[0; MAX_CANONICAL_SOURCE_CONTEXT]).is_ok());
        assert!(
            canonical_source_context_digest_data(&[0; MAX_CANONICAL_SOURCE_CONTEXT + 1]).is_err()
        );
    }
}
