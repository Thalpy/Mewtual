//! Exact-receiver ownership handoff, separate from Root's covering save and
//! final Source/native admission. No SDK owner can be extracted or replaced
//! through a caller-supplied constructor. Both receiver halves stay closed.
use super::*;

/// Full preflight has completed and these exclusive loans prevent the receiver
/// or candidate changing before the field-move edge. This is NOT Source,
/// governance, native-session or covering-save evidence. The owning host must
/// establish those independently immediately before `install`.
///
/// Drop/cancellation leaves the original candidate in its original slot and
/// the receiver unchanged. No work is retried, generated or disposed on Drop.
///
/// Exclusive receiver loans also prevent a caller changing or inspecting its
/// SDK pair between preflight and the eventual final admission:
/// ```compile_fail,E0502
/// use catcoms_mls::{MlsDevice, ServerGroup};
/// use catcoms_mls::group::text_current_add::PreparedTextCurrentAdd;
/// fn no_intervening_receiver_access(slot: &mut Option<PreparedTextCurrentAdd>,
///                                  d: &mut MlsDevice, g: &mut ServerGroup) {
///     let prepared = PreparedTextCurrentAdd::prepare_install_into(slot, d, g).unwrap();
///     let _ = g.epoch();
///     prepared.cancel();
/// }
/// ```
#[must_use = "keep the exclusive preparation until final admission or cancel it"]
pub struct PreparedTextCurrentAddInstallation<'a> {
    slot: &'a mut Option<PreparedTextCurrentAdd>,
    device: &'a mut MlsDevice,
    group: &'a mut ServerGroup,
}

/// Old graph plus the original operation's buffers and accounting tails. This
/// object has no operational, extraction or adoption API. Dispose of it OUTSIDE
/// host writers/native guards: graph and final-fee destructors may take locks.
/// The installed pair independently retains the new candidate's full debit.
///
/// Do not accidentally discard the return under the final writer:
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use catcoms_mls::group::text_current_add::PreparedTextCurrentAddInstallation;
/// fn cannot_discard_retired(prepared: PreparedTextCurrentAddInstallation<'_>) {
///     prepared.install();
/// }
/// ```
#[must_use = "retain the retired graph and drop it outside host writers"]
pub struct RetiredTextCurrentAdd {
    _cell: PreparedTextCurrentAdd,
}

impl PreparedTextCurrentAdd {
    /// Check the exact live parent and completed candidate before a host's
    /// final no-IO admission edge. This preflight reads SDK/provider state and
    /// may allocate bounded identity/hash scratch; call it outside final locks.
    /// Every error preserves the SAME slot and both original receiver halves.
    ///
    /// This library ownership operation does not authorize installation. Root
    /// must cover the exact candidate/Header/request in its durable save, retain
    /// the selected original Source and independently check native custody.
    pub fn prepare_install_into<'a>(
        slot: &'a mut Option<Self>,
        device: &'a mut MlsDevice,
        group: &'a mut ServerGroup,
    ) -> Result<PreparedTextCurrentAddInstallation<'a>, MlsError> {
        let cell = slot.as_ref().ok_or_else(bad)?;
        let result = cell.completed_result_data()?;
        cell.original.require_same(device, group)?;
        if !group.is_active()
            || !group.has_local_device(&device.device_id())
            || !group.has_local_signer(&cell.original.author)
            || group.group.pending_commit().is_some()
            || group.group.pending_proposals().next().is_some()
        {
            return Err(bad());
        }
        let candidate = &cell.graph;
        let paid = cell.admission.lease();
        if !candidate
            .device
            .sdk_graph_lease
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(p, &paid))
            || !candidate
                .group
                .sdk_graph_lease
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &paid))
            || candidate.group.group_id_ref() != cell.original.group
            || candidate.device.public_key_ref() != cell.original.author
            || !candidate.group.is_active()
            || !candidate
                .group
                .has_local_device(&candidate.device.device_id())
            || !candidate.group.has_local_signer(&cell.original.author)
            || candidate.group.group.own_leaf_index().u32() != cell.original.own
            || candidate.group.epoch() != result.after_epoch_data()
            || candidate.group.epoch_authenticator_id() != *result.after_authenticator_data()
            || candidate.group.group.pending_commit().is_some()
            || candidate.group.group.pending_proposals().next().is_some()
        {
            return Err(bad());
        }
        // `completed_result_data` requires provider_closed, set only after the
        // real candidate guard transitioned to CLOSED. Never remove/unlock that
        // provider policy. Its later Drop cannot reopen or fail the live pair.
        Ok(PreparedTextCurrentAddInstallation {
            slot,
            device,
            group,
        })
    }
}

impl PreparedTextCurrentAddInstallation<'_> {
    /// End the exclusive loans without changing either input. The retained
    /// candidate remains available for an exact retry after a final refusal.
    pub fn cancel(self) {}

    /// Infallible ownership moves only: no allocation, callback, SDK operation,
    /// hashing, provider lock or payload/fee destruction. Root must hold its
    /// actual final admission before calling. Return the old pair for deferred
    /// disposal rather than dropping it under that admission's writers.
    pub fn install(self) -> RetiredTextCurrentAdd {
        let mut cell = self
            .slot
            .take()
            .expect("exclusive preflight retains candidate");
        std::mem::swap(self.device, &mut cell.graph.device);
        std::mem::swap(self.group, &mut cell.graph.group);
        // graph.policy/_lease now still describe the NEW, closed candidate,
        // while graph.device/group are OLD. This retired cell must never enter
        // an operational method again or become a new candidate's parent.
        cell.phase = Phase::Installed;
        RetiredTextCurrentAdd { _cell: cell }
    }
}

impl fmt::Debug for PreparedTextCurrentAddInstallation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTextCurrentAddInstallation")
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for RetiredTextCurrentAdd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetiredTextCurrentAdd")
            .finish_non_exhaustive()
    }
}

pub(crate) fn metadata_layout_bytes() -> Option<usize> {
    let fields = [
        std::mem::size_of::<PreparedTextCurrentAddInstallation<'static>>(),
        std::mem::size_of::<Result<PreparedTextCurrentAddInstallation<'static>, MlsError>>(),
        std::mem::size_of::<RetiredTextCurrentAdd>(),
    ];
    fields
        .into_iter()
        .try_fold(0usize, |sum, bytes| sum.checked_add(bytes.checked_mul(4)?))
}
