//! Cooperative registry replay using the server's real MLS state and one-shot transport.
//! Not an autonomous actor worker: callers own wakeups, aggregate limits and native lifecycle
//! cancellation. Remote managed ingress and receipt/seed discovery remain separate integration.

use crate::store::epoch_budget::EpochStorageBudget;
use crate::store::{
    EpochIntentBudget, EpochRegistryState, RegistryReplayHold, RegistryReplayPass,
    RegistryReplayProgress, RegistryReplayStep, RegistryReplayTicket,
};
use crate::{AppError, Server, ServerStore};
use catcoms_rt::{CryptoRngCore, MeshTransport, PublishSubmission};
use catcoms_sync::{RegistrySyncInstance, SyncError};

/// Fixed saved-id traversal additionally bound to the exact live server sync instance at begin.
/// The underlying pass also binds the vault mount, numeric server, full group/device and epoch.
/// No caller-supplied body, ciphertext or submission ticket enters the send adapter.
pub struct ServerRegistryReplay {
    instance: RegistrySyncInstance,
    doc_id: u128,
    pass: RegistryReplayPass,
}

impl std::fmt::Debug for ServerRegistryReplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Match the underlying pass's redaction: the concrete document id is private scope too.
        f.debug_struct("ServerRegistryReplay")
            .field("progress", &self.progress())
            .finish_non_exhaustive()
    }
}

impl ServerRegistryReplay {
    /// Snapshot traversal counts, never intent retirement or remote delivery.
    pub fn progress(&self) -> RegistryReplayProgress {
        self.pass.progress()
    }

    /// Resume a failed storage step at the same saved id without repairing/resetting its budget.
    pub fn retry_failed(&mut self) -> Result<(), AppError> {
        self.pass.retry_failed()
    }
}

/// One cooperative result. Only an Attempt returning Submitted advances the submission count;
/// Duplicate/refusal leaves this saved id selected for a newly checked/resealed retry.
pub enum RegistryReplaySendStep {
    Complete,
    Wait {
        retry_at_ms: u64,
    },
    Paused,
    Held {
        intent_id: [u8; 32],
        reason: RegistryReplayHold,
        state: EpochRegistryState,
    },
    Attempt {
        intent_id: [u8; 32],
        result: Result<PublishSubmission, SyncError>,
        state: EpochRegistryState,
    },
}

impl std::fmt::Debug for RegistryReplaySendStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete => f.write_str("Complete"),
            Self::Wait { retry_at_ms } => f
                .debug_struct("Wait")
                .field("retry_at_ms", retry_at_ms)
                .finish(),
            Self::Paused => f.write_str("Paused"),
            Self::Held { reason, .. } => f
                .debug_struct("Held")
                .field("reason", reason)
                .finish_non_exhaustive(),
            Self::Attempt { result, .. } => f
                .debug_struct("Attempt")
                .field("result", result)
                .finish_non_exhaustive(),
        }
    }
}

/// Construct immediately after Prepared, before any fallible dispatch work. Dropping an awaited
/// send first cancels its transport future, then invalidates this exact ticket. Cancellation is
/// ambiguous after driver admission, so this is duplicate-safe retry, NOT rollback of local/network
/// state. The next attempt uses the saved id and fresh MLS sealing; no ciphertext is retained here.
struct AttemptGuard<'a> {
    pass: &'a mut RegistryReplayPass,
    ticket: Option<RegistryReplayTicket>,
}

impl AttemptGuard<'_> {
    fn submitted(&mut self) -> Result<(), AppError> {
        self.pass
            .submitted(self.ticket.as_ref().expect("guard owns a current attempt"))?;
        self.ticket = None;
        Ok(())
    }
}

impl Drop for AttemptGuard<'_> {
    fn drop(&mut self) {
        if let Some(ticket) = &self.ticket {
            // This private guard exclusively owns both pass and fresh ticket, so no external
            // acknowledgement can race it. Never panic in Drop while another failure unwinds.
            let _ = self.pass.retry_submission(ticket);
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Begin a saved OWN-intent pass bound now, not lazily on first send, to this live Server.
    /// Begin checks the ledger, not source Open/current; every actual step rechecks the source.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_registry_replay(
        &mut self,
        store: &ServerStore,
        server: u64,
        bucket: u8,
        expected_doc_id: u128,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<ServerRegistryReplay, AppError> {
        let pass = self.sync.with_registry_context(|group, device, _, _| {
            store.begin_registry_replay(
                server,
                group,
                bucket,
                expected_doc_id,
                device,
                budget,
                intents,
            )
        })?;
        Ok(ServerRegistryReplay {
            instance: self.sync.registry_instance(),
            doc_id: expected_doc_id,
            pass,
        })
    }

    /// Prepare and submit at most one saved registry intent under exclusive server/store borrows.
    /// The actual current group/device/clock/RNG come from this Server, never caller snapshots.
    /// Known membership, source gate and routing cannot interleave with the awaited dispatch.
    /// This does NOT enforce a native UI lock: the future live actor must cancel the await on its
    /// own lock/shutdown policy and bound aggregate work. No driver deadline is invented here.
    pub async fn send_registry_replay_step(
        &mut self,
        store: &mut ServerStore,
        cursor: &mut ServerRegistryReplay,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<RegistryReplaySendStep, AppError> {
        if !self.sync.matches_registry_instance(&cursor.instance) {
            return Err(AppError::Invalid(
                "registry replay belongs to a replaced server".into(),
            ));
        }
        let step = self
            .sync
            .with_registry_context(|group, device, clock, rng| {
                store.step_registry_replay(
                    &mut cursor.pass,
                    group,
                    device,
                    clock,
                    rng,
                    budget,
                    intents,
                )
            })?;
        match step {
            RegistryReplayStep::Complete => Ok(RegistryReplaySendStep::Complete),
            RegistryReplayStep::Wait { retry_at_ms } => {
                Ok(RegistryReplaySendStep::Wait { retry_at_ms })
            }
            RegistryReplayStep::Paused => Ok(RegistryReplaySendStep::Paused),
            RegistryReplayStep::Held {
                intent_id,
                reason,
                state,
            } => Ok(RegistryReplaySendStep::Held {
                intent_id,
                reason,
                state,
            }),
            RegistryReplayStep::Prepared {
                intent_id,
                ticket,
                op,
                state,
            } => {
                let mut guard = AttemptGuard {
                    pass: &mut cursor.pass,
                    ticket: Some(ticket),
                };
                let result = self
                    .sync
                    .publish_local_registry_once(cursor.doc_id, op)
                    .await;
                if matches!(result, Ok(PublishSubmission::Submitted)) {
                    guard.submitted()?;
                }
                // Guard invalidates duplicate/error attempts before returning; intents stay saved
                // for every result, including Submitted. Only owner settlement can finalize them.
                drop(guard);
                Ok(RegistryReplaySendStep::Attempt {
                    intent_id,
                    result,
                    state,
                })
            }
            RegistryReplayStep::AwaitingSubmission => {
                // No raw ticket escapes this adapter. This state would indicate an internal bug,
                // not success or a timeout which permits skipping an intent.
                Err(AppError::Invalid(
                    "registry replay already has an outstanding submission".into(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
