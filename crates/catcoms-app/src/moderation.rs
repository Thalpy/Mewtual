//! Signed, group-bound moderation records.
//!
//! The CRDT op envelope already proves which device authored a delta, but that attribution is not
//! available from the materialized Automerge tree. Moderation history has to remain independently
//! inspectable after catch-up/snapshot, so every semantic record carries a second, explicit device
//! signature over all of its fields and the server group id. This proves the attestation's signer;
//! it does **not** close the honest-client role-authorization residual documented as R7.

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, ScalarValue, Value, ROOT};
use catcoms_crypto::{verify_with_public_bytes, DeviceId};
use catcoms_wire::Encoder;

use crate::{int_field, str_field, AppError};

/// One moderation document per server.
pub(crate) const MODERATION_DOC: u128 = 0;
/// A moderator's explanation is replicated to every member; keep it useful but bounded.
pub const MAX_MOD_REASON_BYTES: usize = 2 * 1024;
/// Evidence snapshots share the chat op path. A cap prevents a forged legacy message from turning
/// one click into an unbounded replicated record.
pub const MAX_MOD_EVIDENCE_BYTES: usize = 32 * 1024;
/// A case stays reviewable without becoming an amplification vehicle.
pub const MAX_MOD_EVIDENCE_IDS: usize = 32;

const EVENT_PREFIX: &str = "e:";
const VOTE_PREFIX: &str = "v:";
const DOMAIN_EVENT: &str = "catcoms/moderation-event/v1";
const DOMAIN_VOTE: &str = "catcoms/moderation-vote/v1";

const ID: &str = "id";
const KIND: &str = "kind";
const ACTOR: &str = "actor";
const SIGNER: &str = "signer";
const TARGET: &str = "target";
const CHANNEL: &str = "channel";
const MESSAGE_ID: &str = "message_id";
const MESSAGE_TEXT: &str = "message_text";
const MESSAGE_TS: &str = "message_ts";
const REASON: &str = "reason";
const EVIDENCE: &str = "evidence";
const CASE_ID: &str = "case_id";
const OUTCOME: &str = "outcome";
const TS: &str = "ts";
const PUBLIC_KEY: &str = "public_key";
const SIGNATURE: &str = "signature";
const CHOICE: &str = "choice";

/// An immutable public moderation attestation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModerationEvent {
    pub id: String,
    /// `warning`, `kick_case`, or `case_resolution`.
    pub kind: String,
    /// Member identity (an owner-certified origin for a linked signer device).
    pub actor: String,
    /// Concrete device fingerprint whose key signed this record.
    pub signer: String,
    pub target: String,
    /// Decimal `u128` channel id for warning evidence; empty otherwise.
    pub channel: String,
    pub message_id: String,
    pub message_text: String,
    pub message_ts: u64,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub case_id: String,
    /// `dismissed`, `removed`, or `remove_failed` on a resolution.
    pub outcome: String,
    pub ts: u64,
    /// The embedded signature is valid and names `signer`. The Server layer additionally binds
    /// `signer` to `actor` through the certified device registry.
    pub signature_valid: bool,
    /// Current-role interpretation supplied by the Server layer. It is deliberately not folded
    /// into `signature_valid`: current authority is not a proof of historical authority.
    pub authorized: bool,
    pub(crate) public_key: Vec<u8>,
    pub(crate) signature: Vec<u8>,
}

/// One identity's latest signed advisory vote on a kick case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModerationVote {
    pub case_id: String,
    pub voter: String,
    pub signer: String,
    pub yes: bool,
    pub ts: u64,
    pub signature_valid: bool,
    /// Derived by the Server reader: the signature belongs to a current member identity. Kept
    /// separate from signature validity so a departed member's historical vote stays attributable
    /// without counting toward a live case.
    pub eligible: bool,
    pub(crate) public_key: Vec<u8>,
    pub(crate) signature: Vec<u8>,
}

/// Materialized moderation document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModerationState {
    pub events: Vec<ModerationEvent>,
    pub votes: Vec<ModerationVote>,
}

