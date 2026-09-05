//! Bounded epoch-close protocol primitives and local lifecycle state.
//!
//! This module owns the security-sensitive, domain-independent part of P1. Creative document
//! schemas provide canonical domain operations and materializations; this layer binds those bytes
//! to a server/document, authenticates owner receipts, serializes the `Open -> Closing -> Settled`
//! gate, and bounds the local recovery transition. Keeping the gate beside [`crate::EncryptedDoc`]
//! is deliberate: callers must not be able to apply an inbound operation on one path while a
//! receipt seals the same epoch on another.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use catcoms_crypto::{verify_with_public_bytes, DeviceId};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_wire::{Decoder, DocType, Encoder};
use sha2::{Digest, Sha256};

use crate::{EncryptedDoc, ReplError, SignedOp};

/// Maximum canonical bytes in a domain-operation envelope.
pub const MAX_DOMAIN_OP_BYTES: usize = 64 * 1024;
/// Maximum whole signed P1 envelope admitted as one epoch operation. The domain operation itself
/// remains capped at 64 KiB; this allowance covers its Automerge delta and signature.
pub const MAX_SIGNED_EPOCH_OP_BYTES: usize = 256 * 1024;
/// Maximum pre-parse size of a close record.
pub const MAX_CLOSE_RECORD_BYTES: usize = 4 * 1024;
/// Maximum pre-parse size of a receipt.
pub const MAX_RECEIPT_BYTES: usize = 1024;
/// Maximum canonical bytes in one recovery snapshot.
pub const MAX_RECOVERY_SNAPSHOT_BYTES: usize = 6 * 1024 * 1024;
/// Maximum plaintext encoding of two retained snapshots plus one staged snapshot.
pub const MAX_RECOVERY_SLOTS_BYTES: usize = 3 * MAX_RECOVERY_SNAPSHOT_BYTES + 1024;
/// Maximum plaintext bytes in one peer's constant-sized receipt/fault book.
pub const MAX_RECEIPT_BOOK_BYTES: usize = 8 * 1024;
/// Maximum plaintext bytes in one owner's high-water/in-flight receipt journal.
pub const MAX_OWNER_RECEIPT_JOURNAL_BYTES: usize = 3 * MAX_RECEIPT_BYTES + 256;
/// Maximum plaintext bytes in one persisted epoch lifecycle gate.
pub const MAX_EPOCH_GATE_BYTES: usize = 3 * 1024 * 1024;
/// Maximum number of vault-sealed pending intents for one logical document.
pub const MAX_INTENTS_PER_DOCUMENT: usize = 10_000;
/// Maximum aggregate canonical domain-operation bytes in one logical document's intent ledger.
pub const MAX_INTENT_BYTES_PER_DOCUMENT: usize = 4 * 1024 * 1024;
/// Maximum encoded plaintext state for one intent ledger, including framing and author ids.
pub const MAX_INTENT_LEDGER_BYTES: usize = 5 * 1024 * 1024;
/// Maximum number of operations in one open epoch.
pub const MAX_EPOCH_OPERATIONS: usize = 20_000;
/// Maximum encoded operation bytes in one open epoch.
pub const MAX_EPOCH_BYTES: usize = 4 * 1024 * 1024;
/// Maximum operations admitted from one non-owner device in one epoch.
pub const MAX_DEVICE_OPERATIONS: usize = 5_000;
/// Maximum encoded bytes admitted from one non-owner device in one epoch.
pub const MAX_DEVICE_BYTES: usize = 1024 * 1024;
/// Bound shared by parked close records and operations arriving after an epoch was sealed.
pub const MAX_QUARANTINED: usize = 1024;
/// Recovery snapshots remain exportable for this long if the eviction warning is not acknowledged.
pub const RECOVERY_GRACE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

const MAX_SERVER_ID_BYTES: usize = 256;
const MAX_LOGICAL_KEY_BYTES: usize = 192;
const MAX_HEADS: usize = 64;
const MAX_RECOVERY_ITEMS: usize = MAX_EPOCH_OPERATIONS;
const MAX_RECOVERY_CONFLICTS: usize = 1024;
const MAX_CONFLICT_VALUES: usize = 4;
const MAX_RECOVERY_TARGET_BYTES: usize = 256;

type Hash32 = [u8; 32];

fn is_epoch_managed_doc_type(doc_type: DocType) -> bool {
    matches!(
        doc_type,
        DocType::StudioIndex | DocType::StudioObject | DocType::PostReplies | DocType::DocRegistry
    )
}

fn hash_parts(domain: &str, parts: &[&[u8]]) -> Hash32 {
    let mut h = Sha256::new();
    for part in std::iter::once(domain.as_bytes()).chain(parts.iter().copied()) {
        let len = u32::try_from(part.len()).expect("P1 hash parts are protocol-bounded");
        h.update(len.to_be_bytes());
        h.update(part);
    }
    h.finalize().into()
}

fn get_fixed<const N: usize>(d: &mut Decoder<'_>) -> Result<[u8; N], ReplError> {
    d.get_bytes()
        .map_err(|_| ReplError::Malformed)?
        .try_into()
        .map_err(|_| ReplError::Malformed)
}

fn put_hash(e: &mut Encoder, value: &Hash32) {
    e.put_bytes(value).expect("fixed hashes fit the wire codec");
}

fn put_optional_hash(e: &mut Encoder, value: Option<&Hash32>) {
    match value {
        Some(value) => {
            e.put_u8(1);
            put_hash(e, value);
        }
        None => {
            e.put_u8(0);
        }
    }
}

fn get_optional_hash(d: &mut Decoder<'_>) -> Result<Option<Hash32>, ReplError> {
    match d.get_u8().map_err(|_| ReplError::Malformed)? {
        0 => Ok(None),
        1 => Ok(Some(get_fixed(d)?)),
        _ => Err(ReplError::Malformed),
    }
}

/// A server-bound logical document name. Epoch document ids are derived from this stable name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalDocument {
    /// MLS group id of the server that owns the document.
    pub server_id: Vec<u8>,
    /// Stable document type discriminator.
    pub doc_type: DocType,
    /// Type-specific stable key (channel id, object id, post id, or registry bucket key).
    pub logical_key: Vec<u8>,
}

impl LogicalDocument {
    /// Construct a bounded logical name suitable for hashing and wire records.
    pub fn new(
        server_id: Vec<u8>,
        doc_type: DocType,
        logical_key: Vec<u8>,
    ) -> Result<Self, ReplError> {
        if !is_epoch_managed_doc_type(doc_type)
            || server_id.is_empty()
            || server_id.len() > MAX_SERVER_ID_BYTES
            || logical_key.is_empty()
            || logical_key.len() > MAX_LOGICAL_KEY_BYTES
        {
            return Err(ReplError::EpochBound);
        }
        Ok(Self {
            server_id,
            doc_type,
            logical_key,
        })
    }
}

/// Deterministic epoch-zero document id for a logical document.
pub fn epoch_zero_id(doc_type: DocType, logical_key: &[u8]) -> u128 {
    let tag = u64::from(doc_type.tag()).to_be_bytes();
    let hash = hash_parts("catcoms-doc-epoch0:v1", &[&tag, logical_key]);
    u128::from_be_bytes(hash[..16].try_into().expect("slice is exactly 16 bytes"))
}

/// Deterministic successor document id selected by a close record.
pub fn epoch_id(
    doc_type: DocType,
    logical_key: &[u8],
    checkpoint_epoch: u64,
    close_record_hash: &Hash32,
) -> u128 {
    let tag = u64::from(doc_type.tag()).to_be_bytes();
    let epoch = checkpoint_epoch.to_be_bytes();
    let hash = hash_parts(
        "catcoms-doc-epoch:v1",
        &[&tag, logical_key, &epoch, close_record_hash],
    );
    u128::from_be_bytes(hash[..16].try_into().expect("slice is exactly 16 bytes"))
}

/// Owner-tenure identifier repeated by every receipt for one document during that tenure.
///
/// `tenure_start_group_epoch` is carried in the receipt. Existing members compare it with the
/// transition they observed; a newcomer can at least recompute the signed identifier without
/// depending on unavailable pre-join MLS history.
pub fn tenure_id(
    server_id: &[u8],
    owner_public_key: &[u8],
    tenure_start_group_epoch: u64,
) -> Hash32 {
    let epoch = tenure_start_group_epoch.to_be_bytes();
    hash_parts("catcoms-tenure:v1", &[server_id, owner_public_key, &epoch])
}

/// Canonical, type-specific operation bytes plus the nonce from which its stable id is derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainOp {
    /// Random 16-byte nonce generated through the sanctioned RNG seam.
    pub nonce: [u8; 16],
    /// Target document type.
    pub doc_type: DocType,
    /// Target logical key.
    pub logical_key: Vec<u8>,
    /// Canonical type-specific operation body.
    pub body: Vec<u8>,
}

impl DomainOp {
    /// Strict canonical encoding used inside a `SignedOp` Automerge change.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        if !is_epoch_managed_doc_type(self.doc_type)
            || self.logical_key.is_empty()
            || self.logical_key.len() > MAX_LOGICAL_KEY_BYTES
        {
            return Err(ReplError::EpochBound);
        }
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.nonce).expect("fixed nonce fits");
        e.put_u16(self.doc_type.tag());
        e.put_bytes(&self.logical_key)
            .map_err(|_| ReplError::EpochBound)?;
        e.put_bytes(&self.body).map_err(|_| ReplError::EpochBound)?;
        let bytes = e.finish();
        if bytes.len() > MAX_DOMAIN_OP_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }

    /// Strictly decode and bound a domain operation.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_DOMAIN_OP_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let nonce = get_fixed(&mut d)?;
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let logical_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let body = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        d.finish().map_err(|_| ReplError::Malformed)?;
        let op = Self {
            nonce,
            doc_type,
            logical_key,
            body,
        };
        // Re-encoding applies all semantic length bounds and also pins canonicality.
        if op.encode()?.as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(op)
    }

    /// Derive the idempotency token from the verified outer author.
    pub fn id(&self, author: &DeviceId) -> Hash32 {
        hash_parts(
            "catcoms-domain-op:v1",
            &[&self.logical_key, author.as_bytes(), &self.nonce],
        )
    }
}

/// One durable local operation intent awaiting inclusion in a receipted closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalIntent {
    /// Local device identity that derives this operation's stable id.
    pub author: DeviceId,
    /// Canonical domain operation replayed into a later open epoch when excluded.
    pub operation: DomainOp,
}

/// Bounded, vault-sealed pending intents for one logical document.
///
/// The caller persists [`Self::encode`] before invoking `EncryptedDoc::edit_domain_gated`.
/// Entries are removed only after the caller has persisted any excluded recovery snapshot and
/// supplied the ids proven inside a receipted closure.
#[derive(Clone, Debug)]
pub struct IntentLedger {
    document: LogicalDocument,
    intents: BTreeMap<Hash32, LocalIntent>,
    encoded_operation_bytes: usize,
}

impl IntentLedger {
    /// Create an empty ledger for one server-bound logical document.
    pub fn new(document: LogicalDocument) -> Self {
        Self {
            document,
            intents: BTreeMap::new(),
            encoded_operation_bytes: 0,
        }
    }

    /// Add an intent before attempting the corresponding edit. Exact retries are idempotent;
    /// nonce reuse with different bytes is a local integrity fault.
    pub fn prepare(&mut self, author: DeviceId, operation: DomainOp) -> Result<Hash32, ReplError> {
        if operation.doc_type != self.document.doc_type
            || operation.logical_key != self.document.logical_key
        {
            return Err(ReplError::EpochScope);
        }
        let encoded_len = operation.encode()?.len();
        let id = operation.id(&author);
        if let Some(existing) = self.intents.get(&id) {
            return if existing.author == author && existing.operation == operation {
                Ok(id)
            } else {
                Err(ReplError::IntentConflict)
            };
        }
        if self.intents.len() == MAX_INTENTS_PER_DOCUMENT
            || self.encoded_operation_bytes.saturating_add(encoded_len)
                > MAX_INTENT_BYTES_PER_DOCUMENT
        {
            return Err(ReplError::EpochBound);
        }
        self.intents.insert(id, LocalIntent { author, operation });
        self.encoded_operation_bytes += encoded_len;
        Ok(id)
    }

    /// Pending intents in stable operation-id order.
    pub fn pending(&self) -> impl ExactSizeIterator<Item = (&Hash32, &LocalIntent)> {
        self.intents.iter()
    }

    /// Number of pending intents.
    pub fn len(&self) -> usize {
        self.intents.len()
    }

