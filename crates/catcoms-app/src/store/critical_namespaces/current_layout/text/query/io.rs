//! Final actual finite-control IO negative, separate from logical retained root readers.
use super::*;

impl V2TextCurrentGroupGuard<'_, '_> {
    /// Check before ANY operational O/H File is opened, including ephemeral final-use
    /// revalidation. The canonical writer stays held through that proof and capture pin;
    /// an admitted outside-writer selector cannot overlap a newly opened control handle.
    /// This negative grants no write/lookup authority and does not expose the raw gate.
    pub(in crate::store) fn require_text_control_write_idle_for_query(
        &self,
    ) -> Result<(), AppError> {
        self.owner.require_text_control_write_idle()
    }

    /// A logical completed query intentionally survives this proof. Actual Source Files/
    /// key and physical OS-session cleanup must have ended. This is NOT a write permit:
    /// future typed stage/commit admission must install its writer-active gate in the SAME
    /// canonical edge before unlocking, then clear it only after actual writer cleanup.
    pub(in crate::store) fn require_text_control_io_quiescent(&self) -> Result<(), AppError> {
        self.owner.require_text_control_io_quiescent()
    }
}
