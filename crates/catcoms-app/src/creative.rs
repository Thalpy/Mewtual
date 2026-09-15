//! Immutable creative blobs. Metadata/Studio documents are intentionally a separate transaction:
//! publish bytes first, and only use the returned CID in an edit after this call succeeds.

use catcoms_rt::{CryptoRngCore, MeshTransport, RequestCancellation};

use crate::{AppError, Cid, Server};

/// PIX1 encoded-byte ceiling, checked before parsing or entering the actor queue.
pub const PIX_MAX_BYTES: usize = 64 * 1024;
pub use catcoms_sync::MAX_BOUNDED_BLOB_BYTES;

/// A real, verified held blob; this is not a Studio frame record or a fileshare index entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedPix {
    pub cid: String,
    pub bytes: usize,
}

/// Validate PIX1 without allocating a raster. Matches the frontend's format, including maximal
/// runs split at 256 pixels and duplicate *entries* (not merely duplicate RGB or role) rejection.
pub fn validate_pix(bytes: &[u8]) -> Result<(), AppError> {
    let invalid = |reason: &str| AppError::Invalid(format!("invalid PIX1: {reason}"));
    if bytes.len() > PIX_MAX_BYTES {
        return Err(invalid("over 64 KiB"));
    }
    if bytes.len() < 7 {
        return Err(invalid("truncated header"));
    }
    if &bytes[..4] != b"PIX1" {
        return Err(invalid("bad magic"));
    }
    let pixels = (usize::from(bytes[4]) + 1) * (usize::from(bytes[5]) + 1);
    let count = usize::from(bytes[6]) + 1;
    if !(4..=16).contains(&count) {
        return Err(invalid("palette size"));
    }
    let start = 7 + 4 * count;
    if bytes.len() < start {
        return Err(invalid("truncated palette"));
    }
    for i in 0..count {
        let entry = &bytes[7 + 4 * i..11 + 4 * i];
        if entry[0] > 8 {
            return Err(invalid("palette role"));
        }
        if bytes[7..7 + 4 * i]
            .chunks_exact(4)
            .any(|other| other == entry)
        {
            return Err(invalid("duplicate palette entry"));
        }
    }
    let mut filled = 0;
    let mut previous = None;
    let mut runs = bytes[start..].chunks_exact(2);
    for run in &mut runs {
        let len = usize::from(run[0]) + 1;
        if usize::from(run[1]) >= count {
            return Err(invalid("index out of palette"));
        }
        if previous == Some(run[1]) {
            return Err(invalid("non-maximal run"));
        }
        filled += len;
        if filled > pixels {
            return Err(invalid("overshoot"));
        }
        // An equal-index successor is necessary when the previous run filled its wire field.
        previous = if len == 256 { None } else { Some(run[1]) };
    }
    if !runs.remainder().is_empty() {
        return Err(invalid("trailing bytes"));
    }
    if filled != pixels {
        return Err(invalid("undershoot"));
    }
    Ok(())
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Validate, stage, promote and flush an immutable PIX blob. No frame/index/expiry reference
    /// is published here. A failed call can leave an orphan but must never justify a reference.
    pub fn publish_pix(&mut self, bytes: &[u8]) -> Result<PublishedPix, AppError> {
        validate_pix(bytes)?;
        if !self.sync.has_persistent_blob_store() {
            return Err(AppError::Invalid(
                "PIX publication requires an attached persistent blob store".into(),
            ));
        }
        let cid = self.sync.publish_blob_bounded(bytes)?;
        Ok(PublishedPix {
            cid: cid.to_hex(),
            bytes: bytes.len(),
        })
    }

    /// Fetch one CID with a caller-declared maximum (at most 9 MiB). A `None` response means
    /// unavailable, not invalid. Consumers still enforce exact declared length and their format.
    pub async fn request_blob_bounded(
        &mut self,
        cid: &Cid,
        max_bytes: usize,
        cancellation: Option<RequestCancellation>,
    ) -> Result<Option<Vec<u8>>, AppError> {
        Ok(self
            .sync
            .request_blob_bounded(cid, max_bytes, cancellation)
            .await?)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Byte-for-byte counterpart of pix.test.ts's existing 4x2 golden vector.
    pub(crate) fn golden() -> Vec<u8> {
        vec![
            0x50, 0x49, 0x58, 0x31, 3, 1, 3, 1, 0x13, 0x12, 0x18, 2, 0xe8, 0xe6, 0xf0, 3, 0x97,
            0x7d, 0xf2, 0, 0xe0, 0x7a, 0xb8, 1, 0, 1, 1, 2, 2, 0, 3,
        ]
    }

    #[test]
    fn pix_golden_and_maximal_run_split() {
        validate_pix(&golden()).unwrap();
        let mut split = golden()[..23].to_vec();
        split[4] = 255;
        split[5] = 1; // 512 pixels: two maximal 256-pixel runs, same palette index
        split.extend([255, 1, 255, 1]);
        validate_pix(&split).unwrap();
        split[23] = 254;
        assert!(validate_pix(&split)
            .unwrap_err()
            .to_string()
            .contains("non-maximal"));
    }

    #[test]
    fn pix_rejects_malformed_and_oversize_inputs() {
        let good = golden();
        for len in 0..good.len() {
            assert!(validate_pix(&good[..len]).is_err(), "prefix {len}");
        }
        for (offset, value) in [(0, 0), (6, 2), (6, 16), (7, 9), (23, 255), (24, 4)] {
            let mut bad = good.clone();
            bad[offset] = value;
            assert!(validate_pix(&bad).is_err(), "offset {offset}");
        }
        let mut duplicate = good.clone();
        duplicate.copy_within(7..11, 11);
        assert!(validate_pix(&duplicate).is_err());
        let mut trailing = good;
        trailing.push(0);
        assert!(validate_pix(&trailing).is_err());
        assert!(validate_pix(&vec![0; PIX_MAX_BYTES + 1]).is_err());
    }
}
