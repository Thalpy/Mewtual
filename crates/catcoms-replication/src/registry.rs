//! P1's first typed checkpoint consumer: deterministic, bounded registry buckets.
//!
//! Pointer epochs are discovery hints, never authority. A larger epoch triggers a receipt-head
//! refresh; it never authorizes a seed or causes an unbounded predecessor walk. The seed pins
//! previously admitted slots, so a newly introduced key cannot displace a live checkpoint entry.
//! Physical CRDT keys are flat to avoid concurrent creation of conflicting map objects.

use std::collections::{BTreeMap, BTreeSet};

use automerge::legacy::{Key, ObjectId, OpType};
use automerge::transaction::Transactable;
use automerge::{AutoCommit, Change, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::DeviceId;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::DocType;

use crate::checkpoint::am_error;
use crate::epoch::hash_parts;
use crate::{
    Admission, CheckpointSeed, CloseRecord, ClosureStats, DomainOp, EncryptedDoc, EpochGate,
    LogicalDocument, ReplError, SealedOp, VerifiedCheckpoint, VerifiedReceipt,
};

/// Live pointer slots in one of the server's 256 buckets.
pub const MAX_REGISTRY_POINTERS: usize = 2048;
/// Rotation warning threshold, leaving room for migration before the hard ceiling.
pub const REGISTRY_WARNING_EPOCH: u64 = 3900;
/// At this epoch the bucket is read-only pending migration.
pub const MAX_REGISTRY_EPOCH: u64 = 4096;

/// Type and logical key must travel together: equal key bytes under two types are distinct.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PointerKey {
    doc_type: DocType,
    key: Vec<u8>,
}

impl PointerKey {
    /// Construct a pointer to a creative document (registry buckets discover themselves).
    pub fn new(doc_type: DocType, key: Vec<u8>) -> Result<Self, ReplError> {
        if !matches!(
            doc_type,
            DocType::StudioIndex | DocType::StudioObject | DocType::PostReplies
        ) || key.is_empty()
            || key.len() > 192
        {
            return Err(ReplError::EpochScope);
        }
        Ok(Self { doc_type, key })
    }
    /// Target type, independent of the registry's own DocRegistry tag.
    pub fn doc_type(&self) -> DocType {
        self.doc_type
    }
    /// Target logical key, with no epoch id or author encoded into it.
    pub fn logical_key(&self) -> &[u8] {
        &self.key
    }
    /// Deterministic bucket; callers need no mutable index to discover this address.
    pub fn bucket(&self) -> u8 {
        hash_parts(
            "catcoms-registry-bucket:v1",
            &[&u64::from(self.doc_type.tag()).to_be_bytes(), &self.key],
        )[0]
    }
    fn suffix(&self) -> String {
        format!("{:04x}/{}", self.doc_type.tag(), hex(&self.key))
    }
    fn from_suffix(suffix: &str) -> Result<Self, ReplError> {
        let (tag, key) = suffix.split_once('/').ok_or(ReplError::Malformed)?;
        if tag.len() != 4 {
            return Err(ReplError::Malformed);
        }
        let tag = u16::from_str_radix(tag, 16).map_err(|_| ReplError::Malformed)?;
        let result = Self::new(
            DocType::from_tag(tag).ok_or(ReplError::Malformed)?,
            unhex(key)?,
        )?;
        if result.suffix() != suffix {
            return Err(ReplError::Malformed);
        }
        Ok(result)
    }
}

/// Fixed bucket key: hash the framed tuple rather than exceed LogicalDocument's key cap with
/// a maximal server id. All 256 addresses can be computed before fetching any registry state.
pub fn registry_document(server_id: &[u8], bucket: u8) -> Result<LogicalDocument, ReplError> {
    LogicalDocument::new(
        server_id.to_vec(),
        DocType::DocRegistry,
        hash_parts(
            "catcoms-registry-key:v1",
            &[server_id, &u64::from(bucket).to_be_bytes()],
        )
        .to_vec(),
    )
}

/// Closed set of registry operations. Canonical JSON is checked by exact re-encoding, rejecting
/// duplicate/unknown fields, alternate number spellings, key order and whitespace on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryOp {
    /// Advertise a checkpoint number. Zero denotes a document that has never rotated.
    Put { key: PointerKey, epoch: u64 },
    /// Replicated reclamation decision. It wins over every put of this key in the same epoch.
    Tombstone { key: PointerKey },
}

