//! Read-only completed PRIVATE result. Every reference comes from the same
//! whole Cell; there is no decoder, owning SDK extraction, clone, serialization,
//! Source/governance/save/install/publication capability or cold-state adopter.
use super::*;

/// Borrowed exact completed outputs. The snapshot contains SECRET key/group
/// storage: callers must use a separately paid authenticated sealing/save path,
/// never logs, diagnostics, transport or an unsealed persistence write.
///
/// This view retains no additional graph/Arc/Weak allocation. Its lifetime
/// keeps the whole original Cell and both independent accounting tails alive.
/// A caller that copies output buffers must acquire its own full debit FIRST.
/// Nothing here attests durable governance, selected Source or covering save.
///
/// A live result cannot outlive or permit moving/dropping its owning Cell:
/// ```compile_fail,E0505
/// use catcoms_mls::group::text_current_add::PreparedTextCurrentAdd;
/// fn cannot_drop_owner(cell: PreparedTextCurrentAdd) {
///     let result = cell.completed_result_data().unwrap();
///     drop(cell);
///     std::hint::black_box(result.commit_data());
/// }
/// ```
/// A secret buffer loan has the same owner lifetime even after its view ends:
/// ```compile_fail,E0515
/// use catcoms_mls::group::text_current_add::PreparedTextCurrentAdd;
/// fn cannot_escape(cell: PreparedTextCurrentAdd) -> &'static [u8] {
///     cell.completed_result_data().unwrap().secret_snapshot_data()
/// }
/// ```
/// The view itself cannot be accidentally passed to a generic serializer:
/// ```compile_fail,E0277
/// use catcoms_mls::group::text_current_add::CompletedTextCurrentAddData;
/// fn cannot_serialize(result: CompletedTextCurrentAddData<'_>) {
///     let _ = serde_json::to_vec(&result);
/// }
/// ```
pub struct CompletedTextCurrentAddData<'a> {
    cell: &'a PreparedTextCurrentAdd,
    commit: &'a [u8],
    welcome: &'a [u8],
    group_info: &'a [u8],
    snapshot: &'a [u8],
    observation: &'a LocalMembershipChange,
}
impl fmt::Debug for CompletedTextCurrentAddData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompletedTextCurrentAddData")
            .field("before_epoch", &self.observation.before_epoch())
            .field("after_epoch", &self.observation.after_epoch())
            .finish_non_exhaustive()
    }
}
impl PreparedTextCurrentAdd {
    /// Borrow only a real COMPLETE Cell's original cached result. This performs
    /// cached comparisons only: no SDK, clock/RNG, hashing, codec, allocation,
    /// callbacks, Source or live-parent operation runs. Real Complete follows
    /// provider closure, actual observation and final snapshot caching; an
    /// uncertain/partial/failed SDK operation cannot acquire this view.
    pub fn completed_result_data(&self) -> Result<CompletedTextCurrentAddData<'_>, MlsError> {
        if self.phase != Phase::Complete
            || !self.provider_closed
            || self.outputs.is_none()
            || self.package.is_some()
        {
            return Err(bad());
        }
        let commit = self.commit.as_deref().ok_or_else(bad)?;
        let welcome = self.welcome.as_deref().ok_or_else(bad)?;
        let group_info = self.group_info.as_deref().ok_or_else(bad)?;
        let snapshot = self.snapshot.as_ref().ok_or_else(bad)?.as_slice();
        let observation = self.observation.as_ref().ok_or_else(bad)?;
        if commit.is_empty()
            || welcome.is_empty()
            || group_info.is_empty()
            || snapshot.is_empty()
            || observation.kind() != LocalMembershipChangeKind::Add
            || observation.group() != self.original.group
            || observation.author() != &self.original.author
            || observation.subject() != &self.subject
            || observation.before_epoch() != self.original.epoch
            || self.original.epoch.checked_add(1) != Some(observation.after_epoch())
            || observation.commit() != commit
            || observation.key_package_reference() != Some(self.reference)
            || observation.credential_digest() != Some(self.credential_digest)
        {
            return Err(bad());
        }
        Ok(CompletedTextCurrentAddData {
            cell: self,
            commit,
            welcome,
            group_info,
            snapshot,
            observation,
        })
    }
}
impl<'a> CompletedTextCurrentAddData<'a> {
    pub fn commit_data(&self) -> &'a [u8] {
        self.commit
    }
    pub fn welcome_data(&self) -> &'a [u8] {
        self.welcome
    }
    pub fn group_info_data(&self) -> &'a [u8] {
        self.group_info
    }
    /// SECRET complete provider snapshot. It is not transport or public DATA.
    /// Its bytes use the existing noncanonical provider-map snapshot encoding.
    pub fn secret_snapshot_data(&self) -> &'a [u8] {
        self.snapshot
    }
    /// Actual nonserialized operation evidence, borrowed without Clone/decoder.
    /// It remains historical DATA and grants no save/publication permission.
    pub fn membership_change_data(&self) -> &'a LocalMembershipChange {
        self.observation
    }
    pub fn after_epoch_data(&self) -> u64 {
        self.observation.after_epoch()
    }
    /// The original successful operation's cached after-state fingerprint DATA.
    /// This is NOT fresh SDK/Source evidence and performs no re-hash or lookup.
    pub fn after_authenticator_data(&self) -> &'a [u8; 32] {
        self.observation.private_text_add_after_authenticator_data()
    }
    pub fn request_data(&self) -> TextCurrentAddRequestView<'a> {
        TextCurrentAddRequestView { cell: self.cell }
    }
    pub fn original_parent_data(&self) -> TextCurrentAddParentView<'a> {
        TextCurrentAddParentView { cell: self.cell }
    }

    /// Exact live whole-Cell identity, not equal request/digests or a shared
    /// original fee. Rust's immutable borrows pin both Cells for this comparison;
    /// no owned pointer/identity escapes for persistence or cold adoption.
    pub fn require_same_cell_data(
        &self,
        other: &CompletedTextCurrentAddData<'_>,
    ) -> Result<(), MlsError> {
        if std::ptr::eq(self.cell, other.cell) {
            Ok(())
        } else {
            Err(bad())
        }
    }
    /// Historical ACCOUNTING-ORIGIN association only. Both actual wrappers must
    /// still hold the sealed original accounting Arc. This neither recomputes
    /// authenticator/full SDK state nor proves currentness, membership, Source,
    /// governance or permission for another operation. Constructor's original
    /// full pair checks remain separate and unchanged.
    pub fn require_original_parent_accounting_data(
        &self,
        device: &MlsDevice,
        group: &ServerGroup,
    ) -> Result<(), MlsError> {
        match (&device.sdk_graph_lease, &group.sdk_graph_lease) {
            (Some(d), Some(g))
                if Arc::ptr_eq(d, &self.cell.original.lease)
                    && Arc::ptr_eq(g, &self.cell.original.lease) =>
            {
                Ok(())
            }
            _ => Err(bad()),
        }
    }
    /// Exact cached request comparison only, not a KeyPackage/governance/source
    /// validation. Differing input cannot relabel this original completed work.
    pub fn require_request_data(
        &self,
        request: TextCurrentAddRequestData<'_>,
    ) -> Result<(), MlsError> {
        if self.cell.package_frame == request.frame
            && self.cell.subject == request.subject
            && self.cell.nonce == request.nonce
            && self.cell.reference == request.reference
            && self.cell.now_ms == request.now_ms
        {
            Ok(())
        } else {
            Err(bad())
        }
    }
}

