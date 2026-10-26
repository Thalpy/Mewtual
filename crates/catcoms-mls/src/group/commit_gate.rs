//! The commit-only entry point for membership-commit records (`docs/design-mls-commit-log.md`).
//!
//! [`ServerGroup::process_incoming`] accepts any MLS protocol message and reports success for
//! application messages and proposals as well as for merged commits. The sync layer's commit log
//! must only ever hold merged commits, because catch-up serves that log to lagging members and an
//! entry there is read as "this moved the group one epoch on". [`ServerGroup::apply_commit`] is the
//! entry point for those records: it succeeds only when it merged a staged commit that took the
//! group from exactly `expected_epoch` to `expected_epoch + 1`.
//!
//! The cheap refusals happen before any decryption. An RFC 9420 `PrivateMessage` carries its group
//! id, epoch and content type in the clear, bound into the AEAD's additional data, so a header that
//! lies does not decrypt. Refusing on the header means a wrapped application message or proposal is
//! never opened on this path: its ratchet generation is not consumed and nothing is decrypted
//! on behalf of whoever wrapped it.

use super::*;

/// Why a message was refused on the commit path. Every refusal leaves the group unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommitRefusal {
    /// Not an MLS protocol message at all (a Welcome, GroupInfo, KeyPackage, or undecodable bytes).
    #[error("not an MLS protocol message")]
    NotProtocolMessage,
    /// A `PublicMessage`. Every commit this product produces is a `PrivateMessage`
    /// (`PURE_CIPHERTEXT_WIRE_FORMAT_POLICY`); an external commit is the main thing that is not.
    #[error("not a PrivateMessage")]
    NotCiphertext,
    /// The header's content type is application data or a proposal.
    #[error("content type is {0}, not a commit")]
    NotCommitContent(&'static str),
    /// The header names another group.
    #[error("message is for another group")]
    WrongGroup,
    /// The header's epoch is not the epoch the record claims, or not this group's current epoch.
    #[error("message epoch {found} does not match expected epoch {expected} (group at {current})")]
    WrongEpoch {
        /// The epoch the caller expected the commit to be built at (the record's label).
        expected: u64,
        /// The epoch in the message header.
        found: u64,
        /// This group's epoch when the message was offered.
        current: u64,
    },
    /// openmls processed it, but `process_incoming` did not report a merged commit. Defensive and
    /// unreachable today: a `Commit` header decodes only as a commit body, and our own pending
    /// commit echoed back is refused earlier, as `MlsError::Protocol`, by the own-leaf rule.
    #[error("processing did not yield a commit to merge")]
    NotStagedCommit,
}

/// The cleartext header of a message that claims to be a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitHeader {
    /// The MLS group id in the header.
    pub group_id: Vec<u8>,
    /// The epoch the commit was built at.
    pub epoch: u64,
}

/// Read the cleartext header of `bytes` and refuse anything that is not a `PrivateMessage` with
/// content type `Commit`. Needs no group state and decrypts nothing, so it is safe to run on
/// untrusted input and on records retained from older builds.
///
/// The parse is **exact**: trailing bytes after the message are refused. openmls's own parse in
/// `process_incoming` ignores them, so without this a commit with junk appended would apply and
/// enter the log with the junk, and the junk would let a committer grind the record's
/// `commit_id` (the fork tie-break) for free. Every honest commit serializes exactly.
pub fn commit_header(bytes: &[u8]) -> Result<CommitHeader, CommitRefusal> {
    let msg_in = MlsMessageIn::tls_deserialize_exact(bytes)
        .map_err(|_| CommitRefusal::NotProtocolMessage)?;
    let protocol = msg_in
        .try_into_protocol_message()
        .map_err(|_| CommitRefusal::NotProtocolMessage)?;
    if protocol.wire_format() != WireFormat::PrivateMessage {
        return Err(CommitRefusal::NotCiphertext);
    }
    match protocol.content_type() {
        ContentType::Commit => {}
        ContentType::Application => return Err(CommitRefusal::NotCommitContent("application")),
        ContentType::Proposal => return Err(CommitRefusal::NotCommitContent("proposal")),
    }
    Ok(CommitHeader {
        group_id: protocol.group_id().as_slice().to_vec(),
        epoch: protocol.epoch().as_u64(),
    })
}

