//! Independently paid PRIVATE Add candidate custody. Limits and original-pair
//! association are negative SDK/resource checks, never Root Source/governance,
//! native Current, destination trust, covering save or publication permission.
use super::owner_restore::{OwnerSdkGraphLease, OwnerSdkGraphReservation, OwnerSdkRestoreLimits};
use super::*;
use openmls_memory_storage::owner_restore::{TextAddCandidateGuard, TextAddOperationLimits};
use std::sync::Arc;

fn bad() -> MlsError {
    MlsError::Internal("unsupported bounded private text Add")
}

/// Separate CUMULATIVE requested-buffer classes; the complete sum is reserved
/// BEFORE copying any source/KP/provider data. No class borrows spare restore,
/// read, SDK or another operation credit. sdk_* additionally need the audited
/// one-Add dynamic lower bound at the future actual SDK entry.
#[derive(Clone, Copy, Debug)]
pub struct TextCurrentAddLimits {
    pub max_snapshot_bytes: usize,
    pub max_provider_backing_bytes: usize,
    pub max_provider_payload_bytes: usize,
    pub prepare_work: usize,
    pub package_typed_bytes: usize,
    pub package_work: usize,
    pub restore_typed_bytes: usize,
    pub restore_work: usize,
    pub operation: TextAddOperationLimits,
    pub sdk_typed_bytes: usize,
    pub sdk_work: usize,
    pub output_bytes: usize,
}
impl TextCurrentAddLimits {
    /// Buffer inventory only. The concrete Resource<G>/Cell/move layouts below
    /// are additional; this number is neither a fee object nor SDK authority.
    pub fn buffer_reservation_bytes(&self) -> Option<usize> {
        self.max_snapshot_bytes
            .checked_mul(2)?
            .checked_add(self.max_provider_backing_bytes)?
            .checked_add(self.max_provider_payload_bytes)?
            .checked_add(self.package_typed_bytes)?
            .checked_add(self.restore_typed_bytes)?
            .checked_add(self.operation.max_typed_bytes)?
            .checked_add(self.operation.max_serialized_bytes)?
            .checked_add(self.operation.max_provider_bytes)?
            .checked_add(self.sdk_typed_bytes)?
            .checked_add(self.output_bytes)
            .filter(|n| *n <= isize::MAX as usize)
    }
    fn require_supported(&self) -> Result<(), MlsError> {
        let fields = [
            self.max_snapshot_bytes,
            self.max_provider_backing_bytes,
            self.max_provider_payload_bytes,
            self.package_typed_bytes,
            self.restore_typed_bytes,
            self.operation.max_typed_bytes,
            self.operation.max_serialized_bytes,
            self.operation.max_provider_bytes,
            self.sdk_typed_bytes,
            self.output_bytes,
        ];
        if fields.iter().any(|n| *n == 0 || *n > isize::MAX as usize)
            || self.max_snapshot_bytes > u32::MAX as usize
            || self.max_provider_payload_bytes
                < self.max_snapshot_bytes.checked_mul(2).ok_or_else(bad)?
            || self.prepare_work == 0
            || self.package_work == 0
            || self.restore_work == 0
            || self.operation.max_work == 0
            || self.sdk_work == 0
        {
            return Err(bad());
        }
        self.buffer_reservation_bytes().ok_or_else(bad)?;
        Ok(())
    }
}

/// Trusted host accounting boundary, like the existing closed restore payer.
/// An implementation must own the actual full buffer+metadata debit, not merely
/// these caps, and must retain no device/group/Store/source/key/callback graph.
/// Root's production payer/journal/admission is intentionally not supplied here.
pub trait TextCurrentAddReservation: Send + Sync + 'static {
    fn limits(&self) -> TextCurrentAddLimits;
}
struct Resource<G> {
    cached: OwnerSdkRestoreLimits,
    _reservation: G,
}
impl<G: TextCurrentAddReservation> OwnerSdkGraphReservation for Resource<G> {
    fn limits(&self) -> OwnerSdkRestoreLimits {
        self.cached
    }
}

