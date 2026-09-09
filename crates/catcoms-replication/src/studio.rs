//! Typed Studio operations, projections, checkpoints and P1 core edit/ingest consumers.
//!
//! Body decoding alone proves neither authorship nor that an Automerge delta implements the
//! operation. `StudioTarget` combines causal validation and exact seed/recovery preflight with
//! existing P1 admission. The application must still own durable intents, accounted storage,
//! publication and settlement; no actor/native Studio write path is enabled by this core API.
//! Physical document ids, group membership and receipt authority remain P1's responsibility.

use catcoms_crypto::DeviceId;
use catcoms_wire::DocType;
use serde_json::{json, Map, Value};

use crate::epoch::MAX_DOMAIN_OP_BYTES;
use crate::registry::hex;
use crate::{DomainOp, LogicalDocument, ReplError};

mod patch;
pub use patch::StudioPatch;
mod index;
pub use index::{
    studio_index_document, validate_index_change, IndexCreation, IndexEntry, IndexRegister,
    IndexSource, IndexValue, StudioIndexProjection, MAX_INDEX_OBJECTS,
};
mod frames;
mod recovery;
mod snapshot;
pub use recovery::{StudioProjection, StudioRecovery};
mod admission;
pub use admission::StudioTarget;
mod epoch;
pub use epoch::catchup;
pub use epoch::{StudioEpoch, MAX_STUDIO_EPOCH_SNAPSHOT_BYTES};
mod references;
pub use frames::{
    flipnote_document, validate_frame_change, FlipnoteFrameProjection, FrameBlob, FrameEntry,
    FrameInsertion, FrameLimits, FrameRegister, FrameSource, FrameValue, FLIPNOTE_FRAME_BYTES,
    FLIPNOTE_MAX_FRAMES,
};
pub use references::operation_blob_cid;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;

/// Stable collection identity, not a list position or an Automerge object id.
pub type ElementId = [u8; 16];
/// Content address or jam patch hash (different meanings despite equal wire lengths).
pub type ContentId = [u8; 32];
/// Encoded frame cap, mirrored by the PIX codec and desktop studio contract.
pub const MAX_FRAME_BYTES: u64 = 64 * 1024;
/// Encoded complete animation cap; aggregate validation belongs to the exporter/materializer.
pub const MAX_EXPORT_BYTES: u64 = 9 * 1024 * 1024;
/// Studio JSON numbers cross the desktop bridge without loss. This is not a clock reading.
pub const MAX_STUDIO_INTEGER: u64 = (1u64 << 53) - 1;

/// Mirrors the app's FileExpiry without importing the app crate into replication. An absent
/// field is unrecorded, null is never, and an integer is an absolute ms-epoch timestamp. Zero
/// is an actual timestamp, never a sentinel. No expiry enforcement or ambient clock is added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioExpiry {
    Unrecorded,
    Never,
    At(u64),
}

impl StudioExpiry {
    fn read(obj: &Map<String, Value>) -> Result<Self, ReplError> {
        match obj.get("expiry") {
            None => Ok(Self::Unrecorded),
            Some(Value::Null) => Ok(Self::Never),
            Some(value) => {
                let ts = value.as_u64().ok_or(ReplError::Malformed)?;
                integer_bound(ts)?;
                Ok(Self::At(ts))
            }
        }
    }

    fn write(self, mut value: Value) -> Result<Value, ReplError> {
        let obj = value.as_object_mut().ok_or(ReplError::Malformed)?;
        match self {
            Self::Unrecorded => {}
            Self::Never => {
                obj.insert("expiry".into(), Value::Null);
            }
            Self::At(ts) => {
                integer_bound(ts)?;
                obj.insert("expiry".into(), ts.into());
            }
        }
        Ok(value)
    }
}

/// Index entries distinguish animations from their linked score objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioKind {
    Flipnote,
    Score,
}

impl StudioKind {
    fn name(self) -> &'static str {
        match self {
            Self::Flipnote => "flipnote",
            Self::Score => "score",
        }
    }
}

/// The complete `IndexOp` body from `studio-contract.ts`. Constructing a value is not validation;
/// `encode`/`decode` check static ranges, and `decode_domain` also checks scope and creator binding.
#[derive(Clone, PartialEq, Eq)]
pub enum IndexOp {
    PutObject {
        object: ElementId,
        kind: StudioKind,
        title: String,
        created_by: DeviceId,
        ts: u64,
        expiry: StudioExpiry,
    },
    TombstoneObject {
        object: ElementId,
    },
    SetTitle {
        object: ElementId,
        title: String,
    },
    SetExpiry {
        object: ElementId,
        expiry: StudioExpiry,
    },
}

