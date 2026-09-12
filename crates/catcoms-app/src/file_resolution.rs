//! Resolve randomized encryptions without trusting a member's whole-file CID as byte identity.
use std::collections::BTreeMap;

use super::{file_manifest_version, Cid, FileEntry, FileManifest, MAX_FILE_BYTES};
use super::{AppError, CryptoRngCore, MeshTransport, RequestCancellation, Server};

/// Bound authentication and provider attempts per chunk independently of the index row limit.
pub(super) const MAX_MANIFEST_VARIANTS: usize = 4;

pub(super) struct ResolvedFile {
    /// Sorted by exact manifest digest, independent of replicated row arrival order.
    pub variants: Vec<(FileEntry, FileManifest)>,
    /// Binds the entire admitted set, so adding/removing even a fallback invalidates caches.
    pub version: [u8; 32],
}

/// Only encryption may differ. Chunk plaintext hashes are checked again by `open_file`; this
/// comparison never makes an alternative wrapped key or ciphertext implicitly authenticated.
pub(super) fn equivalent(a: &FileManifest, b: &FileManifest) -> bool {
    a.plaintext_cid == b.plaintext_cid
        && a.total_size == b.total_size
        && a.mime == b.mime
        && a.chunks.len() == b.chunks.len()
        && a.chunks.iter().zip(&b.chunks).all(|(a, b)| {
            a.plaintext_cid == b.plaintext_cid && a.size == b.size && a.mime == b.mime
        })
}

/// Reject an index-authored size/chunk plan before any local I/O. Otherwise a peer can claim a
/// tiny upload's whole CID while referencing the same large held donor chunk hundreds of times,
/// making a small user action trigger file-sized repeated decryption. MIME inheritance remains
/// the existing dedup behavior; plaintext chunk identities and sizes come from this upload.
pub(super) fn reusable_upload_entry(
    entries: &[FileEntry],
    cid: &Cid,
    total_size: u64,
    chunk_identities: &[(Cid, u64)],
    mut verify: impl FnMut(&FileManifest) -> bool,
) -> Option<FileEntry> {
    resolve(entries, cid)?
        .variants
        .into_iter()
        .find_map(|(entry, manifest)| {
            let matches_upload = manifest.total_size == total_size
                && manifest.chunks.len() == chunk_identities.len()
                && manifest
                    .chunks
                    .iter()
                    .zip(chunk_identities)
                    .all(|(chunk, (cid, size))| chunk.plaintext_cid == *cid && chunk.size == *size);
            (matches_upload && verify(&manifest)).then_some(entry)
        })
}

/// Metadata-only, fail-closed resolution shared by uploads, downloads, previews and inventory.
/// Legacy chunk layouts and differing MIME claims remain conflicts unless they agree exactly.
pub(super) fn resolve(entries: &[FileEntry], cid: &Cid) -> Option<ResolvedFile> {
    let mut variants = BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.cid == cid.as_bytes()) {
        let version = file_manifest_version(&entry.file_ref);
        if variants.contains_key(&version) {
            continue;
        }
        if variants.len() == MAX_MANIFEST_VARIANTS {
            return None;
        }
        let manifest = FileManifest::decode_or_legacy(&entry.file_ref).ok()?;
        if manifest.total_size > MAX_FILE_BYTES as u64 {
            return None;
        }
        if let Some((_, first)) = variants.values().next() {
            if !equivalent(first, &manifest) {
                return None;
            }
        }
        variants.insert(version, (entry.clone(), manifest));
    }
    let first = *variants.keys().next()?;
    let version = if variants.len() == 1 {
        first // Preserve the existing single-manifest cache identity.
    } else {
        let mut hash = blake3::Hasher::new();
        hash.update(b"mewtual-file-media-variants/v1");
        hash.update(&(variants.len() as u64).to_be_bytes());
        for version in variants.keys() {
            hash.update(version);
        }
        *hash.finalize().as_bytes()
    };
    Some(ResolvedFile {
        variants: variants.into_values().collect(),
        version,
    })
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Exhaust local alternatives before waiting for any provider. Every returned byte is opened
    /// against its own exact reference; equivalence only establishes which plaintext is allowed.
    pub(super) async fn fetch_resolved_chunk(
        &mut self,
        resolved: &ResolvedFile,
        index: usize,
        cancellation: Option<RequestCancellation>,
    ) -> Result<(Vec<u8>, Option<String>), AppError> {
        if index >= resolved.variants[0].1.chunks.len() {
            return Err(AppError::Invalid(format!("chunk {index} is out of range")));
        }
        let cancelled = || {
            cancellation
                .as_ref()
                .is_some_and(RequestCancellation::is_cancelled)
        };
        if cancelled() {
            return Err(AppError::Invalid("file download cancelled".into()));
        }
        let mut missing = Vec::new();
        let mut last_error = AppError::Invalid(format!(
            "file not available yet; no connected peer has chunk {index}"
        ));
        for (_, manifest) in &resolved.variants {
            let reference = &manifest.chunks[index];
            if let Some(ciphertext) = self.sync.get_blob(&reference.ciphertext_cid) {
                match self.sync.open_file(&ciphertext, reference) {
                    Ok(plaintext) => return Ok((plaintext, None)),
                    Err(error) => {
                        last_error = AppError::Invalid(format!(
                            "chunk {index} could not be decrypted: {error}"
                        ))
                    }
                }
                // The content-addressed ciphertext is already readable. Fetching the same CID
                // cannot repair this bad exact key/reference, and reopening it would double work.
            } else {
                missing.push(reference);
            }
        }
        for reference in missing {
            if cancelled() {
                return Err(AppError::Invalid("file download cancelled".into()));
            }
            match self
                .fetch_and_open_chunk_cancellable(reference, index, cancellation.clone())
                .await
            {
                Ok(chunk) => return Ok(chunk),
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }
}
