//! Trusted main-window, fixed public bootstrap metadata only. This is deliberately not a
//! generic crypto/result IPC command. Native Session keys, plaintext, SDP, ICE, and action
//! constructors never cross stock IPC. Main-only capability policy enforces the surface.
use super::application::Host;
use super::bootstrap::{self, MetadataProjection, PublicProjection, Status};
use super::channel::Refused;
use crate::AppState;
use catcoms_rt::{CryptoRngCore, OsCryptoRng};
use serde::Serialize;
use std::io::Write;
use std::sync::Arc;
use tauri::State;

const REFUSED: &str = "voice channel unavailable";
pub(super) const PUBLIC_REPLY_BYTES: usize = 2304;

#[derive(Serialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
enum Metadata {
    Ready {
        owner: String,
        executor: String,
        port: u16,
        transcript: PublicProjection,
    },
    Pending {
        owner: String,
        executor: String,
    },
    Closed {
        owner: String,
        executor: String,
        native_setup_drained: bool,
    },
}

/// Do not use Tauri's blanket Serialize response implementation here: serde_json's growing
/// String can retain more capacity than its final length. This wrapper serializes into one
/// prepaid, hard-capped allocation, then transfers that SAME body into the framework.
pub(crate) struct SetupReply {
    metadata: Metadata,
    _projection: MetadataProjection,
}
impl SetupReply {
    pub(super) fn new(
        status: Status,
        owner: String,
        executor: String,
        port: u16,
        projection: MetadataProjection,
    ) -> Result<Self, String> {
        let metadata = match status {
            Status::Ready(output) => Metadata::Ready {
                owner,
                executor,
                port,
                transcript: projection
                    .transcript(output)
                    .map_err(|_| REFUSED.to_string())?,
            },
            Status::Pending => Metadata::Pending { owner, executor },
            Status::Closed {
                native_setup_drained,
            } => Metadata::Closed {
                owner,
                executor,
                native_setup_drained,
            },
        };
        Ok(Self {
            metadata,
            _projection: projection,
        })
    }

    pub(super) fn json(&self) -> tauri::Result<String> {
        let mut output = JsonBuffer {
            bytes: Vec::with_capacity(PUBLIC_REPLY_BYTES),
        };
        serde_json::to_writer(&mut output, &self.metadata)?;
        // serde_json emits UTF8. This checked conversion moves, rather than copies, its one
        // allocation; the original PublicProjection remains held until framework handoff.
        String::from_utf8(output.bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error).into())
    }
}
impl tauri::ipc::IpcResponse for SetupReply {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        self.json().map(Into::into)
    }
}
struct JsonBuffer {
    bytes: Vec<u8>,
}
impl Write for JsonBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > PUBLIC_REPLY_BYTES - self.bytes.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                REFUSED,
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Closed canonical identities prevent oversized/escaped metadata projections before any
/// Registry/RNG/crypto work. Cleanup after End/lock uses the same original identity, never a
/// renderer-selected generation, port, Session or assertion that a provider has drained.
fn identity(owner: &str, executor: &str) -> Result<[u8; 32], String> {
    let lower_hex = |bytes: &[u8]| {
        bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    };
    if owner.len() != 22
        || !owner.starts_with("voice-")
        || !lower_hex(&owner.as_bytes()[6..])
        || executor.len() != 64
        || !lower_hex(executor.as_bytes())
    {
        return Err(REFUSED.into());
    }
    let mut value = [0; 32];
    hex::decode_to_slice(executor, &mut value).map_err(|_| REFUSED.to_string())?;
    Ok(value)
}

pub(super) async fn setup_inner(
    state: &AppState,
    host: &Host,
    owner: String,
    executor: String,
    rng: &mut impl CryptoRngCore,
) -> Result<SetupReply, String> {
    let id = identity(&owner, &executor)?;
    // A bound port alone is not readiness. Refuse before native admission/RNG when startup
    // failed, has not run, or the actual service has exited. Never start a replacement here.
    let port = host.port().map_err(|_| REFUSED.to_string())?;
    let status = bootstrap::ensure(state, &owner, id, rng)
        .await
        .map_err(|_| REFUSED.to_string())?;
    let projection = state
        .voice_channels
        .project(&owner, id)
        .map_err(|_| REFUSED.to_string())?;
    SetupReply::new(status, owner, executor, port, projection)
}

#[tauri::command]
pub(crate) async fn setup_voice_channel(
    state: State<'_, AppState>,
    host: State<'_, Arc<Host>>,
    owner: String,
    executor: String,
) -> Result<SetupReply, String> {
    crate::require_unlocked_session(&state).await?;
    setup_inner(&state, &host, owner, executor, &mut OsCryptoRng).await
}

fn drained(result: Result<Status, Refused>) -> Result<(), String> {
    match result {
        Ok(Status::Closed {
            native_setup_drained: true,
        }) => Ok(()),
        // A generic error, Pending, Ready, or closed-but-running is not proof of cleanup.
        _ => Err(REFUSED.into()),
    }
}

pub(super) async fn settle_inner(
    state: &AppState,
    owner: &str,
    executor: &str,
) -> Result<(), String> {
    let executor = identity(owner, executor)?;
    drained(bootstrap::settle_setup(state, owner, executor).await)
}

#[tauri::command]
pub(crate) async fn settle_voice_channel_setup(
    state: State<'_, AppState>,
    owner: String,
    executor: String,
) -> Result<(), String> {
    settle_inner(&state, &owner, &executor).await
}

pub(super) async fn collect_inner(
    state: &AppState,
    owner: &str,
    executor: &str,
) -> Result<(), String> {
    let executor = identity(owner, executor)?;
    drained(bootstrap::collect_channel(state, owner, executor).await)
}

#[tauri::command]
pub(crate) async fn collect_voice_channel(
    state: State<'_, AppState>,
    owner: String,
    executor: String,
) -> Result<(), String> {
    collect_inner(&state, &owner, &executor).await
}

#[cfg(test)]
mod tests;
