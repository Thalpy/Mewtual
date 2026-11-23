//! Negative-only original Source fence for a native registry row. This holds
//! the original counter and its trusted payer custody, never a positive Source
//! or UI owner. The concrete Root producer supplies fee-only custody there.
use super::*;

/// Opaque cloneable fence issued only by a genuine SourceOwner. It cannot be
/// decoded, open a source, enter a key or certify worker/SDK/physical drainage.
/// Dropping a handle does not close anything. Drop actual handles outside
/// guards: the last counter can release its original payer custody.
#[derive(Clone)]
pub struct NativeChatSourceClose {
    original: Arc<Counter>,
}
impl NativeChatSourceClose {
    /// Cached retirement state only. It does not authorize a native command.
    pub fn is_open(&self) -> bool {
        self.original.open()
    }

    /// Irreversibly stop new key entry on this exact Source. SC atomics only:
    /// no notification, allocation, wait, payer release or positive permission.
    /// The original owner must still retire outside guards and drain actual
    /// admitted Ends/SDK/transport work before acknowledging completion.
    pub fn close(&self) {
        self.original.close_admission_sc();
    }
}
impl std::fmt::Debug for NativeChatSourceClose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeChatSourceClose { .. }")
    }
}
impl NativeChatSourceOwner {
    /// Capture before publishing a native row. Clones share this ORIGINAL
    /// counter; retaining the fence grants no Source/Host/issuer ownership.
    pub fn close_handle(&self) -> NativeChatSourceClose {
        NativeChatSourceClose {
            original: self.original.counter.clone(),
        }
    }
}
