//! Main-window public work discovery only. Historical observation intentionally survives
//! End/lock; the original authenticated byte adapters still authorize all effects/cleanup.
use super::*;
use crate::voice_handoff::ledger::{
    ConfirmationPurpose, PendingStatus, Purpose, WorkMetadata, WorkProjection,
};
use serde::ser::SerializeStruct;

/// One serial bootstrap projection pays this complete bounded graph, replacing rather than
/// overlapping SetupReply. The actual original Row/selection stay owned through serialization.
pub(crate) struct WorkReply {
    owner: String,
    executor: String,
    work: WorkProjection,
    _projection: MetadataProjection,
}

struct Hex<'a>(&'a [u8]);
impl Serialize for Hex<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut buffer = [0u8; 64];
        let used = self.0.len() * 2;
        let output = buffer
            .get_mut(..used)
            .ok_or_else(|| serde::ser::Error::custom(REFUSED))?;
        hex::encode_to_slice(self.0, output).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(std::str::from_utf8(output).map_err(serde::ser::Error::custom)?)
    }
}
struct Decimal(u64);
impl Serialize for Decimal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Pinned serde_json1.0.150 streams collect_str's Display through its writer adapter,
        // without an intermediate String. Only u64's bounded20-digit formatting is used.
        // Re-audit this property if the serializer changes; Tauri uses our fixed writer below.
        serializer.collect_str(&self.0)
    }
}
struct PurposeMetadata(
    Purpose,
    Option<catcoms_sync::voice_v2::presence::PairSelector>,
);
impl Serialize for PurposeMetadata {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_struct("Purpose", 5)?;
        match self.0 {
            Purpose::Derive(selector) => {
                fields.serialize_field("kind", "derive")?;
                fields.serialize_field("peer", &Hex(selector.peer.as_bytes()))?;
                fields.serialize_field("join", &Hex(&selector.join))?;
                fields.serialize_field("keyRef", &Hex(&selector.key_ref))?;
                fields.serialize_field("epoch", &Decimal(selector.epoch))?;
            }
            Purpose::Confirmation { parent, request } => {
                fields.serialize_field("kind", "confirmation")?;
                fields.serialize_field("parent", &Hex(&parent.logical()))?;
                let selector = self.1.ok_or_else(|| serde::ser::Error::custom(REFUSED))?;
                fields.serialize_field(
                    "parentPurpose",
                    &PurposeMetadata(Purpose::Derive(selector), None),
                )?;
                match request {
                    ConfirmationPurpose::Start(predecessor) => {
                        fields.serialize_field("request", "start")?;
                        fields.serialize_field(
                            "predecessor",
                            &predecessor.as_ref().map(|id| Hex(id)),
                        )?;
                    }
                    ConfirmationPurpose::Respond(trigger) => {
                        fields.serialize_field("request", "respond")?;
                        fields.serialize_field("trigger", &Hex(&trigger.logical()))?;
                    }
                    ConfirmationPurpose::Incoming(role) => {
                        fields.serialize_field("request", "incoming")?;
                        fields.serialize_field("role", &role)?;
                    }
                }
            }
            Purpose::Sdp { .. } => return Err(serde::ser::Error::custom(REFUSED)),
        }
        fields.end()
    }
}
struct ActionMetadata((u64, u64, [u8; 32]));
impl Serialize for ActionMetadata {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_struct("Action", 3)?;
        fields.serialize_field("action", &Decimal(self.0 .0))?;
        fields.serialize_field("version", &Decimal(self.0 .1))?;
        fields.serialize_field("inputTag", &Hex(&self.0 .2))?;
        fields.end()
    }
}
struct Descriptor<'a>(&'a WorkMetadata);
impl Serialize for Descriptor<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_struct("Work", 5)?;
        fields.serialize_field("row", &Hex(&self.0.row.logical()))?;
        fields.serialize_field(
            "purpose",
            &PurposeMetadata(self.0.purpose, self.0.parent_selector),
        )?;
        let (state, handed_off) = match self.0.pending {
            // Registration history survives ordinary Return/compaction. "attached" is
            // not live-Phase, current engine, fresh Input or SDK/Workflow drainage authority.
            PendingStatus::NotPending => ("attached", None),
            PendingStatus::Open => ("pending", None),
            PendingStatus::Closing => ("closing", None),
            PendingStatus::Closed { actor_handoff } => ("closedPending", Some(actor_handoff)),
        };
        fields.serialize_field("state", state)?;
        fields.serialize_field("actorHandoff", &handed_off)?;
        fields.serialize_field("identity", &self.0.action.map(ActionMetadata))?;
        fields.end()
    }
}
// Keep Serialize off WorkReply itself: Tauri's blanket IpcResponse for Serialize would
// bypass our bounded writer. This borrowed view is used only by the fixed-buffer path.
struct Snapshot<'a>(&'a WorkReply);
impl Serialize for Snapshot<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_struct("Snapshot", 5)?;
        fields.serialize_field("kind", "snapshot")?;
        fields.serialize_field("owner", &self.0.owner)?;
        fields.serialize_field("executor", &self.0.executor)?;
        fields.serialize_field("nextSlot", &self.0.work.next_slot)?;
        fields.serialize_field("work", &self.0.work.metadata.as_ref().map(Descriptor))?;
        fields.end()
    }
}
impl WorkReply {
    /// Fixture-only observation of the same capped serialization while retaining its actual
    /// projection through the bounded public-metadata control pipe. Shipping IPC consumes it.
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_json(&self) -> tauri::Result<String> {
        self.json()
    }

