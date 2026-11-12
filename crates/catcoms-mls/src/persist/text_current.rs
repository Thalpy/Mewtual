//! DISTINCT paid CLOSED CURRENT exporter-text SDK restore. This family allows no
//! SDK mutations and no initialized current sender ratchets. It grants no Source,
//! Admission, membership, Current, projection or publication authority.
use super::owner_restore::{OwnerSdkGraphLease, OwnerSdkGraphReservation, OwnerSdkRestoreLimits};
use super::*;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextCurrentSdkRestoreLimits {
    pub max_typed_bytes: usize,
    pub max_work: usize,
}
/// Trusted HOST fee owner, acquired before decoding. It must retain no SDK graph,
/// provider/key/source/Store or sender. Limits are sampled ONCE before SDK locks.
pub trait TextCurrentSdkGraphReservation: Send + Sync + 'static {
    fn limits(&self) -> TextCurrentSdkRestoreLimits;
}
struct Resource<G> {
    _reservation: G,
    cached: OwnerSdkRestoreLimits,
}
impl<G: TextCurrentSdkGraphReservation> OwnerSdkGraphReservation for Resource<G> {
    fn limits(&self) -> OwnerSdkRestoreLimits {
        self.cached
    }
}
// Concrete fixed returned/root/move metadata, ADDITIONAL to every counted native
// sequence growth and secret buffer. Raw provider/payload fees remain separate.
const FIXED_GRAPH_LAYOUT: usize = 4 * std::mem::size_of::<MlsDevice>()
    + 4 * std::mem::size_of::<ServerGroup>()
    + 2 * std::mem::size_of::<OpenMlsRustCrypto>()
    + 4 * std::mem::size_of::<TextCurrentSdkRestoreAdmission>()
    + 4 * std::mem::size_of::<Result<(MlsDevice, ServerGroup), MlsError>>()
    + 2 * std::mem::size_of::<Decoder<'static>>();
/// Non-Clone actual shared graph payment. Negative caps are not actual source proof.
pub struct TextCurrentSdkRestoreAdmission {
    lease: OwnerSdkGraphLease,
    limits: TextCurrentSdkRestoreLimits,
}
impl std::fmt::Debug for TextCurrentSdkRestoreAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextCurrentSdkRestoreAdmission")
            .finish_non_exhaustive()
    }
}
impl TextCurrentSdkRestoreAdmission {
    pub fn new<G: TextCurrentSdkGraphReservation>(reservation: G) -> Result<Self, MlsError> {
        let bad = || MlsError::Internal("unsupported or exhausted text-current SDK restore");
        let mut limits = reservation.limits();
        let header = std::alloc::Layout::array::<usize>(2).map_err(|_| bad())?;
        let (layout, _) = header
            .extend(std::alloc::Layout::new::<Resource<G>>())
            .map_err(|_| bad())?;
        limits.max_typed_bytes = limits
            .max_typed_bytes
            .checked_sub(layout.pad_to_align().size())
            .and_then(|n| n.checked_sub(FIXED_GRAPH_LAYOUT))
            .filter(|n| *n > 0)
            .ok_or_else(bad)?;
        if limits.max_work == 0 {
            return Err(bad());
        }
        let cached = OwnerSdkRestoreLimits {
            max_typed_bytes: limits.max_typed_bytes,
            max_work: limits.max_work,
        };
        Ok(Self {
            lease: Arc::new(Resource {
                _reservation: reservation,
                cached,
            }),
            limits,
        })
    }
}
/// Only CURRENT Operational graphs with <=64 real leaf slots, zero SDK past
/// epochs/proposals/own-leaf debt, exact founder32 ring or Welcome0, and ALL-NULL
/// current sender ratchets. Unsupported graphs are preserved/refused, never trimmed.
/// Input/raw provider accounting is independently acquired by the actual host.
pub fn restore_server_text_current_bounded(
    snapshot: &[u8],
    max_bytes: usize,
    max_backing_bytes: usize,
    admission: TextCurrentSdkRestoreAdmission,
) -> Result<(MlsDevice, ServerGroup), MlsError> {
    let bad = || MlsError::Internal("unsupported or exhausted text-current SDK restore");
    // Declare the strong payment BEFORE every provider/partial graph/guard. Reverse
    // local cleanup drains those on ALL errors before this final fee holder drops.
    let TextCurrentSdkRestoreAdmission { lease, limits } = admission;
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
        .begin_text_current_restore(
            group_id,
            signer,
            openmls_memory_storage::owner_restore::TextCurrentRestoreLimits {
                max_typed_bytes: limits.max_typed_bytes,
                max_work: limits.max_work,
            },
        )
        .map_err(|_| bad())?;
    #[cfg(test)]
    TEXT_LOAD_ENTRIES.with(|n| n.set(n.get() + 1));
    let mut device = MlsDevice::restore(provider, signer)?;
    let mut group = ServerGroup::load_text_current(&device, group_id)?;
    if !group.is_active()
        || !group.has_local_device(&device.device_id())
        || !group.has_local_signer(signer)
    {
        return Err(bad());
    }
    policy.complete().map_err(|_| bad())?;
    device.sdk_graph_lease = Some(lease.clone());
    group.sdk_graph_lease = Some(lease.clone());
    Ok((device, group))
}
#[cfg(test)]
thread_local! {static TEXT_LOAD_ENTRIES:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
#[cfg(test)]
mod tests;
