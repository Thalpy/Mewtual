//! Canonical index-root commitment DATA. No selected-root or complete-miss issuer.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(in crate::store) enum TextIndexCaptureRoleData {
    Directory = 1,
    Token = 2,
    Publication = 3,
}

/// Literal domain || b32 Cid || role byte || canonical Empty/Present root descriptor.
/// A caller can hash Empty DATA; only its actual complete selected-source proof can say
/// that Empty is this query's real root. Pointer/native/source identities never enter it.
pub(in crate::store) fn index_capture_data(
    context: TextContextIdData,
    role: TextIndexCaptureRoleData,
    root: TextRootData,
) -> Result<[u8; 32], AppError> {
    let mut canonical = Encoder::with_capacity(36 + 1 + 118);
    put(&mut canonical, context.as_bytes())?;
    canonical.put_u8(role as u8);
    root.encode(&mut canonical)?;
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/private-chat-text-index-capture/v1\0");
    hash.update(canonical.as_bytes());
    Ok(*hash.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_capture_roles_root_and_context_are_canonical_distinct_data() {
        let context = TextContextIdData::from_bytes(std::array::from_fn(|i| i as u8));
        let directory = index_capture_data(
            context,
            TextIndexCaptureRoleData::Directory,
            TextRootData::Empty,
        )
        .unwrap();
        // Literal independent golden preimage/domain; this has no actual Empty authority.
        assert_eq!(directory, GOLDEN);
        assert_ne!(
            directory,
            index_capture_data(
                context,
                TextIndexCaptureRoleData::Token,
                TextRootData::Empty
            )
            .unwrap()
        );
        assert_ne!(
            directory,
            index_capture_data(
                context,
                TextIndexCaptureRoleData::Publication,
                TextRootData::Empty
            )
            .unwrap()
        );
        assert_ne!(
            directory,
            index_capture_data(
                TextContextIdData::from_bytes([0; 32]),
                TextIndexCaptureRoleData::Directory,
                TextRootData::Empty
            )
            .unwrap()
        );
    }
    const GOLDEN: [u8; 32] = [
        0x6f, 0x82, 0x9b, 0xc7, 0xa6, 0x58, 0x38, 0xc3, 0x2c, 0x32, 0x4a, 0xec, 0xe7, 0x45, 0x88,
        0xe2, 0x61, 0xc4, 0x0a, 0x55, 0xae, 0x4d, 0x64, 0x27, 0xcd, 0x6c, 0xce, 0xea, 0xe5, 0x2b,
        0x74, 0xc5,
    ];
}