fn valid_event_kind(kind: &str) -> bool {
    matches!(kind, "warning" | "kick_case" | "case_resolution")
}

fn valid_outcome(kind: &str, outcome: &str) -> bool {
    match kind {
        "case_resolution" => matches!(outcome, "dismissed" | "removed" | "remove_failed"),
        _ => outcome.is_empty(),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn event_payload(group_id: &[u8], e: &ModerationEvent) -> Result<Vec<u8>, AppError> {
    let mut out = Encoder::with_capacity(512 + e.message_text.len() + e.reason.len());
    out.put_str(DOMAIN_EVENT)
        .and_then(|e2| e2.put_bytes(group_id))
        .and_then(|e2| e2.put_str(&e.id))
        .and_then(|e2| e2.put_str(&e.kind))
        .and_then(|e2| e2.put_str(&e.actor))
        .and_then(|e2| e2.put_str(&e.signer))
        .and_then(|e2| e2.put_str(&e.target))
        .and_then(|e2| e2.put_str(&e.channel))
        .and_then(|e2| e2.put_str(&e.message_id))
        .and_then(|e2| e2.put_str(&e.message_text))
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    out.put_u64(e.message_ts);
    out.put_str(&e.reason)
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    out.put_u32(e.evidence_ids.len() as u32);
    for id in &e.evidence_ids {
        out.put_str(id)
            .map_err(|error| AppError::Invalid(error.to_string()))?;
    }
    out.put_str(&e.case_id)
        .and_then(|e2| e2.put_str(&e.outcome))
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    out.put_u64(e.ts);
    out.put_bytes(&e.public_key)
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    Ok(out.finish())
}

fn vote_payload(group_id: &[u8], vote: &ModerationVote) -> Result<Vec<u8>, AppError> {
    let mut out = Encoder::with_capacity(256);
    out.put_str(DOMAIN_VOTE)
        .and_then(|e| e.put_bytes(group_id))
        .and_then(|e| e.put_str(&vote.case_id))
        .and_then(|e| e.put_str(&vote.voter))
        .and_then(|e| e.put_str(&vote.signer))
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    out.put_u8(u8::from(vote.yes)).put_u64(vote.ts);
    out.put_bytes(&vote.public_key)
        .map_err(|error| AppError::Invalid(error.to_string()))?;
    Ok(out.finish())
}

pub(crate) fn validate_event_shape(e: &ModerationEvent) -> Result<(), AppError> {
    if !valid_id(&e.id) || !valid_event_kind(&e.kind) || !valid_outcome(&e.kind, &e.outcome) {
        return Err(AppError::Invalid("invalid moderation event shape".into()));
    }
    if !valid_id(&e.actor) || !valid_id(&e.signer) {
        return Err(AppError::Invalid("invalid moderation signer".into()));
    }
    if !e.target.is_empty() && !valid_id(&e.target) {
        return Err(AppError::Invalid("invalid moderation target".into()));
    }
    if e.reason.len() > MAX_MOD_REASON_BYTES || e.message_text.len() > MAX_MOD_EVIDENCE_BYTES {
        return Err(AppError::Invalid("moderation evidence is too large".into()));
    }
    if e.evidence_ids.len() > MAX_MOD_EVIDENCE_IDS || e.evidence_ids.iter().any(|id| !valid_id(id))
    {
        return Err(AppError::Invalid("invalid moderation evidence list".into()));
    }
    match e.kind.as_str() {
        "warning"
            if e.target.is_empty()
                || e.channel.parse::<u128>().is_err()
                || !valid_id(&e.message_id)
                || e.reason.trim().is_empty() =>
        {
            Err(AppError::Invalid(
                "warning is missing message evidence or reason".into(),
            ))
        }
        "kick_case"
            if e.target.is_empty() || e.reason.trim().is_empty() || !e.case_id.is_empty() =>
        {
            Err(AppError::Invalid(
                "kick case is missing its target or reason".into(),
            ))
        }
        "case_resolution" if !valid_id(&e.case_id) || e.target.is_empty() => Err(
            AppError::Invalid("case resolution is missing its case or target".into()),
        ),
        _ => Ok(()),
    }
}

pub(crate) fn sign_event(
    group_id: &[u8],
    mut event: ModerationEvent,
    sign: impl FnOnce(&[u8]) -> Result<[u8; 64], AppError>,
) -> Result<ModerationEvent, AppError> {
    validate_event_shape(&event)?;
    event.signature = sign(&event_payload(group_id, &event)?)?.to_vec();
    event.signature_valid = true;
    Ok(event)
}

pub(crate) fn sign_vote(
    group_id: &[u8],
    mut vote: ModerationVote,
    sign: impl FnOnce(&[u8]) -> Result<[u8; 64], AppError>,
) -> Result<ModerationVote, AppError> {
    if !valid_id(&vote.case_id) || !valid_id(&vote.voter) || !valid_id(&vote.signer) {
        return Err(AppError::Invalid("invalid moderation vote".into()));
    }
    vote.signature = sign(&vote_payload(group_id, &vote)?)?.to_vec();
    vote.signature_valid = true;
    Ok(vote)
}

fn put_bytes(
    doc: &mut AutoCommit,
    obj: &ObjId,
    key: &str,
    value: &[u8],
) -> Result<(), automerge::AutomergeError> {
    doc.put(obj, key, ScalarValue::Bytes(value.to_vec()))
}

pub(crate) fn write_event(
    doc: &mut AutoCommit,
    e: &ModerationEvent,
) -> Result<(), automerge::AutomergeError> {
    let key = format!("{EVENT_PREFIX}{}", e.id);
    write_event_at_key(doc, &key, e)
}

/// Write using an explicit CRDT address. Production callers always use the signed id-derived
/// address above; accepting the key here keeps the alias-rejection path directly testable.
fn write_event_at_key(
    doc: &mut AutoCommit,
    key: &str,
    e: &ModerationEvent,
) -> Result<(), automerge::AutomergeError> {
    let obj = doc.put_object(ROOT, key, ObjType::Map)?;
    doc.put(&obj, ID, e.id.as_str())?;
    doc.put(&obj, KIND, e.kind.as_str())?;
    doc.put(&obj, ACTOR, e.actor.as_str())?;
    doc.put(&obj, SIGNER, e.signer.as_str())?;
    doc.put(&obj, TARGET, e.target.as_str())?;
    doc.put(&obj, CHANNEL, e.channel.as_str())?;
    doc.put(&obj, MESSAGE_ID, e.message_id.as_str())?;
    doc.put(&obj, MESSAGE_TEXT, e.message_text.as_str())?;
    doc.put(&obj, MESSAGE_TS, e.message_ts as i64)?;
    doc.put(&obj, REASON, e.reason.as_str())?;
    doc.put(&obj, EVIDENCE, e.evidence_ids.join(","))?;
    doc.put(&obj, CASE_ID, e.case_id.as_str())?;
    doc.put(&obj, OUTCOME, e.outcome.as_str())?;
    doc.put(&obj, TS, e.ts as i64)?;
    put_bytes(doc, &obj, PUBLIC_KEY, &e.public_key)?;
    put_bytes(doc, &obj, SIGNATURE, &e.signature)?;
    Ok(())
}

pub(crate) fn write_vote(
    doc: &mut AutoCommit,
    vote: &ModerationVote,
) -> Result<(), automerge::AutomergeError> {
    let key = format!("{VOTE_PREFIX}{}:{}", vote.case_id, vote.voter);
    let obj = doc.put_object(ROOT, key, ObjType::Map)?;
    doc.put(&obj, CASE_ID, vote.case_id.as_str())?;
    doc.put(&obj, ACTOR, vote.voter.as_str())?;
    doc.put(&obj, SIGNER, vote.signer.as_str())?;
    doc.put(&obj, CHOICE, vote.yes)?;
    doc.put(&obj, TS, vote.ts as i64)?;
    put_bytes(doc, &obj, PUBLIC_KEY, &vote.public_key)?;
    put_bytes(doc, &obj, SIGNATURE, &vote.signature)?;
    Ok(())
}

fn bytes_field(doc: &AutoCommit, obj: &ObjId, key: &str) -> Vec<u8> {
    match doc.get(obj, key) {
        Ok(Some((Value::Scalar(value), _))) => match value.as_ref() {
            ScalarValue::Bytes(bytes) => bytes.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn bool_field(doc: &AutoCommit, obj: &ObjId, key: &str) -> bool {
    match doc.get(obj, key) {
        Ok(Some((Value::Scalar(value), _))) => matches!(value.as_ref(), ScalarValue::Boolean(true)),
        _ => false,
    }
}

fn event_from_obj(doc: &AutoCommit, obj: &ObjId, group_id: &[u8]) -> Option<ModerationEvent> {
    let evidence = str_field(doc, obj, EVIDENCE);
    let mut event = ModerationEvent {
        id: str_field(doc, obj, ID),
        kind: str_field(doc, obj, KIND),
        actor: str_field(doc, obj, ACTOR),
        signer: str_field(doc, obj, SIGNER),
        target: str_field(doc, obj, TARGET),
        channel: str_field(doc, obj, CHANNEL),
        message_id: str_field(doc, obj, MESSAGE_ID),
        message_text: str_field(doc, obj, MESSAGE_TEXT),
        message_ts: int_field(doc, obj, MESSAGE_TS),
        reason: str_field(doc, obj, REASON),
        evidence_ids: evidence
            .split(',')
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect(),
        case_id: str_field(doc, obj, CASE_ID),
        outcome: str_field(doc, obj, OUTCOME),
        ts: int_field(doc, obj, TS),
        signature_valid: false,
        authorized: false,
        public_key: bytes_field(doc, obj, PUBLIC_KEY),
        signature: bytes_field(doc, obj, SIGNATURE),
    };
    if validate_event_shape(&event).is_err()
        || event.public_key.len() != 32
        || event.signature.len() != 64
    {
        return None;
    }
    let signer_id = DeviceId::from_public_key_bytes(&event.public_key);
    let signature: [u8; 64] = event.signature.as_slice().try_into().ok()?;
    event.signature_valid = crate::fingerprint(&signer_id) == event.signer
        && verify_with_public_bytes(
            &event.public_key,
            &event_payload(group_id, &event).ok()?,
            &signature,
        );
    Some(event)
}

fn vote_from_obj(doc: &AutoCommit, obj: &ObjId, group_id: &[u8]) -> Option<ModerationVote> {
    let mut vote = ModerationVote {
        case_id: str_field(doc, obj, CASE_ID),
        voter: str_field(doc, obj, ACTOR),
        signer: str_field(doc, obj, SIGNER),
        yes: bool_field(doc, obj, CHOICE),
        ts: int_field(doc, obj, TS),
        signature_valid: false,
        eligible: false,
        public_key: bytes_field(doc, obj, PUBLIC_KEY),
        signature: bytes_field(doc, obj, SIGNATURE),
    };
    if !valid_id(&vote.case_id)
        || !valid_id(&vote.voter)
        || !valid_id(&vote.signer)
        || vote.public_key.len() != 32
        || vote.signature.len() != 64
    {
        return None;
    }
    let signer_id = DeviceId::from_public_key_bytes(&vote.public_key);
    let signature: [u8; 64] = vote.signature.as_slice().try_into().ok()?;
    vote.signature_valid = crate::fingerprint(&signer_id) == vote.signer
        && verify_with_public_bytes(
            &vote.public_key,
            &vote_payload(group_id, &vote).ok()?,
            &signature,
        );
    Some(vote)
}

/// Read every validly-shaped candidate. Signature failures remain visible with
/// `signature_valid=false` so the UI can say why it ignored a forged record.
pub(crate) fn read_state(doc: &AutoCommit, group_id: &[u8]) -> ModerationState {
    let mut state = ModerationState::default();
    for key in doc.keys(ROOT) {
        let candidates = doc.get_all(ROOT, &key).unwrap_or_default();
        for (value, obj) in candidates {
            if !matches!(value, Value::Object(ObjType::Map)) {
                continue;
            }
            if key.starts_with(EVENT_PREFIX) {
                if let Some(event) = event_from_obj(doc, &obj, group_id) {
                    // The signed semantic id must agree with the CRDT address. Without this bind,
                    // one captured record could be copied under many keys to amplify a timeline.
                    if key == format!("{EVENT_PREFIX}{}", event.id) {
                        state.events.push(event);
                    }
                }
            } else if key.starts_with(VOTE_PREFIX) {
                if let Some(vote) = vote_from_obj(doc, &obj, group_id) {
                    if key == format!("{VOTE_PREFIX}{}:{}", vote.case_id, vote.voter) {
                        state.votes.push(vote);
                    }
                }
            }
        }
    }
    // Every client gets the same stable order. Conflicting recasts reduce to the latest signed
    // candidate per (case, identity), with its signer/id as deterministic tie-breakers.
    state
        .events
        .sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
    state.votes.sort_by(|a, b| {
        a.case_id
            .cmp(&b.case_id)
            .then_with(|| a.voter.cmp(&b.voter))
            .then_with(|| b.ts.cmp(&a.ts))
            .then_with(|| a.signer.cmp(&b.signer))
    });
    state
        .votes
        .dedup_by(|a, b| a.case_id == b.case_id && a.voter == b.voter);
    state
}

pub(crate) fn case_is_open(events: &[ModerationEvent], case_id: &str) -> bool {
    events
        .iter()
        .any(|e| e.id == case_id && e.kind == "kick_case" && e.authorized)
        && !events.iter().any(|e| {
            e.kind == "case_resolution" && e.case_id == case_id && e.signature_valid && e.authorized
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_mls::MlsDevice;

    fn signed_event(group: &[u8]) -> ModerationEvent {
        let device = MlsDevice::generate().unwrap();
        let signer = crate::fingerprint(&device.device_id());
        let event = ModerationEvent {
            id: "a1".into(),
            kind: "warning".into(),
            actor: signer.clone(),
            signer,
            target: "ab01".into(),
            channel: "7".into(),
            message_id: "c1".into(),
            message_text: "snapshot".into(),
            message_ts: 5,
            reason: "be kind".into(),
            ts: 6,
            public_key: device.public_key_bytes(),
            ..ModerationEvent::default()
        };
        sign_event(group, event, |payload| {
            device
                .sign(payload)
                .map_err(|error| AppError::Invalid(error.to_string()))
        })
        .unwrap()
    }

    #[test]
    fn signed_evidence_is_group_bound_and_tamper_evident() {
        let event = signed_event(b"group-a");
        let mut doc = AutoCommit::new();
        write_event(&mut doc, &event).unwrap();
        assert!(read_state(&doc, b"group-a").events[0].signature_valid);
        assert!(!read_state(&doc, b"group-b").events[0].signature_valid);

        let (_, obj) = doc.get(ROOT, "e:a1").unwrap().unwrap();
        doc.put(&obj, REASON, "different reason").unwrap();
        assert!(!read_state(&doc, b"group-a").events[0].signature_valid);
    }

    #[test]
    fn signed_evidence_cannot_be_aliased_under_another_timeline_key() {
        let event = signed_event(b"group-a");
        let mut doc = AutoCommit::new();
        write_event_at_key(&mut doc, "e:ffff", &event).unwrap();
        assert!(read_state(&doc, b"group-a").events.is_empty());
    }

    #[test]
    fn evidence_and_reason_bounds_are_enforced() {
        let mut event = ModerationEvent {
            id: "aa".into(),
            kind: "kick_case".into(),
            actor: "aa".into(),
            signer: "bb".into(),
            target: "cc".into(),
            reason: "x".repeat(MAX_MOD_REASON_BYTES + 1),
            ..ModerationEvent::default()
        };
        assert!(validate_event_shape(&event).is_err());
        event.reason = "reason".into();
        event.evidence_ids = vec!["aa".into(); MAX_MOD_EVIDENCE_IDS + 1];
        assert!(validate_event_shape(&event).is_err());
    }
}