/// The header-only part of [`ServerGroup::apply_commit`], for callers that want to refuse before
/// any side effect of their own: `bytes` must be exactly one `PrivateMessage` commit for
/// `group_id`, built at `expected_epoch`, and `expected_epoch` must be `current_epoch`.
/// `apply_commit` runs this same function, so the two cannot drift apart.
pub fn check_commit_header(
    bytes: &[u8],
    group_id: &[u8],
    expected_epoch: u64,
    current_epoch: u64,
) -> Result<(), CommitRefusal> {
    let header = commit_header(bytes)?;
    if header.group_id != group_id {
        return Err(CommitRefusal::WrongGroup);
    }
    if header.epoch != expected_epoch || expected_epoch != current_epoch {
        return Err(CommitRefusal::WrongEpoch {
            expected: expected_epoch,
            found: header.epoch,
            current: current_epoch,
        });
    }
    Ok(())
}

/// A commit that [`ServerGroup::apply_commit`] merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedCommit {
    /// Whether the commit removed at least one member (drives the routing-secret rotation).
    pub removed: bool,
}

impl ServerGroup {
    /// Apply one membership commit that claims to be built at `expected_epoch`.
    ///
    /// Succeeds only if the bytes are exactly one `PrivateMessage` commit for this group, built at
    /// `expected_epoch`, which must also be the current epoch, and processing merged it (openmls
    /// then always moves the group to `expected_epoch + 1`). Every other input is refused with
    /// [`MlsError::NotACommit`] before decryption, or with the openmls error, and the group is left
    /// as it was. The checks [`ServerGroup::process_incoming`] runs on a staged commit (external
    /// sender refusal, Add credential binding, M-1, own-leaf refusal) all still run.
    ///
    /// Use this, never `process_incoming`, for anything that is about to be recorded as a commit.
    pub fn apply_commit(
        &mut self,
        device: &MlsDevice,
        bytes: &[u8],
        expected_epoch: u64,
    ) -> Result<AppliedCommit, MlsError> {
        check_commit_header(bytes, &self.group_id(), expected_epoch, self.epoch())?;
        match self.process_incoming(device, bytes)? {
            Incoming::CommitApplied { removed } => {
                // openmls advances by exactly one on every merge, so no test can break this. It is
                // an assertion, not an error: the merge has already happened, and refusing now
                // would leave the group advanced while the caller skipped its bookkeeping (log,
                // routing rotation, eviction).
                debug_assert_eq!(
                    self.epoch(),
                    expected_epoch + 1,
                    "a merged commit advances the epoch by exactly one"
                );
                Ok(AppliedCommit { removed })
            }
            // Unreachable with a `Commit` header (the body decodes by that content type), and kept
            // as a refusal so that if it ever happens nothing is recorded.
            Incoming::Application(_) | Incoming::Other => {
                Err(CommitRefusal::NotStagedCommit.into())
            }
        }
    }

    /// Test fixture: this member's self-update **proposal**, serialized. The product has no
    /// proposal flow; this exists so tests can offer one where a commit is expected. It also
    /// queues the proposal in this member's own openmls proposal store, so a later commit from
    /// this group would carry it: use it on a group whose further commits the test does not need.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_self_update_proposal(&mut self, device: &MlsDevice) -> Result<Vec<u8>, MlsError> {
        let (message, _reference) = self
            .group
            .propose_self_update(
                device.provider(),
                device.signer(),
                LeafNodeParameters::default(),
            )
            .map_err(proto)?;
        message.tls_serialize_detached().map_err(proto)
    }

    /// Test fixture: an **external commit** into this group by `outsider`, built from a GroupInfo
    /// that `member` exports. The product has no external-join flow and refuses these; this exists
    /// so tests can offer one where a member commit is expected.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_external_commit(
        &self,
        member: &MlsDevice,
        outsider: &MlsDevice,
    ) -> Result<Vec<u8>, MlsError> {
        let info = self
            .group
            .export_group_info(member.provider().crypto(), member.signer(), true)
            .map_err(proto)?
            .tls_serialize_detached()
            .map_err(proto)?;
        let MlsMessageBodyIn::GroupInfo(info) = MlsMessageIn::tls_deserialize(&mut &info[..])
            .map_err(proto)?
            .extract()
        else {
            return Err(MlsError::WrongMessageType);
        };
        // The builder form takes the same inputs; the one-call form keeps the fixture short.
        #[allow(deprecated)]
        let (_group, commit, _info) = MlsGroup::join_by_external_commit(
            outsider.provider(),
            outsider.signer(),
            None,
            info,
            &crate::config::join_config(),
            None,
            None,
            b"",
            outsider.credential(),
        )
        .map_err(proto)?;
        commit.tls_serialize_detached().map_err(proto)
    }
}
