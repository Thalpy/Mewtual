//! Borrow the complete outer Sync framing and bounded first-owner metadata before the common
//! decoder clones fields. Nested provider-map preflight runs inside the bounded MLS decoder,
//! before provider allocation/SDK load. This is not a general history reader or Current witness.
use super::*;
use initialization_snapshot::{
    MAX_INITIAL_ADMINS, MAX_INITIAL_FINALIZATIONS, MAX_INITIAL_RENDEZVOUS, METADATA_FEE,
};

/// Negative allocation limits, not payment, membership, source authentication or runtime authority.
/// The real canonical App consumer copies these from the SAME mounted reservation as its read fee.
/// SDK typed JSON/group heaps and active-server admission require separate enforcement.
#[derive(Clone, Copy, Debug)]
pub struct OwnerInitializationRestoreLimits {
    /// Complete Sync snapshot bytes, excluding its canonical vault wrapper.
    pub max_snapshot_bytes: usize,
    /// Complete encoded MLS frame, including its storage map, public key and group labels.
    pub max_mls_bytes: usize,
    /// Provider table allocation, independently of copied Vec payload and typed SDK heaps.
    pub max_mls_backing_bytes: usize,
}

/// Private decode strategy only. The public legacy path retains its exact decoder/grammar, and
/// the private checkpoint selects this mode only AFTER the complete borrowed preflight succeeds.
#[derive(Clone, Copy)]
pub(super) enum MlsDecode {
    Legacy,
    OwnerCheckpoint(OwnerInitializationRestoreLimits),
}
impl MlsDecode {
    pub(super) fn restore(self, bytes: &[u8]) -> Result<(MlsDevice, ServerGroup), SyncError> {
        Ok(match self {
            Self::Legacy => restore_server(bytes)?,
            Self::OwnerCheckpoint(limits) => catcoms_mls::persist::restore_server_provider_bounded(
                bytes,
                limits.max_mls_bytes,
                limits.max_mls_backing_bytes,
            )?,
        })
    }
}

#[cfg(test)]
std::thread_local! {
    // Mark the actual common decoder, not merely the SDK call. "Decode/clone everything, then
    // check the caps" must fail the early-refusal regressions even if it never reaches OpenMLS.
    static DECODER_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(super) fn note_decoder_entry() {
    DECODER_ENTRIES.with(|entries| entries.set(entries.get() + 1));
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Decode the exact narrow grammar emitted by the private first-owner initializer.
    ///
    /// Unsupported history/obligations are never truncated into an empty node. Complete outer
    /// framing, mandatory checkpoint metadata and owner-fact count/length preflights precede
    /// the common decoder's first clone. The bounded MLS decoder then checks its nested map
    /// before provider allocation and SDK load. Generic compatibility restore is unchanged.
    /// This returns an unbound restored owner tail: only actual original saved/current/head
    /// custody may later bind it, and SDK typed heap accounting remains an activation gate.
    pub fn restore_owner_initialization(
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
            MlsDecode::OwnerCheckpoint(limits),
        )
    }
}

fn frame<'a>(decoder: &mut Decoder<'a>) -> Result<&'a [u8], SyncError> {
    decoder.get_bytes().map_err(|_| SyncError::Malformed)
}

