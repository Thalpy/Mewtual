//! The live, ordered source fence of ONE genuinely joined node incarnation.
//!
//! The App's joined-member source and this node share one fence. It gives final member-save
//! admission and every MLS transition of the node one proved order:
//! - every merge on the node passes through `with_observed_mls_transition`, which marks the
//!   fence `merging` BEFORE the merge and settles it afterwards;
//! - a pending fork contest marks it `contested`;
//! - the App's governed install records which finalized node state its replayed prefix covers;
//! - final admission `try_lock`s the same mutex, checks, and issues its save-source end while
//!   still holding it.
//!
//! An admission that wins keeps its end through actual completion. A transition, removal or
//! close that wins makes the next admission refuse.
//!
//! The monotone `closed` flag sits BESIDE the mutex. Core negatives and the save-source
//! lifecycle read only that atomic and never lock. Every planned store of it happens inside a
//! locked section, except two cases where the mutex is already unusable: a poisoned lock, and
//! an unwind inside a section.
//!
//! The fence also retains this node's FINALIZED transitions (exact commit digest, epochs,
//! post-merge authenticator, membership delta) until the source consumes them. A provisional
//! contest or a staged own commit is never a transition. A merge that leaves the state
//! unchanged records nothing. A merge that changed the state and then returned an error is
//! still recorded, because it happened.
//!
//! This is local observation DATA. It grants nothing; selection still needs the pinned,
//! replayed, signed governing record. Transition DATA stays readable after the node's own
//! removal, so the Remove(self) record can still be replayed.
//!
//! Brackets do not nest; no production path nests them. Recorded transitions are contiguous:
//! a bracket that finds the node already moved since the last settlement (a mutation outside
//! the observer) records a terminal `HistoryGap` instead of a transition.
//!
//! Every invite-joined node carries a fence from its Welcome, including legacy members that
//! never host a joined source. The per-merge cost is two device-id roster listings and one
//! short lock; nodes without join facts pay nothing.
use crate::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

/// Finalized transitions retained until the joined source consumes them.
///
/// A node that finalizes more before the source catches up records a terminal `HistoryGap`
/// rather than forgetting one. That close is deliberate: a warm node cannot rebuild an
/// observation it dropped. One extra slot is always reserved for the transition that removes
/// this node, so a Remove(self) stays replayable even on a full ring. The ring is allocated
/// once (capacity + 1), so recording under the lock never allocates.
pub const V2_JOINED_TRANSITION_CAPACITY: usize = 64;

/// Membership delta of one finalized merge, by `DeviceId` bytes (the hash of a member's
/// signature key). The source compares a record's subject through
/// `DeviceId::from_public_key_bytes`.
///
/// A V2 Add or Remove changes exactly one member. `Neutral` (nobody added or removed) and
/// `Multiple` have no Core transition. The source must report them as unsupported; it can
/// never treat them as progress. `install_selected` refuses to cover them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum V2TransitionDelta {
    Neutral,
    Added([u8; 32]),
    Removed([u8; 32]),
    Multiple,
}
impl std::fmt::Debug for V2TransitionDelta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never log member keys.
        f.write_str(match self {
            Self::Neutral => "Neutral",
            Self::Added(_) => "Added(..)",
            Self::Removed(_) => "Removed(..)",
            Self::Multiple => "Multiple",
        })
    }
}

/// One finalized merge on this node. Fixed-size DATA, so copying it under the lock never
/// allocates.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2AppliedTransition {
    before_epoch: u64,
    after_epoch: u64,
    commit: Option<[u8; 32]>,
    authenticator: [u8; 32],
    delta: V2TransitionDelta,
    own_active: bool,
}
impl std::fmt::Debug for V2AppliedTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2AppliedTransition")
            .field("before_epoch", &self.before_epoch)
            .field("after_epoch", &self.after_epoch)
            .field("delta", &self.delta)
            .field("own_active", &self.own_active)
            .finish_non_exhaustive()
    }
}
impl V2AppliedTransition {
    pub fn before_epoch_data(&self) -> u64 {
        self.before_epoch
    }
    pub fn after_epoch_data(&self) -> u64 {
        self.after_epoch
    }
    /// `blake3` of the exact applied MLS commit bytes: the same digest an Add/Remove record's
    /// `commit` names. Recorded transitions always carry it; a merge whose bytes the recorder
    /// did not receive is a terminal `HistoryGap` instead.
    pub fn commit_data(&self) -> Option<[u8; 32]> {
        self.commit
    }
    /// The epoch authenticator right after the merge. A LOCAL association, never signed.
    pub fn authenticator_data(&self) -> &[u8; 32] {
        &self.authenticator
    }
    pub fn delta_data(&self) -> V2TransitionDelta {
        self.delta
    }
    /// Whether this node's own leaf is still an active member after the merge.
    pub fn own_active_data(&self) -> bool {
        self.own_active
    }
}

