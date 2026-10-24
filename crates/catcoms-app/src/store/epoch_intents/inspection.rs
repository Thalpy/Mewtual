//! Detached read-only local draft reconstruction. No source, writer or authority is minted.
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_replication::studio::{StudioLocalDraft, StudioTarget};

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
#[derive(Debug)]
pub(crate) struct StudioInspectedDraft {
    pub(crate) target: StudioTarget,
    pub(crate) prepared: bool,
    pub(crate) draft: Option<StudioLocalDraft>,
}
impl StudioInspectionCapture {
    pub(crate) fn rebuild(self) -> Result<(StudioInspectionStamp, StudioInspectedDraft), AppError> {
        let mut result = StudioInspectedDraft {
            target: self.stamp.target,
            prepared: false,
            draft: None,
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
                result.draft = state.local_draft()?;
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
