//! Borrow-only exact saved-A query input. This is NOT Selected, Current, a generic source
//! adopter, an accepted-index hit/miss, a reader lease or an SDK/signing permission.
//! Root's actual accepted-query constructor must pay its new complete invoice BEFORE
//! cloning Saved or copying/opening/decoding these borrowed exact encrypted frames.
use super::*;
use catcoms_replication::chat::cold_actions::OriginalActionDigest;
use catcoms_replication::chat::SignedAction;

/// Internal immutable facts loan from the actual Saved issuer. The work module can
/// construct only these borrowed DATA references; it cannot construct the closed query
/// source below. No File/key/Store, raw SDK owner or mutable/owning slot is exposed.
pub(super) struct SavedRetainQueryFactsData<'a> {
    pub(super) archive_path: &'a Path,
    pub(super) archive: &'a [u8],
    pub(super) archive_native: SessionId,
    pub(super) token: &'a [u8],
}

/// Constructor is deliberately private: callers cannot fill labels/paths from an
/// arbitrary parsed head and claim they came from the same original saved SDK Cell.
pub(in crate::store) struct RetainedAQuerySourceData<'a> {
    saved: &'a Arc<SavedV2RetainedAction>,
    resources: &'a V2TextReadResources,
    target: &'a Path,
    target_id: SessionId,
    pages: &'a Path,
    pages_id: SessionId,
    head_path: &'a Path,
    archive_path: &'a Path,
    head_frame: &'a [u8],
    archive_frame: &'a [u8],
    archive_native: SessionId,
    accepted_archive_frame: &'a [u8],
    accepted_archive: &'a TextArchiveData,
    head_id: TextHeadIdData,
    action: &'a SignedAction,
    digest: OriginalActionDigest,
    original: &'a OriginalV2TextRetainTransactionObservation,
}
impl PreparedV2TextRetainTransaction {
    /// Pure same-original DATA association. No account wait, allocation, File/key,
    /// mutable/native SDK operation, codec/signature work or live-source/current success
    /// occurs here. The existing opaque packet is only borrowed through its readonly view.
    /// Borrowing self also keeps the original SDK packet AND its real SDK fee together.
    pub(in crate::store) fn borrow_saved_a_query_source_data(
        &self,
    ) -> Result<RetainedAQuerySourceData<'_>, AppError> {
        let saved = self.saved.as_ref().ok_or_else(refused)?;
        saved
            .original
            .require_same_original(&self.observe_original())?;
        if !Arc::ptr_eq(&saved.parent, &self.parent) {
            return Err(refused());
        }
        saved.resources().require_same_original(self.resources())?;
        let candidate = self.candidate.as_ref().ok_or_else(refused)?;
        if saved.selected.as_slice() != candidate.frame_data()
            || saved.head != TextHeadIdData::from_bytes(*candidate.id_data().as_bytes())
        {
            return Err(refused());
        }
        let facts = saved.selected_facts.borrow_query_data();
        let (_, _, archive) = self
            .producer_data()
            .artifact_frames_data()
            .find(|(role, _, _)| *role == TextArtifactRoleData::Archive)
            .ok_or_else(refused)?;
        let token = self.token.as_ref().ok_or_else(refused)?;
        if facts.archive != archive || facts.token != token.as_bytes() {
            return Err(refused());
        }
        Ok(RetainedAQuerySourceData {
            saved,
            resources: saved.resources(),
            target: &self.parent.target,
            target_id: self.parent.target_id,
            pages: self.parent.pages()?,
            pages_id: self.parent.pages_id,
            head_path: &self.parent.head_path,
            archive_path: facts.archive_path,
            head_frame: &saved.selected,
            archive_frame: facts.archive,
            archive_native: facts.archive_native,
            accepted_archive_frame: &self.parent.archive_frame,
            accepted_archive: &self.parent.archive,
            head_id: saved.head,
            action: self.producer_data().action_data()?,
            digest: self.producer_data().original_action_digest_data()?,
            original: &saved.original,
        })
    }
}
impl<'a> RetainedAQuerySourceData<'a> {
    /// Clone only AFTER the new query has reserved its own complete invoice. The genuine
    /// Saved validator later proves current O_A/H_A/T_A on the SAME original read body.
    pub(in crate::store::persistence::v2_text) fn saved_data(
        &self,
    ) -> &'a Arc<SavedV2RetainedAction> {
        self.saved
    }
    pub(in crate::store::persistence::v2_text) fn resources(&self) -> &'a V2TextReadResources {
        self.resources
    }
    pub(in crate::store::persistence::v2_text) fn current_head_frame(&self) -> &'a [u8] {
        self.head_frame
    }
    pub(in crate::store::persistence::v2_text) fn current_archive_frame(&self) -> &'a [u8] {
        self.archive_frame
    }
    pub(in crate::store::persistence::v2_text) fn current_archive_native_data(&self) -> SessionId {
        self.archive_native
    }
    /// The unchanged accepted Directory is anchored to H_G, not the new unaccepted
    /// retained-action manifest in H_A. Copy this only under the NEW query's own fee.
    pub(in crate::store::persistence::v2_text) fn accepted_archive_frame(&self) -> &'a [u8] {
        self.accepted_archive_frame
    }
    pub(in crate::store::persistence::v2_text) fn accepted_archive_data(
        &self,
    ) -> &'a TextArchiveData {
        self.accepted_archive
    }
    pub(in crate::store::persistence::v2_text) fn target_data(&self) -> (&'a Path, SessionId) {
        (self.target, self.target_id)
    }
    pub(in crate::store::persistence::v2_text) fn pages_data(&self) -> (&'a Path, SessionId) {
        (self.pages, self.pages_id)
    }
    pub(in crate::store::persistence::v2_text) fn paths_data(&self) -> (&'a Path, &'a Path) {
        (self.head_path, self.archive_path)
    }
    pub(in crate::store::persistence::v2_text) fn head_id_data(&self) -> TextHeadIdData {
        self.head_id
    }
    /// Query copies only fixed original-action/digest/signer labels under its own fee.
    /// This original action is not accepted/projected/publishable merely by being lent.
    pub(in crate::store::persistence::v2_text) fn action_data(&self) -> &'a SignedAction {
        self.action
    }
    pub(in crate::store::persistence::v2_text) fn original_action_digest_data(
        &self,
    ) -> OriginalActionDigest {
        self.digest
    }
    pub(in crate::store::persistence::v2_text) fn original_data(
        &self,
    ) -> &'a OriginalV2TextRetainTransactionObservation {
        self.original
    }
}
impl std::fmt::Debug for RetainedAQuerySourceData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedAQuerySourceData")
            .finish_non_exhaustive()
    }
}