    /// Whether no pending intent remains.
    pub fn is_empty(&self) -> bool {
        self.intents.is_empty()
    }

    /// Remove intents proven final by a receipted closure.
    ///
    /// This method performs no persistence itself. Settlement must first persist excluded content
    /// in recovery and atomically commit the returned ledger state with the installed checkpoint.
    pub fn remove_receipted(&mut self, operation_ids: &BTreeSet<Hash32>) -> usize {
        let before = self.intents.len();
        self.intents.retain(|id, intent| {
            if operation_ids.contains(id) {
                self.encoded_operation_bytes = self
                    .encoded_operation_bytes
                    .saturating_sub(intent.operation.encode().map_or(0, |bytes| bytes.len()));
                false
            } else {
                true
            }
        });
        before - self.intents.len()
    }

    /// Canonical plaintext bytes; callers vault-seal this value before disk persistence.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.document.server_id)
            .map_err(|_| ReplError::EpochBound)?;
        e.put_u16(self.document.doc_type.tag());
        e.put_bytes(&self.document.logical_key)
            .map_err(|_| ReplError::EpochBound)?;
        e.put_u32(u32::try_from(self.intents.len()).map_err(|_| ReplError::EpochBound)?);
        for intent in self.intents.values() {
            e.put_bytes(intent.author.as_bytes())
                .expect("device identity fits");
            e.put_bytes(&intent.operation.encode()?)
                .map_err(|_| ReplError::EpochBound)?;
        }
        let bytes = e.finish();
        if bytes.len() > MAX_INTENT_LEDGER_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }

    /// Restore a ledger, re-deriving every operation id and all aggregate accounting.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_INTENT_LEDGER_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let server_id = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let logical_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let document = LogicalDocument::new(server_id, doc_type, logical_key)?;
        let count = usize::try_from(d.get_u32().map_err(|_| ReplError::Malformed)?)
            .map_err(|_| ReplError::EpochBound)?;
        if count > MAX_INTENTS_PER_DOCUMENT {
            return Err(ReplError::EpochBound);
        }
        let mut ledger = Self::new(document);
        let mut prior_id = None;
        for _ in 0..count {
            let author = DeviceId::from_bytes(get_fixed(&mut d)?);
            let operation = DomainOp::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?;
            let id = operation.id(&author);
            if prior_id.is_some_and(|prior| prior >= id) {
                return Err(ReplError::Malformed);
            }
            if ledger.prepare(author, operation)? != id {
                return Err(ReplError::Malformed);
            }
            prior_id = Some(id);
        }
        d.finish().map_err(|_| ReplError::Malformed)?;
        if ledger.encode()?.as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(ledger)
    }
}

/// Signed proposal to close one open epoch at an exact, causally closed head set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseRecord {
    /// MLS group id.
    pub server_id: Vec<u8>,
    /// Document type.
    pub doc_type: DocType,
    /// Concrete epoch document id.
    pub doc_id: u128,
    /// Epoch number being closed.
    pub closed_epoch: u64,
    /// Distinct, ascending Automerge heads.
    pub heads: Vec<Hash32>,
    /// Raw Ed25519 leaf key of the author.
    pub author_public_key: Vec<u8>,
    /// Signature over the domain-separated close transcript.
    pub signature: [u8; 64],
}

impl CloseRecord {
    /// Build and sign a canonical close. Callers still validate closure size/content separately.
    pub fn sign(
        doc: &LogicalDocument,
        doc_id: u128,
        closed_epoch: u64,
        mut heads: Vec<Hash32>,
        author: &MlsDevice,
    ) -> Result<Self, ReplError> {
        heads.sort_unstable();
        heads.dedup();
        if closed_epoch == u64::MAX || heads.is_empty() || heads.len() > MAX_HEADS {
            return Err(ReplError::EpochBound);
        }
        let mut record = Self {
            server_id: doc.server_id.clone(),
            doc_type: doc.doc_type,
            doc_id,
            closed_epoch,
            heads,
            author_public_key: author.public_key_bytes(),
            signature: [0; 64],
        };
        record.signature = author.sign(&record.signature_hash())?;
        if record.encode().len() > MAX_CLOSE_RECORD_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(record)
    }

    fn unsigned_bytes(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.server_id).expect("server id was bounded");
        e.put_u16(self.doc_type.tag());
        e.put_u128(self.doc_id);
        e.put_u64(self.closed_epoch);
        e.put_u16(u16::try_from(self.heads.len()).expect("heads are bounded"));
        for head in &self.heads {
            put_hash(&mut e, head);
        }
        e.put_bytes(&self.author_public_key)
            .expect("public key fits");
        e.finish()
    }

    fn signature_hash(&self) -> Hash32 {
        let tag = u64::from(self.doc_type.tag()).to_be_bytes();
        let doc_id = self.doc_id.to_be_bytes();
        let epoch = self.closed_epoch.to_be_bytes();
        let unsigned = self.unsigned_bytes();
        hash_parts(
            "catcoms-close-sig:v1",
            &[&self.server_id, &tag, &doc_id, &epoch, &unsigned],
        )
    }

    /// Stable protocol hash used by receipts and successor-id derivation.
    pub fn hash(&self) -> Hash32 {
        let unsigned = self.unsigned_bytes();
        hash_parts("catcoms-close:v1", &[&unsigned])
    }

    /// Author device id committed by `author_public_key`.
    pub fn author(&self) -> DeviceId {
        DeviceId::from_public_key_bytes(&self.author_public_key)
    }

    /// Verify signature, scope, canonical heads and current-membership authority.
    ///
    /// A close named by `historical_receipt` may be admitted after its author was removed. The
    /// unforgeable verification capability is still bound here to the exact document, epoch and
    /// close hash instead of being treated as a confused-deputy boolean.
    pub fn verify_for(
        &self,
        doc: &LogicalDocument,
        expected_doc_id: u128,
        group: &ServerGroup,
        historical_receipt: Option<&VerifiedReceipt>,
    ) -> Result<(), ReplError> {
        if self.server_id != doc.server_id
            || self.doc_type != doc.doc_type
            || self.doc_id != expected_doc_id
            || self.server_id != group.group_id()
        {
            return Err(ReplError::EpochScope);
        }
        if self.heads.is_empty()
            || self.heads.len() > MAX_HEADS
            || self.heads.windows(2).any(|pair| pair[0] >= pair[1])
            || self.closed_epoch == u64::MAX
            || !verify_with_public_bytes(
                &self.author_public_key,
                &self.signature_hash(),
                &self.signature,
            )
        {
            return Err(ReplError::EpochAuthority);
        }
        match group.member_signature_key(&self.author()) {
            Some(expected) if expected == self.author_public_key => {}
            Some(_) => return Err(ReplError::EpochAuthority),
            None if historical_receipt.is_some_and(|verified| {
                verified.document == *doc
                    && verified.closed_epoch == self.closed_epoch
                    && verified.close_record_hash == self.hash()
            }) => {}
            None => return Err(ReplError::EpochAuthority),
        }
        Ok(())
    }

    /// Canonical wire bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.unsigned_bytes())
            .expect("bounded close bytes fit");
        e.put_bytes(&self.signature).expect("fixed signature fits");
        e.finish()
    }

    /// Strict decoding with the pre-parse close cap.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_CLOSE_RECORD_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut outer = Decoder::new(bytes);
        let unsigned = outer.get_bytes().map_err(|_| ReplError::Malformed)?;
        let signature = get_fixed(&mut outer)?;
        outer.finish().map_err(|_| ReplError::Malformed)?;
        let mut d = Decoder::new(unsigned);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let server_id = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let doc_id = d.get_u128().map_err(|_| ReplError::Malformed)?;
        let closed_epoch = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let count = usize::from(d.get_u16().map_err(|_| ReplError::Malformed)?);
        if count == 0 || count > MAX_HEADS {
            return Err(ReplError::EpochBound);
        }
        let mut heads = Vec::with_capacity(count);
        for _ in 0..count {
            heads.push(get_fixed(&mut d)?);
        }
        let author_public_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        d.finish().map_err(|_| ReplError::Malformed)?;
        if server_id.is_empty()
            || server_id.len() > MAX_SERVER_ID_BYTES
            || author_public_key.len() != 32
            || closed_epoch == u64::MAX
            || heads.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(ReplError::Malformed);
        }
        let record = Self {
            server_id,
            doc_type,
            doc_id,
            closed_epoch,
            heads,
            author_public_key,
            signature,
        };
        if record.encode().as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(record)
    }

    /// Authenticate a close and validate its actual dependency-closed operation set and budgets.
    ///
    /// Keeping authority and closure validation in one public operation prevents a caller from
    /// accidentally treating structurally valid heads as an authorized close. A checkpoint epoch
    /// supplies its sole unsigned seed hash; epoch zero supplies `None`.
    #[allow(clippy::too_many_arguments)]
    pub fn verify_and_validate(
        &self,
        logical_document: &LogicalDocument,
        expected_doc_id: u128,
        group: &ServerGroup,
        historical_receipt: Option<&VerifiedReceipt>,
        encrypted_doc: &mut EncryptedDoc,
        unsigned_seed: Option<Hash32>,
    ) -> Result<ClosureStats, ReplError> {
        self.verify_for(logical_document, expected_doc_id, group, historical_receipt)?;
        if self.server_id != logical_document.server_id
            || self.doc_type != logical_document.doc_type
            || encrypted_doc.doc_type() != self.doc_type
            || encrypted_doc.doc_id() != self.doc_id
        {
            return Err(ReplError::EpochScope);
        }
        let operations = encrypted_doc.signed_ops_for_heads(&self.heads, unsigned_seed)?;
        let mut encoded_bytes = 0usize;
        let mut by_device: BTreeMap<DeviceId, (usize, usize)> = BTreeMap::new();
        let mut domain_op_ids = std::collections::BTreeSet::new();
        for operation in &operations {
            if operation.doc_type != self.doc_type
                || operation.doc_id != self.doc_id
                || !operation.verify()
            {
                return Err(ReplError::EpochAuthority);
            }
            let change = automerge::Change::from_bytes(operation.delta.clone())
                .map_err(|error| ReplError::Automerge(error.to_string()))?;
            if change.actor_id().to_bytes() != operation.author_device.as_bytes() {
                return Err(ReplError::EpochAuthority);
            }
            let domain = operation.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
            if domain.doc_type != self.doc_type
                || domain.logical_key != logical_document.logical_key
            {
                return Err(ReplError::EpochScope);
            }
            if !domain_op_ids.insert(domain.id(&operation.author_device)) {
                // Reusing a nonce within one logical document is a semantic replay even when the
                // signed envelope or Automerge change differs.
                return Err(ReplError::Malformed);
            }
            let len = operation.encode().len();
            if len > MAX_SIGNED_EPOCH_OP_BYTES {
                return Err(ReplError::EpochBound);
            }
            encoded_bytes = encoded_bytes
                .checked_add(len)
                .ok_or(ReplError::EpochBound)?;
            let entry = by_device.entry(operation.author_device).or_default();
            entry.0 += 1;
            entry.1 = entry.1.checked_add(len).ok_or(ReplError::EpochBound)?;
        }
        if operations.len() > MAX_EPOCH_OPERATIONS || encoded_bytes > MAX_EPOCH_BYTES {
            return Err(ReplError::EpochBound);
        }
        let owner = group
            .designated_committer()
            .ok_or(ReplError::EpochAuthority)?;
        let reached_lower_bound = operations.len() >= 10_000
            || encoded_bytes >= 2 * 1024 * 1024
            || by_device.iter().any(|(author, (count, bytes))| {
                *author != owner && (*count >= MAX_DEVICE_OPERATIONS || *bytes >= MAX_DEVICE_BYTES)
            });
        if !reached_lower_bound {
            return Err(ReplError::EpochBound);
        }
        Ok(ClosureStats {
            operation_count: operations.len(),
            encoded_bytes,
            by_device,
            operations,
        })
    }
}

/// Verified resource accounting for one dependency-closed close candidate.
#[derive(Clone, Debug)]
pub struct ClosureStats {
    /// Signed user operations in dependency order.
    pub operations: Vec<SignedOp>,
    /// Total user-operation count.
    pub operation_count: usize,
    /// Whole signed-envelope bytes charged to the epoch.
    pub encoded_bytes: usize,
    /// Per verified author `(operation count, encoded bytes)`.
    pub by_device: BTreeMap<DeviceId, (usize, usize)>,
}