    fn json(&self) -> tauri::Result<String> {
        let mut output = JsonBuffer {
            bytes: Vec::with_capacity(PUBLIC_REPLY_BYTES),
        };
        serde_json::to_writer(&mut output, &Snapshot(self))?;
        String::from_utf8(output.bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error).into())
    }
}
impl tauri::ipc::IpcResponse for WorkReply {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        self.json().map(Into::into)
    }
}

pub(in crate::voice_handoff) fn read_inner(
    state: &AppState,
    owner: String,
    executor: String,
    from_slot: u8,
) -> Result<WorkReply, String> {
    let id = identity(&owner, &executor)?;
    if from_slot > 32
        || owner.capacity() > PUBLIC_REPLY_BYTES
        || executor.capacity() > PUBLIC_REPLY_BYTES
    {
        return Err(REFUSED.into());
    }
    // Pay/serialize BEFORE building any descriptor/handle. The original canonical registry
    // supplies selection; never call Coordinator's lazy selected-owner factory for a read.
    let projection = state
        .voice_channels
        .project(&owner, id)
        .map_err(|_| REFUSED.to_string())?;
    // Include actual String capacities, the full reply and nested one-row snapshot copies,
    // capped JSON, its Vec header, fixed hex and conservative serializer/guard scratch.
    // This synchronous pull has no task, oneshot, await or unbounded descriptor collection.
    let reply_bytes = std::mem::size_of::<WorkReply>()
        + 2 * std::mem::size_of::<WorkProjection>()
        + std::mem::size_of::<Vec<u8>>()
        + PUBLIC_REPLY_BYTES
        + 64
        + 1024
        + owner.capacity()
        + executor.capacity();
    projection
        .require_reply_memory(reply_bytes)
        .map_err(|_| REFUSED.to_string())?;
    let work = state
        .voice_handoff
        .project_work(
            projection
                .existing_selected()
                .map_err(|_| REFUSED.to_string())?,
            from_slot,
        )
        .map_err(|_| REFUSED.to_string())?;
    Ok(WorkReply {
        owner,
        executor,
        work,
        _projection: projection,
    })
}

#[cfg(test)]
mod tests;