impl RegistryOp {
    fn key(&self) -> &PointerKey {
        match self {
            Self::Put { key, .. } | Self::Tombstone { key } => key,
        }
    }
    /// Canonical operation body to put in a nonce-bearing DomainOp and durable intent.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        let key = self.key();
        PointerKey::new(key.doc_type, key.key.clone())?;
        let mut body = serde_json::Map::new();
        if let Self::Put { epoch, .. } = self {
            body.insert("epoch".into(), (*epoch).into());
        }
        body.insert("key".into(), hex(&key.key).into());
        body.insert(
            "t".into(),
            match self {
                Self::Put { .. } => "put_pointer",
                Self::Tombstone { .. } => "tombstone_pointer",
            }
            .into(),
        );
        body.insert("type".into(), key.doc_type.tag().into());
        serde_json::to_vec(&body).map_err(|_| ReplError::Malformed)
    }
    /// Bounded decode of the exact canonical body.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > 1024 {
            return Err(ReplError::EpochBound);
        }
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ReplError::Malformed)?;
        let obj = value.as_object().ok_or(ReplError::Malformed)?;
        let tag = obj
            .get("type")
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok())
            .ok_or(ReplError::Malformed)?;
        let key = PointerKey::new(
            DocType::from_tag(tag).ok_or(ReplError::Malformed)?,
            unhex(
                obj.get("key")
                    .and_then(|v| v.as_str())
                    .ok_or(ReplError::Malformed)?,
            )?,
        )?;
        let result = match obj.get("t").and_then(|v| v.as_str()) {
            Some("put_pointer") => Self::Put {
                key,
                epoch: obj
                    .get("epoch")
                    .and_then(|v| v.as_u64())
                    .ok_or(ReplError::Malformed)?,
            },
            Some("tombstone_pointer") => Self::Tombstone { key },
            _ => return Err(ReplError::Malformed),
        };
        if result.encode()? != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(result)
    }
    /// Bind this body to its deterministic bucket and supplied RNG-generated nonce.
    pub fn domain_op(&self, server: &[u8], nonce: [u8; 16]) -> Result<DomainOp, ReplError> {
        let document = registry_document(server, self.key().bucket())?;
        Ok(DomainOp {
            nonce,
            doc_type: DocType::DocRegistry,
            logical_key: document.logical_key,
            body: self.encode()?,
        })
    }
    fn entry(&self) -> (String, ScalarValue) {
        match self {
            Self::Put { key, epoch } => (format!("p/{}", key.suffix()), ScalarValue::Uint(*epoch)),
            Self::Tombstone { key } => (format!("d/{}", key.suffix()), ScalarValue::Boolean(true)),
        }
    }
}

/// A bounded visible projection plus overflow evidence for settlement's recovery snapshot.
/// Overflow must not silently disappear when a caller installs the next checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryProjection {
    document: LogicalDocument,
    bucket: u8,
    /// Checkpoint epoch of this registry, distinct from target pointer epochs.
    pub epoch: u64,
    /// Stable admitted pointers, in canonical type/key order.
    pub pointers: BTreeMap<PointerKey, u64>,
    /// Live keys that did not fit. These are retained in the open history and recovery.
    pub overflow: BTreeMap<PointerKey, u64>,
    /// Deletion evidence for recovery/Restore; never copied into a seed.
    pub tombstones: BTreeSet<PointerKey>,
}

