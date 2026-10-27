//! Native action access without an AppHandle/AppState ownership cycle. A process listener
//! retains only Weak references to the actual existing state, never policy snapshots. Each
//! call upgrades all fields before any ledger/payment admission and reuses the ordinary UI
//! gate. Upgraded ownership is transient; it does not replace lease/Root revocation checks.
use super::ledger::Ledger;
use crate::voice_authority::Coordinator;
use crate::{AppState, ServerEntry, ServerStore, UiSessionGate};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use tokio::sync::Mutex;

// Pinned Rust 1.94 ArcInner is repr(C, align(2)), two atomic usize counters followed by T:
// https://github.com/rust-lang/rust/blob/1.94.0/library/alloc/src/sync.rs#L377-L392
// Re-audited for the 1.89 -> 1.91 -> 1.94 bumps. 1.91 matched 1.89 (plain repr(C)); 1.94 adds
// `align(2)`, mirrored here. It cannot change the size on any target we build, because an
// atomic usize is already at least 2-aligned, but the model should say what std says.
// Model the WHOLE allocation to include alignment and tail padding (notably AtomicBool).
// Re-audit on a toolchain/layout change; allocator metadata/slack remains outside this bound.
#[repr(C, align(2))]
struct ArcAllocation<T> {
    strong: std::sync::atomic::AtomicUsize,
    weak: std::sync::atomic::AtomicUsize,
    value: T,
}
fn allocation_bytes<T>() -> usize {
    std::mem::size_of::<ArcAllocation<T>>()
}

type Servers = Mutex<HashMap<u64, ServerEntry>>;
type Store = Mutex<Option<ServerStore>>;

/// Stored in the original paid IO state, before its infrastructure credit in drop order.
/// Never clone into an unaccounted task or hold an upgraded context in the process host.
#[derive(Clone)]
pub(super) struct Context {
    servers: Weak<Servers>,
    store: Weak<Store>,
    resumable: Weak<Mutex<bool>>,
    requested: Weak<AtomicBool>,
    generation: Weak<AtomicU64>,
    authorized: Weak<AtomicU64>,
    commit: Weak<Mutex<()>>,
    authority: Weak<StdMutex<Coordinator>>,
    ledger: Weak<Ledger>,
    channels: Weak<super::bootstrap::Registry>,
}
/// Inline in one bounded native operation. It has no AppHandle/AppState, serialized authority,
/// replacement actor, cached open flag or alternate mutex. Drop outside native guards before
/// spawning admitted work; that worker needs only its original actor/selected/grant/commit.
pub(super) struct LiveContext {
    pub(super) servers: Arc<Servers>,
    store: Arc<Store>,
    resumable: Arc<Mutex<bool>>,
    requested: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
    pub(super) commit: Arc<Mutex<()>>,
    pub(super) authority: Arc<StdMutex<Coordinator>>,
    pub(super) ledger: Arc<Ledger>,
    pub(super) channels: Arc<super::bootstrap::Registry>,
}
impl Context {
    pub(super) fn new(state: &AppState) -> Self {
        Self {
            servers: Arc::downgrade(&state.servers),
            store: Arc::downgrade(&state.store),
            resumable: Arc::downgrade(&state.session_resumable),
            requested: Arc::downgrade(&state.session_lock_requested),
            generation: Arc::downgrade(&state.ui_session_generation),
            authorized: Arc::downgrade(&state.ui_authorized_generation),
            commit: Arc::downgrade(&state.ui_session_commit),
            authority: Arc::downgrade(&state.voice_authority),
            ledger: Arc::downgrade(&state.voice_handoff),
            channels: Arc::downgrade(&state.voice_channels),
        }
    }
    /// All-or-nothing before any protocol or resource effect. Partial upgrades are locals:
    /// a later dead Weak drops those temporary strong refs outside every native fence.
    pub(super) fn upgrade(&self) -> Option<LiveContext> {
        Some(LiveContext {
            servers: self.servers.upgrade()?,
            store: self.store.upgrade()?,
            resumable: self.resumable.upgrade()?,
            requested: self.requested.upgrade()?,
            generation: self.generation.upgrade()?,
            authorized: self.authorized.upgrade()?,
            commit: self.commit.upgrade()?,
            authority: self.authority.upgrade()?,
            ledger: self.ledger.upgrade()?,
            channels: self.channels.upgrade()?,
        })
    }
    /// Weak references keep the original Arc allocations (not their dropped heap contents)
    /// alive after native state destruction. Include those ten inline values/control blocks,
    /// plus one upgraded wrapper/borrowed gate, in the unchanged process IO envelope. Existing
    /// app-owned maps/store heaps while live are not duplicated or newly paid by this context.
    pub(super) fn retained_layout_bytes() -> usize {
        allocation_bytes::<Servers>()
            + allocation_bytes::<Store>()
            + allocation_bytes::<Mutex<bool>>()
            + allocation_bytes::<AtomicBool>()
            + 2 * allocation_bytes::<AtomicU64>()
            + allocation_bytes::<Mutex<()>>()
            + allocation_bytes::<StdMutex<Coordinator>>()
            + allocation_bytes::<Ledger>()
            + allocation_bytes::<super::bootstrap::Registry>()
            + std::mem::size_of::<LiveContext>()
            + std::mem::size_of::<UiSessionGate<'_>>()
    }
}
impl LiveContext {
    /// Observe the actual mount without suspending before engine admission. This is only
    /// a preflight: callers repeat original generation/authority checks at the final fence.
    /// Keep resumable -> store ordering; local contention spends no effect authority.
    pub(super) fn try_unlocked(&self) -> Result<bool, catcoms_sync::voice_v2::VoiceError> {
        use catcoms_sync::voice_v2::VoiceError;
        let resumable = self
            .resumable
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        let store = self.store.try_lock().map_err(|_| VoiceError::Capacity)?;
        Ok(*resumable && store.is_some())
    }

    pub(super) fn gate(&self) -> UiSessionGate<'_> {
        UiSessionGate {
            resumable: &self.resumable,
            requested: &self.requested,
            generation: &self.generation,
            authorized: &self.authorized,
            store: &self.store,
        }
    }
}

#[cfg(test)]
mod tests;