/// Why the fence closed. Once set it never changes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V2JoinedFenceTerminal {
    /// A finalized merge left this node's own leaf inactive.
    Removed,
    /// The node incarnation was dropped.
    Dropped,
    /// The node changed in a way this fence cannot account for: a merge without its commit
    /// bytes, or more unconsumed transitions than the ring retains.
    HistoryGap { epoch: u64 },
    /// The v1 owner checkpoint install bound this node (it bypasses the transition observer).
    V1Owner,
    /// The joined source cancelled or replaced itself.
    Cancelled,
    /// The joined source observed a founder fork (two valid signed siblings).
    Forked,
}

/// Fence refusals. None of them is absence or proof that anything happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V2JoinedFenceRefusal {
    /// Final admission found the fence momentarily held; retry. Nothing changed.
    Busy,
    /// Closed for good (terminal, poisoned or dropped).
    Closed,
    /// A merge is in progress or a fork contest is pending on this node.
    Transitioning,
    /// The source has not installed governance that covers this node's finalized state.
    Unselected,
    /// The request's Admission is not the source's currently selected own Admission.
    NotOwn,
    /// A governed install named a state this node never finalized.
    Unknown,
}

/// Copyable observation of the fence, for the source's diagnostics and catch-up decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct V2JoinedFenceObservation {
    pub settled_epoch: u64,
    pub join_epoch: u64,
    pub merging: bool,
    pub contested: bool,
    pub terminal: Option<V2JoinedFenceTerminal>,
    pub selected_epoch: Option<u64>,
    pub unconsumed: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Selected {
    epoch: u64,
    authenticator: [u8; 32],
    own: [u8; 32],
}

struct FenceState {
    /// The node's finalized (epoch, authenticator).
    settled: (u64, [u8; 32]),
    /// The (epoch, authenticator) the Welcome put the node in.
    join: (u64, [u8; 32]),
    merging: bool,
    contested: bool,
    terminal: Option<V2JoinedFenceTerminal>,
    transitions: VecDeque<V2AppliedTransition>,
    selected: Option<Selected>,
}
impl FenceState {
    /// The single authorization predicate, under the lock.
    fn authorized(&self, own: &[u8; 32]) -> Result<(), V2JoinedFenceRefusal> {
        if self.terminal.is_some() {
            return Err(V2JoinedFenceRefusal::Closed);
        }
        if self.merging || self.contested {
            return Err(V2JoinedFenceRefusal::Transitioning);
        }
        match self.selected {
            Some(selected)
                if selected.epoch == self.settled.0 && selected.authenticator == self.settled.1 =>
            {
                if &selected.own == own {
                    Ok(())
                } else {
                    Err(V2JoinedFenceRefusal::NotOwn)
                }
            }
            _ => Err(V2JoinedFenceRefusal::Unselected),
        }
    }
    fn terminate(&mut self, closed: &AtomicBool, terminal: V2JoinedFenceTerminal) {
        if self.terminal.is_none() {
            self.terminal = Some(terminal);
        }
        closed.store(true, Ordering::Release);
    }
}

struct FenceInner {
    closed: AtomicBool,
    state: Mutex<FenceState>,
}
impl FenceInner {
    fn lock(&self) -> Result<MutexGuard<'_, FenceState>, V2JoinedFenceRefusal> {
        match self.state.lock() {
            Ok(state) => Ok(state),
            Err(poisoned) => {
                drop(poisoned);
                self.closed.store(true, Ordering::Release);
                Err(V2JoinedFenceRefusal::Closed)
            }
        }
    }
    fn try_lock(&self) -> Result<MutexGuard<'_, FenceState>, V2JoinedFenceRefusal> {
        match self.state.try_lock() {
            Ok(state) => Ok(state),
            Err(TryLockError::WouldBlock) => Err(V2JoinedFenceRefusal::Busy),
            Err(TryLockError::Poisoned(poisoned)) => {
                drop(poisoned);
                self.closed.store(true, Ordering::Release);
                Err(V2JoinedFenceRefusal::Closed)
            }
        }
    }
    fn terminate(&self, terminal: V2JoinedFenceTerminal) {
        // On poison `lock` has already stored `closed`.
        if let Ok(mut state) = self.lock() {
            let _unwind = CloseOnUnwind(&self.closed);
            state.terminate(&self.closed, terminal);
        }
    }
}

