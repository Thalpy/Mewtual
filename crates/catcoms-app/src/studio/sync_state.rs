//! Local Studio coordinator observations, never receipt or network authority.

/// Which operation failed. A pause does not by itself imply corrupt storage or a peer fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudioSyncPauseCause {
    /// Local inventory, replay, rotation or handoff processing failed.
    Background,
    /// Processing incoming Studio history failed.
    Incoming,
}

/// Actor-local ordered state. Native adds its actor incarnation before forwarding to the UI.
/// Both pause and successful explicit resume are reported; a late Read reply is not a state ACK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioSyncObservation {
    /// Non-wrapping actor-local transition order; native supplies the actor incarnation.
    pub sequence: u64,
    /// Whether background coordinator work is currently held until successful explicit access.
    pub paused: bool,
    /// Actual failed work category, absent after resume or if no classified cause is available.
    pub cause: Option<StudioSyncPauseCause>,
    /// Bounded local diagnostics. Never sent to another peer or interpreted as authorization.
    pub detail: Option<String>,
}
