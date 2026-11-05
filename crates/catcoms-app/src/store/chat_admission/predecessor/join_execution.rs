//! JOIN-only negative observation for FIRST durable counter admission. Not a scope permission.
//! An admitted exact DATA candidate later repairs independently of the worker's lifetime;
//! separate original source checks still fence every real network ticket.
use super::*;

pub(in crate::store) struct OriginalJoinExecutionObservation {
    key: ChatAdmissionKey,
    original: Option<Record>,
    mount: Arc<()>,
    source: [u8; 32],
    revision: u64,
    identity: Arc<()>,
    negative: RequestCancellation,
}
impl std::fmt::Debug for OriginalJoinExecutionObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OriginalJoinExecutionObservation")
            .finish_non_exhaustive()
    }
}

impl Registry {
    pub(in crate::store::chat_admission) fn validate_join_execution(
        &self,
        capture: &ChatAdmissionCapture,
        observation: &OriginalJoinExecutionObservation,
    ) -> Result<(), AppError> {
        if observation.negative.is_cancelled()
            || observation.key != capture.key
            || observation.original != capture.original
            || !Arc::ptr_eq(&observation.mount, &capture.mount_identity)
        {
            return Err(refused());
        }
        if self.slots.iter().flatten().any(|entry| {
            entry.kind == Kind::JoinCheckpoint
                && entry.key == observation.key
                && entry.original == observation.original
                && Arc::ptr_eq(&entry.mount, &observation.mount)
                && entry.source == observation.source
                && entry.revision == observation.revision
                && Arc::ptr_eq(&entry.identity, &observation.identity)
                && !entry.negative.is_cancelled()
        }) {
            Ok(())
        } else {
            Err(refused())
        }
    }
}

impl ServerStore {
    /// Capture only an ACTUAL registered original JOIN task under the admission serializer.
    /// Caller prepays fixed observation/key metadata before this bounded copy. This yields no
    /// grant/ticket, cannot observe a later task by key, and carries neither sender nor Store.
    pub(in crate::store) fn capture_original_join_execution(
        &self,
        capture: &ChatAdmissionCapture,
        task: &OriginalPredecessorTask,
        source: [u8; 32],
        revision: u64,
    ) -> Result<OriginalJoinExecutionObservation, AppError> {
        let observation = OriginalJoinExecutionObservation {
            key: capture.key.clone(),
            original: capture.original.clone(),
            mount: capture.mount_identity.clone(),
            source,
            revision,
            identity: task.owner.identity(),
            negative: task.observer(),
        };
        let runtime = self.chat_mount.locked()?;
        let permit = JournalAdmissionPermit::for_original(self, capture, &runtime);
        permit.validate_original()?;
        runtime
            .predecessors
            .validate_join_execution(capture, &observation)?;
        Ok(observation)
    }
}

impl OriginalPredecessorTask {
    /// Inert original identity used by the actual JOIN network context. It cannot keep this
    /// sender alive or serve as a scope/Current constructor.
    pub(in crate::store) fn join_execution_identity(&self) -> Arc<()> {
        self.owner.identity()
    }
}