/// Declared AFTER the MutexGuard in every locked section, so it drops first while the lock is
/// still held. A panic inside the section closes the fence before the lock poisons.
struct CloseOnUnwind<'a>(&'a AtomicBool);
impl Drop for CloseOnUnwind<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(true, Ordering::Release);
        }
    }
}

/// The joined source's handle. Not Clone: the source owns its one handle.
pub struct V2JoinedSourceFence {
    inner: Arc<FenceInner>,
}
impl std::fmt::Debug for V2JoinedSourceFence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2JoinedSourceFence")
            .finish_non_exhaustive()
    }
}

/// Final-admission custody: the fence stays locked while the caller issues its save-source end.
/// Holding it blocks node transitions only for that constant-time section. Drop releases it.
///
/// Never touch the node (no merge, no drop) on the thread that holds this: both re-lock the
/// fence. A panic while it is held closes the fence before the lock poisons, so lock-free
/// negatives observe it at once.
#[must_use = "issue the save-source end while this admission is held"]
pub struct V2JoinedFenceAdmission<'a> {
    // FIRST field: fields drop in declaration order, so this runs while the guard still holds.
    _unwind: CloseOnUnwind<'a>,
    _state: MutexGuard<'a, FenceState>,
}
impl std::fmt::Debug for V2JoinedFenceAdmission<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2JoinedFenceAdmission")
            .finish_non_exhaustive()
    }
}

impl V2JoinedSourceFence {
    /// Monotone negative for Core origins and the save-source lifecycle: one atomic load, no
    /// lock, allocation or callback. `false` can be stale immediately; only `admit_final`
    /// orders an admission.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Copyable observation. Readable after close: terminal fences are still DATA.
    pub fn observe_data(&self) -> Result<V2JoinedFenceObservation, V2JoinedFenceRefusal> {
        let state = self.inner.lock()?;
        Ok(V2JoinedFenceObservation {
            settled_epoch: state.settled.0,
            join_epoch: state.join.0,
            merging: state.merging,
            contested: state.contested,
            terminal: state.terminal,
            selected_epoch: state.selected.map(|selected| selected.epoch),
            unconsumed: state.transitions.len(),
        })
    }

    /// The unconsumed finalized transition that left `before_epoch`, if retained. Readable after
    /// close, so a Remove(self) record can still be replayed against its exact local commit.
    pub fn transition_data(
        &self,
        before_epoch: u64,
    ) -> Result<Option<V2AppliedTransition>, V2JoinedFenceRefusal> {
        let state = self.inner.lock()?;
        Ok(state
            .transitions
            .iter()
            .find(|transition| transition.before_epoch == before_epoch)
            .copied())
    }

    /// The joined source's governed install: its replayed, selected prefix now covers the
    /// node's finalized state `(epoch, authenticator)`, under `own` (the bytes of the source's
    /// current own Admission ID).
    ///
    /// Refuses unless that state is the join state or a recorded transition's post-merge state.
    /// The source cannot name a state this node never finalized. It also refuses to cover a
    /// transition Core has no step for (`Neutral` or `Multiple` delta), or one that left this
    /// node inactive. Consumed transitions up to `epoch` are released. A closed fence refuses:
    /// removal and fork are final here. `own` is the 32 bytes of an `OwnerFactId`; the
    /// handle, not this label, is the capability.
    pub fn install_selected(
        &self,
        epoch: u64,
        authenticator: [u8; 32],
        own: [u8; 32],
    ) -> Result<(), V2JoinedFenceRefusal> {
        let mut state = self.inner.lock()?;
        let _unwind = CloseOnUnwind(&self.inner.closed);
        if state.terminal.is_some() {
            return Err(V2JoinedFenceRefusal::Closed);
        }
        let finalized = state.join == (epoch, authenticator)
            || state.transitions.iter().any(|transition| {
                transition.after_epoch == epoch && transition.authenticator == authenticator
            });
        if !finalized {
            return Err(V2JoinedFenceRefusal::Unknown);
        }
        let unsupported = state
            .transitions
            .iter()
            .take_while(|transition| transition.after_epoch <= epoch)
            .any(|transition| {
                matches!(
                    transition.delta,
                    V2TransitionDelta::Neutral | V2TransitionDelta::Multiple
                ) || !transition.own_active
            });
        if unsupported {
            return Err(V2JoinedFenceRefusal::Unknown);
        }
        while state
            .transitions
            .front()
            .is_some_and(|transition| transition.after_epoch <= epoch)
        {
            state.transitions.pop_front();
        }
        state.selected = Some(Selected {
            epoch,
            authenticator,
            own,
        });
        Ok(())
    }

