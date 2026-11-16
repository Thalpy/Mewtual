//! Recover only original signed bytes whose raw identity is ALREADY in this exact applied
//! graph. No Automerge change reconstruction, operation/value walk or mutable AutoCommit
//! observer is allowed here. This private issuer does not weaken SavedLocalChatOp::decode.
use super::*;
use crate::op::{InspectionSignature, SignedOpFrame};
use sha2::{Digest, Sha256};

/// Each atomic phase fits one turn. Keeping the phase across yields prevents rehashing or
/// reverifying a near-limit candidate merely because its later comparison/copy needs credit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecoveryPhase {
    Hash,
    Verify,
    Retain,
}

#[derive(Clone, Copy, Default, Debug)]
pub(super) struct RecoveryWork {
    pub(super) records: usize,
    pub(super) bytes: usize,
    pub(super) signatures: usize,
    #[cfg(test)]
    pub(super) hash_bytes: usize,
}
impl RecoveryWork {
    fn pay(&mut self, bytes: usize) -> bool {
        if bytes > MAX_LOCAL_CHAT_RECOVERY_SCANNED_BYTES - self.bytes {
            return false;
        }
        self.bytes += bytes;
        true
    }
}

pub(super) fn recover(
    doc: &EncryptedDoc,
    device: &MlsDevice,
    progress: &mut SavedLocalChatRecovery,
) -> Result<Option<SavedLocalChatOp>, ReplError> {
    let mut work = RecoveryWork::default();
    let result = step(doc, device, progress, &mut work);
    #[cfg(test)]
    {
        progress.last_work = work;
    }
    result
}

fn step(
    doc: &EncryptedDoc,
    device: &MlsDevice,
    progress: &mut SavedLocalChatRecovery,
    work: &mut RecoveryWork,
) -> Result<Option<SavedLocalChatOp>, ReplError> {
    // This vendor observer borrows the existing hash index; unlike get_change_by_hash it
    // neither reconstructs a Change nor commits a pending transaction. Queued signed
    // originals are NOT applied-graph evidence, even when our retained log contains them.
    if !doc.doc.has_applied_change(&progress.change) {
        return Err(ReplError::Malformed);
    }
    while let Some(op) = doc.log.get(progress.position) {
        if work.records == MAX_LOCAL_CHAT_RECOVERY_SCANNED_OPS {
            return Ok(None);
        }
        work.records += 1;
        if progress.active.is_none() {
            if op.doc_type != DocType::Channel
                || op.doc_id != doc.doc_id
                || op.author_device != device.device_id()
                || op.author_pubkey.len() != 32
                || op.domain_op.is_some()
            {
                progress.position += 1;
                continue;
            }
            // Full-key comparison is bounded and paid before touching those bytes.
            if !work.pay(32) {
                return Ok(None);
            }
            if op.author_pubkey != progress.signer {
                progress.position += 1;
                continue;
            }
            let frame = SignedOpFrame::new(op).map_err(|_| ReplError::Malformed)?;
            if frame.encoded_len() > MAX_SAVED_LOCAL_CHAT_BYTES {
                // Unrelated local changes may legitimately exceed the saved-chat cap.
                // Skip them without reconstruction; an oversized target still fails at
                // completion because no bounded exact original can be established.
                progress.position += 1;
                continue;
            }
            progress.active = Some(RecoveryPhase::Hash);
        }
        let frame = SignedOpFrame::new(op).map_err(|_| ReplError::Malformed)?;
        if progress.active == Some(RecoveryPhase::Hash) {
            // Raw SHA input plus bounded framing/actor observations, charged BEFORE work.
            if !work.pay(op.delta.len() + 128) {
                return Ok(None);
            }
            #[cfg(test)]
            {
                work.hash_bytes += op.delta.len();
            }
            let Some(actor) = raw_identity(&op.delta, progress.change)? else {
                progress.active = None;
                progress.position += 1;
                continue;
            };
            if actor != *device.device_id().as_bytes() {
                return Err(ReplError::ActorMismatch);
            }
            progress.active = Some(RecoveryPhase::Verify);
        }
        if progress.active == Some(RecoveryPhase::Verify) {
            let signature_bytes = frame.signature_work_bytes();
            if signature_bytes == 0 {
                return Err(ReplError::Malformed);
            }
            // Count both constructing and reading the signature preimage, the 32-byte
            // DeviceId hash and Ed25519's fixed R/public-key hash input (64 bytes). This largest
            // atomic phase is <512KiB because the complete original frame is <=256KiB.
            if work.signatures == HISTORY_SIGNATURES_PER_TURN || !work.pay(2 * signature_bytes + 96)
            {
                return Ok(None);
            }
            work.signatures += 1;
            if frame.verify_signature() != InspectionSignature::Valid {
                return Err(ReplError::BadSignature);
            }
            progress.active = Some(RecoveryPhase::Retain);
        }
        if let Some(previous) = &progress.found {
            if !work.pay(2 * frame.encoded_len()) {
                return Ok(None);
            }
            // Full exact envelope equality, including the original signature; never two
            // full encodings merely to compare or an actor/sequence surrogate for identity.
            if previous.0 != *op {
                return Err(ReplError::BadSignature);
            }
        } else {
            if !work.pay(frame.encoded_len()) {
                return Ok(None);
            }
            progress.found = Some(SavedLocalChatOp(op.clone()));
        }
        progress.active = None;
        progress.position += 1;
    }
    // Keep dynamic-tail semantics: a later appended contradictory original is scanned on
    // the next visit. This result is not a captured whole-history completeness receipt.
    if !doc.doc.has_applied_change(&progress.change) {
        return Err(ReplError::Malformed);
    }
    let found = progress.found.as_ref().ok_or(ReplError::Malformed)?;
    let frame = SignedOpFrame::new(&found.0).map_err(|_| ReplError::Malformed)?;
    if !work.pay(frame.encoded_len()) {
        return Ok(None);
    }
    Ok(Some(found.clone()))
}