/// Bound a counted input BEFORE entering its loop. A declared count whose minimum framing
/// cannot fit is malformed; supported framing outside this closed checkpoint is unsupported.
fn count(decoder: &mut Decoder<'_>, cap: usize, minimum: usize) -> Result<usize, SyncError> {
    let amount = usize::try_from(decoder.get_u32().map_err(|_| SyncError::Malformed)?)
        .map_err(|_| SyncError::Malformed)?;
    if minimum != 0 && amount > decoder.remaining() / minimum {
        return Err(SyncError::Malformed);
    }
    if amount > cap {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    Ok(amount)
}

fn bounded(bytes: &[u8], cap: usize) -> Result<(), SyncError> {
    if bytes.len() > cap {
        Err(SyncError::OwnerInitializationUnsupported)
    } else {
        Ok(())
    }
}

pub(super) fn preflight(
    snapshot: &[u8],
    limits: OwnerInitializationRestoreLimits,
) -> Result<(), SyncError> {
    let bad = || SyncError::Malformed;
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
    let metadata = snapshot
        .len()
        .checked_sub(mls.len())
        .and_then(|size| size.checked_sub(4))
        .ok_or_else(bad)?;
    if metadata > METADATA_FEE {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    let (group, signer) = {
        let mut outer = Decoder::new(mls);
        frame(&mut outer)?; // the bounded provider decoder owns its map/count/backing invoice
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
        let slots = count(&mut routing, MAX_ROUTING_SECRETS, 44)?;
        for _ in 0..slots {
            routing.get_u64().map_err(|_| bad())?;
            if frame(&mut routing)?.len() != 32 {
                return Err(bad());
            }
        }
        routing.finish().map_err(|_| bad())?;
    }
    {
        let mut ledger = Decoder::new(frame(&mut decoder)?);
        count(&mut ledger, 0, 20)?; // consumed and revoked nonce sets must both be EMPTY
        count(&mut ledger, 0, 20)?;
        ledger.finish().map_err(|_| bad())?;
    }
    count(&mut decoder, 0, 4)?; // document snapshots: refuse before any owned graph
    count(&mut decoder, 0, 4)?; // original commit log: never discard history to fit
    count(&mut decoder, 0, 4)?; // peer descriptors
    if frame(&mut decoder)?.len() != 32 {
        return Err(bad());
    }
    decoder.get_u64().map_err(|_| bad())?;
    count(&mut decoder, 0, 4)?; // obsolete fingerprint grants are not a checkpoint input
    let rendezvous = count(&mut decoder, MAX_INITIAL_RENDEZVOUS, 8)?;
    for _ in 0..rendezvous {
        let address = decoder.get_str().map_err(|_| bad())?;
        bounded(address.as_bytes(), 1024)?;
        bounded(frame(&mut decoder)?, 128)?;
    }
    count(&mut decoder, 0, 148)?; // direct response cache is not emitted by the initializer

    // These are bounded original bytes, not decoded authority. Existing semantic decoders
    // remain responsible for actual tenure, policy, signatures and supported field values.
    bounded(required_frame(&mut decoder)?, 256)?;
    bounded(required_frame(&mut decoder)?, 4101)?;
    {
        let mut durable = Decoder::new(required_frame(&mut decoder)?);
        if !matches!(durable.get_u8().map_err(|_| bad())?, 1 | 2) {
            return Err(bad());
        }
        count(&mut durable, 0, 1)?;
        durable.finish().map_err(|_| bad())?;
    }
    member_finalization::preflight_owner_initialization_pending(
        required_frame(&mut decoder)?,
        MAX_INITIAL_FINALIZATIONS,
    )?;
    if decoder.is_empty() {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    if decoder.get_str().map_err(|_| bad())? != "catcoms/admin-authority/v2" {
        return Err(bad());
    }
    let admins = count(&mut decoder, MAX_INITIAL_ADMINS, 36)?;
    for _ in 0..admins {
        if frame(&mut decoder)?.len() != 32 {
            return Err(bad());
        }
    }
    local_chat_scope::LocalChatScope::preflight_owner_initialization(
        required_frame(&mut decoder)?,
        group,
        signer,
    )?;
    owner_evidence::InitialOwnerEvidence::preflight_owner_initialization(required_frame(
        &mut decoder,
    )?)?;
    decoder.finish().map_err(|_| bad())?;
    Ok(())
}

fn required_frame<'a>(decoder: &mut Decoder<'a>) -> Result<&'a [u8], SyncError> {
    if decoder.is_empty() {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    frame(decoder)
}

#[cfg(test)]
mod tests;
