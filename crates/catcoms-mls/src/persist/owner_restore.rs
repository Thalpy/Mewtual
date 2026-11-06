//! Actual opt-in initial SDK consumer, with resource custody independent of source/read buffers.
//! A resource lease grants no source authentication, Fresh, membership, Saved or Current.
use super::*;
use std::sync::Arc;

/// Negative per-load requested-buffer/work limits. These are not a resource reservation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerSdkRestoreLimits {
    /// Cumulative requested typed buffers/fixed metadata for ONE initial graph, including growth overlap.
    pub max_typed_bytes: usize,
    /// Monotonic lexical/probe/element work allowance, independent of wall-clock deadlines.
    pub max_work: usize,
}
/// Trusted HOST accounting boundary. The repository implementation is an actual private App
/// shared-pool RAII debit, acquired before decoding. Arbitrary Rust implementations are trusted
/// host code, not authenticated payment. Implementations must own only accounting/caps/metadata,
/// never a Store, Server, provider, keys, or a source owner (which would create resource cycles).
///
/// The loader queries limits ONCE before SDK locks. No method/callback to this object runs from
/// a getter/decoder. Its eventual Drop runs only after both actual device and group drain.
pub trait OwnerSdkGraphReservation: Send + Sync + 'static {
    fn limits(&self) -> OwnerSdkRestoreLimits;
}
pub(crate) type OwnerSdkGraphLease = Arc<dyn OwnerSdkGraphReservation>;

/// Non-Clone host resource custody. Constructor is a documented trusted-Rust boundary;
/// neither negative caps nor this object confer source/authentication/Current authority.
/// Its cached caps are read exactly once BEFORE SDK locks, and no callback is made thereafter.
pub struct OwnerSdkRestoreAdmission {
    lease: OwnerSdkGraphLease,
    limits: OwnerSdkRestoreLimits,
}
impl OwnerSdkRestoreAdmission {
    pub fn new<G: OwnerSdkGraphReservation>(reservation: G) -> Result<Self, MlsError> {
        let bad = || MlsError::Internal("unsupported or exhausted closed SDK restore");
        let mut limits = reservation.limits();
        let header = std::alloc::Layout::array::<usize>(2).map_err(|_| bad())?;
        let (layout, _) = header
            .extend(std::alloc::Layout::new::<G>())
            .map_err(|_| bad())?;
        limits.max_typed_bytes = limits
            .max_typed_bytes
            .checked_sub(layout.pad_to_align().size())
            .filter(|n| *n > 0)
            .ok_or_else(bad)?;
        if limits.max_work == 0 {
            return Err(bad());
        }
        Ok(Self {
            lease: Arc::new(reservation),
            limits,
        })
    }
}
impl std::fmt::Debug for OwnerSdkRestoreAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerSdkRestoreAdmission")
            .finish_non_exhaustive()
    }
}

/// Restore only the exact pinned SDK founder/first-Add profile under an actual retained fee.
/// Raw provider-map/backing/payload allowance is independent and must already be reserved.
/// Ordinary `restore_server` and `restore_server_provider_bounded` keep their legacy grammar.
///
/// The policy is installed BEFORE MlsDevice::restore's signer getter. Unsupported/malformed
/// graphs refuse without changing the source, and Complete/Cancelled never permit later SDK
/// mutation. A new operation admission is required for Add/live ratchets. This is not complete
/// live-chat, whole-Server/provider/RSS accounting, original-source proof or native activation.
pub fn restore_server_owner_checkpoint_bounded(
    snapshot: &[u8],
    max_bytes: usize,
    max_backing_bytes: usize,
    admission: OwnerSdkRestoreAdmission,
) -> Result<(MlsDevice, ServerGroup), MlsError> {
    let bad = || MlsError::Internal("unsupported or exhausted closed SDK restore");
    // LOCAL strong holder is declared before provider/device/group/SDK guard. Every partial
    // getter/graph and its locks drain BEFORE the final host fee can refund. No host callback.
    let OwnerSdkRestoreAdmission { lease, limits } = admission;
    if snapshot.len() > max_bytes {
        return Err(bad());
    }
    let payload = max_bytes.checked_mul(2).ok_or_else(bad)?;
    let mut outer = Decoder::new(snapshot);
    let storage = outer.get_bytes().map_err(|_| bad())?;
    let signer = outer.get_bytes().map_err(|_| bad())?;
    let group_id = outer.get_bytes().map_err(|_| bad())?;
    outer.finish().map_err(|_| bad())?;
    if signer.len() != 32 || group_id.len() != 16 {
        return Err(bad());
    }
    let map = provider_restore::deserialize_map_bounded(storage, max_backing_bytes, payload)?;
    let provider = OpenMlsRustCrypto::default();
    *provider.storage().values.write().map_err(|_| bad())? = map;
    let policy = provider
        .storage()
        .begin_owner_checkpoint_restore(
            group_id,
            signer,
            openmls_memory_storage::owner_restore::OwnerCheckpointRestoreLimits {
                max_typed_bytes: limits.max_typed_bytes,
                max_work: limits.max_work,
            },
        )
        .map_err(|_| bad())?;
    #[cfg(test)]
    SDK_LOAD_ENTRIES.with(|n| n.set(n.get() + 1));
    let mut device = MlsDevice::restore(provider, signer)?;
    let mut group = ServerGroup::load_owner_checkpoint(&device, group_id)?;
    // Borrow actual own_leaf rather than materializing/cloning a roster to test local identity.
    // Historical membership and external root facts still require the original host consumer.
    if !group.is_active()
        || !group.has_local_device(&device.device_id())
        || !group.has_local_signer(signer)
    {
        return Err(bad());
    }
    policy.complete().map_err(|_| bad())?;
    // Same payment, TWO final holders. Public tuple users may drop either object first; the
    // graph/provider/signers in BOTH must drain before the host fee is released.
    device.sdk_graph_lease = Some(lease.clone());
    group.sdk_graph_lease = Some(lease.clone());
    Ok((device, group))
}

#[cfg(test)]
std::thread_local! {
    // This is the ACTUAL SDK entry, after ALL thirteen row preflights, not a helper counter.
    static SDK_LOAD_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
mod tests;
