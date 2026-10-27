//! Cooperative traversal of a fixed saved ledger, not a worker or a transport outbox. Each step
//! prepares at most one operation through the existing checked replay path. The caller owns
//! waking/pacing across passes and all current-session checks immediately before network send.

use std::sync::Arc;

use catcoms_crypto::DeviceId;
use catcoms_rt::Clock;

use super::*;

/// Per-pass work pacing, including failed attempts. This is deliberately not a server-wide
/// ingest/rate limit: the future coordinator must bound active passes and aggregate work too.
pub(super) const REPLAY_INTERVAL_MS: u64 = 100;

/// An opaque local submission-attempt identity. Neither this ticket nor its acknowledgement
/// proves delivery, receipt coverage, current membership or permission to send. A fresh allocation
/// for each prepared attempt avoids sequence wrap; an old ticket keeps its allocation distinct.
#[derive(Clone)]
pub struct RegistryReplayTicket(Arc<()>);

impl std::fmt::Debug for RegistryReplayTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegistryReplayTicket { .. }")
    }
}

/// Snapshot traversal counts, not network delivery/finality or the current ledger's size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistryReplayProgress {
    /// Own saved ids selected when the pass began; later additions are not part of this pass.
    pub selected: usize,
    /// Selected ids advanced past: `submitted + held`.
    pub visited: usize,
    /// Caller-acknowledged local submission attempts. Durable intents are NOT retired by this.
    pub submitted: usize,
    /// Conservative holds reported once in this pass; the saved intent remains for recovery.
    pub held: usize,
}

/// One cooperative step. Debug never displays operation ids, source content or ciphertext.
pub enum RegistryReplayStep {
    /// The captured ids were traversed, possibly including holds. This is not "all edits sent".
    Complete,
    /// A monotonic-clock deadline, not an absolute expiry; no disk work happened this step.
    Wait { retry_at_ms: u64 },
    /// The previous attempt errored/unwound. Explicit retry keeps the same id and pacing.
    Paused,
    /// No new work until the exact prepared attempt is acknowledged or explicitly retried.
    AwaitingSubmission,
    /// Nothing was reauthored. This id is now visited, but remains saved for explicit recovery.
    Held {
        intent_id: [u8; 32],
        reason: RegistryReplayHold,
        state: EpochRegistryState,
    },
    /// Saved and flushed, never sent by this API. The caller must recheck its session/server
    /// incarnation, member, MLS epoch and Open gate at send time, then acknowledge submission.
    Prepared {
        intent_id: [u8; 32],
        ticket: RegistryReplayTicket,
        op: SealedOp,
        state: EpochRegistryState,
    },
}

impl std::fmt::Debug for RegistryReplayStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete => f.write_str("Complete"),
            Self::Wait { retry_at_ms } => f
                .debug_struct("Wait")
                .field("retry_at_ms", retry_at_ms)
                .finish(),
            Self::Paused => f.write_str("Paused"),
            Self::AwaitingSubmission => f.write_str("AwaitingSubmission"),
            Self::Held { reason, .. } => f
                .debug_struct("Held")
                .field("reason", reason)
                .finish_non_exhaustive(),
            Self::Prepared { .. } => f.write_str("Prepared { .. }"),
        }
    }
}

enum PassPhase {
    Ready,
    AwaitingSubmission(Arc<()>),
    Paused,
}

/// A non-cloneable, mount/author/scope-bound cursor containing only the initially selected ids
/// (at most 10,000 / 320,000 bytes), never saved operation bodies or queued ciphertext.
///
/// Drop/restart loses only traversal progress. All intents remain durable and exact retained-log
/// retries stay idempotent. Losing a ticket requires dropping/restarting the pass, never advancing
/// on a timeout. An epoch change also requires a fresh pass, not silently retargeting this one.
/// Mount identity is NOT an unlock/server-incarnation token: native lock can keep the store open.
/// The future actor must cancel/drop work when its own lifecycle changes and bound active passes.
pub struct RegistryReplayPass {
    mount: Arc<()>,
    server: u64,
    group: Vec<u8>,
    author: DeviceId,
    bucket: u8,
    doc_id: u128,
    ids: Box<[[u8; 32]]>,
    progress: RegistryReplayProgress,
    phase: PassPhase,
    next_allowed_ms: Option<u64>,
}

impl std::fmt::Debug for RegistryReplayPass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryReplayPass")
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}

impl RegistryReplayPass {
    pub fn progress(&self) -> RegistryReplayProgress {
        self.progress
    }

    /// Local submission acknowledgement only. Advance once for this EXACT outstanding attempt;
    /// duplicate, stale and cross-pass tickets leave the cursor unchanged. Never retires intents.
    pub fn submitted(&mut self, ticket: &RegistryReplayTicket) -> Result<(), AppError> {
        self.check_ticket(ticket)?;
        self.progress.submitted += 1;
        self.progress.visited += 1;
        self.phase = PassPhase::Ready;
        Ok(())
    }

    /// Failed/cancelled submission: invalidate this ticket and retry the SAME saved id, resealing
    /// under then-current membership rather than retaining possibly stale ciphertext. No timeout
    /// or cursor advancement is implied, and the already-charged deadline is preserved.
    pub fn retry_submission(&mut self, ticket: &RegistryReplayTicket) -> Result<(), AppError> {
        self.check_ticket(ticket)?;
        self.phase = PassPhase::Ready;
        Ok(())
    }