    /// Signing-gate check (blocking lock, for the source that already holds the node loan).
    pub fn require_authorized(&self, own: &[u8; 32]) -> Result<(), V2JoinedFenceRefusal> {
        let state = self.inner.lock()?;
        state.authorized(own)
    }

    /// FINAL save admission: `try_lock` (never waits), check, and return custody that keeps the
    /// fence locked while the caller issues its save-source end. Lock order: the outbox's mount
    /// slot, cell table and cell first, then this fence, then the save-source issuance mutex.
    /// No IO, allocation or SDK work happens here.
    pub fn admit_final(
        &self,
        own: &[u8; 32],
    ) -> Result<V2JoinedFenceAdmission<'_>, V2JoinedFenceRefusal> {
        let state = self.inner.try_lock()?;
        state.authorized(own)?;
        Ok(V2JoinedFenceAdmission {
            _unwind: CloseOnUnwind(&self.inner.closed),
            _state: state,
        })
    }

    /// The source cancelled or replaced itself. Irreversible.
    pub fn cancel(&self) {
        self.inner.terminate(V2JoinedFenceTerminal::Cancelled);
    }

    /// The source observed a founder fork. Irreversible.
    pub fn record_fork(&self) {
        self.inner.terminate(V2JoinedFenceTerminal::Forked);
    }
}

/// The node's side of its fence. It closes the fence when the node incarnation drops, because
/// `ChannelSync` has no Drop of its own.
#[derive(Default)]
pub(crate) struct JoinedFenceNode(Option<Arc<FenceInner>>);
impl JoinedFenceNode {
    /// A fresh fence for a genuinely joined node, settled at its post-Welcome state.
    pub(crate) fn for_join(group: &ServerGroup) -> Self {
        let state = (group.epoch(), group.epoch_authenticator_id());
        Self(Some(Arc::new(FenceInner {
            closed: AtomicBool::new(false),
            state: Mutex::new(FenceState {
                settled: state,
                join: state,
                merging: false,
                contested: false,
                terminal: None,
                // +1: the reserved Remove(self) slot.
                transitions: VecDeque::with_capacity(V2_JOINED_TRANSITION_CAPACITY + 1),
                selected: None,
            }),
        })))
    }
    pub(crate) fn handle(&self) -> Option<V2JoinedSourceFence> {
        self.0.as_ref().map(|inner| V2JoinedSourceFence {
            inner: inner.clone(),
        })
    }
    /// Close for good and detach. Later transitions on this node are not observed.
    pub(crate) fn terminate(&mut self, terminal: V2JoinedFenceTerminal) {
        if let Some(inner) = self.0.take() {
            inner.terminate(terminal);
        }
    }
}
impl Drop for JoinedFenceNode {
    fn drop(&mut self) {
        self.terminate(V2JoinedFenceTerminal::Dropped);
    }
}

/// Pre-merge capture, taken only when this node has a fence.
pub(crate) struct V2MergeBracket {
    before: (u64, [u8; 32]),
    ids: Vec<[u8; 32]>,
}

/// Current members' `DeviceId` bytes, sorted. One roster listing; bounded by the group size;
/// only fenced nodes pay for it. No per-member signature-key lookup (that rescans the roster).
fn roster_ids(group: &ServerGroup) -> Vec<[u8; 32]> {
    let mut ids: Vec<[u8; 32]> = group
        .member_device_ids()
        .iter()
        .map(|id| *id.as_bytes())
        .collect();
    ids.sort_unstable();
    ids
}