impl ClosureStats {
    /// Author-bound operation ids proven inside this validated dependency-closed set.
    pub fn domain_operation_ids(&self) -> Result<BTreeSet<Hash32>, ReplError> {
        self.operations
            .iter()
            .map(|operation| {
                operation
                    .parsed_domain_op()?
                    .ok_or(ReplError::Malformed)
                    .map(|domain| domain.id(&operation.author_device))
            })
            .collect()
    }
}

/// Checkpoint selected when the current owner tenure first receipts a logical document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InheritedCheckpoint {
    /// The document has no receipted predecessor.
    EpochZero,
    /// A previously receipted checkpoint, authenticated again by the current owner.
    Checkpoint {
        /// Epoch number of the inherited checkpoint.
        epoch: u64,
        /// Close that determines the inherited checkpoint document id.
        close_record_hash: Hash32,
        /// Exact Automerge seed change expected for that checkpoint.
        seed_change_hash: Hash32,
    },
}

impl InheritedCheckpoint {
    fn encode_into(&self, e: &mut Encoder) {
        match self {
            Self::EpochZero => {
                e.put_u64(0);
                put_optional_hash(e, None);
                put_optional_hash(e, None);
            }
            Self::Checkpoint {
                epoch,
                close_record_hash,
                seed_change_hash,
            } => {
                e.put_u64(*epoch);
                put_optional_hash(e, Some(close_record_hash));
                put_optional_hash(e, Some(seed_change_hash));
            }
        }
    }

    fn decode_from(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        let epoch = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let close = get_optional_hash(d)?;
        let seed = get_optional_hash(d)?;
        match (epoch, close, seed) {
            (0, None, None) => Ok(Self::EpochZero),
            (epoch, Some(close_record_hash), Some(seed_change_hash)) if epoch > 0 => {
                Ok(Self::Checkpoint {
                    epoch,
                    close_record_hash,
                    seed_change_hash,
                })
            }
            _ => Err(ReplError::Malformed),
        }
    }

    /// Epoch number selected by this inherited checkpoint (`0` for epoch zero).
    pub fn epoch(&self) -> u64 {
        match self {
            Self::EpochZero => 0,
            Self::Checkpoint { epoch, .. } => *epoch,
        }
    }
}

/// Owner-signed finality record for one close and its deterministic checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    /// Logical scope.
    pub document: LogicalDocument,
    /// Epoch finalized by this receipt.
    pub closed_epoch: u64,
    /// Winning close record.
    pub close_record_hash: Hash32,
    /// Exact seed change admitted for the successor checkpoint.
    pub seed_change_hash: Hash32,
    /// Start epoch of this owner tenure, carried so newcomers can recompute `tenure_id`.
    pub tenure_start_group_epoch: u64,
    /// Derived owner-tenure id.
    pub tenure_id: Hash32,
    /// Checkpoint inherited by the first receipt and repeated by later receipts.
    pub inherited: InheritedCheckpoint,
    /// Current owner's raw Ed25519 leaf key.
    pub owner_public_key: Vec<u8>,
    /// Signature over the canonical receipt transcript.
    pub signature: [u8; 64],
}

/// Capability proving that a receipt passed current-owner and tenure verification.
///
/// The inner reference is deliberately private. Security-sensitive consumers such as historical
/// close admission take this type instead of trusting a caller-supplied `bool` or raw receipt.
#[derive(Clone, Debug)]
pub struct VerifiedReceipt {
    document: LogicalDocument,
    closed_epoch: u64,
    close_record_hash: Hash32,
    receipt_hash: Hash32,
}

/// Owner-signed choice that resolves one visible receipt-equivocation fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptRepair {
    /// Logical document whose receipt book is faulted.
    pub document: LogicalDocument,
    /// Tenure in which both conflicting receipts were signed.
    pub tenure_id: Hash32,
    /// Conflicting receipt hashes, stored ascending so delivery order cannot affect the record.
    pub receipt_hashes: [Hash32; 2],
    /// One of `receipt_hashes`, selected by the current owner.
    pub selected_receipt_hash: Hash32,
    /// Strictly increasing repair generation for this document/tenure.
    pub repair_sequence: u64,
    /// Current owner's raw Ed25519 leaf key.
    pub owner_public_key: Vec<u8>,
    /// Signature over the complete repair transcript.
    pub signature: [u8; 64],
}

/// Fresh owner proof that one receipt is the current head for a requester's keyed query.
///
/// This is not a lease and says nothing about future receipts. It only prevents a member from
/// replaying an old receipt when the same device has become owner in two separate tenures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptHeadProof {
    /// Logical document queried by the requester.
    pub document: LogicalDocument,
    /// Exact receipt selected by the current owner.
    pub receipt_hash: Hash32,
    /// Tenure start repeated from that receipt.
    pub tenure_start_group_epoch: u64,
    /// Full authenticated requester identity from the outer request.
    pub requester: DeviceId,
    /// Fresh request nonce.
    pub request_nonce: [u8; 16],
    /// Current owner's raw Ed25519 leaf key.
    pub owner_public_key: Vec<u8>,
    /// Signature over the complete request-bound transcript.
    pub signature: [u8; 64],
}

impl ReceiptHeadProof {
    /// Sign a request-bound current-head statement. The sync layer serves this only after
    /// authenticating the requester as a current member and checking its request freshness.
    pub fn sign(
        receipt: &Receipt,
        requester: DeviceId,
        request_nonce: [u8; 16],
        owner: &MlsDevice,
    ) -> Result<Self, ReplError> {
        if receipt.owner_public_key != owner.public_key_bytes() {
            return Err(ReplError::EpochAuthority);
        }
        let mut proof = Self {
            document: receipt.document.clone(),
            receipt_hash: receipt.hash(),
            tenure_start_group_epoch: receipt.tenure_start_group_epoch,
            requester,
            request_nonce,
            owner_public_key: owner.public_key_bytes(),
            signature: [0; 64],
        };
        proof.signature = owner.sign(&proof.signature_hash())?;
        if proof.encode().len() > MAX_RECEIPT_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(proof)
    }

    fn unsigned_bytes(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.document.server_id)
            .expect("server id was bounded");
        e.put_u16(self.document.doc_type.tag());
        e.put_bytes(&self.document.logical_key)
            .expect("logical key was bounded");
        put_hash(&mut e, &self.receipt_hash);
        e.put_u64(self.tenure_start_group_epoch);
        e.put_bytes(self.requester.as_bytes())
            .expect("device identity fits");
        e.put_bytes(&self.request_nonce).expect("nonce fits");
        e.put_bytes(&self.owner_public_key).expect("owner key fits");
        e.finish()
    }

    fn signature_hash(&self) -> Hash32 {
        hash_parts(
            "catcoms-receipt-head-proof-sig:v1",
            &[&self.unsigned_bytes()],
        )
    }

    /// Verify freshness binding, current-owner authority, and the exact selected receipt.
    pub fn verify(
        &self,
        group: &ServerGroup,
        receipt: &Receipt,
        expected_requester: DeviceId,
        expected_nonce: &[u8; 16],
    ) -> Result<VerifiedReceipt, ReplError> {
        let owner = DeviceId::from_public_key_bytes(&self.owner_public_key);
        if self.document != receipt.document
            || self.document.server_id != group.group_id()
            || self.receipt_hash != receipt.hash()
            || self.tenure_start_group_epoch != receipt.tenure_start_group_epoch
            || self.requester != expected_requester
            || &self.request_nonce != expected_nonce
            || self.owner_public_key != receipt.owner_public_key
            || group.designated_committer() != Some(owner)
            || group.member_signature_key(&owner).as_deref() != Some(&self.owner_public_key)
            || !verify_with_public_bytes(
                &self.owner_public_key,
                &self.signature_hash(),
                &self.signature,
            )
        {
            return Err(ReplError::EpochAuthority);
        }
        receipt.verify_current_owner(group, self.tenure_start_group_epoch)
    }

    /// Canonical wire bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.unsigned_bytes()).expect("proof cap fits");
        e.put_bytes(&self.signature).expect("signature fits");
        e.finish()
    }

    /// Strict decode under the shared one-KiB receipt-record cap.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut outer = Decoder::new(bytes);
        let unsigned = outer.get_bytes().map_err(|_| ReplError::Malformed)?;
        let signature = get_fixed(&mut outer)?;
        outer.finish().map_err(|_| ReplError::Malformed)?;
        let mut d = Decoder::new(unsigned);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let server_id = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let logical_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let proof = Self {
            document: LogicalDocument::new(server_id, doc_type, logical_key)?,
            receipt_hash: get_fixed(&mut d)?,
            tenure_start_group_epoch: d.get_u64().map_err(|_| ReplError::Malformed)?,
            requester: DeviceId::from_bytes(get_fixed(&mut d)?),
            request_nonce: get_fixed(&mut d)?,
            owner_public_key: d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec(),
            signature,
        };
        d.finish().map_err(|_| ReplError::Malformed)?;
        if proof.owner_public_key.len() != 32 || proof.encode().as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(proof)
    }
}

impl Receipt {
    /// Sign a receipt. The caller must persist its owner journal before publishing the result.
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        document: LogicalDocument,
        closed_epoch: u64,
        close_record_hash: Hash32,
        seed_change_hash: Hash32,
        tenure_start_group_epoch: u64,
        inherited: InheritedCheckpoint,
        owner: &MlsDevice,
    ) -> Result<Self, ReplError> {
        if closed_epoch == u64::MAX || inherited.epoch() > closed_epoch {
            return Err(ReplError::EpochScope);
        }
        let owner_public_key = owner.public_key_bytes();
        let mut receipt = Self {
            tenure_id: tenure_id(
                &document.server_id,
                &owner_public_key,
                tenure_start_group_epoch,
            ),
            document,
            closed_epoch,
            close_record_hash,
            seed_change_hash,
            tenure_start_group_epoch,
            inherited,
            owner_public_key,
            signature: [0; 64],
        };
        receipt.signature = owner.sign(&receipt.signature_hash())?;
        if receipt.encode().len() > MAX_RECEIPT_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(receipt)
    }

    fn unsigned_bytes(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.document.server_id)
            .expect("server id was bounded");
        e.put_u16(self.document.doc_type.tag());
        e.put_bytes(&self.document.logical_key)
            .expect("logical key was bounded");
        e.put_u64(self.closed_epoch);
        put_hash(&mut e, &self.close_record_hash);
        put_hash(&mut e, &self.seed_change_hash);
        e.put_u64(self.tenure_start_group_epoch);
        put_hash(&mut e, &self.tenure_id);
        self.inherited.encode_into(&mut e);
        e.put_bytes(&self.owner_public_key)
            .expect("public key fits");
        e.finish()
    }

    fn signature_hash(&self) -> Hash32 {
        let unsigned = self.unsigned_bytes();
        hash_parts("catcoms-receipt-sig:v1", &[&unsigned])
    }

    /// Stable receipt identity.
    pub fn hash(&self) -> Hash32 {
        let unsigned = self.unsigned_bytes();
        hash_parts("catcoms-receipt:v1", &[&unsigned, &self.signature])
    }

    /// Verify scope, tenure derivation, current-owner authority, and signature.
    ///
    /// `expected_tenure_start_group_epoch` must come from locally observed owner succession or a
    /// fresh owner-authenticated head response. The signing key alone is insufficient when the
    /// same device becomes owner in two non-contiguous tenures: an old receipt would still verify.
    pub fn verify_current_owner(
        &self,
        group: &ServerGroup,
        expected_tenure_start_group_epoch: u64,
    ) -> Result<VerifiedReceipt, ReplError> {
        if self.document.server_id != group.group_id() {
            return Err(ReplError::EpochScope);
        }
        let owner = DeviceId::from_public_key_bytes(&self.owner_public_key);
        if group.designated_committer() != Some(owner)
            || group.member_signature_key(&owner).as_deref() != Some(&self.owner_public_key)
            || self.tenure_start_group_epoch != expected_tenure_start_group_epoch
            || self.tenure_start_group_epoch > group.epoch()
            || self.tenure_id
                != tenure_id(
                    &self.document.server_id,
                    &self.owner_public_key,
                    self.tenure_start_group_epoch,
                )
            || self.closed_epoch == u64::MAX
            || self.inherited.epoch() > self.closed_epoch
            || !verify_with_public_bytes(
                &self.owner_public_key,
                &self.signature_hash(),
                &self.signature,
            )
        {
            return Err(ReplError::EpochAuthority);
        }
        Ok(VerifiedReceipt {
            document: self.document.clone(),
            closed_epoch: self.closed_epoch,
            close_record_hash: self.close_record_hash,
            receipt_hash: self.hash(),
        })
    }

    /// Canonical receipt bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.unsigned_bytes())
            .expect("bounded receipt fits");
        e.put_bytes(&self.signature).expect("fixed signature fits");
        e.finish()
    }

    /// Strict receipt decoding with the one-KiB pre-parse cap.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut outer = Decoder::new(bytes);
        let unsigned = outer.get_bytes().map_err(|_| ReplError::Malformed)?;
        let signature = get_fixed(&mut outer)?;
        outer.finish().map_err(|_| ReplError::Malformed)?;
        let mut d = Decoder::new(unsigned);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let server_id = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let logical_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let document = LogicalDocument::new(server_id, doc_type, logical_key)?;
        let closed_epoch = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let close_record_hash = get_fixed(&mut d)?;
        let seed_change_hash = get_fixed(&mut d)?;
        let tenure_start_group_epoch = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let encoded_tenure = get_fixed(&mut d)?;
        let inherited = InheritedCheckpoint::decode_from(&mut d)?;
        let owner_public_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        d.finish().map_err(|_| ReplError::Malformed)?;
        if owner_public_key.len() != 32
            || encoded_tenure
                != tenure_id(
                    &document.server_id,
                    &owner_public_key,
                    tenure_start_group_epoch,
                )
        {
            return Err(ReplError::Malformed);
        }
        let receipt = Self {
            document,
            closed_epoch,
            close_record_hash,
            seed_change_hash,
            tenure_start_group_epoch,
            tenure_id: encoded_tenure,
            inherited,
            owner_public_key,
            signature,
        };
        if receipt.closed_epoch == u64::MAX
            || receipt.inherited.epoch() > receipt.closed_epoch
            || receipt.encode().as_slice() != bytes
        {
            return Err(ReplError::Malformed);
        }
        Ok(receipt)
    }
}