    /// Resume an error/unwind at the same id. This does not reconcile budgets, fix malformed
    /// recovery or bless a changed epoch: the next attempt repeats all existing store checks.
    pub fn retry_failed(&mut self) -> Result<(), AppError> {
        if !matches!(self.phase, PassPhase::Paused) {
            return Err(invalid("replay pass is not paused"));
        }
        self.phase = PassPhase::Ready;
        Ok(())
    }

    fn check_ticket(&self, ticket: &RegistryReplayTicket) -> Result<(), AppError> {
        if matches!(&self.phase, PassPhase::AwaitingSubmission(held) if Arc::ptr_eq(held, &ticket.0))
        {
            Ok(())
        } else {
            Err(invalid("replay submission ticket is not current"))
        }
    }

    /// Private deterministic seam: the state is already Paused before replay can error/unwind,
    /// including AFTER a durable rename. No error, panic or abandoned Prepared result skips its
    /// id. Held intentionally advances traversal before return, without retiring the intent.
    pub(super) fn step_with(
        &mut self,
        clock: &dyn Clock,
        replay: impl FnOnce([u8; 32]) -> Result<(RegistryReplayOutcome, EpochRegistryState), AppError>,
    ) -> Result<RegistryReplayStep, AppError> {
        match self.phase {
            PassPhase::Paused => return Ok(RegistryReplayStep::Paused),
            PassPhase::AwaitingSubmission(_) => return Ok(RegistryReplayStep::AwaitingSubmission),
            PassPhase::Ready => {}
        }
        let Some(&intent_id) = self.ids.get(self.progress.visited) else {
            return Ok(RegistryReplayStep::Complete);
        };
        let now = clock.monotonic_ms();
        if let Some(deadline) = self.next_allowed_ms {
            if now < deadline {
                return Ok(RegistryReplayStep::Wait {
                    retry_at_ms: deadline,
                });
            }
        }
        self.phase = PassPhase::Paused;
        // Saturation would let every retry at u64::MAX run immediately. Refuse before disk work.
        self.next_allowed_ms = Some(
            now.checked_add(REPLAY_INTERVAL_MS)
                .ok_or_else(|| invalid("replay clock exhausted"))?,
        );
        let (outcome, state) = replay(intent_id)?;
        match outcome {
            RegistryReplayOutcome::Held(reason) => {
                self.progress.held += 1;
                self.progress.visited += 1;
                self.phase = PassPhase::Ready;
                Ok(RegistryReplayStep::Held {
                    intent_id,
                    reason,
                    state,
                })
            }
            RegistryReplayOutcome::Prepared(op) => {
                let attempt = Arc::new(());
                self.phase = PassPhase::AwaitingSubmission(attempt.clone());
                Ok(RegistryReplayStep::Prepared {
                    intent_id,
                    ticket: RegistryReplayTicket(attempt),
                    op,
                    state,
                })
            }
        }
    }
}

impl ServerStore {
    /// Snapshot one checked registry ledger's OWN saved ids in canonical id order. Begin does
    /// not read/flush the epoch or promise it is Open/current. The caller captures doc_id just as
    /// for a normal Save; every actual replay step rechecks it. An empty pass says only that this
    /// ledger had no saved intents for this device. Later additions require a later pass; missing
    /// snapshotted ids pause rather than silently becoming delivery/finality acknowledgements.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_registry_replay(
        &self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        expected_doc_id: u128,
        device: &MlsDevice,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<RegistryReplayPass, AppError> {
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("replay device is not a current member"));
        }
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let state = self.checked_epoch_replay_state(server, &document, budget, intents)?;
        let author = device.device_id();
        let ids: Vec<_> = state
            .pending()
            .filter(|(_, intent)| intent.author == author)
            .map(|(id, _)| *id)
            .collect();
        let progress = RegistryReplayProgress {
            selected: ids.len(),
            ..Default::default()
        };
        Ok(RegistryReplayPass {
            mount: self.replay_mount.clone(),
            server,
            group: group.group_id(),
            author,
            bucket,
            doc_id: expected_doc_id,
            ids: ids.into_boxed_slice(),
            progress,
            phase: PassPhase::Ready,
            next_allowed_ms: None,
        })
    }

    /// Perform at most one paced replay attempt. Waiting/paused/complete steps do no disk work.
    /// Wrong mount/group/author calls refuse without consuming a valid pass. The current member,
    /// concrete epoch, typed recovery and both budgets are rechecked by the existing replay path
    /// on every actual attempt. Neither a prepared result nor submitted() is a live send permit.
    #[allow(clippy::too_many_arguments)]
    pub fn step_registry_replay(
        &mut self,
        pass: &mut RegistryReplayPass,
        group: &ServerGroup,
        device: &MlsDevice,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<RegistryReplayStep, AppError> {
        if !Arc::ptr_eq(&pass.mount, &self.replay_mount)
            || pass.group != group.group_id()
            || pass.author != device.device_id()
        {
            return Err(invalid(
                "replay pass belongs to another mount, group or device",
            ));
        }
        let (server, bucket, doc_id) = (pass.server, pass.bucket, pass.doc_id);
        pass.step_with(clock, |id| {
            self.replay_registry_intent(
                server, group, bucket, doc_id, device, id, rng, budget, intents,
            )
        })
    }
}
