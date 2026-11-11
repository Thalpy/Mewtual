//! HOST trust DATA from the actual independent pin/source expectation and original G.
//! This never adopts an SDK record as its own trust root, makes old G current, or
//! issues a verifier/signing/projection permission. Root pays/bounds the returned DATA.
use super::super::archive::TextArchiveOperationData;
use super::super::head::{TextParentData, TextPendingData, TextRootData};
use super::super::transaction::PreparedV2TextRetainTransaction;
use super::*;
use catcoms_replication::chat::{FounderGenesisAnchorData, MAX_GROUP_ID_BYTES};

/// No caller constructor. The only producer below requires actual original Current,
/// independent pin/source receipts, cached original G and the SAME selected retained A.
pub(in crate::store) struct V2TextFounderAnchorData {
    anchor: FounderGenesisAnchorData,
    // LAST: original historical/source association remains separately prepaid.
    _resources: V2TextReadResources,
}
pub(in crate::store) const V2_TEXT_FOUNDER_ANCHOR_LAYOUT_BYTES: usize =
    2 * std::mem::size_of::<V2TextFounderAnchorData>()
        + 2 * std::mem::size_of::<FounderGenesisAnchorData>()
        + 2 * MAX_GROUP_ID_BYTES
        + 256;
impl V2TextFounderAnchorData {
    /// Cached exact-source association only. Used by Root's private domain binder
    /// after the actual producer's complete SavedA proof; this does not renew it.
    pub(in crate::store) fn require_original_source_data(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self._resources.require_same_original(guard.resources())?;
        self._resources
            .require_bound_original_negative(guard.store(), guard.source())
    }
    /// Immutable DATA loan for the private Root/Core expectation constructor. No
    /// mutable trust-root selector or owning error can discard the original anchor.
    pub(in crate::store) fn core_data(&self) -> &FounderGenesisAnchorData {
        &self.anchor
    }
}

impl SelectedV2TextHead {
    /// Use only the cached historical G that this SAME original A actually retained.
    /// Accounts/codecs/allocations precede a fresh complete SavedA/source/name proof.
    /// Current is A here; attempting to revalidate old O_G as current would be wrong.
    pub(in crate::store) fn founder_anchor_from_retained_data(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<V2TextFounderAnchorData, AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        retained.producer_data().require_selected_data(self)?;
        retained
            .resources()
            .require_same_original(&self.resources)?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        // This authenticates the independently retained original pin and full source
        // expectation, rather than trusting the profile/header supplied by an SDK plan.
        let context = original_context(guard, &key)?;
        let (expected, _, _) = guard.original_genesis_context_data()?;
        let e = expected.context.spec_data();
        let public = guard.source().data();
        let g = self.archive.spec_data();
        if self.head.context != context
            || self.head.generation != 1
            || self.head.previous != TextParentData::Genesis(context.spec_data().genesis_parent)
            || self.head.pending != TextPendingData::None
            || self.head.tokens != TextRootData::Empty
            || self.head.publications != TextRootData::Empty
            || g.context != context.id_data()?
            || g.operation != TextArchiveOperationData::Genesis
            || g.previous.is_some()
            || g.manifest_sequence != 1
            || g.header != public.header
            || g.image != public.state
            || public.header.id() != e.header
            || public.profile.id() != e.profile
            || public.admission.id() != e.admission
            || public.header.spec().sequence != 1
            || e.group.is_empty()
            || e.group.len() > MAX_GROUP_ID_BYTES
        {
            return Err(refused());
        }
        // The trusted context fixes the Header ID; that actual signed Header commits
        // state. Profile-ID equality plus authenticated pin fixes founder/tenure.
        let anchor = FounderGenesisAnchorData {
            group: e.group.to_vec(),
            founder: e.signer,
            profile: e.profile,
            admission: catcoms_replication::chat::owner_facts::OwnerFactId::from_bytes(
                *e.admission.as_bytes(),
            ),
            tenure: public.profile.tenure(),
            header: e.header,
            state: public.header.spec().state,
        };
        let result = V2TextFounderAnchorData {
            anchor,
            _resources: self.resources.clone(),
        };
        retained
            .saved_retained_action()?
            .validate_selected_under_current_guard(guard)?;
        Ok(result)
    }

    /// Genesis HOST anchor for the ORDINARY-member path, with no founder A/I/S/B coupling.
    ///
    /// Same trust source as [`Self::founder_anchor_from_retained_data`]: the independently
    /// retained original pin and full source expectation authenticated by `original_context`,
    /// and the exact selected archive G equal to the public source genesis. The difference is
    /// that it requires THIS selected O to be the empty operational promotion itself
    /// (generation 1, Genesis parent, no pending transaction, empty token/publication roots),
    /// and proves that O is still selected under the guard instead of proving a retained A.
    ///
    /// Consequence the caller must keep: while O is generation 1 the governing prefix is
    /// exactly genesis, so a general verifier built from this anchor is current. Once O
    /// advances (any founder certification), this producer and the caller's per-send
    /// `validate_under_current_guard` both refuse; later prefixes need a separate replay
    /// producer. This anchor is DATA; it issues no verifier, signing or publication right.
    pub(in crate::store) fn genesis_member_anchor_data(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<V2TextFounderAnchorData, AppError> {
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        let context = original_context(guard, &key)?;
        let (expected, _, _) = guard.original_genesis_context_data()?;
        let e = expected.context.spec_data();
        let public = guard.source().data();
        let g = self.archive.spec_data();
        if self.head.context != context
            || self.head.generation != 1
            || self.head.previous != TextParentData::Genesis(context.spec_data().genesis_parent)
            || self.head.pending != TextPendingData::None
            || self.head.tokens != TextRootData::Empty
            || self.head.publications != TextRootData::Empty
            || g.context != context.id_data()?
            || g.operation != TextArchiveOperationData::Genesis
            || g.previous.is_some()
            || g.manifest_sequence != 1
            || g.header != public.header
            || g.image != public.state
            || public.header.id() != e.header
            || public.profile.id() != e.profile
            || public.admission.id() != e.admission
            || public.header.spec().sequence != 1
            || e.group.is_empty()
            || e.group.len() > MAX_GROUP_ID_BYTES
        {
            return Err(refused());
        }
        let anchor = FounderGenesisAnchorData {
            group: e.group.to_vec(),
            founder: e.signer,
            profile: e.profile,
            admission: catcoms_replication::chat::owner_facts::OwnerFactId::from_bytes(
                *e.admission.as_bytes(),
            ),
            tenure: public.profile.tenure(),
            header: e.header,
            state: public.header.spec().state,
        };
        // A crash-left founder WAL or WAL stage means an uncertain founder transaction (for
        // example a retained A for some token) may still be recovered later. The member token
        // namespace must not open beside it, or one request could be authored in both. Only
        // a proven NotFound is absence; any other result, including an IO error, refuses.
        // The founder lane cannot later create one on this host: lane exclusivity forbids it.
        let (target, _, _) = guard.target_directory_data();
        for name in ["text.wal", "text.wal.stage.v1"] {
            match std::fs::symlink_metadata(fixed_child(target, name)?) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(refused()),
            }
        }
        let result = V2TextFounderAnchorData {
            anchor,
            _resources: self.resources.clone(),
        };
        // Last: the SAME frames are still the selected O under this guard.
        self.validate_under_current_guard(guard)?;
        Ok(result)
    }
}
