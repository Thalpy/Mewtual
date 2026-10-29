//! Paid exact negative metadata for the original admitted physical intent.
//! There is no fallback tombstone/NoProof inference and no positive producer.
use super::*;
use crate::voice_call_activation::physical::{
    PhysicalHistory, PhysicalTerminal, PHYSICAL_REPLY_BYTES,
};
use crate::voice_call_activation::PhysicalProjection;

enum Original {
    Bound(PhysicalHistory),
    // Exact retained permanently closed native parent only. This observation
    // has NO executor, join, SDK or key-generation authority.
    NoIntent,
}
pub(crate) struct PhysicalReply {
    original: Original,
    _projection: PhysicalProjection,
}
const NO_INTENT_REPLY_BYTES: usize = 256;
struct Output {
    bytes: Vec<u8>,
    limit: usize,
}
impl std::io::Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > self.limit)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "physical receipt bound",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn ascii(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("hex is ASCII")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata<'a> {
    kind: &'static str,
    owner: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    executor: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    join: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt: Option<&'a str>,
}
impl PhysicalReply {
    /// Only bounded public metadata is serialized. The SAME original history or
    /// activation payment remains alive through the one capped output allocation.
    fn json(&self) -> tauri::Result<String> {
        let limit = match &self.original {
            Original::NoIntent => NO_INTENT_REPLY_BYTES,
            Original::Bound(_) => PHYSICAL_REPLY_BYTES,
        };
        self.json_with_limit(limit)
    }
    fn json_with_limit(&self, limit: usize) -> tauri::Result<String> {
        let mut output = Output {
            bytes: Vec::with_capacity(limit),
            limit,
        };
        match &self.original {
            Original::NoIntent => serde_json::to_writer(
                &mut output,
                &Metadata {
                    kind: "no-physical-intent",
                    owner: self._projection.original().id(),
                    executor: None,
                    join: None,
                    session: None,
                    receipt: None,
                },
            )?,
            Original::Bound(history) => {
                let (mut executor, mut join, mut session, mut receipt) =
                    ([0; 64], [0; 64], [0; 32], [0; 64]);
                hex::encode_to_slice(history.executor(), &mut executor)
                    .expect("fixed executor hex");
                hex::encode_to_slice(history.join(), &mut join).expect("fixed join hex");
                let terminal = history.terminal();
                let kind = match terminal {
                    None => "pending",
                    Some(PhysicalTerminal::NotStarted) => "not-started",
                    Some(PhysicalTerminal::Collected(_)) => "collected",
                };
                let receipt_text = if let Some(terminal) = terminal {
                    hex::encode_to_slice(history.receipt_id(terminal), &mut receipt)
                        .expect("fixed receipt hex");
                    Some(ascii(&receipt))
                } else {
                    None
                };
                let session_text = if let Some(PhysicalTerminal::Collected(serial)) = terminal {
                    hex::encode_to_slice(serial, &mut session).expect("fixed serial hex");
                    Some(ascii(&session))
                } else {
                    None
                };
                serde_json::to_writer(
                    &mut output,
                    &Metadata {
                        kind,
                        owner: history.owner(),
                        executor: Some(ascii(&executor)),
                        join: Some(ascii(&join)),
                        session: session_text,
                        receipt: receipt_text,
                    },
                )?;
            }
        }
        String::from_utf8(output.bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error).into())
    }
    #[cfg(test)]
    pub(crate) fn fixture_body_with_limit(self, limit: usize) -> tauri::Result<String> {
        // Exercise the actual metadata/serializer failure branch with a smaller
        // writer, never a caller-provided terminal or forged returned boolean.
        self.json_with_limit(limit.min(PHYSICAL_REPLY_BYTES))
    }
}
pub(super) fn no_intent_reply_layout_bytes() -> usize {
    std::mem::size_of::<PhysicalReply>()
        + std::mem::size_of::<Metadata<'static>>()
        + std::mem::size_of::<Output>()
        + NO_INTENT_REPLY_BYTES
        + 128
}
pub(super) fn bound_reply_layout_bytes() -> usize {
    std::mem::size_of::<PhysicalReply>()
        + std::mem::size_of::<Metadata<'static>>()
        + std::mem::size_of::<Output>()
        + PHYSICAL_REPLY_BYTES
        + 64
        + 64
        + 32
        + 64 // actual fixed executor/join/session/receipt hex buffers
}
impl tauri::ipc::IpcResponse for PhysicalReply {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        self.json().map(Into::into)
    }
}

/// Exact history recovery remains available after End/lock and key-row cleanup.
/// Lookup never creates a row/history or reserves new ordinary capacity.
pub(super) async fn observe_inner(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<PhysicalReply, String> {
    // Real Session/header return can make a closed original collectible without
    // another positive call. Destructors/lane wakes run before any authority
    // guard; observation itself still creates no history or new payment.
    state.voice_channels.collect_retired(&state.voice_handoff);
    let _commit = state.ui_session_commit.clone().lock_owned().await;
    let authority = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let handle = authority.admitted_activation_ref(owner).ok_or(REFUSED)?;
    let projection = handle.physical_projection().map_err(|_| REFUSED)?;
    let original = if let Some(history) = handle.physical_history(executor) {
        Original::Bound(history)
    } else if handle.no_physical_intent() {
        Original::NoIntent
    } else {
        return Err(REFUSED.into());
    };
    Ok(PhysicalReply {
        original,
        _projection: projection,
    })
}

pub(super) async fn acknowledge_inner(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
    receipt: [u8; 32],
) -> Result<(), String> {
    let _commit = state.ui_session_commit.clone().lock_owned().await;
    let authority = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let handle = authority.admitted_activation(owner).ok_or(REFUSED)?;
    let history = handle.physical_history(executor).ok_or(REFUSED)?;
    history
        .acknowledge_receipt(receipt)
        .map_err(|_| REFUSED.to_string())
}

#[cfg(test)]
pub(super) fn reply_json_for_fixture(reply: &PhysicalReply) -> String {
    reply.json().unwrap()
}
