//! PRIVATE, independently paid Add candidate. Its parent stays immutable and
//! mutation-closed. No raw device/group/provider, mutable SDK loan, Source,
//! governance, save, install, transport or publication authority is returned.
use super::*;
use crate::device::text_add_package::{self, PackageBudget, ValidatedTextAddPackage};
use crate::persist::text_current_add::{self, CandidateGraph, TextCurrentAddAdmission};
use crate::persist::OwnerSdkGraphLease;
use std::sync::Arc;
use zeroize::Zeroizing;
#[cfg(test)]
mod faults;
mod invoice;
pub(super) mod observation;
mod work;
#[cfg(test)]
use faults::TestCut;

fn bad() -> MlsError {
    MlsError::Internal("unsupported bounded private text Add")
}

/// Untrusted request DATA. Full package validation happens before candidate SDK
/// loading. Root must independently bind the SAME request to its actual durable
/// governance reservation and selected Source before irreversible key/SDK work.
#[derive(Clone, Copy)]
pub struct TextCurrentAddRequestData<'a> {
    frame: &'a [u8],
    subject: [u8; 32],
    nonce: [u8; 16],
    reference: [u8; 32],
    now_ms: u64,
}
impl<'a> TextCurrentAddRequestData<'a> {
    pub fn from_untrusted_data(
        frame: &'a [u8],
        subject: [u8; 32],
        nonce: [u8; 16],
        reference: [u8; 32],
        now_ms: u64,
    ) -> Self {
        Self {
            frame,
            subject,
            nonce,
            reference,
            now_ms,
        }
    }
}
impl fmt::Debug for TextCurrentAddRequestData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextCurrentAddRequestData")
            .field("package_bytes", &self.frame.len())
            .finish_non_exhaustive()
    }
}

