//! Verify one exact private membership Commit on a separately paid graph.
catcoms_rt::redacted_debug! {
    TextCurrentReceivedAddData<'_>;
    TextCurrentReceivedRemoveData<'_>;
    VerifiedTextCurrentEviction;
    PreparedTextCurrentReceivedAdd;
    PreparedTextReceivedAddInstallation<'_>;
    RetiredTextReceivedAdd;
}

use super::*;
use crate::device::key_package_identity::credential_digest_with;
use openmls_traits::OpenMlsProvider;

pub const MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES: usize = 32 * 1024;

/// Public expectations from one signed governing Admission. The host must
/// authenticate the Admission and original selected prefix independently.
pub struct TextCurrentReceivedAddData<'a> {
    pub commit: &'a [u8],
    pub digest: [u8; 32],
    pub epoch: u64,
    pub committer: [u8; 32],
    pub subject: [u8; 32],
    pub key_package: [u8; 32],
    pub credential: [u8; 32],
}

/// The signer, exact serialized Commit and removed subject named by an
/// independently verified governing Removal. This path serves survivors.
pub struct TextCurrentReceivedRemoveData<'a> {
    pub commit: &'a [u8],
    pub digest: [u8; 32],
    pub epoch: u64,
    pub committer: [u8; 32],
    pub subject: [u8; 32],
}

#[derive(Clone, Copy)]
enum Change {
    Add {
        key_package: [u8; 32],
        credential: [u8; 32],
    },
    Remove {
        local: bool,
    },
}

struct Expected<'a> {
    commit: &'a [u8],
    digest: [u8; 32],
    epoch: u64,
    committer: [u8; 32],
    subject: [u8; 32],
    change: Change,
}

struct StagedMembership {
    graph: CandidateGraph,
    staged: Box<StagedCommit>,
    original: OriginalParent,
    roster: Vec<DeviceId>,
    subject: DeviceId,
    epoch: u64,
}

/// This node was the exact leaf removed by the authenticated Commit. The
/// disposable graph is retired before this proof is returned.
pub struct VerifiedTextCurrentEviction {
    original: OriginalParent,
    epoch: u64,
    digest: [u8; 32],
}

impl VerifiedTextCurrentEviction {
    pub fn verify_own_eviction(
        device: &MlsDevice,
        group: &ServerGroup,
        request: TextCurrentReceivedRemoveData<'_>,
        admission: TextCurrentAddAdmission,
    ) -> Result<Self, MlsError> {
        let expected = remove_request(request, true);
        let proof =
            PreparedTextCurrentReceivedAdd::stage_change(device, group, &expected, admission)?;
        Ok(Self {
            original: proof.original,
            epoch: proof.epoch,
            digest: expected.digest,
        })
    }

    pub fn require_original_data(
        &self,
        device: &MlsDevice,
        group: &ServerGroup,
    ) -> Result<(), MlsError> {
        self.original.require_same(device, group)
    }

    pub fn removed_data(&self) -> (u64, [u8; 32]) {
        (self.epoch, self.digest)
    }
}