impl ReceiptRepair {
    /// Sign a repair after the application has durably persisted the choice it is about to make.
    pub fn sign(
        document: LogicalDocument,
        tenure_id: Hash32,
        mut receipt_hashes: [Hash32; 2],
        selected_receipt_hash: Hash32,
        repair_sequence: u64,
        owner: &MlsDevice,
    ) -> Result<Self, ReplError> {
        receipt_hashes.sort_unstable();
        if receipt_hashes[0] == receipt_hashes[1]
            || !receipt_hashes.contains(&selected_receipt_hash)
        {
            return Err(ReplError::Malformed);
        }
        let mut repair = Self {
            document,
            tenure_id,
            receipt_hashes,
            selected_receipt_hash,
            repair_sequence,
            owner_public_key: owner.public_key_bytes(),
            signature: [0; 64],
        };
        repair.signature = owner.sign(&repair.signature_hash())?;
        if repair.encode().len() > MAX_RECEIPT_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(repair)
    }

    fn unsigned_bytes(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.document.server_id)
            .expect("server id was bounded");
        e.put_u16(self.document.doc_type.tag());
        e.put_bytes(&self.document.logical_key)
            .expect("logical key was bounded");
        put_hash(&mut e, &self.tenure_id);
        put_hash(&mut e, &self.receipt_hashes[0]);
        put_hash(&mut e, &self.receipt_hashes[1]);
        put_hash(&mut e, &self.selected_receipt_hash);
        e.put_u64(self.repair_sequence);
        e.put_bytes(&self.owner_public_key).expect("owner key fits");
        e.finish()
    }

    fn signature_hash(&self) -> Hash32 {
        let unsigned = self.unsigned_bytes();
        hash_parts("catcoms-repair-sig:v1", &[&unsigned])
    }

    /// Stable repair record hash.
    pub fn hash(&self) -> Hash32 {
        let unsigned = self.unsigned_bytes();
        hash_parts("catcoms-repair:v1", &[&unsigned, &self.signature])
    }

    /// Verify document scope, current-owner authority, canonical hash order and signature.
    pub fn verify_current_owner(&self, group: &ServerGroup) -> Result<(), ReplError> {
        if self.document.server_id != group.group_id() {
            return Err(ReplError::EpochScope);
        }
        let owner = DeviceId::from_public_key_bytes(&self.owner_public_key);
        if self.receipt_hashes[0] >= self.receipt_hashes[1]
            || !self.receipt_hashes.contains(&self.selected_receipt_hash)
            || group.designated_committer() != Some(owner)
            || group.member_signature_key(&owner).as_deref() != Some(&self.owner_public_key)
            || !verify_with_public_bytes(
                &self.owner_public_key,
                &self.signature_hash(),
                &self.signature,
            )
        {
            return Err(ReplError::EpochAuthority);
        }
        Ok(())
    }

    /// Canonical wire bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.unsigned_bytes())
            .expect("bounded repair fits");
        e.put_bytes(&self.signature).expect("signature fits");
        e.finish()
    }

    /// Strict decoding under the receipt/repair one-KiB pre-parse cap.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut outer = Decoder::new(bytes);
        let unsigned = outer.get_bytes().map_err(|_| ReplError::Malformed)?;
        let signature = get_fixed(&mut outer)?;
        outer.finish().map_err(|_| ReplError::Malformed)?;
        let mut d = Decoder::new(unsigned);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let server_id = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let logical_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let document = LogicalDocument::new(server_id, doc_type, logical_key)?;
        let tenure_id = get_fixed(&mut d)?;
        let receipt_hashes = [get_fixed(&mut d)?, get_fixed(&mut d)?];
        let selected_receipt_hash = get_fixed(&mut d)?;
        let repair_sequence = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let owner_public_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        d.finish().map_err(|_| ReplError::Malformed)?;
        if owner_public_key.len() != 32
            || receipt_hashes[0] >= receipt_hashes[1]
            || !receipt_hashes.contains(&selected_receipt_hash)
        {
            return Err(ReplError::Malformed);
        }
        let repair = Self {
            document,
            tenure_id,
            receipt_hashes,
            selected_receipt_hash,
            repair_sequence,
            owner_public_key,
            signature,
        };
        if repair.encode().as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(repair)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TenureSelection {
    tenure_id: Hash32,
    tenure_start_group_epoch: u64,
    inherited: InheritedCheckpoint,
    owner_public_key: Vec<u8>,
}

impl From<&Receipt> for TenureSelection {
    fn from(receipt: &Receipt) -> Self {
        Self {
            tenure_id: receipt.tenure_id,
            tenure_start_group_epoch: receipt.tenure_start_group_epoch,
            inherited: receipt.inherited.clone(),
            owner_public_key: receipt.owner_public_key.clone(),
        }
    }
}

fn put_tenure(e: &mut Encoder, tenure: Option<&TenureSelection>) {
    match tenure {
        Some(tenure) => {
            e.put_u8(1);
            put_hash(e, &tenure.tenure_id);
            e.put_u64(tenure.tenure_start_group_epoch);
            tenure.inherited.encode_into(e);
            e.put_bytes(&tenure.owner_public_key)
                .expect("owner key fits");
        }
        None => {
            e.put_u8(0);
        }
    }
}

fn get_tenure(d: &mut Decoder<'_>) -> Result<Option<TenureSelection>, ReplError> {
    match d.get_u8().map_err(|_| ReplError::Malformed)? {
        0 => Ok(None),
        1 => {
            let tenure = TenureSelection {
                tenure_id: get_fixed(d)?,
                tenure_start_group_epoch: d.get_u64().map_err(|_| ReplError::Malformed)?,
                inherited: InheritedCheckpoint::decode_from(d)?,
                owner_public_key: d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec(),
            };
            if tenure.owner_public_key.len() != 32 {
                return Err(ReplError::Malformed);
            }
            Ok(Some(tenure))
        }
        _ => Err(ReplError::Malformed),
    }
}

fn put_receipt(e: &mut Encoder, receipt: Option<&Receipt>) {
    match receipt {
        Some(receipt) => {
            e.put_u8(1);
            e.put_bytes(&receipt.encode()).expect("receipt cap fits");
        }
        None => {
            e.put_u8(0);
        }
    }
}

fn get_receipt(d: &mut Decoder<'_>) -> Result<Option<Receipt>, ReplError> {
    match d.get_u8().map_err(|_| ReplError::Malformed)? {
        0 => Ok(None),
        1 => Ok(Some(Receipt::decode(
            d.get_bytes().map_err(|_| ReplError::Malformed)?,
        )?)),
        _ => Err(ReplError::Malformed),
    }
}

fn canonical_receipt_pair(a: Receipt, b: Receipt) -> (Receipt, Receipt) {
    if a.hash() < b.hash() {
        (a, b)
    } else {
        (b, a)
    }
}

/// Result of atomically ingesting a receipt and transitioning its epoch gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptIngest {
    /// First receipt or a strictly newer high-water receipt was installed.
    Advanced,
    /// Exact receipt already held.
    Duplicate,
    /// Valid but older receipt ignored below the local high-water.
    Stale,
    /// Same-tenure owner equivocation; the document must become read-only.
    Fault,
}

/// Constant-sized peer receipt state for one logical document.
#[derive(Clone, Debug, Default)]
pub struct ReceiptBook {
    document: Option<LogicalDocument>,
    tenure: Option<TenureSelection>,
    latest: Option<Receipt>,
    previous_until_installed: Option<Receipt>,
    fault: Option<(Receipt, Receipt)>,
    repair_sequence: u64,
}

impl ReceiptBook {
    /// Verify and ingest a receipt while atomically sealing its concrete epoch gate.
    ///
    /// `expected_tenure_start_group_epoch` comes from an observed succession or a fresh
    /// [`ReceiptHeadProof`]. Receipt state and the admission boundary change under the gate's one
    /// lock: an operation is therefore wholly before the seal or quarantined after it. A detected
    /// equivocation moves the gate to its persisted read-only `Fault` phase.
    pub fn ingest_and_seal(
        &mut self,
        receipt: Receipt,
        group: &ServerGroup,
        expected_tenure_start_group_epoch: u64,
        gate: &EpochGate,
    ) -> Result<(ReceiptIngest, Vec<AdmittedOperation>), ReplError> {
        let verified = receipt.verify_current_owner(group, expected_tenure_start_group_epoch)?;
        if gate.epoch != receipt.closed_epoch {
            return Err(ReplError::EpochScope);
        }

        // Work on a copy so every receipt-book rejection leaves both states unchanged. The final
        // swap happens inside the gate lock that excludes edit admission and receipt settlement.
        let mut next = self.clone();
        let outcome = next.ingest_verified(receipt)?;
        let operations = gate.transition_verified_receipt(&verified, outcome, || *self = next)?;
        Ok((outcome, operations))
    }

    fn ingest_verified(&mut self, receipt: Receipt) -> Result<ReceiptIngest, ReplError> {
        if self
            .document
            .as_ref()
            .is_some_and(|document| document != &receipt.document)
        {
            return Err(ReplError::EpochScope);
        }
        self.document
            .get_or_insert_with(|| receipt.document.clone());
        if self.fault.is_some() {
            // Fault is a terminal read-only state until a signed repair. Do not let a third
            // receipt mutate which evidence or provisional head survives based on delivery order.
            return Ok(ReceiptIngest::Fault);
        }
        let selection = TenureSelection::from(&receipt);
        if let Some(current) = &self.tenure {
            if current.tenure_id == selection.tenure_id && current != &selection {
                let prior = self.latest.clone().unwrap_or_else(|| receipt.clone());
                let evidence = canonical_receipt_pair(prior, receipt);
                self.tenure = Some(TenureSelection::from(&evidence.0));
                self.latest = Some(evidence.0.clone());
                self.fault = Some(evidence);
                return Ok(ReceiptIngest::Fault);
            }
        }
        if self
            .tenure
            .as_ref()
            .is_some_and(|current| current.tenure_id != selection.tenure_id)
        {
            self.previous_until_installed = self.latest.take();
            self.tenure = Some(selection);
        } else if self.tenure.is_none() {
            self.tenure = Some(selection);
        }

        if let Some(latest) = &self.latest {
            if latest.tenure_id == receipt.tenure_id {
                if latest.closed_epoch == receipt.closed_epoch {
                    if latest.hash() == receipt.hash() {
                        return Ok(ReceiptIngest::Duplicate);
                    }
                    let evidence = canonical_receipt_pair(latest.clone(), receipt);
                    self.latest = Some(evidence.0.clone());
                    self.fault = Some(evidence);
                    return Ok(ReceiptIngest::Fault);
                }
                if latest.closed_epoch > receipt.closed_epoch {
                    return Ok(ReceiptIngest::Stale);
                }
            }
            self.previous_until_installed = self.latest.take();
        }
        self.latest = Some(receipt);
        Ok(ReceiptIngest::Advanced)
    }

