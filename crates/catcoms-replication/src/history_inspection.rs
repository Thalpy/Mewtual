//! Read-only decoding of retained raw history before any Automerge parse or expansion.
//!
//! This is not an ingestion, signature or authorization API. Historical actor names are
//! inspected as data, never promoted into full signing identities. Refusal leaves the exact
//! original available through the bounded frame reader; it never deletes or rejects history.
use crate::ReplError;

/// Maximum raw change bytes parsed by the ephemeral admin projection.
pub const MAX_INSPECTED_CHANGE_BYTES: usize = 64 * 1024;

/// Explicit inspection result; a decoding limit is not a reason to request user history repair.
pub enum LegacyChangeInspection {
    /// Raw framing, actor sizes and expanded cell/op/string budgets passed before parsing.
    Decoded(automerge::ExpandedChange),
    /// Compressed/non-change chunks or unsupported historical actor names remain raw-readable.
    Unsupported,
    /// Raw bytes, table size or expanded work exceeds the bounded observer policy.
    TooLarge,
    /// The supported raw grammar/checksum is malformed. This does not classify its signature.
    Invalid,
}
impl std::fmt::Debug for LegacyChangeInspection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Decoded(_) => "LegacyChangeInspection::Decoded { .. }",
            Self::Unsupported => "LegacyChangeInspection::Unsupported",
            Self::TooLarge => "LegacyChangeInspection::TooLarge",
            Self::Invalid => "LegacyChangeInspection::Invalid",
        })
    }
}

/// Inspect a bounded raw kind-1 change. Byte and RLE/actor/cell/predecessor/string preflight
/// happens BEFORE Change::from_bytes, then the existing bounded parser/expander may run.
/// A 16-byte legacy actor is readable but supplies no author key or group-membership proof.
/// Expanded actor references can consume more memory than the raw-byte ceiling; the separate
/// cell, actor-name and predecessor budgets bound that intermediate (including cloned names).
pub fn inspect_legacy_change(bytes: &[u8]) -> LegacyChangeInspection {
    inspect_with_parser(bytes, automerge::Change::from_bytes)
}

// Keep the pre-parse boundary observable without executing a dangerous allocation in a
// regression test. Production supplies only the real parser, never an RPC/caller callback.
fn inspect_with_parser<E>(
    bytes: &[u8],
    parse: impl FnOnce(Vec<u8>) -> Result<automerge::Change, E>,
) -> LegacyChangeInspection {
    if bytes.len() > MAX_INSPECTED_CHANGE_BYTES {
        return LegacyChangeInspection::TooLarge;
    }
    if bytes.len() < 10 || bytes[..4] != [0x85, 0x6f, 0x4a, 0x83] {
        return LegacyChangeInspection::Invalid;
    }
    if bytes[8] != 1 {
        return LegacyChangeInspection::Unsupported;
    }
    match crate::bounded_change::check_legacy_inspection(bytes) {
        Ok(()) => {}
        Err(ReplError::EpochBound) => return LegacyChangeInspection::TooLarge,
        Err(ReplError::EpochAuthority) => return LegacyChangeInspection::Unsupported,
        Err(_) => return LegacyChangeInspection::Invalid,
    }
    let Ok(change) = parse(bytes.to_vec()) else {
        return LegacyChangeInspection::Invalid;
    };
    // Change::from_bytes does not enforce its chunk checksum. Automerge hashes the kind,
    // canonical payload-length encoding and payload; its parser separately refuses overlong
    // lengths. Match that rule without rewriting the retained original.
    if change.hash().0[..4] != bytes[4..8] {
        return LegacyChangeInspection::Invalid;
    }
    if change.len() > 512 {
        return LegacyChangeInspection::TooLarge;
    }
    LegacyChangeInspection::Decoded(change.decode())
}

#[cfg(test)]
mod tests;
