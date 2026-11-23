//! Same-parent paid private preparation of one surviving-founder Remove.
use super::*;
use openmls_traits::OpenMlsProvider;

struct Complete {
    commit: Vec<u8>,
    snapshot: Zeroizing<Vec<u8>>,
    fingerprint: (u64, [u8; 32]),
}

pub struct CompletedTextCurrentRemoveData<'a> {
    complete: &'a Complete,
    target: &'a [u8; 32],
}

impl CompletedTextCurrentRemoveData<'_> {
    pub fn commit_data(&self) -> &[u8] {
        &self.complete.commit
    }
    pub fn secret_snapshot_data(&self) -> &[u8] {
        &self.complete.snapshot
    }
    pub fn selected_epoch_data(&self) -> (u64, [u8; 32]) {
        self.complete.fingerprint
    }
    pub fn target_data(&self) -> &[u8; 32] {
        self.target
    }
}

/// The host must reserve the original governing intent and establish the
/// separate native key authority before it enters this closed candidate.
pub struct PreparedTextCurrentRemove {
    outputs: Option<SdkOutputs>,
    complete: Option<Complete>,
    target: [u8; 32],
    index: LeafNodeIndex,
    entered: bool,
    original: OriginalParent,
    graph: CandidateGraph,
    admission: TextCurrentAddAdmission,
}

impl std::fmt::Debug for PreparedTextCurrentRemove {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedTextCurrentRemove")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for CompletedTextCurrentRemoveData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletedTextCurrentRemoveData")
            .finish_non_exhaustive()
    }
}

impl PreparedTextCurrentRemove {
    pub fn prepare(
        device: &MlsDevice,
        group: &ServerGroup,
        target: [u8; 32],
        mut admission: TextCurrentAddAdmission,
    ) -> Result<Self, MlsError> {
        let original = OriginalParent::observe_membership(device, group, false)?;
        let mut members = group.group.members();
        let index = members
            .find(|member| member.signature_key.as_slice() == target)
            .map(|member| member.index)
            .filter(|index| index.u32() != original.own)
            .ok_or_else(bad)?;
        let committer = group
            .group
            .members()
            .min_by_key(|member| member.index.u32())
            .ok_or_else(bad)?;
        if committer.index.u32() != original.own
            || committer.signature_key.as_slice() != original.author
        {
            return Err(bad());
        }
        let limits = admission.limits();
        let work = limits
            .max_provider_backing_bytes
            .checked_mul(513)
            .and_then(|work| work.checked_add(limits.max_snapshot_bytes.checked_mul(32)?))
            .and_then(|work| work.checked_add(64 * 4096))
            .ok_or_else(bad)?;
        if work > limits.prepare_work {
            return Err(bad());
        }
        invoice::require(limits, 0)?;
        admission.enter_preparation()?;
        let snapshot = crate::persist::snapshot_server_bounded(
            device,
            group,
            limits.max_snapshot_bytes,
            limits.max_provider_backing_bytes,
        )?;
        invoice::require(limits, snapshot.len())?;
        let graph = text_current_add::restore_candidate(
            &snapshot,
            original.epoch,
            original.own,
            &admission,
        )?;
        if graph.group.epoch() != original.epoch
            || graph.group.epoch_authenticator_id() != original.authenticator
            || graph.group.group_id_ref() != original.group
            || graph.device.public_key_ref() != original.author
        {
            return Err(bad());
        }
        original.require_same(device, group)?;
        Ok(Self {
            outputs: None,
            complete: None,
            target,
            index,
            entered: false,
            original,
            graph,
            admission,
        })
    }

    /// Enter once. Any error after entry retains the original private
    /// staging owner and grants no further builder or merge invocation.
    pub fn execute_private(
        &mut self,
        device: &MlsDevice,
        group: &ServerGroup,
    ) -> Result<(), MlsError> {
        self.original.require_same(device, group)?;
        if self.complete.is_some() {
            return Ok(());
        }
        if self.entered {
            return Err(bad());
        }
        self.entered = true;
        let result = self.execute_once();
        if result.is_err() {
            self.graph.policy.fail();
        }
        result
    }

    fn execute_once(&mut self) -> Result<(), MlsError> {
        let device = &self.graph.device;
        let roster = self.graph.group.member_device_ids();
        let limits = self.admission.limits();
        self.graph.policy.enter_add().map_err(|_| bad())?;
        let (commit, welcome, group_info) = self
            .graph
            .group
            .group
            .commit_builder()
            .consume_proposal_store(true)
            .force_self_update(true)
            .propose_removals([self.index])
            .load_psks(device.provider().storage())
            .and_then(|builder| {
                builder.build_private_text_remove(
                    device.provider().rand(),
                    device.provider().crypto(),
                    device.signer(),
                    self.index,
                )
            })
            .map_err(proto)?
            .stage_commit(device.provider())
            .map_err(proto)?
            .into_messages();
        self.outputs = Some(SdkOutputs {
            commit,
            welcome,
            group_info,
        });
        self.graph.policy.outputs_cached().map_err(|_| bad())?;
        let outputs = self.outputs.as_ref().ok_or_else(bad)?;
        if outputs.welcome.is_some() {
            return Err(bad());
        }
        let mut remaining = limits.output_bytes.min(MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES);
        let commit = super::work::encode(&outputs.commit, &mut remaining)?;
        self.graph.policy.enter_merge().map_err(|_| bad())?;
        self.graph
            .group
            .group
            .merge_pending_commit(device.provider())
            .map_err(proto)?;
        self.graph
            .policy
            .close_merged(device.provider().storage())
            .map_err(|_| bad())?;
        let epoch = self.original.epoch.checked_add(1).ok_or_else(bad)?;
        let target = DeviceId::from_public_key_bytes(&self.target);
        let next = self.graph.group.member_device_ids();
        if !self.graph.group.is_active()
            || self.graph.group.epoch() != epoch
            || self.graph.group.group.own_leaf_index().u32() != self.original.own
            || next.len() + 1 != roster.len()
            || next.contains(&target)
            || !roster
                .iter()
                .filter(|device| **device != target)
                .all(|device| next.contains(device))
        {
            return Err(bad());
        }
        let fingerprint = (epoch, self.graph.group.epoch_authenticator_id());
        let snapshot = crate::persist::snapshot_server_bounded(
            device,
            &self.graph.group,
            limits.max_snapshot_bytes,
            limits.max_provider_backing_bytes,
        )?;
        self.complete = Some(Complete {
            commit,
            snapshot,
            fingerprint,
        });
        Ok(())
    }

    pub fn completed_data(&self) -> Option<CompletedTextCurrentRemoveData<'_>> {
        Some(CompletedTextCurrentRemoveData {
            complete: self.complete.as_ref()?,
            target: &self.target,
        })
    }
}

pub(crate) const fn metadata_layout_bytes() -> usize {
    4 * (std::mem::size_of::<PreparedTextCurrentRemove>()
        + std::mem::size_of::<Result<PreparedTextCurrentRemove, MlsError>>()
        + std::mem::size_of::<CompletedTextCurrentRemoveData<'static>>())
}