    /// Latest verified receipt, if any.
    pub fn latest(&self) -> Option<&Receipt> {
        self.latest.as_ref()
    }

    /// Whether owner equivocation has made the document read-only.
    pub fn is_faulted(&self) -> bool {
        self.fault.is_some()
    }

    /// Drop the predecessor receipt once the latest checkpoint is durably installed.
    pub fn mark_latest_installed(&mut self) {
        self.previous_until_installed = None;
    }

    /// Update receipt bookkeeping after the caller has persisted the losing checkpoint as a
    /// recovery snapshot. Returns that losing receipt so the caller can retain its evidence.
    /// Settlement orchestration must also move the associated gate out of `Fault`; that
    /// recovery-dependent transition is intentionally not part of this core primitive yet.
    pub fn apply_repair(
        &mut self,
        repair: &ReceiptRepair,
        group: &ServerGroup,
    ) -> Result<Receipt, ReplError> {
        repair.verify_current_owner(group)?;
        let (a, b) = self.fault.clone().ok_or(ReplError::ReceiptConflict)?;
        if repair.document != a.document
            || repair.document != b.document
            || repair.tenure_id != a.tenure_id
            || repair.tenure_id != b.tenure_id
            || repair.repair_sequence <= self.repair_sequence
        {
            return Err(ReplError::ReceiptConflict);
        }
        let mut hashes = [a.hash(), b.hash()];
        hashes.sort_unstable();
        if hashes != repair.receipt_hashes {
            return Err(ReplError::ReceiptConflict);
        }
        let (selected, losing) = if a.hash() == repair.selected_receipt_hash {
            (a, b)
        } else if b.hash() == repair.selected_receipt_hash {
            (b, a)
        } else {
            return Err(ReplError::ReceiptConflict);
        };
        self.tenure = Some(TenureSelection::from(&selected));
        self.latest = Some(selected);
        self.previous_until_installed = None;
        self.fault = None;
        self.repair_sequence = repair.repair_sequence;
        Ok(losing)
    }

    /// Canonical peer-local state. The caller vault-seals these bytes and re-verifies held
    /// receipts against the current group before using them as network authority after restore.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        let mut e = Encoder::new();
        e.put_u8(1);
        put_tenure(&mut e, self.tenure.as_ref());
        put_receipt(&mut e, self.latest.as_ref());
        put_receipt(&mut e, self.previous_until_installed.as_ref());
        match &self.fault {
            Some((a, b)) => {
                e.put_u8(1);
                e.put_bytes(&a.encode()).expect("receipt cap fits");
                e.put_bytes(&b.encode()).expect("receipt cap fits");
            }
            None => {
                e.put_u8(0);
            }
        }
        e.put_u64(self.repair_sequence);
        let bytes = e.finish();
        if bytes.len() > MAX_RECEIPT_BOOK_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }

    /// Restore peer-local receipt/fault state without trusting redundant derived fields.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_RECEIPT_BOOK_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let tenure = get_tenure(&mut d)?;
        let latest = get_receipt(&mut d)?;
        let previous_until_installed = get_receipt(&mut d)?;
        let fault = match d.get_u8().map_err(|_| ReplError::Malformed)? {
            0 => None,
            1 => Some((
                Receipt::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?,
                Receipt::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?,
            )),
            _ => return Err(ReplError::Malformed),
        };
        let repair_sequence = d.get_u64().map_err(|_| ReplError::Malformed)?;
        d.finish().map_err(|_| ReplError::Malformed)?;

        let document = latest.as_ref().map(|receipt| receipt.document.clone());
        if tenure.is_some() != latest.is_some() {
            return Err(ReplError::Malformed);
        }
        if let (Some(tenure), Some(latest)) = (&tenure, &latest) {
            if tenure != &TenureSelection::from(latest) {
                return Err(ReplError::Malformed);
            }
        }
        if let Some((a, b)) = &fault {
            if a.hash() >= b.hash()
                || a.document != b.document
                || a.tenure_id != b.tenure_id
                || latest
                    .as_ref()
                    .is_none_or(|latest| latest.hash() != a.hash() && latest.hash() != b.hash())
            {
                return Err(ReplError::Malformed);
            }
        }
        if [&previous_until_installed]
            .into_iter()
            .flatten()
            .chain(fault.as_ref().into_iter().flat_map(|(a, b)| [a, b]))
            .any(|receipt| document.as_ref() != Some(&receipt.document))
        {
            return Err(ReplError::Malformed);
        }
        Ok(Self {
            document,
            tenure,
            latest,
            previous_until_installed,
            fault,
            repair_sequence,
        })
    }
}

/// Crash-journaled owner state for producing receipts for one logical document.
///
/// `prepare` mutates the journal but does not authorize publication by itself: the caller must
/// atomically persist [`Self::encode`] before sending [`Self::in_flight`]. Restoring those bytes
/// after a crash yields exactly the same signed receipt for republication.
#[derive(Clone, Debug, Default)]
pub struct OwnerReceiptJournal {
    document: Option<LogicalDocument>,
    tenure: Option<TenureSelection>,
    high_water: Option<Receipt>,
    in_flight: Option<Receipt>,
}

impl OwnerReceiptJournal {
    /// Prepare one irrevocable decision. An exact retry is idempotent; any conflicting pending
    /// decision or inherited checkpoint is rejected.
    pub fn prepare(
        &mut self,
        receipt: Receipt,
        group: &ServerGroup,
        expected_tenure_start_group_epoch: u64,
    ) -> Result<(), ReplError> {
        receipt.verify_current_owner(group, expected_tenure_start_group_epoch)?;
        self.prepare_verified(receipt)
    }

    fn prepare_verified(&mut self, receipt: Receipt) -> Result<(), ReplError> {
        if self
            .document
            .as_ref()
            .is_some_and(|document| document != &receipt.document)
        {
            return Err(ReplError::EpochScope);
        }
        if let Some(pending) = &self.in_flight {
            return if pending.hash() == receipt.hash() {
                Ok(())
            } else {
                Err(ReplError::ReceiptConflict)
            };
        }
        let selection = TenureSelection::from(&receipt);
        let changes_tenure = self
            .tenure
            .as_ref()
            .is_some_and(|tenure| tenure != &selection);
        if (self.tenure.is_none() || changes_tenure)
            && (receipt.closed_epoch != receipt.inherited.epoch()
                || self.tenure.as_ref().is_some_and(|tenure| {
                    selection.tenure_start_group_epoch <= tenure.tenure_start_group_epoch
                }))
        {
            return Err(ReplError::ReceiptConflict);
        }
        if !changes_tenure {
            if let Some(high_water) = &self.high_water {
                if receipt.closed_epoch < high_water.closed_epoch {
                    return Err(ReplError::ReceiptConflict);
                }
                if receipt.closed_epoch == high_water.closed_epoch {
                    return if receipt.hash() == high_water.hash() {
                        Ok(())
                    } else {
                        Err(ReplError::ReceiptConflict)
                    };
                }
                if receipt.closed_epoch != high_water.closed_epoch + 1 {
                    return Err(ReplError::ReceiptConflict);
                }
            }
        }

        // All fallible consistency checks precede mutation. A rejected first decision must not
        // bind an otherwise-empty journal to an attacker's document or partial tenure.
        self.document
            .get_or_insert_with(|| receipt.document.clone());
        if changes_tenure {
            // A returning owner starts a distinct journal tenure. The new in-flight first receipt
            // is the persist-before-publish decision; the old high-water is no longer needed to
            // reproduce it after a crash.
            self.high_water = None;
            self.tenure = Some(selection.clone());
        } else if self.tenure.is_none() {
            self.tenure = Some(selection);
        }
        self.in_flight = Some(receipt);
        Ok(())
    }

    /// Decision that must be published (or republished after restore) once the journal is durable.
    pub fn in_flight(&self) -> Option<&Receipt> {
        self.in_flight.as_ref()
    }

    /// Mark the exact in-flight receipt published, advancing constant-sized high-water state.
    pub fn mark_published(&mut self, receipt_hash: Hash32) -> Result<(), ReplError> {
        let pending = self.in_flight.take().ok_or(ReplError::ReceiptConflict)?;
        if pending.hash() != receipt_hash {
            self.in_flight = Some(pending);
            return Err(ReplError::ReceiptConflict);
        }
        self.high_water = Some(pending);
        Ok(())
    }

    /// Canonical plaintext journal bytes; the application seals these in its vault.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(1);
        put_receipt(&mut e, self.high_water.as_ref());
        put_receipt(&mut e, self.in_flight.as_ref());
        put_tenure(&mut e, self.tenure.as_ref());
        e.finish()
    }

    /// Restore a journal. Receipt signatures are deliberately not re-authorized here because the
    /// bytes are vault-sealed owner-local state; callers still verify before network publication.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_OWNER_RECEIPT_JOURNAL_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let high_water = get_receipt(&mut d)?;
        let in_flight = get_receipt(&mut d)?;
        let tenure = get_tenure(&mut d)?;
        d.finish().map_err(|_| ReplError::Malformed)?;
        let journal = Self {
            document: high_water
                .as_ref()
                .or(in_flight.as_ref())
                .map(|receipt| receipt.document.clone()),
            high_water,
            in_flight,
            tenure,
        };
        if journal.document.is_some() != journal.tenure.is_some() {
            return Err(ReplError::Malformed);
        }
        for receipt in [&journal.high_water, &journal.in_flight]
            .into_iter()
            .flatten()
        {
            if journal.document.as_ref() != Some(&receipt.document)
                || journal.tenure.as_ref() != Some(&TenureSelection::from(receipt))
            {
                return Err(ReplError::Malformed);
            }
        }
        match (&journal.high_water, &journal.in_flight) {
            (None, Some(first)) if first.closed_epoch != first.inherited.epoch() => {
                return Err(ReplError::Malformed);
            }
            (Some(high_water), Some(in_flight))
                if in_flight.closed_epoch != high_water.closed_epoch + 1 =>
            {
                return Err(ReplError::Malformed);
            }
            _ => {}
        }
        Ok(journal)
    }
}

/// Minimal metadata charged by the epoch gate for one already signature-verified `SignedOp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmittedOperation {
    /// Whole signed-envelope hash (ordinary replication dedup identity).
    pub op_hash: Hash32,
    /// Author-bound domain-operation idempotency token.
    pub domain_op_id: Hash32,
    /// Verified outer author.
    pub author: DeviceId,
    /// Whole signed-envelope encoded size.
    pub encoded_len: usize,
}

/// Outcome of an operation racing the epoch lifecycle gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Newly accepted into the open epoch.
    Accepted,
    /// Already accepted earlier.
    Duplicate,
    /// Arrived after sealing and consumed one bounded quarantine slot.
    Quarantined,
    /// Arrived after sealing when the quarantine was already full.
    RejectedQuarantineFull,
}

/// Lifecycle state of one concrete epoch document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochPhase {
    /// Local and inbound operations may be admitted.
    Open,
    /// A verified receipt sealed the operation set; settlement is in progress.
    Closing,
    /// Checkpoint installed and old-epoch operations permanently rejected.
    Settled,
    /// Conflicting valid owner receipts require an owner-signed repair before work can resume.
    Fault,
}

#[derive(Debug)]
struct EpochGateInner {
    owner: DeviceId,
    phase: EpochPhase,
    receipt_hash: Option<Hash32>,
    operations: BTreeMap<Hash32, AdmittedOperation>,
    operation_ids: BTreeMap<Hash32, Hash32>,
    total_bytes: usize,
    by_device: BTreeMap<DeviceId, (usize, usize)>,
    quarantine: VecDeque<Hash32>,
}

/// One per-document mutex used by local edits, inbound ingest, and receipt settlement.
#[derive(Debug)]
pub struct EpochGate {
    document: LogicalDocument,
    doc_id: u128,
    epoch: u64,
    inner: Mutex<EpochGateInner>,
}

impl EpochGate {
    /// Create an open gate bound to one server, logical document and concrete epoch id.
    pub fn new(document: LogicalDocument, doc_id: u128, epoch: u64, owner: DeviceId) -> Self {
        Self {
            document,
            doc_id,
            epoch,
            inner: Mutex::new(EpochGateInner {
                owner,
                phase: EpochPhase::Open,
                receipt_hash: None,
                operations: BTreeMap::new(),
                operation_ids: BTreeMap::new(),
                total_bytes: 0,
                by_device: BTreeMap::new(),
                quarantine: VecDeque::new(),
            }),
        }
    }