/// Receiver-only original validated request DATA, not the public untrusted
/// constructor or independent package/governance validation authority.
pub struct TextCurrentAddRequestView<'a> {
    cell: &'a PreparedTextCurrentAdd,
}
impl fmt::Debug for TextCurrentAddRequestView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextCurrentAddRequestView")
            .finish_non_exhaustive()
    }
}
impl<'a> TextCurrentAddRequestView<'a> {
    pub fn key_package_frame_data(&self) -> &'a [u8] {
        &self.cell.package_frame
    }
    pub fn subject_data(&self) -> &'a [u8; 32] {
        &self.cell.subject
    }
    pub fn invite_nonce_data(&self) -> &'a [u8; 16] {
        &self.cell.nonce
    }
    pub fn key_package_reference_data(&self) -> &'a [u8; 32] {
        &self.cell.reference
    }
    pub fn credential_digest_data(&self) -> &'a [u8; 32] {
        &self.cell.credential_digest
    }
    /// Explicit validation time used by this original operation, not NOW.
    pub fn validation_time_ms_data(&self) -> u64 {
        self.cell.now_ms
    }
}

/// Original immutable parent's cached DATA. Equal labels cannot construct this
/// view or replace a real selected Source/current proof. It has no SDK getter.
pub struct TextCurrentAddParentView<'a> {
    cell: &'a PreparedTextCurrentAdd,
}
impl fmt::Debug for TextCurrentAddParentView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextCurrentAddParentView")
            .field("epoch", &self.cell.original.epoch)
            .finish_non_exhaustive()
    }
}
impl<'a> TextCurrentAddParentView<'a> {
    pub fn group_id_data(&self) -> &'a [u8; 16] {
        &self.cell.original.group
    }
    pub fn author_public_key_data(&self) -> &'a [u8; 32] {
        &self.cell.original.author
    }
    pub fn epoch_data(&self) -> u64 {
        self.cell.original.epoch
    }
    pub fn own_leaf_index_data(&self) -> u32 {
        self.cell.original.own
    }
    pub fn authenticator_data(&self) -> &'a [u8; 32] {
        &self.cell.original.authenticator
    }
    /// Digest of THIS original fork encoding only. HashMap iteration makes
    /// equal cold SDK state potentially encode differently. This is never a
    /// canonical state hash, SavedSource ID or cold-adoption capability.
    pub fn snapshot_encoding_digest_data(&self) -> &'a [u8; 32] {
        &self.cell.original_snapshot_id
    }
}

/// Exact new inline view/Result/input/fixed return moves. Four copies per call
/// conservatively cover argument, local, return and host move. All buffers were
/// already paid/owned by Cell; caller-owned collections/copies need their own
/// separate invoice and are not hidden here as unlimited borrowed aliases.
pub(crate) fn metadata_layout_bytes() -> Option<usize> {
    let fields = [
        std::mem::size_of::<CompletedTextCurrentAddData<'static>>(),
        std::mem::size_of::<Result<CompletedTextCurrentAddData<'static>, MlsError>>(),
        std::mem::size_of::<TextCurrentAddRequestView<'static>>(),
        std::mem::size_of::<TextCurrentAddParentView<'static>>(),
        std::mem::size_of::<TextCurrentAddRequestData<'static>>(),
        std::mem::size_of::<Result<(), MlsError>>(),
        std::mem::size_of::<(u64, u64, u32, u64)>(),
    ];
    fields
        .into_iter()
        .try_fold(0usize, |n, b| n.checked_add(b.checked_mul(4)?))
}