impl std::fmt::Debug for IndexOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Titles and identities are private document content, not diagnostic context.
        f.debug_struct("IndexOp").finish_non_exhaustive()
    }
}

impl IndexOp {
    /// Encode the exact sorted-key JSON expected by the frontend, bounded before serialization.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        let value = match self {
            Self::PutObject {
                object,
                kind,
                title,
                created_by,
                ts,
                expiry,
            } => {
                title_bound(title)?;
                integer_bound(*ts)?;
                expiry.write(
                    json!({"op":"put_object", "object":hex(object), "kind":kind.name(),
                    "title":title, "created_by":hex(created_by.as_bytes()), "ts":ts}),
                )?
            }
            Self::TombstoneObject { object } => {
                json!({"op":"tombstone_object", "object":hex(object)})
            }
            Self::SetTitle { object, title } => {
                title_bound(title)?;
                json!({"op":"set_title", "object":hex(object), "title":title})
            }
            Self::SetExpiry { object, expiry } => {
                expiry.write(json!({"op":"set_expiry", "object":hex(object)}))?
            }
        };
        canonical(&value)
    }

    /// Reject unknown/duplicate fields, whitespace, noncanonical strings/numbers and bad ids.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        let value = parse(bytes)?;
        let obj = object(&value)?;
        let id = fixed_hex(text(obj, "object")?)?;
        let op = match text(obj, "op")? {
            "put_object" => Self::PutObject {
                object: id,
                kind: match text(obj, "kind")? {
                    "flipnote" => StudioKind::Flipnote,
                    "score" => StudioKind::Score,
                    _ => return Err(ReplError::Malformed),
                },
                title: text(obj, "title")?.into(),
                created_by: DeviceId::from_bytes(fixed_hex(text(obj, "created_by")?)?),
                ts: unsigned(obj, "ts")?,
                expiry: StudioExpiry::read(obj)?,
            },
            "tombstone_object" => Self::TombstoneObject { object: id },
            "set_title" => Self::SetTitle {
                object: id,
                title: text(obj, "title")?.into(),
            },
            "set_expiry" => Self::SetExpiry {
                object: id,
                expiry: StudioExpiry::read(obj)?,
            },
            _ => return Err(ReplError::Malformed),
        };
        exact(bytes, op.encode()?)?;
        Ok(op)
    }

    /// Validate a complete P1 envelope against a caller's trusted logical target and verified
    /// outer author. The supplied identity must come from signed-op admission, not UI/profile data.
    /// A creator claim is checked in full; any member may rename/tombstone under the shared model.
    pub fn decode_domain(
        document: &LogicalDocument,
        domain: &DomainOp,
        verified_author: &DeviceId,
    ) -> Result<Self, ReplError> {
        scope(document, domain, DocType::StudioIndex)?;
        let op = Self::decode(&domain.body)?;
        if let Self::PutObject { created_by, .. } = &op {
            if created_by != verified_author {
                return Err(ReplError::EpochAuthority);
            }
        }
        Ok(op)
    }
}

/// Header values cannot be interchanged (for example a number as a title or a string as fps).
#[derive(Clone, PartialEq, Eq)]
pub enum FlipnoteHeader {
    Title(String),
    Fps(u8),
    /// None explicitly unlinks the score. Existence/type/call checks require the materializer.
    Score(Option<ElementId>),
}

impl std::fmt::Debug for FlipnoteHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlipnoteHeader").finish_non_exhaustive()
    }
}

/// The complete Flipnote operation grammar, not an assertion that every operation is enabled.
/// Sound/score operations stay unavailable to live editing until their stateful validators land.
#[derive(Clone, PartialEq, Eq)]
pub enum FlipnoteOp {
    InsertFrame {
        frame: ElementId,
        after: Option<ElementId>,
        cid: ContentId,
        bytes: u64,
    },
    RemoveFrame {
        frame: ElementId,
    },
    ReplaceFrame {
        frame: ElementId,
        cid: ContentId,
        bytes: u64,
    },
    SetSfx {
        sfx: ElementId,
        frame: ElementId,
        patch: ContentId,
        note: u8,
    },
    RemoveSfx {
        sfx: ElementId,
    },
    SetPatch {
        patch: ContentId,
        descriptor: StudioPatch,
    },
    RemovePatch {
        patch: ContentId,
    },
    SetExport {
        export: ElementId,
        cid: ContentId,
        bytes: u64,
        expiry: StudioExpiry,
    },
    RemoveExport {
        export: ElementId,
    },
    SetHeader(FlipnoteHeader),
}

impl std::fmt::Debug for FlipnoteOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlipnoteOp").finish_non_exhaustive()
    }
}

