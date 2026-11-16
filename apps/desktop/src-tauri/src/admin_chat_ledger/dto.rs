//! Closed privileged bridge DTOs. Never serialize an Automerge ExpandedChange directly: its
//! 64-bit operation counters and scalar values would silently round in JavaScript.
use super::*;
use automerge::legacy::{ElementId, Key, OpType};
use catcoms_replication::history_inspection::LegacyChangeInspection;
use catcoms_replication::op::InspectionSignature;
use serde::Serialize;

pub(super) const MAX_REPLY_BYTES: usize = 1024 * 1024;

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub(super) enum Reply {
    Page {
        availability: &'static str,
        revision: String,
        coverage: Coverage,
        rows: Vec<LegacyRow>,
        #[serde(rename = "knownRowCount")]
        known_row_count: u32,
        #[serde(rename = "nextCursor")]
        next_cursor: Option<String>,
    },
    Detail {
        availability: &'static str,
        revision: String,
        // One fixed-size row stays off every pending Reply's inline frame. Serde
        // serializes this Box transparently, preserving the existing IPC object.
        row: Box<LegacyRow>,
        original: Original,
        #[serde(rename = "typedBody")]
        typed_body: Option<()>,
        #[serde(rename = "legacyChange")]
        legacy_change: Option<Decoded>,
        #[serde(rename = "signatureHex")]
        signature_hex: Option<String>,
        #[serde(rename = "factIds")]
        fact_ids: Vec<String>,
        #[serde(rename = "missingFactIds")]
        missing_fact_ids: Vec<String>,
        #[serde(rename = "evidenceTruncated")]
        evidence_truncated: bool,
    },
    Stale {
        revision: String,
    },
    Unavailable {
        code: &'static str,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Coverage {
    source: &'static str,
    typed: &'static str,
    legacy: &'static str,
    owner_evidence: &'static str,
    unattributed_changes: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyRow {
    kind: &'static str,
    row_id: String,
    op_hash: String,
    namespace_group_id: String,
    channel: String,
    author_public_key: Option<String>,
    author_id: Option<String>,
    author_fingerprint: Option<String>,
    claimed_key_byte_count: u32,
    key_shape: &'static str,
    signed_doc_type: u16,
    signed_doc_id: String,
    scope: &'static str,
    signature: &'static str,
    group_binding: &'static str,
    authority: &'static str,
    change_hash: Option<String>,
    graph: &'static str,
    presented_ms: Option<i64>,
    preview: Option<()>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Original {
    encoding: &'static str,
    bytes: String,
    offset: u64,
    total_bytes: u64,
    next_offset: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Decoded {
    status: &'static str,
    actor: Option<String>,
    sequence: Option<String>,
    start_op: Option<String>,
    parents: Vec<String>,
    mutations: Vec<Mutation>,
    mutations_truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Mutation {
    operation_id: String,
    object_id: String,
    key: MutationKey,
    operation: &'static str,
    insert: bool,
    value: Option<Scalar>,
    predecessors: Vec<String>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum MutationKey {
    Map {
        value: String,
    },
    Sequence {
        #[serde(rename = "elementId")]
        element_id: Option<String>,
    },
}

#[derive(Serialize)]
struct Scalar {
    kind: &'static str,
    text: String,
}

fn count(value: usize) -> Result<u32, AdminLedgerError> {
    u32::try_from(value).map_err(|_| AdminLedgerError::Capacity)
}

// Combined signed frames can exceed a field's u32 limit. IPC offsets remain exact JavaScript
// integers, while the native host continues copying only one independently bounded raw chunk.
fn safe_offset(value: usize) -> Result<u64, AdminLedgerError> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value <= MAX_SAFE_INTEGER)
        .ok_or(AdminLedgerError::Capacity)
}

fn row(source: AdminLegacyRow) -> Result<LegacyRow, AdminLedgerError> {
    if source.namespace_group.is_empty() || source.namespace_group.len() > 256 {
        return Err(AdminLedgerError::EvidenceUnavailable);
    }
    let original = source.original;
    let author_id = original.claimed_device.map(|device| device.to_string());
    Ok(LegacyRow {
        kind: "legacy-change",
        row_id: source.row_id,
        op_hash: hex::encode(original.op_hash),
        namespace_group_id: hex::encode(source.namespace_group),
        channel: source.channel.to_string(),
        author_public_key: original.claimed_public_key.map(hex::encode),
        author_fingerprint: author_id.as_ref().map(|id| id[..8].to_owned()),
        author_id,
        claimed_key_byte_count: count(original.claimed_key_bytes)?,
        key_shape: if original.claimed_key_bytes == 32 {
            "ed25519"
        } else {
            "invalid-length"
        },
        signed_doc_type: original.signed_doc_type as u16,
        signed_doc_id: original.signed_doc_id.to_string(),
        scope: if original.scope_matches {
            "matching"
        } else {
            "mismatched"
        },
        signature: match original.signature {
            InspectionSignature::Valid => "valid",
            InspectionSignature::Invalid => "invalid",
            InspectionSignature::NotChecked(_) => "not-checked",
        },
        group_binding: "not-signed",
        authority: "legacy-unproven",
        change_hash: None,
        graph: "unavailable",
        presented_ms: None,
        preview: None,
    })
}

pub(super) fn page(source: AdminLedgerPage) -> Result<Reply, AdminLedgerError> {
    if source.rows.len() > 32 {
        return Err(AdminLedgerError::Capacity);
    }
    Ok(Reply::Page {
        availability: "incomplete",
        revision: source.revision,
        coverage: Coverage {
            source: "retained-local-inventory",
            typed: "not-active",
            legacy: "incomplete",
            owner_evidence: "not-active",
            unattributed_changes: "unknown",
        },
        rows: source.rows.into_iter().map(row).collect::<Result<_, _>>()?,
        known_row_count: count(source.known_row_count)?,
        next_cursor: source.next_cursor,
    })
}

fn unavailable_decode(status: &'static str) -> Decoded {
    Decoded {
        status,
        actor: None,
        sequence: None,
        start_op: None,
        parents: Vec::new(),
        mutations: Vec::new(),
        mutations_truncated: false,
    }
}

fn scalar(value: &automerge::ScalarValue) -> Scalar {
    use automerge::ScalarValue::*;
    let (kind, text) = match value {
        Str(value) => ("string", value.to_string()),
        Int(value) => ("int", value.to_string()),
        Uint(value) => ("uint", value.to_string()),
        F64(value) => ("float", value.to_string()),
        Boolean(value) => ("bool", value.to_string()),
        Null => ("null", "null".into()),
        Bytes(value) => ("bytes", hex::encode(value)),
        Counter(value) => ("counter", value.to_string()),
        Timestamp(value) => ("timestamp", value.to_string()),
        Unknown { type_code, bytes } => ("unknown", format!("{type_code}:{}", hex::encode(bytes))),
    };
    Scalar { kind, text }
}

fn projection_text_fits(decoded: &Decoded) -> bool {
    let mut remaining = 64 * 1024usize;
    let mut charge = |text: &str| match remaining.checked_sub(text.len()) {
        Some(next) => {
            remaining = next;
            true
        }
        None => false,
    };
    for text in decoded
        .actor
        .iter()
        .chain(decoded.sequence.iter())
        .chain(decoded.start_op.iter())
        .chain(&decoded.parents)
    {
        if !charge(text) {
            return false;
        }
    }
    for mutation in &decoded.mutations {
        if !charge(&mutation.operation_id)
            || !charge(&mutation.object_id)
            || !charge(mutation.operation)
        {
            return false;
        }
        let key = match &mutation.key {
            MutationKey::Map { value } => Some(value),
            MutationKey::Sequence { element_id } => element_id.as_ref(),
        };
        if key.is_some_and(|key| !charge(key)) {
            return false;
        }
        if let Some(value) = &mutation.value {
            if !charge(value.kind) || !charge(&value.text) {
                return false;
            }
        }
        if mutation.predecessors.iter().any(|id| !charge(id)) {
            return false;
        }
    }
    true
}

fn decode(source: LegacyChangeInspection) -> (Decoded, bool) {
    let change = match source {
        LegacyChangeInspection::Unsupported => return (unavailable_decode("unsupported"), false),
        LegacyChangeInspection::TooLarge => return (unavailable_decode("too-large"), false),
        LegacyChangeInspection::Invalid => return (unavailable_decode("invalid"), false),
        LegacyChangeInspection::Decoded(change) => change,
    };
    // Historical malformed identifiers must not make the frontend discard an otherwise readable
    // original. The closed decoded grammar accepts only positive counters and supported actor
    // widths; refusing this optional projection does not reject or rewrite the retained frame.
    let actor_supported = |actor: &automerge::ActorId| matches!(actor.to_bytes().len(), 16 | 32);
    let op_id_supported =
        |id: &automerge::legacy::OpId| id.counter() != 0 && actor_supported(id.actor());
    if change.seq == 0
        || !actor_supported(&change.actor_id)
        || change.operations.iter().any(|op| {
            matches!(&op.obj, automerge::legacy::ObjectId::Id(id) if !op_id_supported(id))
                || matches!(&op.key, Key::Seq(ElementId::Id(id)) if !op_id_supported(id))
                || op.pred.iter().any(|id| !op_id_supported(id))
        })
    {
        return (unavailable_decode("unsupported"), false);
    }
    // The agreed DTO has no mark name/expand fields. Preserve exact originals and refuse this
    // projection rather than call a lossy mark/unmark interpretation a complete decoded change.
    if change
        .operations
        .iter()
        .any(|op| matches!(op.action, OpType::MarkBegin(_) | OpType::MarkEnd(_)))
    {
        return (unavailable_decode("unsupported"), false);
    }
    let mut truncated = change.deps.len() > 128 || change.operations.len() > 512;
    let mut mutations = Vec::with_capacity(change.operations.len().min(512));
    for (index, op) in change.operations.into_iter().take(512).enumerate() {
        let Some(counter) = change.start_op.get().checked_add(index as u64) else {
            return (unavailable_decode("invalid"), false);
        };
        let operation_id = format!("{counter}@{}", change.actor_id);
        let (operation, value) = match &op.action {
            OpType::Make(kind) => (
                match kind {
                    automerge::ObjType::Map => "make-map",
                    automerge::ObjType::List => "make-list",
                    automerge::ObjType::Text => "make-text",
                    automerge::ObjType::Table => "make-table",
                },
                None,
            ),
            OpType::Delete => ("delete", None),
            OpType::Increment(value) => (
                "increment",
                Some(Scalar {
                    kind: "int",
                    text: value.to_string(),
                }),
            ),
            OpType::Put(value) => ("set", Some(scalar(value))),
            OpType::MarkBegin(_) | OpType::MarkEnd(_) => unreachable!("marks refused above"),
        };
        let key = match op.key {
            Key::Map(value) => MutationKey::Map {
                value: value.to_string(),
            },
            Key::Seq(ElementId::Head) => MutationKey::Sequence { element_id: None },
            Key::Seq(ElementId::Id(value)) => MutationKey::Sequence {
                element_id: Some(value.to_string()),
            },
        };
        truncated |= op.pred.len() > 128;
        mutations.push(Mutation {
            operation_id,
            object_id: op.obj.to_string(),
            key,
            operation,
            insert: op.insert,
            value,
            predecessors: op.pred.iter().take(128).map(ToString::to_string).collect(),
        });
    }
    let decoded = Decoded {
        status: "decoded",
        actor: Some(hex::encode(change.actor_id.to_bytes())),
        sequence: Some(change.seq.to_string()),
        start_op: Some(change.start_op.to_string()),
        parents: change
            .deps
            .iter()
            .take(128)
            .map(ToString::to_string)
            .collect(),
        mutations,
        mutations_truncated: truncated,
    };
    // Native textual IDs/hex can expand beyond the parser's retained-byte allowance. Refuse only
    // this projection, before IPC serialization; the original signed frame remains chunk-readable.
    if !projection_text_fits(&decoded) {
        return (unavailable_decode("too-large"), true);
    }
    (decoded, truncated)
}

pub(super) fn detail(source: AdminLedgerDetail) -> Result<Reply, AdminLedgerError> {
    if source.frame_bytes.len() > 64 * 1024 {
        return Err(AdminLedgerError::Capacity);
    }
    let total = safe_offset(source.row.original.original_bytes)?;
    let (decoded, evidence_truncated) = decode(source.decoded);
    Ok(Reply::Detail {
        availability: "incomplete",
        revision: source.revision,
        row: Box::new(row(source.row)?),
        original: Original {
            encoding: "hex",
            bytes: hex::encode(source.frame_bytes),
            offset: safe_offset(source.frame_offset)?,
            total_bytes: total,
            next_offset: source.next_offset.map(safe_offset).transpose()?,
        },
        typed_body: None,
        legacy_change: Some(decoded),
        signature_hex: Some(hex::encode(source.signature)),
        fact_ids: Vec::new(),
        missing_fact_ids: Vec::new(),
        evidence_truncated,
    })
}

/// This writer refuses before growing past the encoded reply ceiling. Its explicit capped growth
/// is amortized: a string of escaped controls must not cause one exact reallocation per scalar.
struct CappedWriter(Vec<u8>);
impl std::io::Write for CappedWriter {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        let next = self
            .0
            .len()
            .checked_add(input.len())
            .ok_or_else(|| std::io::Error::other("reply limit"))?;
        if next > MAX_REPLY_BYTES {
            return Err(std::io::Error::other("reply limit"));
        }
        if next > self.0.capacity() {
            let capacity = next
                .max(self.0.capacity().saturating_mul(2).max(1024))
                .min(MAX_REPLY_BYTES);
            self.0
                .try_reserve_exact(capacity - self.0.len())
                .map_err(std::io::Error::other)?;
        }
        self.0.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode(mut reply: Reply) -> String {
    fn attempt(reply: &Reply) -> Option<String> {
        let mut writer = CappedWriter(Vec::new());
        serde_json::to_writer(&mut writer, reply).ok()?;
        String::from_utf8(writer.0).ok()
    }
    if let Some(encoded) = attempt(&reply) {
        return encoded;
    }
    // A large escaped decoded projection must not prevent exact raw-frame chunk retrieval.
    if let Reply::Detail {
        legacy_change,
        evidence_truncated,
        ..
    } = &mut reply
    {
        *legacy_change = Some(unavailable_decode("too-large"));
        *evidence_truncated = true;
        if let Some(encoded) = attempt(&reply) {
            return encoded;
        }
    }
    r#"{"status":"unavailable","code":"evidence-unavailable"}"#.to_owned()
}

#[cfg(test)]
mod tests;