impl RegistryProjection {
    /// Materialize the complete current CRDT. Tombstones are clock-independent; seeded slots
    /// survive every newly added key. Concurrent pointer numbers take their maximum because
    /// they are only discovery hints; authoritative history always comes from a receipt head.
    pub fn read(
        document: &LogicalDocument,
        bucket: u8,
        epoch: u64,
        doc: &AutoCommit,
    ) -> Result<Self, ReplError> {
        if *document != registry_document(&document.server_id, bucket)?
            || epoch > MAX_REGISTRY_EPOCH
        {
            return Err(ReplError::EpochScope);
        }
        let headers = header(document, bucket, epoch);
        let mut entries = BTreeMap::new();
        let mut slots = BTreeSet::new();
        let mut tombstones = BTreeSet::new();
        let mut header_count = 0;
        for key in doc.keys(ROOT) {
            let values = doc.get_all(ROOT, &key).map_err(am_error)?;
            if let Some(expected) = headers.get(&key) {
                for (value, _) in &values {
                    if !scalar_eq(value, expected) {
                        return Err(ReplError::EpochScope);
                    }
                }
                header_count += 1;
                continue;
            }
            if let Some(id) = key.strip_prefix("_p1/op/") {
                if unhex(id)?.len() != 32
                    || values
                        .iter()
                        .any(|(v, _)| !scalar_eq(v, &ScalarValue::Uint(1)))
                {
                    return Err(ReplError::Malformed);
                }
                continue;
            }
            let (kind, suffix) = key.split_once('/').ok_or(ReplError::Malformed)?;
            let pointer = PointerKey::from_suffix(suffix)?;
            if pointer.bucket() != bucket {
                return Err(ReplError::EpochScope);
            }
            match kind {
                "p" => {
                    let mut max = 0;
                    for (value, _) in values {
                        let Value::Scalar(value) = value else {
                            return Err(ReplError::Malformed);
                        };
                        let ScalarValue::Uint(epoch) = value.as_ref() else {
                            return Err(ReplError::Malformed);
                        };
                        max = max.max(*epoch);
                    }
                    entries.insert(pointer, max);
                }
                "d" | "s" => {
                    if values
                        .iter()
                        .any(|(v, _)| !scalar_eq(v, &ScalarValue::Boolean(true)))
                    {
                        return Err(ReplError::Malformed);
                    }
                    if kind == "d" {
                        tombstones.insert(pointer);
                    } else {
                        slots.insert(pointer);
                    }
                }
                _ => return Err(ReplError::Malformed),
            }
        }
        if (header_count != headers.len()
            && (!entries.is_empty() || !tombstones.is_empty() || epoch != 0))
            || slots.len() > MAX_REGISTRY_POINTERS
            || slots.iter().any(|key| !entries.contains_key(key))
        {
            return Err(ReplError::Malformed);
        }
        entries.retain(|key, _| !tombstones.contains(key));
        let mut pointers = BTreeMap::new();
        for key in slots {
            if let Some(value) = entries.remove(&key) {
                pointers.insert(key, value);
            }
        }
        let free = MAX_REGISTRY_POINTERS - pointers.len();
        for key in entries.keys().take(free).cloned().collect::<Vec<_>>() {
            pointers.insert(
                key.clone(),
                entries.remove(&key).expect("key came from map"),
            );
        }
        Ok(Self {
            document: document.clone(),
            bucket,
            epoch,
            pointers,
            overflow: entries,
            tombstones,
        })
    }

    /// Encode a successor from live admitted pointers only. The flat keys and all Automerge
    /// change metadata have a fixed order. Markers, tombstones and overflow remain in recovery.
    pub fn checkpoint(&self, close_hash: [u8; 32]) -> Result<CheckpointSeed, ReplError> {
        let next = self.epoch.checked_add(1).ok_or(ReplError::EpochBound)?;
        if next > MAX_REGISTRY_EPOCH || self.pointers.len() > MAX_REGISTRY_POINTERS {
            return Err(ReplError::EpochBound);
        }
        let mut fields = header(&self.document, self.bucket, next);
        for (key, value) in &self.pointers {
            if key.bucket() != self.bucket {
                return Err(ReplError::EpochScope);
            }
            fields.insert(format!("p/{}", key.suffix()), ScalarValue::Uint(*value));
            fields.insert(format!("s/{}", key.suffix()), ScalarValue::Boolean(true));
        }
        CheckpointSeed::build(&self.document, next, close_hash, |doc| {
            for (key, value) in fields {
                doc.put(ROOT, key, value).map_err(am_error)?;
            }
            Ok(())
        })
    }

    /// Receipt-bound verification additionally checks that the seed has exactly one slot for
    /// each pointer, no overflow/deletion/intent state, and the canonical raw change bytes.
    pub fn verify_checkpoint(
        receipt: &VerifiedReceipt,
        bucket: u8,
        bytes: &[u8],
    ) -> Result<VerifiedCheckpoint, ReplError> {
        CheckpointSeed::verify(receipt, bytes, |document, epoch, doc| {
            let projection = Self::read(document, bucket, epoch, doc)?;
            if !projection.tombstones.is_empty()
                || !projection.overflow.is_empty()
                || doc.keys(ROOT).any(|key| key.starts_with("_p1/op/"))
                || doc.keys(ROOT).filter(|key| key.starts_with("s/")).count()
                    != projection.pointers.len()
            {
                return Err(ReplError::Malformed);
            }
            // Rebuild at the same epoch; validation must not accept a different field order or
            // a hidden primitive just because the visible projection happens to be equivalent.
            let mut predecessor = projection;
            predecessor.epoch -= 1;
            if predecessor.checkpoint(receipt.close_record_hash())?.bytes() != bytes {
                return Err(ReplError::Malformed);
            }
            Ok(())
        })
    }
}