impl FlipnoteOp {
    /// Canonical body only. The complete envelope has its own (smaller available body) budget.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        let value = match self {
            Self::InsertFrame {
                frame,
                after,
                cid,
                bytes,
            } => {
                blob_size(*bytes, MAX_FRAME_BYTES)?;
                if after.as_ref() == Some(frame) {
                    return Err(ReplError::Malformed);
                }
                json!({"op":"insert_frame", "frame":hex(frame), "after":after.as_ref().map(|id| hex(id)), "cid":hex(cid), "bytes":bytes})
            }
            Self::RemoveFrame { frame } => json!({"op":"remove_frame", "frame":hex(frame)}),
            Self::ReplaceFrame { frame, cid, bytes } => {
                blob_size(*bytes, MAX_FRAME_BYTES)?;
                json!({"op":"replace_frame", "frame":hex(frame), "cid":hex(cid), "bytes":bytes})
            }
            Self::SetSfx {
                sfx,
                frame,
                patch,
                note,
            } => {
                if *note > 127 {
                    return Err(ReplError::Malformed);
                }
                json!({"op":"set_sfx", "sfx":hex(sfx), "frame":hex(frame), "patch":hex(patch), "note":note})
            }
            Self::RemoveSfx { sfx } => json!({"op":"remove_sfx", "sfx":hex(sfx)}),
            Self::SetPatch { patch, descriptor } => {
                if *patch != descriptor.id() {
                    return Err(ReplError::Malformed);
                }
                json!({"op":"set_patch", "patch":hex(patch), "descriptor":descriptor.value()})
            }
            Self::RemovePatch { patch } => json!({"op":"remove_patch", "patch":hex(patch)}),
            Self::SetExport {
                export,
                cid,
                bytes,
                expiry,
            } => {
                blob_size(*bytes, MAX_EXPORT_BYTES)?;
                expiry.write(
                    json!({"op":"set_export", "export":hex(export), "cid":hex(cid), "bytes":bytes}),
                )?
            }
            Self::RemoveExport { export } => json!({"op":"remove_export", "export":hex(export)}),
            Self::SetHeader(FlipnoteHeader::Title(title)) => {
                title_bound(title)?;
                json!({"op":"set_header", "field":"title", "value":title})
            }
            Self::SetHeader(FlipnoteHeader::Fps(fps)) => {
                if !(1..=24).contains(fps) {
                    return Err(ReplError::Malformed);
                }
                json!({"op":"set_header", "field":"fps", "value":fps})
            }
            Self::SetHeader(FlipnoteHeader::Score(score)) => {
                json!({"op":"set_header", "field":"score", "value":score.as_ref().map(|id| hex(id))})
            }
        };
        canonical(&value)
    }

    /// Strict bounded parser. Blob lengths remain declarations; fetch must check the exact
    /// actual length, CID, PIX dimensions/palette (or export format) before consuming bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        let value = parse(bytes)?;
        let obj = object(&value)?;
        let op = match text(obj, "op")? {
            "insert_frame" => Self::InsertFrame {
                frame: fixed_hex(text(obj, "frame")?)?,
                after: optional_id(field(obj, "after")?)?,
                cid: fixed_hex(text(obj, "cid")?)?,
                bytes: unsigned(obj, "bytes")?,
            },
            "remove_frame" => Self::RemoveFrame {
                frame: fixed_hex(text(obj, "frame")?)?,
            },
            "replace_frame" => Self::ReplaceFrame {
                frame: fixed_hex(text(obj, "frame")?)?,
                cid: fixed_hex(text(obj, "cid")?)?,
                bytes: unsigned(obj, "bytes")?,
            },
            "set_sfx" => Self::SetSfx {
                sfx: fixed_hex(text(obj, "sfx")?)?,
                frame: fixed_hex(text(obj, "frame")?)?,
                patch: fixed_hex(text(obj, "patch")?)?,
                note: u8::try_from(unsigned(obj, "note")?).map_err(|_| ReplError::Malformed)?,
            },
            "remove_sfx" => Self::RemoveSfx {
                sfx: fixed_hex(text(obj, "sfx")?)?,
            },
            "set_patch" => Self::SetPatch {
                patch: fixed_hex(text(obj, "patch")?)?,
                descriptor: StudioPatch::new(field(obj, "descriptor")?)?,
            },
            "remove_patch" => Self::RemovePatch {
                patch: fixed_hex(text(obj, "patch")?)?,
            },
            "set_export" => Self::SetExport {
                export: fixed_hex(text(obj, "export")?)?,
                cid: fixed_hex(text(obj, "cid")?)?,
                bytes: unsigned(obj, "bytes")?,
                expiry: StudioExpiry::read(obj)?,
            },
            "remove_export" => Self::RemoveExport {
                export: fixed_hex(text(obj, "export")?)?,
            },
            "set_header" => Self::SetHeader(match text(obj, "field")? {
                "title" => FlipnoteHeader::Title(text(obj, "value")?.into()),
                "fps" => FlipnoteHeader::Fps(
                    u8::try_from(unsigned(obj, "value")?).map_err(|_| ReplError::Malformed)?,
                ),
                "score" => FlipnoteHeader::Score(optional_id(field(obj, "value")?)?),
                _ => return Err(ReplError::Malformed),
            }),
            _ => return Err(ReplError::Malformed),
        };
        exact(bytes, op.encode()?)?;
        Ok(op)
    }

    /// Validate the complete envelope for a caller-selected Flipnote document. StudioObject
    /// also contains scores, so the caller must select this codec from its verified root kind.
    /// No author is accepted from the body: frame/export authors come from the outer signature.
    pub fn decode_domain(document: &LogicalDocument, domain: &DomainOp) -> Result<Self, ReplError> {
        scope(document, domain, DocType::StudioObject)?;
        Self::decode(&domain.body)
    }
}