    /// Epoch number protected by this gate.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Reject use of this gate as security state for another server or epoch document.
    pub(crate) fn verify_scope(
        &self,
        document: &LogicalDocument,
        doc_id: u128,
    ) -> Result<(), ReplError> {
        if &self.document != document || self.doc_id != doc_id {
            return Err(ReplError::EpochScope);
        }
        Ok(())
    }

    /// Current lifecycle phase.
    pub fn phase(&self) -> EpochPhase {
        self.inner.lock().expect("epoch gate poisoned").phase
    }

    /// Update the owner whose operations are exempt from the per-device share.
    ///
    /// Succession does not replace an open epoch, so this update uses the same gate as admission.
    /// Operations already charged to the former owner remain charged if that device keeps editing.
    pub fn update_owner(&self, owner: DeviceId) {
        self.inner.lock().expect("epoch gate poisoned").owner = owner;
    }

    /// Admit a local operation. Local callers never consume quarantine: their durable intent is
    /// rendered as an overlay and replayed into the successor instead.
    pub fn admit_local(&self, op: AdmittedOperation) -> Result<Admission, ReplError> {
        let mut inner = self.inner.lock().expect("epoch gate poisoned");
        if inner.phase != EpochPhase::Open {
            return Err(ReplError::EpochClosed);
        }
        let owner = inner.owner;
        Self::admit_open(&mut inner, owner, op)
    }

    /// Admit and commit one local document mutation while the lifecycle lock is still held.
    ///
    /// This is the only admission entry point used by [`EncryptedDoc`]. Keeping the state swap in
    /// the critical section prevents settlement from freezing admitted metadata before the
    /// corresponding Automerge change and signed log entry become visible.
    pub(crate) fn admit_local_and_commit<F>(
        &self,
        op: AdmittedOperation,
        commit: F,
    ) -> Result<Admission, ReplError>
    where
        F: FnOnce(),
    {
        let mut inner = self.inner.lock().expect("epoch gate poisoned");
        if inner.phase != EpochPhase::Open {
            return Err(ReplError::EpochClosed);
        }
        let owner = inner.owner;
        let admission = Self::admit_open(&mut inner, owner, op)?;
        if admission == Admission::Accepted {
            commit();
        }
        Ok(admission)
    }

    /// Admit an inbound operation or quarantine its hash if a receipt already sealed the epoch.
    pub fn admit_inbound(&self, op: AdmittedOperation) -> Result<Admission, ReplError> {
        let mut inner = self.inner.lock().expect("epoch gate poisoned");
        Self::admit_inbound_locked(&mut inner, op)
    }

    /// Admit and apply one inbound mutation under the same lifecycle lock used by sealing.
    pub(crate) fn admit_inbound_and_commit<F>(
        &self,
        op: AdmittedOperation,
        commit: F,
    ) -> Result<Admission, ReplError>
    where
        F: FnOnce(),
    {
        let mut inner = self.inner.lock().expect("epoch gate poisoned");
        let admission = Self::admit_inbound_locked(&mut inner, op)?;
        if admission == Admission::Accepted {
            commit();
        }
        Ok(admission)
    }

    fn admit_inbound_locked(
        inner: &mut EpochGateInner,
        op: AdmittedOperation,
    ) -> Result<Admission, ReplError> {
        if inner.operations.contains_key(&op.op_hash) {
            return Ok(Admission::Duplicate);
        }
        if let Some(existing_hash) = inner.operation_ids.get(&op.domain_op_id) {
            return if *existing_hash == op.op_hash {
                Ok(Admission::Duplicate)
            } else {
                Err(ReplError::IntentConflict)
            };
        }
        match inner.phase {
            EpochPhase::Open => {
                let owner = inner.owner;
                Self::admit_open(inner, owner, op)
            }
            EpochPhase::Closing => {
                if inner.quarantine.len() == MAX_QUARANTINED {
                    Ok(Admission::RejectedQuarantineFull)
                } else {
                    inner.quarantine.push_back(op.op_hash);
                    Ok(Admission::Quarantined)
                }
            }
            EpochPhase::Settled | EpochPhase::Fault => Err(ReplError::EpochClosed),
        }
    }

    fn admit_open(
        inner: &mut EpochGateInner,
        owner: DeviceId,
        op: AdmittedOperation,
    ) -> Result<Admission, ReplError> {
        if inner.operations.contains_key(&op.op_hash) {
            return Ok(Admission::Duplicate);
        }
        if let Some(existing_hash) = inner.operation_ids.get(&op.domain_op_id) {
            return if *existing_hash == op.op_hash {
                Ok(Admission::Duplicate)
            } else {
                Err(ReplError::IntentConflict)
            };
        }
        if op.encoded_len == 0
            || op.encoded_len > MAX_SIGNED_EPOCH_OP_BYTES
            || inner.operations.len() == MAX_EPOCH_OPERATIONS
            || inner.total_bytes.saturating_add(op.encoded_len) > MAX_EPOCH_BYTES
        {
            return Err(ReplError::EpochBound);
        }
        let per_device = inner.by_device.get(&op.author).copied().unwrap_or_default();
        if op.author != owner
            && (per_device.0 >= MAX_DEVICE_OPERATIONS
                || per_device.1.saturating_add(op.encoded_len) > MAX_DEVICE_BYTES)
        {
            return Err(ReplError::EpochBound);
        }
        inner.operations.insert(op.op_hash, op);
        inner.operation_ids.insert(op.domain_op_id, op.op_hash);
        inner.total_bytes += op.encoded_len;
        let entry = inner.by_device.entry(op.author).or_default();
        entry.0 += 1;
        entry.1 += op.encoded_len;
        Ok(Admission::Accepted)
    }

    /// Low-level authority-checked seal used when no receipt book is being installed.
    /// Production receipt ingest should use [`ReceiptBook::ingest_and_seal`] so the two persisted
    /// states cannot disagree even transiently in memory.
    pub fn seal_verified_receipt(
        &self,
        verified: &VerifiedReceipt,
    ) -> Result<Vec<AdmittedOperation>, ReplError> {
        self.transition_verified_receipt(verified, ReceiptIngest::Advanced, || {})
    }

    fn transition_verified_receipt<F>(
        &self,
        verified: &VerifiedReceipt,
        outcome: ReceiptIngest,
        commit_receipt_book: F,
    ) -> Result<Vec<AdmittedOperation>, ReplError>
    where
        F: FnOnce(),
    {
        if verified.document != self.document || verified.closed_epoch != self.epoch {
            return Err(ReplError::EpochScope);
        }
        let mut inner = self.inner.lock().expect("epoch gate poisoned");
        if outcome == ReceiptIngest::Stale {
            commit_receipt_book();
            return Ok(Vec::new());
        }
        if outcome == ReceiptIngest::Fault {
            match inner.phase {
                EpochPhase::Open | EpochPhase::Closing | EpochPhase::Fault => {
                    inner.phase = EpochPhase::Fault;
                    inner.receipt_hash = None;
                    commit_receipt_book();
                    return Ok(inner.operations.values().copied().collect());
                }
                EpochPhase::Settled => return Err(ReplError::ReceiptConflict),
            }
        }
        let receipt_hash = verified.receipt_hash;
        match inner.phase {
            EpochPhase::Open => {
                inner.phase = EpochPhase::Closing;
                inner.receipt_hash = Some(receipt_hash);
            }
            EpochPhase::Closing if inner.receipt_hash == Some(receipt_hash) => {}
            EpochPhase::Closing | EpochPhase::Settled | EpochPhase::Fault => {
                return Err(ReplError::ReceiptConflict);
            }
        }
        commit_receipt_book();
        Ok(inner.operations.values().copied().collect())
    }

    /// Finish settlement and permanently discard post-seal quarantined hashes.
    pub fn mark_settled(&self, receipt_hash: Hash32) -> Result<(), ReplError> {
        let mut inner = self.inner.lock().expect("epoch gate poisoned");
        if inner.phase != EpochPhase::Closing || inner.receipt_hash != Some(receipt_hash) {
            return Err(ReplError::ReceiptConflict);
        }
        inner.quarantine.clear();
        inner.phase = EpochPhase::Settled;
        Ok(())
    }

    /// Accepted operation hashes frozen by a seal, in deterministic order.
    pub fn accepted_hashes(&self) -> Vec<Hash32> {
        self.inner
            .lock()
            .expect("epoch gate poisoned")
            .operations
            .keys()
            .copied()
            .collect()
    }

    /// Number of post-seal operations awaiting deterministic rejection.
    pub fn quarantined_len(&self) -> usize {
        self.inner
            .lock()
            .expect("epoch gate poisoned")
            .quarantine
            .len()
    }

    /// Canonical gate state for atomic persistence beside the epoch log and snapshot.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        let inner = self.inner.lock().expect("epoch gate poisoned");
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.document.server_id)
            .map_err(|_| ReplError::EpochBound)?;
        e.put_u16(self.document.doc_type.tag());
        e.put_bytes(&self.document.logical_key)
            .map_err(|_| ReplError::EpochBound)?;
        e.put_u128(self.doc_id);
        e.put_u64(self.epoch);
        e.put_bytes(inner.owner.as_bytes())
            .expect("device identity fits");
        e.put_u8(match inner.phase {
            EpochPhase::Open => 1,
            EpochPhase::Closing => 2,
            EpochPhase::Settled => 3,
            EpochPhase::Fault => 4,
        });
        put_optional_hash(&mut e, inner.receipt_hash.as_ref());
        e.put_u32(u32::try_from(inner.operations.len()).map_err(|_| ReplError::EpochBound)?);
        for operation in inner.operations.values() {
            put_hash(&mut e, &operation.op_hash);
            put_hash(&mut e, &operation.domain_op_id);
            e.put_bytes(operation.author.as_bytes())
                .expect("device identity fits");
            e.put_u32(u32::try_from(operation.encoded_len).map_err(|_| ReplError::EpochBound)?);
        }
        e.put_u32(u32::try_from(inner.quarantine.len()).map_err(|_| ReplError::EpochBound)?);
        let mut quarantine: Vec<_> = inner.quarantine.iter().copied().collect();
        quarantine.sort_unstable();
        for operation_hash in &quarantine {
            put_hash(&mut e, operation_hash);
        }
        let bytes = e.finish();
        if bytes.len() > MAX_EPOCH_GATE_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }

    /// Restore lifecycle state after a crash, recomputing all accounting from operation metadata.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_EPOCH_GATE_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let server_id = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let logical_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let document = LogicalDocument::new(server_id, doc_type, logical_key)?;
        let doc_id = d.get_u128().map_err(|_| ReplError::Malformed)?;
        let epoch = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let owner = DeviceId::from_bytes(get_fixed(&mut d)?);
        let phase = match d.get_u8().map_err(|_| ReplError::Malformed)? {
            1 => EpochPhase::Open,
            2 => EpochPhase::Closing,
            3 => EpochPhase::Settled,
            4 => EpochPhase::Fault,
            _ => return Err(ReplError::Malformed),
        };
        let receipt_hash = get_optional_hash(&mut d)?;
        let operation_count = usize::try_from(d.get_u32().map_err(|_| ReplError::Malformed)?)
            .map_err(|_| ReplError::EpochBound)?;
        if operation_count > MAX_EPOCH_OPERATIONS {
            return Err(ReplError::EpochBound);
        }
        let mut inner = EpochGateInner {
            owner,
            phase: EpochPhase::Open,
            receipt_hash: None,
            operations: BTreeMap::new(),
            operation_ids: BTreeMap::new(),
            total_bytes: 0,
            by_device: BTreeMap::new(),
            quarantine: VecDeque::new(),
        };
        let mut prior_operation_hash = None;
        for _ in 0..operation_count {
            let operation = AdmittedOperation {
                op_hash: get_fixed(&mut d)?,
                domain_op_id: get_fixed(&mut d)?,
                author: DeviceId::from_bytes(get_fixed(&mut d)?),
                encoded_len: usize::try_from(d.get_u32().map_err(|_| ReplError::Malformed)?)
                    .map_err(|_| ReplError::EpochBound)?,
            };
            if prior_operation_hash.is_some_and(|prior| prior >= operation.op_hash) {
                return Err(ReplError::Malformed);
            }
            prior_operation_hash = Some(operation.op_hash);
            // Admission exemptions are evaluated against the owner at the time of each edit.
            // Ownership can change without replacing the open epoch, so replaying today's owner
            // rule here could reject a valid persisted set authored by the former owner. The vault
            // protects this state; recompute invariant totals and apply the current rule only to
            // operations arriving after restore.
            if operation.encoded_len == 0
                || operation.encoded_len > MAX_SIGNED_EPOCH_OP_BYTES
                || inner.operations.len() == MAX_EPOCH_OPERATIONS
                || inner.total_bytes.saturating_add(operation.encoded_len) > MAX_EPOCH_BYTES
                || inner.operations.contains_key(&operation.op_hash)
                || inner.operation_ids.contains_key(&operation.domain_op_id)
            {
                return Err(ReplError::Malformed);
            }
            inner.total_bytes += operation.encoded_len;
            let entry = inner.by_device.entry(operation.author).or_default();
            entry.0 += 1;
            entry.1 += operation.encoded_len;
            inner
                .operation_ids
                .insert(operation.domain_op_id, operation.op_hash);
            inner.operations.insert(operation.op_hash, operation);
        }
        let quarantine_count = usize::try_from(d.get_u32().map_err(|_| ReplError::Malformed)?)
            .map_err(|_| ReplError::EpochBound)?;
        if quarantine_count > MAX_QUARANTINED {
            return Err(ReplError::EpochBound);
        }
        let mut quarantine_set = std::collections::BTreeSet::new();
        let mut prior_quarantine_hash = None;
        for _ in 0..quarantine_count {
            let operation_hash = get_fixed(&mut d)?;
            if inner.operations.contains_key(&operation_hash)
                || !quarantine_set.insert(operation_hash)
                || prior_quarantine_hash.is_some_and(|prior| prior >= operation_hash)
            {
                return Err(ReplError::Malformed);
            }
            prior_quarantine_hash = Some(operation_hash);
            inner.quarantine.push_back(operation_hash);
        }
        d.finish().map_err(|_| ReplError::Malformed)?;
        match phase {
            EpochPhase::Open if receipt_hash.is_none() && inner.quarantine.is_empty() => {}
            EpochPhase::Closing if receipt_hash.is_some() => {}
            EpochPhase::Settled if receipt_hash.is_some() && inner.quarantine.is_empty() => {}
            EpochPhase::Fault if receipt_hash.is_none() => {}
            _ => return Err(ReplError::Malformed),
        }
        inner.phase = phase;
        inner.receipt_hash = receipt_hash;
        Ok(Self {
            document,
            doc_id,
            epoch,
            inner: Mutex::new(inner),
        })
    }
}