/// Author a registry change with schema and exact successor-size preflight. The caller must
/// persist its DomainOp intent first; returned bytes may be published only after vault commit.
#[allow(clippy::too_many_arguments)]
pub fn edit_registry(
    doc: &mut EncryptedDoc,
    gate: &EpochGate,
    bucket: u8,
    device: &MlsDevice,
    group: &ServerGroup,
    rng: &mut impl CryptoRngCore,
    domain: &DomainOp,
) -> Result<SealedOp, ReplError> {
    let logical = registry_document(&group.group_id(), bucket)?;
    let operation = validate_domain(&logical, bucket, domain)?;
    let epoch = gate.epoch();
    if epoch >= MAX_REGISTRY_EPOCH {
        return Err(ReplError::EpochBound);
    }
    let fields = header(&logical, bucket, epoch);
    let entry = operation.entry();
    let before = doc.doc().clone();
    doc.edit_domain_preflight_gated(
        &logical,
        gate,
        device,
        group,
        rng,
        domain,
        |staged| {
            for (key, value) in fields {
                // Re-putting an immutable header with equal concurrent values makes Automerge
                // emit Delete cleanup operations. Preserve those harmless equal values instead.
                if staged.get(ROOT, &key)?.is_none() {
                    staged.put(ROOT, key, value)?;
                }
            }
            if !staged
                .get(ROOT, &entry.0)?
                .is_some_and(|(value, _)| scalar_eq(&value, &entry.1))
            {
                staged.put(ROOT, entry.0, entry.1)?;
            }
            Ok(())
        },
        |domain, change| validate_registry_change(&logical, bucket, epoch, domain, change, &before),
        |staged| preflight(&logical, bucket, epoch, staged),
    )
    .map(|(sealed, _)| sealed)
}

/// Inbound registry admission uses the same semantic and full-projection checks as local edits.
pub fn ingest_registry(
    doc: &mut EncryptedDoc,
    gate: &EpochGate,
    bucket: u8,
    sealed: &SealedOp,
    group: &ServerGroup,
    device: &MlsDevice,
) -> Result<Admission, ReplError> {
    let logical = registry_document(&group.group_id(), bucket)?;
    let epoch = gate.epoch();
    if epoch >= MAX_REGISTRY_EPOCH {
        return Err(ReplError::EpochBound);
    }
    let before = doc.doc().clone();
    doc.ingest_domain_preflight_gated(
        &logical,
        gate,
        sealed,
        group,
        device,
        |domain, change| validate_registry_change(&logical, bucket, epoch, domain, change, &before),
        |staged| preflight(&logical, bucket, epoch, staged),
    )
}

pub(crate) fn preflight(
    document: &LogicalDocument,
    bucket: u8,
    epoch: u64,
    doc: &AutoCommit,
) -> Result<(), ReplError> {
    // A future close changes only a fixed-size seed actor/hash, not the encoded length. Do not
    // estimate using projection JSON bytes: raw Automerge field names/framing count too.
    RegistryProjection::read(document, bucket, epoch, doc)?.checkpoint([0; 32])?;
    Ok(())
}

/// Build the owner's seed candidate from the close's verified dependency closure, never from
/// the live document's possibly larger projection. The returned operation ids let settlement
/// distinguish included intents from excluded work before pruning anything.
pub fn checkpoint_registry_close(
    doc: &mut EncryptedDoc,
    gate: &EpochGate,
    bucket: u8,
    close: &CloseRecord,
    group: &ServerGroup,
    receipt: Option<&VerifiedReceipt>,
) -> Result<(CheckpointSeed, ClosureStats), ReplError> {
    let logical = registry_document(&group.group_id(), bucket)?;
    gate.verify_scope(&logical, doc.doc_id())?;
    if close.closed_epoch != gate.epoch() {
        return Err(ReplError::EpochScope);
    }
    let unsigned_seed = doc.checkpoint_origin().map(|origin| origin.seed_hash());
    let closure =
        close.verify_and_validate(&logical, doc.doc_id(), group, receipt, doc, unsigned_seed)?;
    let projection = doc.projection_for_closure(&closure.operations)?;
    let materialized = RegistryProjection::read(&logical, bucket, gate.epoch(), &projection)?;
    Ok((materialized.checkpoint(close.hash())?, closure))
}