fn scope(document: &LogicalDocument, domain: &DomainOp, kind: DocType) -> Result<(), ReplError> {
    LogicalDocument::new(
        document.server_id.clone(),
        document.doc_type,
        document.logical_key.clone(),
    )?;
    if document.doc_type != kind
        || domain.doc_type != kind
        || domain.logical_key != document.logical_key
    {
        return Err(ReplError::EpochScope);
    }
    // Check before encode clones body bytes; the latter charges framing, nonce and logical key.
    if domain.body.len() > MAX_DOMAIN_OP_BYTES {
        return Err(ReplError::EpochBound);
    }
    domain.encode()?;
    Ok(())
}

fn blob_size(bytes: u64, max: u64) -> Result<(), ReplError> {
    if bytes == 0 || bytes > max {
        return Err(ReplError::EpochBound);
    }
    Ok(())
}

fn integer_bound(value: u64) -> Result<(), ReplError> {
    if value > MAX_STUDIO_INTEGER {
        return Err(ReplError::EpochBound);
    }
    Ok(())
}

fn title_bound(value: &str) -> Result<(), ReplError> {
    // No new product-specific title limit: the existing envelope budget bounds UTF-8 bytes and
    // serialization expansion. A future smaller UI limit must not silently change this schema.
    if value.len() > MAX_DOMAIN_OP_BYTES {
        return Err(ReplError::EpochBound);
    }
    Ok(())
}

fn parse(bytes: &[u8]) -> Result<Value, ReplError> {
    if bytes.len() > MAX_DOMAIN_OP_BYTES {
        return Err(ReplError::EpochBound);
    }
    serde_json::from_slice(bytes).map_err(|_| ReplError::Malformed)
}

fn canonical(value: &Value) -> Result<Vec<u8>, ReplError> {
    // All permitted object KEYS are fixed ASCII, including patch keys, so Rust key order equals
    // the frontend's UTF-16 order. User strings occur only as VALUES. Explicit sorting also
    // protects the format if another workspace dependency enables serde_json/preserve_order.
    fn sort(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.sort_keys();
                for value in map.values_mut() {
                    sort(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    sort(value);
                }
            }
            _ => {}
        }
    }
    let mut value = value.clone();
    sort(&mut value);
    let bytes = serde_json::to_vec(&value).map_err(|_| ReplError::Malformed)?;
    if bytes.len() > MAX_DOMAIN_OP_BYTES {
        return Err(ReplError::EpochBound);
    }
    Ok(bytes)
}

fn exact(bytes: &[u8], encoded: Vec<u8>) -> Result<(), ReplError> {
    if bytes != encoded {
        return Err(ReplError::Malformed);
    }
    Ok(())
}

fn object(value: &Value) -> Result<&Map<String, Value>, ReplError> {
    value.as_object().ok_or(ReplError::Malformed)
}

fn field<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a Value, ReplError> {
    obj.get(key).ok_or(ReplError::Malformed)
}

fn text<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a str, ReplError> {
    field(obj, key)?.as_str().ok_or(ReplError::Malformed)
}

fn unsigned(obj: &Map<String, Value>, key: &str) -> Result<u64, ReplError> {
    field(obj, key)?.as_u64().ok_or(ReplError::Malformed)
}

fn fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], ReplError> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ReplError::Malformed);
    }
    let mut id = [0u8; N];
    for (byte, pair) in id.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        // The ASCII/length check above proves both nibble slices exist and are UTF-8.
        *byte = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| ReplError::Malformed)?,
            16,
        )
        .map_err(|_| ReplError::Malformed)?;
    }
    Ok(id)
}

fn optional_id(value: &Value) -> Result<Option<ElementId>, ReplError> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(fixed_hex(
        value.as_str().ok_or(ReplError::Malformed)?,
    )?))
}