/// Why a bounded materialized version was retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RecoveryReason {
    /// Operations excluded by the receipted close.
    Excluded = 1,
    /// A previously selected branch was rewound.
    Rewound = 2,
    /// Conflict values overflowed the checkpoint's canonical cap.
    ConflictOverflow = 3,
    /// Losing side of a repaired receipt fault.
    Repair = 4,
}

impl RecoveryReason {
    fn decode(value: u8) -> Result<Self, ReplError> {
        match value {
            1 => Ok(Self::Excluded),
            2 => Ok(Self::Rewound),
            3 => Ok(Self::ConflictOverflow),
            4 => Ok(Self::Repair),
            _ => Err(ReplError::Malformed),
        }
    }
}

/// One deleted collection id retained for safe intent replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryTombstone {
    /// Random 16-byte element id (rendered as 32 lowercase hex characters at the UI contract).
    pub element_id: [u8; 16],
    /// Operation that created the tombstone.
    pub op_id: Hash32,
    /// Verified author of that operation.
    pub author: DeviceId,
}

/// Ordering/authorship metadata for one collection element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryElement {
    /// Stable element id.
    pub element_id: [u8; 16],
    /// Stable predecessor, or collection start.
    pub predecessor: Option<[u8; 16]>,
    /// Insertion operation id.
    pub op_id: Hash32,
    /// Verified inserting author.
    pub author: DeviceId,
}

/// One value of a bounded scalar or same-id insertion conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryConflictValue {
    /// Canonical type-specific value bytes.
    pub value: Vec<u8>,
    /// Operation defining this value.
    pub op_id: Hash32,
    /// Verified author.
    pub author: DeviceId,
}

/// Bounded conflict entry carried by a recovery snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryConflict {
    /// Canonical field name or element id bytes.
    pub target: Vec<u8>,
    /// At most four deterministic values.
    pub values: Vec<RecoveryConflictValue>,
}

/// Canonical, vault-sealed recovery materialization for one logical document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverySnapshot {
    /// Logical target.
    pub doc_type: DocType,
    /// Logical key.
    pub logical_key: Vec<u8>,
    /// Epoch or branch epoch represented.
    pub epoch: u64,
    /// Base close when the materialization is relative to a checkpoint.
    pub base_close_record_hash: Option<Hash32>,
    /// Why it was retained.
    pub reason: RecoveryReason,
    /// Type-specific canonical projection.
    pub projection: Vec<u8>,
    /// Deleted element metadata.
    pub tombstones: Vec<RecoveryTombstone>,
    /// Element order/authorship metadata.
    pub elements: Vec<RecoveryElement>,
    /// Bounded conflicts.
    pub conflicts: Vec<RecoveryConflict>,
    /// Operations already reflected in the projection.
    pub applied_ops: Vec<Hash32>,
}

impl RecoverySnapshot {
    /// Stable identity used by the staged-slot state machine.
    pub fn id(&self) -> Result<Hash32, ReplError> {
        let bytes = self.encode()?;
        Ok(hash_parts("catcoms-recovery:v1", &[&bytes]))
    }

    /// Canonical plaintext bytes; callers vault-seal before persistence.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        if !is_epoch_managed_doc_type(self.doc_type)
            || self.logical_key.is_empty()
            || self.logical_key.len() > MAX_LOGICAL_KEY_BYTES
            || self.tombstones.len() > MAX_RECOVERY_ITEMS
            || self.elements.len() > MAX_RECOVERY_ITEMS
            || self.conflicts.len() > MAX_RECOVERY_CONFLICTS
            || self.applied_ops.len() > MAX_RECOVERY_ITEMS
            || self.conflicts.iter().any(|conflict| {
                conflict.target.is_empty()
                    || conflict.target.len() > MAX_RECOVERY_TARGET_BYTES
                    || conflict.values.is_empty()
                    || conflict.values.len() > MAX_CONFLICT_VALUES
                    || conflict
                        .values
                        .iter()
                        .any(|value| value.value.len() > MAX_DOMAIN_OP_BYTES)
            })
            || self
                .tombstones
                .windows(2)
                .any(|pair| pair[0].element_id >= pair[1].element_id)
            || self
                .elements
                .windows(2)
                .any(|pair| pair[0].element_id >= pair[1].element_id)
            || self
                .conflicts
                .windows(2)
                .any(|pair| pair[0].target >= pair[1].target)
            || self.conflicts.iter().any(|conflict| {
                conflict
                    .values
                    .windows(2)
                    .any(|pair| pair[0].op_id >= pair[1].op_id)
            })
            || self.applied_ops.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(ReplError::EpochBound);
        }
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_u16(self.doc_type.tag());
        e.put_bytes(&self.logical_key)
            .map_err(|_| ReplError::EpochBound)?;
        e.put_u64(self.epoch);
        put_optional_hash(&mut e, self.base_close_record_hash.as_ref());
        e.put_u8(self.reason as u8);
        e.put_bytes(&self.projection)
            .map_err(|_| ReplError::EpochBound)?;
        e.put_u32(u32::try_from(self.tombstones.len()).expect("count bounded"));
        for tombstone in &self.tombstones {
            e.put_bytes(&tombstone.element_id).expect("fixed id fits");
            put_hash(&mut e, &tombstone.op_id);
            e.put_bytes(tombstone.author.as_bytes())
                .expect("fixed author fits");
        }
        e.put_u32(u32::try_from(self.elements.len()).expect("count bounded"));
        for element in &self.elements {
            e.put_bytes(&element.element_id).expect("fixed id fits");
            match &element.predecessor {
                Some(predecessor) => {
                    e.put_u8(1);
                    e.put_bytes(predecessor).expect("fixed id fits");
                }
                None => {
                    e.put_u8(0);
                }
            }
            put_hash(&mut e, &element.op_id);
            e.put_bytes(element.author.as_bytes())
                .expect("fixed author fits");
        }
        e.put_u32(u32::try_from(self.conflicts.len()).expect("count bounded"));
        for conflict in &self.conflicts {
            e.put_bytes(&conflict.target)
                .map_err(|_| ReplError::EpochBound)?;
            e.put_u8(u8::try_from(conflict.values.len()).expect("values bounded"));
            for value in &conflict.values {
                e.put_bytes(&value.value)
                    .map_err(|_| ReplError::EpochBound)?;
                put_hash(&mut e, &value.op_id);
                e.put_bytes(value.author.as_bytes())
                    .expect("fixed author fits");
            }
        }
        e.put_u32(u32::try_from(self.applied_ops.len()).expect("count bounded"));
        for op in &self.applied_ops {
            put_hash(&mut e, op);
        }
        let bytes = e.finish();
        if bytes.len() > MAX_RECOVERY_SNAPSHOT_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }

    /// Strict decoding with count checks before allocation.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_RECOVERY_SNAPSHOT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let tag = d.get_u16().map_err(|_| ReplError::Malformed)?;
        let doc_type = DocType::from_tag(tag).ok_or(ReplError::Malformed)?;
        let logical_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let epoch = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let base_close_record_hash = get_optional_hash(&mut d)?;
        let reason = RecoveryReason::decode(d.get_u8().map_err(|_| ReplError::Malformed)?)?;
        let projection = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();

        let tombstone_count = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if tombstone_count > MAX_RECOVERY_ITEMS {
            return Err(ReplError::EpochBound);
        }
        let mut tombstones = Vec::with_capacity(tombstone_count);
        for _ in 0..tombstone_count {
            tombstones.push(RecoveryTombstone {
                element_id: get_fixed(&mut d)?,
                op_id: get_fixed(&mut d)?,
                author: DeviceId::from_bytes(get_fixed(&mut d)?),
            });
        }

        let element_count = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if element_count > MAX_RECOVERY_ITEMS {
            return Err(ReplError::EpochBound);
        }
        let mut elements = Vec::with_capacity(element_count);
        for _ in 0..element_count {
            let element_id = get_fixed(&mut d)?;
            let predecessor = match d.get_u8().map_err(|_| ReplError::Malformed)? {
                0 => None,
                1 => Some(get_fixed(&mut d)?),
                _ => return Err(ReplError::Malformed),
            };
            elements.push(RecoveryElement {
                element_id,
                predecessor,
                op_id: get_fixed(&mut d)?,
                author: DeviceId::from_bytes(get_fixed(&mut d)?),
            });
        }

        let conflict_count = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if conflict_count > MAX_RECOVERY_CONFLICTS {
            return Err(ReplError::EpochBound);
        }
        let mut conflicts = Vec::with_capacity(conflict_count);
        for _ in 0..conflict_count {
            let target = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
            if target.is_empty() || target.len() > MAX_RECOVERY_TARGET_BYTES {
                return Err(ReplError::EpochBound);
            }
            let value_count = usize::from(d.get_u8().map_err(|_| ReplError::Malformed)?);
            if value_count == 0 || value_count > MAX_CONFLICT_VALUES {
                return Err(ReplError::EpochBound);
            }
            let mut values = Vec::with_capacity(value_count);
            for _ in 0..value_count {
                let value = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
                if value.len() > MAX_DOMAIN_OP_BYTES {
                    return Err(ReplError::EpochBound);
                }
                values.push(RecoveryConflictValue {
                    value,
                    op_id: get_fixed(&mut d)?,
                    author: DeviceId::from_bytes(get_fixed(&mut d)?),
                });
            }
            conflicts.push(RecoveryConflict { target, values });
        }

        let applied_count = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if applied_count > MAX_RECOVERY_ITEMS {
            return Err(ReplError::EpochBound);
        }
        let mut applied_ops = Vec::with_capacity(applied_count);
        for _ in 0..applied_count {
            applied_ops.push(get_fixed(&mut d)?);
        }
        d.finish().map_err(|_| ReplError::Malformed)?;
        let snapshot = Self {
            doc_type,
            logical_key,
            epoch,
            base_close_record_hash,
            reason,
            projection,
            tombstones,
            elements,
            conflicts,
            applied_ops,
        };
        if snapshot.encode()?.as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(snapshot)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StagedRecovery {
    snapshot: RecoverySnapshot,
    staged_at_ms: u64,
}

/// Result of staging or advancing a recovery transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryTransition {
    /// Snapshot moved directly into a retained slot.
    Promoted,
    /// Both retained slots were occupied; settlement must remain `Closing` until this deadline or
    /// explicit acknowledgement.
    EvictionPending {
        /// Snapshot that will be evicted.
        oldest_snapshot: Hash32,
        /// Incoming snapshot waiting in the staged slot.
        staged_snapshot: Hash32,
        /// Receiver-clock deadline.
        deadline_ms: u64,
    },
    /// No staged transition was ready to advance.
    Unchanged,
}

