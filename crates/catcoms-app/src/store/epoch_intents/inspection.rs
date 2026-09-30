//! Detached read-only local draft reconstruction. No source, writer or authority is minted.
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_replication::studio::{StudioDraftArchive, StudioLocalDraft, StudioTarget};

pub(crate) struct StudioInspectionCapture {
    stamp: StudioInspectionStamp,
    plain: Option<Zeroizing<Vec<u8>>>,
}
pub(crate) struct StudioInspectionStamp {
    mount: Arc<()>,
    server: u64,
    document: LogicalDocument,
    target: StudioTarget,
    author: DeviceId,
    version: Option<(blake3::Hash, u64)>,
}
impl StudioInspectionStamp {
    /// The target this stamp was taken for, so a currency check cannot be pointed at another.
    pub(crate) fn target(&self) -> StudioTarget {
        self.target
    }
}
/// What this rebuild is for. The authorization-shaped checks are identical in every arm; what
/// differs is whether typed reconstruction is a **requirement** or an **observation**.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StudioInspectionPurpose {
    /// Existing behaviour: a `StudioLocalDraft` projection, and a reconstruction failure is the
    /// answer.
    Draft,
    /// Structural decode plus the ledger's own envelopes. Typed reconstruction is **attempted and
    /// labelled, never required**: the archive's entries come from the branch's saved order and
    /// its envelopes from the ledger, so a branch nobody can replay archives exactly as well as
    /// one anybody can. Refusing to archive a draft because it cannot be rebuilt would destroy
    /// precisely the work most worth preserving.
    Archive,
}

#[derive(Debug)]
pub(crate) struct StudioInspectedDraft {
    pub(crate) target: StudioTarget,
    /// What this rebuild was asked for, carried so a caller that finished an archive against a
    /// `Draft` rebuild is told *that*, rather than being told there is no branch to archive. The
    /// two produce the same `archive: None` and are not the same mistake.
    pub(crate) purpose: StudioInspectionPurpose,
    pub(crate) prepared: bool,
    pub(crate) draft: Option<StudioLocalDraft>,
    /// Present only for [`StudioInspectionPurpose::Archive`], and only when a branch is live.
    pub(crate) archive: Option<StudioDraftArchive>,
    /// `Ok(())` when typed reconstruction succeeded, `Err(reason)` when it did not. Under
    /// `Archive` an `Err` still yields a complete archive; under `Draft` this arm is unreachable
    /// because the failure was returned instead.
    pub(crate) replayable: Result<(), String>,
}
impl StudioInspectionCapture {
    pub(crate) fn rebuild(self) -> Result<(StudioInspectionStamp, StudioInspectedDraft), AppError> {
        self.rebuild_for(StudioInspectionPurpose::Draft)
    }
    pub(crate) fn rebuild_for(
        self,
        purpose: StudioInspectionPurpose,
    ) -> Result<(StudioInspectionStamp, StudioInspectedDraft), AppError> {
        let mut result = StudioInspectedDraft {
            target: self.stamp.target,
            purpose,
            prepared: false,
            draft: None,
            archive: None,
            replayable: Ok(()),
        };
        if let Some(plain) = self.plain {
            let scope = scope_bytes(self.stamp.server, &self.stamp.document)?;
            let state = EpochIntentState::decode(&plain, &scope, &self.stamp.document)?;
            if let Some(metadata) = state.handoff_metadata() {
                metadata.check_target(self.stamp.target).map_err(invalid)?;
                if metadata
                    .overlay()
                    .is_some_and(|o| o.author() != self.stamp.author)
                {
                    return Err(invalid("overlay inspection author mismatch"));
                }
                result.prepared = metadata.is_prepared();
                match purpose {
                    StudioInspectionPurpose::Draft => result.draft = state.local_draft()?,
                    StudioInspectionPurpose::Archive => {
                        // The attempt runs first so its outcome can be recorded *in* the archive,
                        // and a successful one is kept: the caller that archives a replayable
                        // draft should not have to pay for a second reconstruction to show it.
                        result.replayable = match state.local_draft() {
                            Ok(draft) => {
                                result.draft = draft;
                                Ok(())
                            }
                            Err(error) => Err(error.to_string()),
                        };
                        if let (Some(overlay), Some(live)) = (state.overlay(), state.live_branch()?)
                        {
                            result.archive = Some(
                                StudioDraftArchive::from_branch(
                                    overlay,
                                    &state.ledger,
                                    metadata.provenance(),
                                    result.replayable.is_ok(),
                                    live.id,
                                    live.content,
                                    metadata.branch_generation(),
                                )
                                .map_err(invalid)?,
                            );
                        }
                    }
                }
            }
        }
        Ok((self.stamp, result))
    }
}
impl ServerStore {
    /// Caller already owns one shared preparation permit and current live membership custody.
    pub(crate) fn capture_studio_inspection(
        &self,
        server: u64,
        group: &[u8],
        target: StudioTarget,
        author: DeviceId,
    ) -> Result<StudioInspectionCapture, AppError> {
        let document = target.document(group).map_err(invalid)?;
        let raw = self.read_scoped_intent_plain(&scope_bytes(server, &document)?)?;
        Ok(StudioInspectionCapture {
            stamp: StudioInspectionStamp {
                mount: self.registry_mount(),
                server,
                document,
                target,
                author,
                version: raw
                    .as_ref()
                    .map(|r| (blake3::hash(&r.plain), r.physical_bytes)),
            },
            plain: raw.map(|r| r.plain),
        })
    }
    pub(crate) fn studio_inspection_is_current(
        &self,
        server: u64,
        group: &[u8],
        target: StudioTarget,
        author: DeviceId,
        stamp: &StudioInspectionStamp,
    ) -> Result<bool, AppError> {
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount())
            || stamp.server != server
            || stamp.document.server_id != group
            || stamp.target != target
            || stamp.author != author
        {
            return Ok(false);
        }
        let raw = self.read_scoped_intent_plain(&scope_bytes(server, &stamp.document)?)?;
        let version = raw
            .as_ref()
            .map(|r| (blake3::hash(&r.plain), r.physical_bytes));
        Ok(version == stamp.version)
    }
}
