//! Protocol-aware NEGATIVE group destinations, used before the Current-layout handler exists.
//!
//! An original group reservation covers every signer. Its presence must remain a fence even
//! on an ID2-only mount whose catalog has no ID3 descriptor. Nothing here opens/adopts a group,
//! cleans an unknown object or grants Current; the actual ID3 transaction is a separate consumer.
use super::*;

pub(in crate::store) const DECLARATION_SEED: &str = "chat-critical-namespaces.current.seed";
pub(in crate::store) const MARKER: &str = "current-layout.bin";

/// Fixed lower-hex suffix of `g-<suffix>`. Signer is deliberately absent from this domain.
/// This DATA selector neither proves absence nor authenticates a physical mounted layout.
pub(in crate::store) fn group_suffix(
    vault: &[u8; 32],
    root: &[u8; 32],
    group: &[u8],
) -> Result<[u8; 64], AppError> {
    if group.is_empty() || group.len() > 256 {
        return Err(refused());
    }
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/private-chat-v2-group-root/v1\0");
    hash.update(vault);
    hash.update(root);
    hash.update(&(group.len() as u32).to_be_bytes());
    hash.update(group);
    let mut suffix = [0; 64];
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in hash.finalize().as_bytes().iter().copied().enumerate() {
        suffix[2 * index] = HEX[(byte >> 4) as usize];
        suffix[2 * index + 1] = HEX[(byte & 15) as usize];
    }
    Ok(suffix)
}

/// The old ID2 inspector cannot classify a Current declaration. Preserve any presence before
/// its old gates/cleaners, including a dangling link or a partial first write. The supported
/// Current opener will replace this refusal with its own complete independently paid inspector.
pub(super) fn refuse_unsupported_layout(dir: &Path) -> Result<(), AppError> {
    for path in [
        dir.join(DECLARATION_SEED),
        dir.join(ARCHIVE_BOOTSTRAP_ROOT).join(MARKER),
    ] {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    Ok(())
}

impl ServerStore {
    /// Caller may hold admission/pins; this takes ONLY Root, never either predecessor mutex.
    /// Group mkdir first needs those same serializers, so an actual pin check can keep pins
    /// held through this final Root observation without recursively acquiring admission.
    pub(in crate::store) fn require_no_v2_group_directory(
        &self,
        group: &[u8],
    ) -> Result<(), AppError> {
        if self.archive_bootstrap_policy.is_none() {
            return Ok(());
        }
        let root = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let root = root.as_ref().ok_or_else(refused)?;
        if self.archive_group_absent_under_original_root(root, group)? {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

#[cfg(test)]
mod tests;