fn remove_request(request: TextCurrentReceivedRemoveData<'_>, local: bool) -> Expected<'_> {
    Expected {
        commit: request.commit,
        digest: request.digest,
        epoch: request.epoch,
        committer: request.committer,
        subject: request.subject,
        change: Change::Remove { local },
    }
}

/// Owns the closed candidate and original receiver identity. There is no
/// raw extraction or live operation on the candidate before final selection.
pub struct PreparedTextCurrentReceivedAdd {
    graph: CandidateGraph,
    snapshot: Zeroizing<Vec<u8>>,
    original: OriginalParent,
    epoch: u64,
    authenticator: [u8; 32],
}

impl PreparedTextCurrentReceivedAdd {
    pub fn prepare(
        device: &MlsDevice,
        group: &ServerGroup,
        request: TextCurrentReceivedAddData<'_>,
        admission: TextCurrentAddAdmission,
    ) -> Result<Self, MlsError> {
        Self::prepare_change(
            device,
            group,
            Expected {
                commit: request.commit,
                digest: request.digest,
                epoch: request.epoch,
                committer: request.committer,
                subject: request.subject,
                change: Change::Add {
                    key_package: request.key_package,
                    credential: request.credential,
                },
            },
            admission,
        )
    }

    pub fn prepare_remove(
        device: &MlsDevice,
        group: &ServerGroup,
        request: TextCurrentReceivedRemoveData<'_>,
        admission: TextCurrentAddAdmission,
    ) -> Result<Self, MlsError> {
        Self::prepare_change(device, group, remove_request(request, false), admission)
    }

    fn prepare_change(
        device: &MlsDevice,
        group: &ServerGroup,
        request: Expected<'_>,
        admission: TextCurrentAddAdmission,
    ) -> Result<Self, MlsError> {
        let limits = admission.limits();
        let StagedMembership {
            mut graph,
            staged,
            original,
            roster,
            subject,
            epoch,
        } = Self::stage_change(device, group, &request, admission)?;
        if matches!(request.change, Change::Remove { local: true }) {
            return Err(bad());
        }
        let provider = graph.device.provider();
        graph.policy.enter_received_merge().map_err(|_| bad())?;
        graph
            .group
            .group
            .merge_staged_commit(provider, *staged)
            .map_err(proto)?;
        graph
            .policy
            .close_merged(provider.storage())
            .map_err(|_| bad())?;
        let next = graph.group.member_device_ids();
        let roster_matches = match request.change {
            Change::Add { .. } => {
                next.len() == roster.len() + 1
                    && next.contains(&subject)
                    && roster.iter().all(|device| next.contains(device))
            }
            Change::Remove { .. } => {
                next.len() + 1 == roster.len()
                    && !next.contains(&subject)
                    && roster
                        .iter()
                        .filter(|device| **device != subject)
                        .all(|device| next.contains(device))
            }
        };
        if !graph.group.is_active()
            || graph.group.epoch() != epoch
            || graph.group.group.own_leaf_index().u32() != original.own
            || !roster_matches
        {
            return Err(bad());
        }
        let authenticator = graph.group.epoch_authenticator_id();
        let snapshot = crate::persist::snapshot_server_bounded(
            &graph.device,
            &graph.group,
            limits.max_snapshot_bytes,
            limits.max_provider_backing_bytes,
        )?;
        original.require_same(device, group)?;
        Ok(Self {
            graph,
            snapshot,
            original,
            epoch,
            authenticator,
        })
    }

    fn stage_change(
        device: &MlsDevice,
        group: &ServerGroup,
        request: &Expected<'_>,
        mut admission: TextCurrentAddAdmission,
    ) -> Result<StagedMembership, MlsError> {
        if request.commit.is_empty()
            || request.commit.len() > MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES
            || blake3::hash(request.commit).as_bytes() != &request.digest
        {
            return Err(bad());
        }
        let original = OriginalParent::observe_membership(
            device,
            group,
            matches!(request.change, Change::Add { .. }),
        )?;
        let committer = group
            .group
            .members()
            .min_by_key(|member| member.index.u32())
            .ok_or_else(bad)?;
        let sender = committer.index;
        let subject = DeviceId::from_public_key_bytes(&request.subject);
        let roster = group.member_device_ids();
        if committer.signature_key.as_slice() != request.committer
            || sender.u32() == original.own
            || original.epoch != request.epoch
        {
            return Err(bad());
        }
        let removed = match request.change {
            Change::Add { .. } if !roster.contains(&subject) => None,
            Change::Add { .. } => return Err(bad()),
            Change::Remove { local } => Some(
                group
                    .group
                    .members()
                    .find(|member| member.signature_key.as_slice() == request.subject)
                    .map(|member| member.index)
                    .filter(|leaf| (leaf.u32() == original.own) == local && *leaf != sender)
                    .ok_or_else(bad)?,
            ),
        };
        check_commit_header(
            request.commit,
            &original.group,
            request.epoch,
            original.epoch,
        )?;
        let message = MlsMessageIn::tls_deserialize_exact(request.commit)
            .map_err(proto)?
            .try_into_protocol_message()
            .map_err(|_| bad())?;
        let limits = admission.limits();
        invoice::require(limits, 0)?;
        let work = limits
            .max_provider_backing_bytes
            .checked_mul(513)
            .and_then(|work| work.checked_add(limits.max_snapshot_bytes.checked_mul(32)?))
            .and_then(|work| work.checked_add(64 * 4096 + MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES))
            .ok_or_else(bad)?;
        if work > limits.prepare_work {
            return Err(bad());
        }
        admission.enter_preparation()?;
        let snapshot = crate::persist::snapshot_server_bounded(
            device,
            group,
            limits.max_snapshot_bytes,
            limits.max_provider_backing_bytes,
        )?;
        invoice::require(limits, snapshot.len())?;
        let mut graph = text_current_add::restore_candidate(
            &snapshot,
            original.epoch,
            original.own,
            &admission,
        )?;
        drop(snapshot);
        if graph.group.epoch() != original.epoch
            || graph.group.epoch_authenticator_id() != original.authenticator
            || graph.group.group_id_ref() != original.group
            || graph.device.public_key_ref() != original.author
        {
            return Err(bad());
        }
        graph.policy.enter_receive().map_err(|_| bad())?;
        let provider = graph.device.provider();
        let locally_removed = matches!(request.change, Change::Remove { local: true });
        let processed = match removed {
            Some(_) if locally_removed => graph
                .group
                .group
                .process_private_text_eviction(provider, message, sender),
            Some(leaf) => graph
                .group
                .group
                .process_private_text_remove(provider, message, sender, leaf),
            None => graph
                .group
                .group
                .process_private_text_add(provider, message, sender),
        }
        .map_err(proto)?;
        let ProcessedMessageContent::StagedCommitMessage(staged) = processed.into_content() else {
            return Err(bad());
        };
        let epoch = request.epoch.checked_add(1).ok_or_else(bad)?;
        if staged.self_removed() != locally_removed || staged.epoch().as_u64() != epoch {
            return Err(bad());
        }
        if let Change::Add {
            key_package,
            credential,
        } = request.change
        {
            let mut adds = staged.add_proposals();
            let add = adds.next().ok_or_else(bad)?;
            let package = add.add_proposal().key_package();
            let leaf = package.leaf_node();
            let membership = membership_from_key_package(package)?;
            if adds.next().is_some()
                || membership.group_id.as_slice() != original.group
                || membership.device_id != subject
                || leaf.signature_key().as_slice() != request.subject
                || package
                    .hash_ref(provider.crypto())
                    .map_err(proto)?
                    .as_slice()
                    != key_package
                || credential_digest_with(leaf.credential(), || {})? != credential
            {
                return Err(bad());
            }
        }
        original.require_same(device, group)?;
        Ok(StagedMembership {
            graph,
            staged,
            original,
            roster,
            subject,
            epoch,
        })
    }

    pub fn selected_epoch_data(&self) -> (u64, [u8; 32]) {
        (self.epoch, self.authenticator)
    }

    pub fn snapshot_data(&self) -> &[u8] {
        &self.snapshot
    }

    /// Exclusive, already checked loans. The host must durably cover this
    /// exact candidate and authorize its original Current before installing.
    pub fn prepare_install<'a>(
        slot: &'a mut Option<Self>,
        device: &'a mut MlsDevice,
        group: &'a mut ServerGroup,
    ) -> Result<PreparedTextReceivedAddInstallation<'a>, MlsError> {
        let original = slot.as_ref().ok_or_else(bad)?;
        original.original.require_same(device, group)?;
        Ok(PreparedTextReceivedAddInstallation {
            slot,
            device,
            group,
        })
    }
}

#[must_use]
pub struct PreparedTextReceivedAddInstallation<'a> {
    slot: &'a mut Option<PreparedTextCurrentReceivedAdd>,
    device: &'a mut MlsDevice,
    group: &'a mut ServerGroup,
}

#[must_use = "drop the old SDK graph after releasing the final writer"]
pub struct RetiredTextReceivedAdd {
    _old: PreparedTextCurrentReceivedAdd,
}

impl PreparedTextReceivedAddInstallation<'_> {
    pub fn install(self) -> RetiredTextReceivedAdd {
        let mut candidate = self.slot.take().expect("exclusive received Add candidate");
        std::mem::swap(self.device, &mut candidate.graph.device);
        std::mem::swap(self.group, &mut candidate.graph.group);
        RetiredTextReceivedAdd { _old: candidate }
    }
}

pub(crate) const fn metadata_layout_bytes() -> usize {
    4 * (std::mem::size_of::<PreparedTextCurrentReceivedAdd>()
        + std::mem::size_of::<StagedMembership>()
        + std::mem::size_of::<VerifiedTextCurrentEviction>()
        + std::mem::size_of::<PreparedTextReceivedAddInstallation<'static>>()
        + std::mem::size_of::<RetiredTextReceivedAdd>())
}