struct OriginalParent {
    group: [u8; 16],
    author: [u8; 32],
    epoch: u64,
    own: u32,
    authenticator: [u8; 32],
    /// Strong accounting-only Arc, no SDK/provider/Source graph cycle and no
    /// Weak metadata surviving after the original resource debit refunds.
    lease: OwnerSdkGraphLease,
}
impl OriginalParent {
    fn observe(device: &MlsDevice, group: &ServerGroup) -> Result<Self, MlsError> {
        let d = device.sdk_graph_lease.as_ref().ok_or_else(bad)?;
        let g = group.sdk_graph_lease.as_ref().ok_or_else(bad)?;
        if !Arc::ptr_eq(d, g) {
            return Err(bad());
        }
        let id: [u8; 16] = group.group_id_ref().try_into().map_err(|_| bad())?;
        let author: [u8; 32] = device.public_key_ref().try_into().map_err(|_| bad())?;
        if !group.is_active()
            || !group.has_local_device(&device.device_id())
            || !group.has_local_signer(&author)
            || group.group.pending_commit().is_some()
            || group.group.pending_proposals().next().is_some()
        {
            return Err(bad());
        }
        let epoch = group.epoch();
        epoch.checked_add(1).ok_or_else(bad)?;
        let slots = group
            .group
            .treesync()
            .full_leaves()
            .count()
            .checked_add(group.group.treesync().blank_leaves().count())
            .ok_or_else(bad)?;
        let first_blank = group.group.treesync().blank_leaves().next();
        if !(1..=64).contains(&slots) || (slots == 64 && first_blank.is_none()) {
            return Err(bad());
        }
        Ok(Self {
            group: id,
            author,
            epoch,
            own: group.group.own_leaf_index().u32(),
            authenticator: group.epoch_authenticator_id(),
            lease: d.clone(),
        })
    }
    fn require_same(&self, device: &MlsDevice, group: &ServerGroup) -> Result<(), MlsError> {
        let d = device.sdk_graph_lease.as_ref().ok_or_else(bad)?;
        let g = group.sdk_graph_lease.as_ref().ok_or_else(bad)?;
        if !Arc::ptr_eq(&self.lease, d)
            || !Arc::ptr_eq(&self.lease, g)
            || group.group_id_ref() != self.group
            || device.public_key_ref() != self.author
            || group.epoch() != self.epoch
            || group.group.own_leaf_index().u32() != self.own
            || group.epoch_authenticator_id() != self.authenticator
        {
            return Err(bad());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prepared,
    AddEntered,
    OutputsCached,
    Serialized,
    MergeEntered,
    Merged,
    Complete,
    Failed,
}
struct SdkOutputs {
    commit: MlsMessageOut,
    welcome: Option<MlsMessageOut>,
    group_info: Option<MlsMessageOut>,
}

/// Whole original operation owner. Fields carrying outputs/SDK payloads precede
/// ALL payment-bearing tails. No owning/mutable slot getter lets SDK data escape
/// its fee, or swap a graph into another provider/accounted original.
pub struct PreparedTextCurrentAdd {
    #[cfg(test)]
    faults: faults::Faults,
    outputs: Option<SdkOutputs>,
    commit: Option<Vec<u8>>,
    welcome: Option<Vec<u8>>,
    group_info: Option<Vec<u8>>,
    snapshot: Option<Zeroizing<Vec<u8>>>,
    observation: Option<LocalMembershipChange>,
    package: Option<ValidatedTextAddPackage>,
    package_frame: Vec<u8>,
    subject: [u8; 32],
    nonce: [u8; 16],
    reference: [u8; 32],
    credential_digest: [u8; 32],
    now_ms: u64,
    original_snapshot_id: [u8; 32],
    phase: Phase,
    output_remaining: usize,
    provider_closed: bool,
    graph: CandidateGraph,
    original: OriginalParent,
    admission: TextCurrentAddAdmission,
}
impl fmt::Debug for PreparedTextCurrentAdd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTextCurrentAdd")
            .field("original_epoch", &self.original.epoch)
            .field("complete", &self.is_complete_data())
            .finish_non_exhaustive()
    }
}
impl PreparedTextCurrentAdd {
    /// Prepare a new immutable-parent fork, preserving the unique admission slot
    /// on EVERY error. After preparation entry that debit is exhausted even if
    /// construction fails; its unchanged owning slot can be dropped outside
    /// locks and replaced with a fresh independent fee after locals drain.
    /// Counters are never reset by retry. All allocations/validation precede
    /// the final infallible take. An existing output checks original/request DATA and
    /// never re-decodes, signs, changes epoch, samples RNG or rebuilds a graph.
    pub fn prepare_into(
        device: &MlsDevice,
        group: &ServerGroup,
        request: TextCurrentAddRequestData<'_>,
        admission: &mut Option<TextCurrentAddAdmission>,
        output: &mut Option<Self>,
    ) -> Result<(), MlsError> {
        if let Some(cell) = output.as_ref() {
            if admission.is_some() {
                return Err(bad());
            }
            cell.original.require_same(device, group)?;
            if cell.package_frame != request.frame
                || cell.subject != request.subject
                || cell.nonce != request.nonce
                || cell.reference != request.reference
                || cell.now_ms != request.now_ms
            {
                return Err(bad());
            }
            return Ok(());
        }
        let paid = admission.as_mut().ok_or_else(bad)?;
        let limits = paid.limits();
        // No SDK/key work is paid by negative caps alone. The actual full
        // independent reservation must cover this entire conservative inventory
        // before even the package/public-crypto or clone preparation starts.
        invoice::require(limits, 0)?;
        let original = OriginalParent::observe(device, group)?;
        // Bound real table probing/copies and fixed key/identity work BEFORE the
        // snapshot/first KP allocation. This pays constructor work, not SDK Add.
        let prepare_work = limits
            .max_provider_backing_bytes
            .checked_mul(513)
            .and_then(|n| n.checked_add(limits.max_snapshot_bytes.checked_mul(32)?))
            .and_then(|n| n.checked_add(64 * 4096))
            .ok_or_else(bad)?;
        if prepare_work > limits.prepare_work {
            return Err(bad());
        }
        // AFTER cheap numeric/pair negatives, BEFORE first package crypto,
        // decode, snapshot copy or typed SDK restore. Preserve Some on error
        // without treating it as a reusable cumulative allocation account.
        paid.enter_preparation()?;
        let mut package_budget =
            PackageBudget::new(limits.package_typed_bytes, limits.package_work);
        let package = text_add_package::validate(
            device,
            request.frame,
            &original.group,
            &request.subject,
            &request.nonce,
            &request.reference,
            request.now_ms,
            &mut package_budget,
        )?;
        let credential_digest = *package.identity().credential_digest();
        let output_remaining = limits
            .output_bytes
            .checked_sub(request.frame.len())
            .ok_or_else(bad)?;
        let package_frame = request.frame.to_vec();
        let snapshot = crate::persist::snapshot_server_bounded(
            device,
            group,
            limits.max_snapshot_bytes,
            limits.max_provider_backing_bytes,
        )?;
        invoice::require(limits, snapshot.len())?;
        let original_snapshot_id = *blake3::hash(&snapshot).as_bytes();
        let graph =
            text_current_add::restore_candidate(&snapshot, original.epoch, original.own, paid)?;
        if graph.group.epoch() != original.epoch
            || graph.group.epoch_authenticator_id() != original.authenticator
            || graph.group.group_id_ref() != original.group
            || graph.device.public_key_ref() != original.author
        {
            return Err(bad());
        }
        original.require_same(device, group)?;
        // No fallible/allocating operation follows the one owning input cut.
        let paid = admission.take().expect("validated private Add admission");
        *output = Some(Self {
            #[cfg(test)]
            faults: Default::default(),
            outputs: None,
            commit: None,
            welcome: None,
            group_info: None,
            snapshot: None,
            observation: None,
            package: Some(package),
            package_frame,
            subject: request.subject,
            nonce: request.nonce,
            reference: request.reference,
            credential_digest,
            now_ms: request.now_ms,
            original_snapshot_id,
            phase: Phase::Prepared,
            output_remaining,
            provider_closed: false,
            graph,
            original,
            admission: paid,
        });
        Ok(())
    }

    pub fn original_epoch_data(&self) -> u64 {
        self.original.epoch
    }
    /// Digest of this exact private-fork source encoding DATA. Provider maps
    /// have no canonical iteration order: cold-restoring equal SDK state may
    /// produce different bytes. This is never a SavedSource ID, a portable SDK
    /// state identity or authority to adopt a cold graph. Root must separately
    /// bind its genuine selected source and durable governance request.
    pub fn original_snapshot_id_data(&self) -> [u8; 32] {
        self.original_snapshot_id
    }
    pub fn subject_data(&self) -> &[u8; 32] {
        &self.subject
    }
    pub fn key_package_reference_data(&self) -> &[u8; 32] {
        &self.reference
    }
    pub fn is_complete_data(&self) -> bool {
        self.phase == Phase::Complete
    }
}

#[cfg(test)]
mod constructor;
