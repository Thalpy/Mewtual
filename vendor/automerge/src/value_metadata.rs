//! Body-free preflight for small trusted ROOT-map fields retained in an untrusted document.

use crate::{Automerge, ObjType};

/// Exact stored value kind, without author identity or scalar content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootValueKind {
    Object(ObjType),
    Bytes,
    String,
    Int,
    Uint,
    Float,
    Counter,
    Timestamp,
    Boolean,
    Null,
    Unknown(u8),
}

/// Fixed metadata for the same current ROOT-map winner selected by unisolated `get`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootValueMetadata {
    pub kind: RootValueKind,
    /// Byte length in the winning operation's raw scalar storage, before any scalar decoding.
    /// Bytes/String/Unknown have their exact content byte count; objects have zero. Numeric
    /// lengths describe their stored encoding, not display text or a Counter after increments.
    pub stored_bytes: usize,
}

/// Caller-paid conservative limits, independently checked before the ROOT-key lookup.
///
/// Total retained operations (including overwritten operations) are capped, rather than merely
/// visible conflicts. The encoded key-column allowance also bounds retained property-name data;
/// physical metadata/raw slab counts additionally cannot exceed `max_ops + 1` (checked).
/// neither scalar-body length nor requested key length can fund extra lookup work. A too-large
/// document becomes unavailable to this reader, without changing any ingestion/storage policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootValueReadLimits {
    pub max_ops: usize,
    pub max_key_bytes: usize,
}

/// Read-only refusal; never permission to truncate, repair or mutate the source document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootValueReadError {
    WorkLimit,
    KeyTooLarge,
    UnsupportedIsolation,
    InvalidStorage,
    UnexpectedKind,
    ValueTooLarge,
}

impl Automerge {
    /// Inspect ROOT's current winning value kind/length without reading or copying its body.
    ///
    /// The same property range, indexed visibility and last-visible winner as unisolated
    /// [`crate::ReadDoc::get`] are used. Unlike `get`, this never gathers conflict Ops, converts
    /// an owned ScalarValue, clones an actor ID, or decodes/UTF8-validates raw scalar bytes.
    /// The query key is limited to 256 UTF8 bytes; explicit limits bound retained operation/key
    /// metadata before lookup. This is a preflight, not a promise of a complete document view.
    ///
    /// Metadata alone is NOT a safe preflight for a later ordinary `get`: get also decodes losing
    /// conflicts and clones an actor ID. Use [`Self::root_bytes`] for a bounded Bytes-field read.
    /// Mutation/replacement requires a new observation. AutoCommit's forwarding never closes a
    /// transaction: committed-only readers must refuse pending operations independently.
    pub fn root_value_metadata(
        &self,
        key: &str,
        limits: RootValueReadLimits,
    ) -> Result<Option<RootValueMetadata>, RootValueReadError> {
        if key.len() > 256 {
            return Err(RootValueReadError::KeyTooLarge);
        }
        self.ops.root_value_metadata(key, limits)
    }

    /// Borrow the current ROOT-map winner's exact Bytes content after work/kind/length checks.
    ///
    /// This shares metadata's exact unisolated winner selection and work limits, then touches only
    /// that winner's raw Bytes slice. Oversized or other-kind values refuse BEFORE raw access. It
    /// never reads losing scalar bodies, reconstructs an Op, or clones actor IDs or value content.
    /// Zero-length Bytes return a present empty slice without touching the raw column.
    /// The immutable borrow prevents a mutation between preflight and byte consumption. No result
    /// here proves a signature, membership, role, durable admission or historical completeness.
    pub fn root_bytes(
        &self,
        key: &str,
        limits: RootValueReadLimits,
        max_bytes: usize,
    ) -> Result<Option<&[u8]>, RootValueReadError> {
        if key.len() > 256 {
            return Err(RootValueReadError::KeyTooLarge);
        }
        self.ops.root_bytes(key, limits, max_bytes)
    }
}