fn validate_domain(
    document: &LogicalDocument,
    bucket: u8,
    domain: &DomainOp,
) -> Result<RegistryOp, ReplError> {
    if domain.doc_type != DocType::DocRegistry || domain.logical_key != document.logical_key {
        return Err(ReplError::EpochScope);
    }
    let op = RegistryOp::decode(&domain.body)?;
    if op.key().bucket() != bucket {
        return Err(ReplError::EpochScope);
    }
    Ok(op)
}

pub(crate) fn validate_registry_change(
    document: &LogicalDocument,
    bucket: u8,
    epoch: u64,
    domain: &DomainOp,
    change: &Change,
    before: &AutoCommit,
) -> Result<(), ReplError> {
    let operation = validate_domain(document, bucket, domain)?;
    let actor: [u8; 32] = change
        .actor_id()
        .to_bytes()
        .try_into()
        .map_err(|_| ReplError::EpochAuthority)?;
    let marker = format!("_p1/op/{}", hex(&domain.id(&DeviceId::from_bytes(actor))));
    let entry = operation.entry();
    let mut allowed = header(document, bucket, epoch);
    allowed.insert(entry.0.clone(), entry.1.clone());
    allowed.insert(marker.clone(), ScalarValue::Uint(1));
    let mut seen = BTreeSet::new();
    // At most five headers, one pointer write, one marker. Never let a forged delta add slots,
    // delete another key/marker, create a nested map, or hide unrelated writes behind the body.
    if change.len() > allowed.len() {
        return Err(ReplError::Malformed);
    }
    if change
        .deps()
        .iter()
        .any(|hash| before.get_change_by_hash(hash).is_none())
    {
        return Err(ReplError::EpochScope);
    }
    for op in change.decode().operations {
        let (ObjectId::Root, Key::Map(key), OpType::Put(value)) = (op.obj, op.key, op.action)
        else {
            return Err(ReplError::Malformed);
        };
        if op.insert || !seen.insert(key.to_string()) || allowed.get(key.as_str()) != Some(&value) {
            return Err(ReplError::Malformed);
        }
        // Automerge predecessors are global operation ids. Checking only obj/key/action would
        // let a Put to an allowed header hide a seed slot or another pointer by referencing its
        // id. Every predecessor must be visible under this exact property at the author's deps.
        let prior_values = before
            .get_all_at(ROOT, key.as_str(), change.deps())
            .map_err(am_error)?;
        for pred in op.pred.iter() {
            if !prior_values.iter().any(|(_, id)| matches!(id, automerge::ObjId::Id(counter, actor, _) if *counter == pred.0 && actor == &pred.1)) {
                return Err(ReplError::Malformed);
            }
        }
    }
    // An idempotent request still earns a marker, so its durable intent can be receipted. It may
    // omit the no-op value write only when its *causal* view already held that value; a value
    // seen solely on another merged branch is not evidence inside this change's future closure.
    let already_applied = before
        .get_at(ROOT, &entry.0, change.deps())
        .map_err(am_error)?
        .is_some_and(|(value, _)| scalar_eq(&value, &entry.1));
    if !seen.contains(&marker) || (!seen.contains(&entry.0) && !already_applied) {
        return Err(ReplError::Malformed);
    }
    Ok(())
}

fn header(document: &LogicalDocument, bucket: u8, epoch: u64) -> BTreeMap<String, ScalarValue> {
    BTreeMap::from([
        ("bucket".into(), ScalarValue::Uint(u64::from(bucket))),
        ("epoch".into(), ScalarValue::Uint(epoch)),
        (
            "key".into(),
            ScalarValue::Str(hex(&document.logical_key).into()),
        ),
        ("kind".into(), ScalarValue::Str("registry".into())),
        ("v".into(), ScalarValue::Uint(1)),
    ])
}

