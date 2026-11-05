//! Closed generation-2 first-Add grammar BEFORE the common decoder's first clone/SDK call.
//! This must not grow into a permissive legacy fallback: unsupported history stays retained.
use super::operation_tail::{self, MAX_COMMIT};
use super::*;

mod parent;

fn bad() -> SyncError {
    SyncError::Malformed
}
fn frame<'a>(decoder: &mut Decoder<'a>) -> Result<&'a [u8], SyncError> {
    decoder.get_bytes().map_err(|_| bad())
}
fn count(decoder: &mut Decoder<'_>, expected: u32) -> Result<(), SyncError> {
    if decoder.get_u32().map_err(|_| bad())? != expected {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    Ok(())
}
fn capped_count(decoder: &mut Decoder<'_>, max: usize, min: usize) -> Result<usize, SyncError> {
    let count = decoder.get_u32().map_err(|_| bad())? as usize;
    if count > decoder.remaining() / min {
        return Err(bad());
    }
    if count > max {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    Ok(count)
}
fn bounded(bytes: &[u8], max: usize) -> Result<(), SyncError> {
    if bytes.len() > max {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    Ok(())
}

/// Complete borrowed checkpoint framing, including the mandatory operation and nested result/
/// commit grammar. This returns no root, verified acceptance or local saved-head authority.
pub(super) fn preflight<'a>(
    snapshot: &'a [u8],
    limits: OwnerInitializationRestoreLimits,
) -> Result<operation_tail::BorrowedOperation<'a>, SyncError> {
    if limits.max_snapshot_bytes == 0
        || limits.max_mls_bytes == 0
        || limits.max_mls_backing_bytes == 0
        || limits.max_mls_bytes > limits.max_snapshot_bytes
        || limits.max_mls_bytes.checked_mul(2).is_none()
    {
        return Err(bad());
    }
    bounded(snapshot, limits.max_snapshot_bytes)?;
    let mut decoder = Decoder::new(snapshot);
    let mls = frame(&mut decoder)?;
    bounded(mls, limits.max_mls_bytes)?;
    let metadata_limit = initialization_snapshot::METADATA_FEE
        .checked_add(operation_tail::MAX_BYTES)
        .and_then(|n| n.checked_add(MAX_COMMIT))
        .ok_or_else(bad)?;
    let metadata = snapshot
        .len()
        .checked_sub(mls.len())
        .and_then(|n| n.checked_sub(4))
        .ok_or_else(bad)?;
    if metadata > metadata_limit {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    let (group, signer) = {
        let mut outer = Decoder::new(mls);
        frame(&mut outer)?;
        let signer = frame(&mut outer)?;
        let group = frame(&mut outer)?;
        outer.finish().map_err(|_| bad())?;
        if signer.len() != 32 || group.is_empty() || group.len() > 256 {
            return Err(bad());
        }
        (group, signer)
    };
    {
        let mut routing = Decoder::new(frame(&mut decoder)?);
        routing.get_u64().map_err(|_| bad())?;
        let slots = capped_count(&mut routing, MAX_ROUTING_SECRETS, 44)?;
        for _ in 0..slots {
            routing.get_u64().map_err(|_| bad())?;
            if frame(&mut routing)?.len() != 32 {
                return Err(bad());
            }
        }
        routing.finish().map_err(|_| bad())?;
    }
    let consumed = {
        let mut ledger = Decoder::new(frame(&mut decoder)?);
        count(&mut ledger, 1)?;
        let nonce: &[u8; 16] = frame(&mut ledger)?.try_into().map_err(|_| bad())?;
        count(&mut ledger, 0)?;
        ledger.finish().map_err(|_| bad())?;
        nonce
    };
    count(&mut decoder, 0)?; // no document graph may be discarded for this closed vertical
    count(&mut decoder, 1)?;
    let original_commit = frame(&mut decoder)?;
    bounded(original_commit, MAX_COMMIT)?;
    operation_tail::preflight_commit(original_commit)?;
    count(&mut decoder, 0)?; // peer descriptor map
    if frame(&mut decoder)?.len() != 32 {
        return Err(bad());
    }
    if decoder.get_u64().map_err(|_| bad())? != 0 {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    count(&mut decoder, 0)?; // obsolete fingerprint grants
    let rendezvous = capped_count(
        &mut decoder,
        initialization_snapshot::MAX_INITIAL_RENDEZVOUS,
        8,
    )?;
    for _ in 0..rendezvous {
        bounded(decoder.get_str().map_err(|_| bad())?.as_bytes(), 1024)?;
        bounded(frame(&mut decoder)?, 128)?;
    }
    count(&mut decoder, 0)?; // no parallel legacy direct-result cache
    bounded(frame(&mut decoder)?, 256)?; // original candidate tenure
    bounded(frame(&mut decoder)?, 4101)?; // exact policy pin
    {
        let mut durable = Decoder::new(frame(&mut decoder)?);
        if !matches!(durable.get_u8().map_err(|_| bad())?, 1 | 2) {
            return Err(bad());
        }
        count(&mut durable, 0)?;
        durable.finish().map_err(|_| bad())?;
    }
    member_finalization::preflight_owner_initialization_pending(
        frame(&mut decoder)?,
        initialization_snapshot::MAX_INITIAL_FINALIZATIONS,
    )?;
    if decoder.get_str().map_err(|_| bad())? != "catcoms/admin-authority/v2" {
        return Err(bad());
    }
    count(&mut decoder, 0)?;
    local_chat_scope::LocalChatScope::preflight_owner_initialization(
        frame(&mut decoder)?,
        group,
        signer,
    )?;
    InitialOwnerEvidence::preflight_owner_initialization(frame(&mut decoder)?)?;
    let operation = operation_tail::preflight(frame(&mut decoder)?)?;
    if operation.group != group
        || operation.owner.as_slice() != signer
        || operation.nonce != consumed
        || operation.commit != original_commit
    {
        return Err(bad());
    }
    decoder.finish().map_err(|_| bad())?;
    Ok(operation)
}

/// Data expected from the original authenticated canonical v2 wrapper. This creates no root,
/// Saved/current status or writer; the host derives these fields from its real sealed source.
pub struct OwnerOperationSourceExpectation<'a> {
    pub group: &'a [u8],
    pub signer: &'a [u8; 32],
    pub profile: OwnerFactId,
    pub predecessor: operation_tail::OwnerOperationPredecessor,
    pub request: [u8; 32],
}

/// Borrow/preflight the complete body and compare every authenticated parent/scope label
/// BEFORE SDK restore. Local encrypted metadata coherence only, never public history finality.
pub fn validate_owner_operation_snapshot(
    snapshot: &[u8],
    limits: OwnerInitializationRestoreLimits,
    expected: OwnerOperationSourceExpectation<'_>,
) -> Result<(), SyncError> {
    let operation = preflight(snapshot, limits)?;
    if operation.group != expected.group
        || operation.owner != expected.signer
        || operation.profile != expected.profile.as_bytes()
        || operation.predecessor != expected.predecessor
        || operation.binding != &expected.request
    {
        return Err(bad());
    }
    Ok(())
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Restore ONLY the strict first covering Add grammar under original host-supplied limits.
    /// No generic restore, root selection or signature/lifetime bypass is a fallback. The node
    /// stays unbound until actual canonical file/parent+Current/head and fact validation succeed.
    pub fn restore_owner_operation_checkpoint(
        snapshot: &[u8],
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
        limits: OwnerInitializationRestoreLimits,
    ) -> Result<Self, SyncError> {
        preflight(snapshot, limits)?;
        Self::restore_with_mls(
            snapshot,
            transport,
            rng,
            clock,
            initialization_restore::MlsDecode::OwnerCheckpoint(limits),
        )
    }
}