/// Both inputs sorted (`roster_ids`).
fn delta(before: &[[u8; 32]], after: &[[u8; 32]]) -> V2TransitionDelta {
    let mut added = after.iter().filter(|id| before.binary_search(*id).is_err());
    let mut removed = before.iter().filter(|id| after.binary_search(*id).is_err());
    match (added.next(), added.next(), removed.next(), removed.next()) {
        (None, _, None, _) => V2TransitionDelta::Neutral,
        (Some(key), None, None, _) => V2TransitionDelta::Added(*key),
        (None, _, Some(key), None) => V2TransitionDelta::Removed(*key),
        _ => V2TransitionDelta::Multiple,
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Open the merge bracket: mark the fence `merging` BEFORE the merge runs. `None` (and no
    /// cost) for every node without a joined fence.
    pub(crate) fn v2_fence_begin_merge(&self) -> Option<V2MergeBracket> {
        let inner = self.v2_fence.0.as_ref()?;
        let bracket = V2MergeBracket {
            before: (self.group.epoch(), self.group.epoch_authenticator_id()),
            ids: roster_ids(&self.group),
        };
        if let Ok(mut state) = inner.lock() {
            let _unwind = CloseOnUnwind(&inner.closed);
            state.merging = true;
        }
        Some(bracket)
    }

    /// Close the merge bracket after the merge and the observer's own hooks. A changed state is
    /// recorded as one finalized transition. `commit` is the digest of the exact merged bytes;
    /// without it a change is an unaccountable `HistoryGap`. An unchanged state (a failed or
    /// no-op merge) records nothing. All allocation happens before the lock.
    pub(crate) fn v2_fence_end_merge(
        &self,
        bracket: Option<V2MergeBracket>,
        commit: Option<[u8; 32]>,
    ) {
        let Some(bracket) = bracket else {
            return;
        };
        let Some(inner) = self.v2_fence.0.as_ref() else {
            return;
        };
        let after = (self.group.epoch(), self.group.epoch_authenticator_id());
        let contested = self.pending.is_some();
        let transition = (after != bracket.before).then(|| V2AppliedTransition {
            before_epoch: bracket.before.0,
            after_epoch: after.0,
            commit,
            authenticator: after.1,
            delta: delta(&bracket.ids, &roster_ids(&self.group)),
            own_active: self.group.is_active()
                && self.group.has_local_signer(&self.device.public_key_bytes()),
        });
        let Ok(mut state) = inner.lock() else {
            return;
        };
        let _unwind = CloseOnUnwind(&inner.closed);
        state.merging = false;
        state.contested = contested;
        // Contiguity: the node must still be where the last settlement left it. Otherwise
        // something mutated it outside the observer, and nothing after that can be accounted.
        if bracket.before != state.settled {
            let epoch = state.settled.0;
            state.terminate(&inner.closed, V2JoinedFenceTerminal::HistoryGap { epoch });
            return;
        }
        let Some(transition) = transition else {
            return;
        };
        state.settled = after;
        // The reserved last slot is only for the transition that removes this node.
        let full = state.transitions.len() >= V2_JOINED_TRANSITION_CAPACITY
            && (transition.own_active || state.transitions.len() > V2_JOINED_TRANSITION_CAPACITY);
        if transition.commit.is_none() || full {
            state.terminate(
                &inner.closed,
                V2JoinedFenceTerminal::HistoryGap {
                    epoch: transition.before_epoch,
                },
            );
            return;
        }
        state.transitions.push_back(transition);
        if !transition.own_active {
            state.terminate(&inner.closed, V2JoinedFenceTerminal::Removed);
        }
    }

    /// Re-derive `contested` from `pending` after any write to it. A pending fork contest (ours
    /// or another committer's) means a merge may be imminent, so authorization pauses.
    pub(crate) fn v2_fence_sync_contest(&self) {
        let Some(inner) = self.v2_fence.0.as_ref() else {
            return;
        };
        if let Ok(mut state) = inner.lock() {
            let _unwind = CloseOnUnwind(&inner.closed);
            state.contested = self.pending.is_some();
        }
    }

    /// The contest-resolution take. In ONE section, clear `contested` and raise `merging`, so no
    /// admission can slip into the gap before the resolving merge's own bracket opens.
    pub(crate) fn v2_fence_take_contest_into_merge(&self) {
        let Some(inner) = self.v2_fence.0.as_ref() else {
            return;
        };
        if let Ok(mut state) = inner.lock() {
            let _unwind = CloseOnUnwind(&inner.closed);
            state.contested = false;
            state.merging = true;
        }
    }
}

#[cfg(test)]
mod tests;