/// Identity of a raw kind-1 Automerge chunk, matching the pinned vendor's chunk hash.
/// The actual applied graph already authenticated/parsed this SHA identity. We inspect
/// only fixed framing and the modern actor here, never operation columns/scalar values.
fn raw_identity(raw: &[u8], expected: ChangeHash) -> Result<Option<[u8; 32]>, ReplError> {
    if raw.len() < 10 || raw[..4] != [0x85, 0x6f, 0x4a, 0x83] || raw[8] != 1 {
        return Ok(None);
    }
    // Match the bounded raw hash before target-specific grammar checks. Unrelated
    // same-key legacy actor names must not make a modern target unrecoverable.
    let hash = ChangeHash(Sha256::digest(&raw[8..]).into());
    if hash != expected {
        return Ok(None);
    }
    let mut at = 9;
    let length = unsigned(raw, &mut at)?;
    let length = usize::try_from(length).map_err(|_| ReplError::Malformed)?;
    if at.checked_add(length) != Some(raw.len()) {
        return Err(ReplError::Malformed);
    }
    let mut header = at;
    let dependencies = unsigned(raw, &mut header)?;
    let skip = usize::try_from(dependencies)
        .ok()
        .and_then(|n| n.checked_mul(32))
        .ok_or(ReplError::Malformed)?;
    header = header
        .checked_add(skip)
        .filter(|end| *end <= raw.len())
        .ok_or(ReplError::Malformed)?;
    if unsigned(raw, &mut header)? != 32 {
        return Err(ReplError::ActorMismatch);
    }
    let end = header.checked_add(32).ok_or(ReplError::Malformed)?;
    let actor = raw
        .get(header..end)
        .ok_or(ReplError::Malformed)?
        .try_into()
        .map_err(|_| ReplError::Malformed)?;
    if hash.0[..4] != raw[4..8] {
        return Err(ReplError::Malformed);
    }
    Ok(Some(actor))
}

/// Canonical unsigned LEB128, with checked bounds and no allocation or dependency walk.
fn unsigned(raw: &[u8], at: &mut usize) -> Result<u64, ReplError> {
    let mut result = 0u64;
    for index in 0..10 {
        let byte = *raw.get(*at).ok_or(ReplError::Malformed)?;
        *at += 1;
        if index == 9 && byte > 1 {
            return Err(ReplError::Malformed);
        }
        result |= u64::from(byte & 127) << (index * 7);
        if byte & 128 == 0 {
            if index > 0 && byte == 0 {
                return Err(ReplError::Malformed);
            }
            return Ok(result);
        }
    }
    Err(ReplError::Malformed)
}

#[cfg(test)]
mod tests;