/// Two retained recovery slots plus one persisted staged slot.
#[derive(Clone, Debug, Default)]
pub struct RecoverySlots {
    retained: VecDeque<RecoverySnapshot>,
    staged: Option<StagedRecovery>,
}

impl RecoverySlots {
    /// Stage a snapshot. A free retained slot promotes synchronously; a full pair produces one
    /// bounded eviction-pending transition.
    pub fn stage(
        &mut self,
        snapshot: RecoverySnapshot,
        now_ms: u64,
    ) -> Result<RecoveryTransition, ReplError> {
        // Encode before mutation so an over-cap snapshot can never occupy the staged slot.
        snapshot.encode()?;
        let snapshot_id = snapshot.id()?;
        for held in self
            .retained
            .iter()
            .chain(self.staged.iter().map(|staged| &staged.snapshot))
        {
            if held.doc_type != snapshot.doc_type || held.logical_key != snapshot.logical_key {
                return Err(ReplError::EpochScope);
            }
            if held.id()? == snapshot_id {
                return if self
                    .staged
                    .as_ref()
                    .is_some_and(|staged| staged.snapshot.id().is_ok_and(|id| id == snapshot_id))
                {
                    self.pending_transition()
                } else {
                    Ok(RecoveryTransition::Unchanged)
                };
            }
        }
        if self.staged.is_some() {
            return Err(ReplError::RecoveryPending);
        }
        if self.retained.len() < 2 {
            self.retained.push_front(snapshot);
            return Ok(RecoveryTransition::Promoted);
        }
        let oldest_snapshot = self
            .retained
            .back()
            .expect("two retained snapshots have an oldest")
            .id()?;
        self.staged = Some(StagedRecovery {
            snapshot,
            staged_at_ms: now_ms,
        });
        Ok(RecoveryTransition::EvictionPending {
            oldest_snapshot,
            staged_snapshot: snapshot_id,
            deadline_ms: now_ms.saturating_add(RECOVERY_GRACE_MS),
        })
    }

    fn pending_transition(&self) -> Result<RecoveryTransition, ReplError> {
        let staged = self.staged.as_ref().ok_or(ReplError::RecoveryPending)?;
        let oldest_snapshot = self
            .retained
            .back()
            .ok_or(ReplError::RecoveryPending)?
            .id()?;
        Ok(RecoveryTransition::EvictionPending {
            oldest_snapshot,
            staged_snapshot: staged.snapshot.id()?,
            deadline_ms: staged.staged_at_ms.saturating_add(RECOVERY_GRACE_MS),
        })
    }

    /// Acknowledge the warning and promote the staged snapshot immediately.
    ///
    /// The UI must echo both ids from the warning. This prevents a delayed acknowledgement from
    /// authorizing a later, unrelated eviction after another transition.
    pub fn acknowledge_eviction(
        &mut self,
        expected_oldest_snapshot: Hash32,
        expected_staged_snapshot: Hash32,
    ) -> Result<RecoveryTransition, ReplError> {
        let actual_oldest = self
            .retained
            .back()
            .ok_or(ReplError::RecoveryPending)?
            .id()?;
        let actual_staged = self
            .staged
            .as_ref()
            .ok_or(ReplError::RecoveryPending)?
            .snapshot
            .id()?;
        if actual_oldest != expected_oldest_snapshot || actual_staged != expected_staged_snapshot {
            return Err(ReplError::RecoveryPending);
        }
        self.promote_staged()
    }

    /// Advance after seven receiver-clock days; otherwise leave the warning pending.
    pub fn advance_time(&mut self, now_ms: u64) -> Result<RecoveryTransition, ReplError> {
        let Some(staged) = &self.staged else {
            return Ok(RecoveryTransition::Unchanged);
        };
        if now_ms < staged.staged_at_ms.saturating_add(RECOVERY_GRACE_MS) {
            return Ok(RecoveryTransition::Unchanged);
        }
        self.promote_staged()
    }

    fn promote_staged(&mut self) -> Result<RecoveryTransition, ReplError> {
        let Some(staged) = self.staged.take() else {
            return Ok(RecoveryTransition::Unchanged);
        };
        if self.retained.len() == 2 {
            self.retained.pop_back();
        }
        self.retained.push_front(staged.snapshot);
        debug_assert!(self.retained.len() <= 2);
        Ok(RecoveryTransition::Promoted)
    }

    /// Newest-first retained snapshots.
    pub fn retained(&self) -> impl ExactSizeIterator<Item = &RecoverySnapshot> {
        self.retained.iter()
    }

    /// Currently staged snapshot, if settlement is waiting for eviction.
    pub fn staged(&self) -> Option<&RecoverySnapshot> {
        self.staged.as_ref().map(|staged| &staged.snapshot)
    }

    /// Canonical plaintext transition state; callers vault-seal before persistence.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        if self.retained.len() > 2 || (self.staged.is_some() && self.retained.len() != 2) {
            return Err(ReplError::Malformed);
        }
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_u8(u8::try_from(self.retained.len()).expect("retained slots bounded"));
        for snapshot in &self.retained {
            e.put_bytes(&snapshot.encode()?)
                .map_err(|_| ReplError::EpochBound)?;
        }
        match &self.staged {
            Some(staged) => {
                e.put_u8(1);
                e.put_u64(staged.staged_at_ms);
                e.put_bytes(&staged.snapshot.encode()?)
                    .map_err(|_| ReplError::EpochBound)?;
            }
            None => {
                e.put_u8(0);
            }
        }
        let bytes = e.finish();
        if bytes.len() > MAX_RECOVERY_SLOTS_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }

    /// Restore a transition after a crash, preserving the exact staged step.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_RECOVERY_SLOTS_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let retained_count = usize::from(d.get_u8().map_err(|_| ReplError::Malformed)?);
        if retained_count > 2 {
            return Err(ReplError::Malformed);
        }
        let mut retained = VecDeque::with_capacity(retained_count);
        for _ in 0..retained_count {
            retained.push_back(RecoverySnapshot::decode(
                d.get_bytes().map_err(|_| ReplError::Malformed)?,
            )?);
        }
        let staged = match d.get_u8().map_err(|_| ReplError::Malformed)? {
            0 => None,
            1 => Some(StagedRecovery {
                staged_at_ms: d.get_u64().map_err(|_| ReplError::Malformed)?,
                snapshot: RecoverySnapshot::decode(
                    d.get_bytes().map_err(|_| ReplError::Malformed)?,
                )?,
            }),
            _ => return Err(ReplError::Malformed),
        };
        d.finish().map_err(|_| ReplError::Malformed)?;
        let slots = Self { retained, staged };
        let mut scope: Option<(DocType, &[u8])> = None;
        let mut ids = BTreeSet::new();
        for snapshot in slots
            .retained
            .iter()
            .chain(slots.staged.iter().map(|staged| &staged.snapshot))
        {
            match scope {
                Some((doc_type, logical_key))
                    if doc_type != snapshot.doc_type || logical_key != snapshot.logical_key =>
                {
                    return Err(ReplError::Malformed);
                }
                None => scope = Some((snapshot.doc_type, &snapshot.logical_key)),
                Some(_) => {}
            }
            if !ids.insert(snapshot.id()?) {
                return Err(ReplError::Malformed);
            }
        }
        if slots.encode()?.as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(slots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn sample_snapshot(epoch: u64) -> RecoverySnapshot {
        RecoverySnapshot {
            doc_type: DocType::StudioObject,
            logical_key: b"score-1".to_vec(),
            epoch,
            base_close_record_hash: None,
            reason: RecoveryReason::Excluded,
            projection: vec![epoch as u8],
            tombstones: Vec::new(),
            elements: Vec::new(),
            conflicts: Vec::new(),
            applied_ops: Vec::new(),
        }
    }

    #[test]
    fn domain_operation_roundtrips_and_is_author_bound() {
        let op = DomainOp {
            nonce: [7; 16],
            doc_type: DocType::StudioObject,
            logical_key: b"score-1".to_vec(),
            body: br#"{"kind":"set_cell","step":0,"row":0}"#.to_vec(),
        };
        let encoded = op.encode().unwrap();
        assert_eq!(DomainOp::decode(&encoded).unwrap(), op);
        assert_ne!(
            op.id(&DeviceId::from_bytes([1; 32])),
            op.id(&DeviceId::from_bytes([2; 32]))
        );
    }

    #[test]
    fn derivations_match_the_v1_golden_vectors() {
        let logical_key = b"score-17";
        assert_eq!(
            format!("{:032x}", epoch_zero_id(DocType::StudioObject, logical_key)),
            "933a6edd06fa00f09cbcb15ededb8b22"
        );
        assert_eq!(
            format!(
                "{:032x}",
                epoch_id(DocType::StudioObject, logical_key, 1, &[0x11; 32])
            ),
            "be0be92f632c9e75cd029d4eb0aace38"
        );
        assert_eq!(
            hex(&tenure_id(b"server", &(0u8..32).collect::<Vec<_>>(), 42)),
            "7712d092223261d6b841e250d09d3a88186b99052fda6fee2ed21b88c2dea820"
        );
        let op = DomainOp {
            nonce: [0x33; 16],
            doc_type: DocType::StudioObject,
            logical_key: logical_key.to_vec(),
            body: Vec::new(),
        };
        assert_eq!(
            hex(&op.id(&DeviceId::from_bytes([0x22; 32]))),
            "17adac49d4daff2b2903da816f0c2cfd2f64600ae98c63bec7c41d8d480d25b1"
        );
    }

    #[test]
    fn identical_logical_key_bytes_are_separated_by_document_type() {
        assert_ne!(
            epoch_zero_id(DocType::StudioObject, b"same-key"),
            epoch_zero_id(DocType::PostReplies, b"same-key")
        );
    }

    #[test]
    fn ten_thousand_receipts_keep_owner_and_peer_state_constant_sized() {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let document = LogicalDocument::new(
            group.group_id(),
            DocType::StudioObject,
            b"score-rotation".to_vec(),
        )
        .unwrap();
        let mut journal = OwnerReceiptJournal::default();
        let mut book = ReceiptBook::default();
        let mut journal_len = None;
        let mut book_len = None;

        for epoch in 0..10_000u64 {
            let signed = Receipt::sign(
                document.clone(),
                epoch,
                [epoch as u8; 32],
                [epoch.wrapping_add(1) as u8; 32],
                group.epoch(),
                InheritedCheckpoint::EpochZero,
                &owner,
            )
            .unwrap();
            // Public ingress verifies each signature. This state-size regression operates below
            // that already-covered boundary so its 10,000 iterations remain a fast CI invariant.
            journal.prepare_verified(signed.clone()).unwrap();
            journal.mark_published(signed.hash()).unwrap();
            assert_eq!(
                book.ingest_verified(signed).unwrap(),
                ReceiptIngest::Advanced
            );
            book.mark_latest_installed();
            let encoded_journal = journal.encode();
            let encoded_book = book.encode().unwrap();
            assert_eq!(
                *journal_len.get_or_insert(encoded_journal.len()),
                encoded_journal.len()
            );
            assert_eq!(
                *book_len.get_or_insert(encoded_book.len()),
                encoded_book.len()
            );
        }
    }

    #[test]
    fn recovery_slots_roundtrip_the_pending_third_snapshot() {
        let mut slots = RecoverySlots::default();
        assert_eq!(
            slots.stage(sample_snapshot(1), 10).unwrap(),
            RecoveryTransition::Promoted
        );
        assert_eq!(
            slots.stage(sample_snapshot(2), 20).unwrap(),
            RecoveryTransition::Promoted
        );
        assert!(matches!(
            slots.stage(sample_snapshot(3), 30).unwrap(),
            RecoveryTransition::EvictionPending { .. }
        ));
        let mut restored = RecoverySlots::decode(&slots.encode().unwrap()).unwrap();
        assert_eq!(restored.retained().len(), 2);
        assert_eq!(restored.staged().unwrap().epoch, 3);
        assert_eq!(
            restored.advance_time(30 + RECOVERY_GRACE_MS - 1).unwrap(),
            RecoveryTransition::Unchanged
        );
        assert_eq!(
            restored.advance_time(30 + RECOVERY_GRACE_MS).unwrap(),
            RecoveryTransition::Promoted
        );
        assert_eq!(
            restored
                .retained()
                .map(|snapshot| snapshot.epoch)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert!(restored.staged().is_none());
    }
}