fn scalar_eq(value: &Value<'_>, expected: &ScalarValue) -> bool {
    matches!(value, Value::Scalar(scalar) if scalar.as_ref() == expected)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>, ReplError> {
    if text.len() > 384
        || !text.len().is_multiple_of(2)
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ReplError::Malformed);
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair).map_err(|_| ReplError::Malformed)?;
            u8::from_str_radix(s, 16).map_err(|_| ReplError::Malformed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::ActorId;

    fn keys(bucket: u8, n: usize, len: usize) -> Vec<PointerKey> {
        let mut out = Vec::new();
        for value in 0u64.. {
            let mut key = vec![b'x'; len];
            key[..8].copy_from_slice(&value.to_be_bytes());
            let key = PointerKey::new(DocType::StudioObject, key).unwrap();
            if key.bucket() == bucket {
                out.push(key);
                if out.len() == n {
                    break;
                }
            }
        }
        out.sort();
        out
    }

    fn source(
        bucket: u8,
        epoch: u64,
        pointers: &[PointerKey],
        slots: bool,
    ) -> (LogicalDocument, AutoCommit) {
        let logical = registry_document(b"registry-server", bucket).unwrap();
        let mut doc = AutoCommit::new().with_actor(ActorId::from(vec![1; 32]));
        for (key, value) in header(&logical, bucket, epoch) {
            doc.put(ROOT, key, value).unwrap();
        }
        for key in pointers {
            doc.put(ROOT, format!("p/{}", key.suffix()), 0u64).unwrap();
            if slots {
                doc.put(ROOT, format!("s/{}", key.suffix()), true).unwrap();
            }
        }
        doc.commit();
        (logical, doc)
    }

    #[test]
    fn canonical_registry_bodies_and_type_bucket_derivations_are_pinned() {
        let key = PointerKey::new(DocType::StudioObject, vec![0xab, 0xcd]).unwrap();
        assert_eq!(
            RegistryOp::Put {
                key: key.clone(),
                epoch: 0
            }
            .encode()
            .unwrap(),
            br#"{"epoch":0,"key":"abcd","t":"put_pointer","type":16}"#
        );
        for bad in [
            br#"{"epoch":0,"key":"abcd","t":"put_pointer","type":16,"x":0}"#.as_slice(),
            br#"{"epoch":0,"epoch":0,"key":"abcd","t":"put_pointer","type":16}"#,
            br#"{"epoch":0.0,"key":"abcd","t":"put_pointer","type":16}"#,
            br#"{"epoch":0,"key":"ABCD","t":"put_pointer","type":16}"#,
        ] {
            assert!(RegistryOp::decode(bad).is_err());
        }
        assert_eq!(key.bucket(), 102);
        assert_eq!(
            hex(&registry_document(b"registry-server", 136)
                .unwrap()
                .logical_key),
            "1445eb6889574fe4e31b11f91e1b369dd243ff971f2a54a9131b9a9c508d26cb"
        );
        assert_ne!(
            key.bucket(),
            PointerKey::new(DocType::PostReplies, key.key.clone())
                .unwrap()
                .bucket()
        );
    }

    #[test]
    fn seeded_slots_survive_smaller_keys_and_only_tombstones_reclaim_them() {
        let keys = keys(0, MAX_REGISTRY_POINTERS + 1, 8);
        let (logical, mut doc) = source(0, 1, &keys[1..], true);
        doc.put(ROOT, format!("p/{}", keys[0].suffix()), 12u64)
            .unwrap();
        doc.commit();
        let full = RegistryProjection::read(&logical, 0, 1, &doc).unwrap();
        assert_eq!(full.pointers.len(), MAX_REGISTRY_POINTERS);
        assert_eq!(full.overflow.keys().collect::<Vec<_>>(), vec![&keys[0]]);
        doc.put(ROOT, format!("d/{}", keys[1].suffix()), true)
            .unwrap();
        doc.commit();
        let reclaimed = RegistryProjection::read(&logical, 0, 1, &doc).unwrap();
        assert!(reclaimed.overflow.is_empty());
        assert!(reclaimed.pointers.contains_key(&keys[0]));
        assert!(!reclaimed.pointers.contains_key(&keys[1]));
        let seed = reclaimed.checkpoint([9; 32]).unwrap();
        let mut next = AutoCommit::new().with_actor(ActorId::from(vec![2; 32]));
        next.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
            .unwrap();
        assert!(next
            .keys(ROOT)
            .all(|key| !key.starts_with("d/") && !key.starts_with("_p1/op/")));
        assert_eq!(
            RegistryProjection::read(&logical, 0, 2, &next)
                .unwrap()
                .pointers,
            reclaimed.pointers
        );
    }

    #[test]
    fn concurrent_puts_and_tombstones_converge_in_both_orders() {
        let keys = keys(0, 2, 8);
        let (logical, base) = source(0, 0, &keys, false);
        let mut a = base.clone().with_actor(ActorId::from(vec![2; 32]));
        let mut b = base.clone().with_actor(ActorId::from(vec![3; 32]));
        a.put(ROOT, format!("p/{}", keys[0].suffix()), 3u64)
            .unwrap();
        a.put(ROOT, format!("p/{}", keys[1].suffix()), 5u64)
            .unwrap();
        a.commit();
        b.put(ROOT, format!("p/{}", keys[0].suffix()), 7u64)
            .unwrap();
        b.put(ROOT, format!("d/{}", keys[1].suffix()), true)
            .unwrap();
        b.commit();
        let ca = a.get_last_local_change().unwrap();
        let cb = b.get_last_local_change().unwrap();
        a.apply_changes([cb]).unwrap();
        b.apply_changes([ca]).unwrap();
        let pa = RegistryProjection::read(&logical, 0, 0, &a).unwrap();
        let pb = RegistryProjection::read(&logical, 0, 0, &b).unwrap();
        assert_eq!(pa, pb);
        assert_eq!(pa.pointers[&keys[0]], 7);
        assert!(!pa.pointers.contains_key(&keys[1]));
        assert_eq!(
            pa.checkpoint([1; 32]).unwrap().bytes(),
            pb.checkpoint([1; 32]).unwrap().bytes()
        );
    }

    #[test]
    fn maximal_bucket_seed_fits_and_rotations_do_not_accumulate_history() {
        let keys = keys(1, MAX_REGISTRY_POINTERS, 192);
        let (logical, mut doc) = source(1, 0, &keys, false);
        for key in &keys {
            doc.put(ROOT, format!("p/{}", key.suffix()), u64::MAX)
                .unwrap();
        }
        doc.commit();
        let mut projection = RegistryProjection::read(&logical, 1, 0, &doc).unwrap();
        let seed = projection.checkpoint([1; 32]).unwrap();
        assert!(seed.bytes().len() < crate::MAX_CHECKPOINT_BYTES);
        let initial = seed.bytes().len();
        // The largest epoch encoding is enough to measure the maximal seed; churn is exercised
        // separately on a small projection to avoid repeating the expensive 2048-key fixture.
        projection.epoch = 4095;
        let seed = projection.checkpoint([255; 32]).unwrap();
        assert!(seed.bytes().len() <= initial + 2);
        assert_eq!(Change::from_bytes(seed.bytes().to_vec()).unwrap().seq(), 1);
        projection.epoch = MAX_REGISTRY_EPOCH;
        assert!(matches!(
            projection.checkpoint([1; 32]),
            Err(ReplError::EpochBound)
        ));
    }

    #[test]
    fn repeated_create_delete_rotations_retire_markers_and_tombstones() {
        let key = keys(0, 1, 8).remove(0);
        let (logical, mut doc) = source(0, 0, &[], false);
        let mut max_seed = 0;
        for epoch in 0..1000 {
            doc.put(ROOT, format!("p/{}", key.suffix()), 0u64).unwrap();
            if epoch % 2 == 1 {
                doc.put(ROOT, format!("d/{}", key.suffix()), true).unwrap();
            }
            doc.put(ROOT, format!("_p1/op/{}", hex(&[42; 32])), 1u64)
                .unwrap();
            doc.commit();
            let seed = RegistryProjection::read(&logical, 0, epoch, &doc)
                .unwrap()
                .checkpoint([3; 32])
                .unwrap();
            max_seed = max_seed.max(seed.bytes().len());
            doc = AutoCommit::new().with_actor(ActorId::from(vec![1; 32]));
            doc.apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
                .unwrap();
            assert_eq!(doc.get_heads().len(), 1);
            assert!(doc
                .keys(ROOT)
                .all(|key| !key.starts_with("d/") && !key.starts_with("_p1/op/")));
            assert_eq!(
                RegistryProjection::read(&logical, 0, epoch + 1, &doc)
                    .unwrap()
                    .pointers
                    .len(),
                usize::from(epoch % 2 == 0)
            );
        }
        assert!(max_seed < 512);
    }
}