/// Unique paid preparation. This does NOT permit mutation of a restored parent.
/// Its only consumer builds a separate private candidate, and actual SDK entry
/// still needs a complete operation invoice and separate Root governance cut.
pub struct TextCurrentAddAdmission {
    limits: TextCurrentAddLimits,
    // Cumulative preparation counters may be instantiated only once. A failed
    // attempt preserves this fee-owning input, but cannot reset its allowance.
    preparation_entered: bool,
    lease: OwnerSdkGraphLease,
}
impl std::fmt::Debug for TextCurrentAddAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextCurrentAddAdmission")
            .finish_non_exhaustive()
    }
}
impl TextCurrentAddAdmission {
    /// Exact concrete fixed fee, including returned/partial/move wrappers, known
    /// before Arc/candidate allocation. G limits are sampled exactly once.
    pub fn metadata_layout_bytes<G: TextCurrentAddReservation>() -> Option<usize> {
        let (arc, _) = std::alloc::Layout::array::<usize>(2)
            .ok()?
            .extend(std::alloc::Layout::new::<Resource<G>>())
            .ok()?;
        arc.pad_to_align()
            .size()
            .checked_add(4 * std::mem::size_of::<Self>())?
            .checked_add(4 * std::mem::size_of::<CandidateGraph>())?
            .checked_add(
                4 * std::mem::size_of::<crate::group::text_current_add::PreparedTextCurrentAdd>(),
            )?
            .checked_add(
                4 * std::mem::size_of::<
                    Result<crate::group::text_current_add::PreparedTextCurrentAdd, MlsError>,
                >(),
            )
    }
    pub fn new<G: TextCurrentAddReservation>(reservation: G) -> Result<Self, MlsError> {
        // Compile-time SDK profile DATA, not a caller switch. Refuse before
        // package verification, snapshot/provider copy or any typed SDK load.
        // The SDK's actual private build entry repeats its own hard guard.
        if !openmls::prelude::MlsGroup::private_text_add_profile_supported_data() {
            return Err(bad());
        }
        let limits = reservation.limits();
        limits.require_supported()?;
        // The host's promised debit includes BOTH terms. We do not call G from
        // inside SDK/provider locks, or claim that arbitrary trusted Rust G is
        // cryptographic/resource authority.
        limits
            .buffer_reservation_bytes()
            .and_then(|n| n.checked_add(Self::metadata_layout_bytes::<G>()?))
            .filter(|n| *n <= isize::MAX as usize)
            .ok_or_else(bad)?;
        let lease: OwnerSdkGraphLease = Arc::new(Resource {
            cached: OwnerSdkRestoreLimits {
                max_typed_bytes: limits.restore_typed_bytes,
                max_work: limits.restore_work,
            },
            _reservation: reservation,
        });
        Ok(Self {
            limits,
            preparation_entered: false,
            lease,
        })
    }
    pub(crate) fn limits(&self) -> TextCurrentAddLimits {
        self.limits
    }
    pub(crate) fn lease(&self) -> OwnerSdkGraphLease {
        self.lease.clone()
    }
    pub(crate) fn enter_preparation(&mut self) -> Result<(), MlsError> {
        if std::mem::replace(&mut self.preparation_entered, true) {
            Err(bad())
        } else {
            Ok(())
        }
    }
}

/// No public SDK/provider extraction or mutable owning loan. SDK locals precede
/// the retained independent payment; either graph half remains closed Some.
pub(crate) struct CandidateGraph {
    pub(crate) group: ServerGroup,
    pub(crate) device: MlsDevice,
    pub(crate) policy: TextAddCandidateGuard,
    _lease: OwnerSdkGraphLease,
}

pub(crate) fn restore_candidate(
    snapshot: &[u8],
    epoch: u64,
    own: u32,
    admission: &TextCurrentAddAdmission,
) -> Result<CandidateGraph, MlsError> {
    let limits = admission.limits;
    // Strong original payment local is declared BEFORE every partial graph,
    // provider/key/map/policy. Reverse local Drop preserves fee-last on errors.
    let lease = admission.lease();
    if snapshot.len() > limits.max_snapshot_bytes {
        return Err(bad());
    }
    let mut outer = Decoder::new(snapshot);
    let storage = outer.get_bytes().map_err(|_| bad())?;
    let signer = outer.get_bytes().map_err(|_| bad())?;
    let group_id = outer.get_bytes().map_err(|_| bad())?;
    outer.finish().map_err(|_| bad())?;
    if signer.len() != 32 || group_id.len() != 16 {
        return Err(bad());
    }
    let map = provider_restore::deserialize_map_bounded(
        storage,
        limits.max_provider_backing_bytes,
        limits.max_provider_payload_bytes,
    )?;
    let provider = OpenMlsRustCrypto::default();
    *provider.storage().values.write().map_err(|_| bad())? = map;
    let mut policy = provider
        .storage()
        .begin_text_add_candidate_restore(
            group_id,
            signer,
            epoch,
            own,
            openmls_memory_storage::owner_restore::OwnerCheckpointRestoreLimits {
                max_typed_bytes: limits.restore_typed_bytes,
                max_work: limits.restore_work,
            },
            limits.operation,
        )
        .map_err(|_| bad())?;
    let mut device = MlsDevice::restore(provider, signer)?;
    let mut group = ServerGroup::load_text_current(&device, group_id)?;
    if !group.is_active()
        || !group.has_local_device(&device.device_id())
        || !group.has_local_signer(signer)
    {
        return Err(bad());
    }
    policy
        .complete_loading(device.provider().storage())
        .map_err(|_| bad())?;
    // complete_loading pre-reserves all64 possible operation insertions. The
    // final bounded snapshot uses THIS same backing cap; reject an inevitably
    // oversized fork now, before SDK Add/key/RNG entry or the admission take.
    // Sixteen actual writes/deletes cannot grow this already-reserved table.
    if device
        .provider()
        .storage()
        .values
        .read()
        .map_err(|_| bad())?
        .allocation_size()
        > limits.max_provider_backing_bytes
    {
        return Err(bad());
    }
    device.sdk_graph_lease = Some(lease.clone());
    group.sdk_graph_lease = Some(lease.clone());
    Ok(CandidateGraph {
        group,
        device,
        policy,
        _lease: lease,
    })
}
