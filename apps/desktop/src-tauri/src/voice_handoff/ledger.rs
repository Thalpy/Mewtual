//! Process-owned registration for the disabled row-zero pilot. Reserve before either native
//! pair/derivation factory, retain the native task independently of its invoke waiter, and attach
//! before exposing input. No renderer descriptor can create a SelectedHandoffOwner or Action.
//!
//! This is deliberately a finite pilot: pending, live and drained rows share 32 positions. A
//! live owner cannot evict old uncertain history to make space. Physical reply/control-flight
//! prepayment, authenticated browser drainage and live dispatch are separate integration gates.
use super::channel::{AuthenticatedRequest, BodyKind, Operation};
use super::custody::{CopyCustody, Phase, PreparedPhase};
use crate::voice_authority::SelectedHandoffOwner;
use catcoms_sync::voice_v2::crypto_actions::{
    CryptoAction, CryptoOperation, CryptoResultReceipt, CryptoResultStatus,
};
use catcoms_sync::voice_v2::presence::{PairReceipt, PairSelector};
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind};
use catcoms_sync::voice_v2::{CallGrant, CallLease, VoiceError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Notify;

const ROWS: usize = 32;
const ROW_BYTES: usize = 2048;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowId {
    slot: usize,
    serial: u64,
}
impl RowId {
    /// Canonical fixed metadata only: native index slot + monotonically increasing serial.
    /// Every operational consumer must independently resolve original provenance/purpose.
    pub(super) fn logical(self) -> [u8; 16] {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&(self.slot as u64).to_be_bytes());
        bytes[8..].copy_from_slice(&self.serial.to_be_bytes());
        bytes
    }
    pub(super) fn from_logical(bytes: [u8; 16]) -> Result<Self, VoiceError> {
        let slot = usize::try_from(u64::from_be_bytes(
            bytes[..8].try_into().expect("fixed slot"),
        ))
        .map_err(|_| VoiceError::Unauthorized)?;
        let serial = u64::from_be_bytes(bytes[8..].try_into().expect("fixed serial"));
        if slot >= ROWS || serial == 0 {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Self { slot, serial })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    Derive(PairSelector),
}
/// Fixed historical evidence. It is neither permission to retry computation nor an RTC grant.
/// Unknown rows are errors, never an assertion that an engine, phase or reply has drained.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Status {
    pub pending: PendingStatus,
    pub action: Option<(u64, u64, [u8; 32])>,
    pub engine: CryptoResultStatus,
    pub acknowledged: bool,
    pub drained: bool,
}
/// Native preparation closure is distinct from an attached action's Phase/engine drainage.
/// A lost actor reply may hide a published Root pair. `Closed` records only that this native
/// Pending can never resume; it does not assert Root absence, settlement or Workflow drainage.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingStatus {
    NotPending,
    Open,
    Closing,
    Closed { actor_handoff: bool },
}
struct Key {
    grant: Weak<CallGrant>,
    executor: [u8; 32],
    lease: CallLease,
    purpose: Purpose,
}
impl Key {
    fn matches(&self, selected: &SelectedHandoffOwner, purpose: Purpose) -> bool {
        self.grant.ptr_eq(&selected.grant_identity())
            && self.executor == selected.executor()
            && self.lease.is_same(selected.lease())
            && self.purpose == purpose
    }
}
struct Pending {
    selected: Arc<SelectedHandoffOwner>,
    prepared: Option<PreparedPhase>,
    worker: bool,
    handed_off: bool,
}
/// Own the actual local preparation between Closing and Closed. Cancellation/unwind cannot
/// publish closure before owners drop. Production retires this synchronously outside native
/// guards; deterministic tests can retain the SAME ticket at this actual transition boundary.
struct PendingRetirement {
    row: Arc<Row>,
    owners: Option<Pending>,
    actor_handoff: bool,
}
impl Drop for PendingRetirement {
    fn drop(&mut self) {
        drop(self.owners.take());
        {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            debug_assert!(matches!(state.body, Body::ClosingPending));
            state.body = Body::ClosedPending;
            state.status.pending = PendingStatus::Closed {
                actor_handoff: self.actor_handoff,
            };
            // No engine settlement, ACK or Phase drainage is inferred from local closure.
        }
        self.row.changed.notify_waiters();
    }
}
enum ClosePending {
    Existing(Status),
    Retire(PendingRetirement),
}
struct Live {
    selected: Arc<SelectedHandoffOwner>,
    action: Arc<CryptoAction>,
    phase: Arc<Phase>,
    worker: bool,
    observer: bool,
}
enum Body {
    Pending(Pending),
    ClosingPending,
    ClosedPending,
    Live(Live),
    Drained,
}
struct State {
    body: Body,
    status: Status,
    // Monotonic native-only proof. It is independent of action/Phase drainage and cannot be
    // inferred from a browser FinalACK, timeout, lease revocation or copied selected fields.
    session_collected: bool,
}
struct Row {
    id: RowId,
    key: Key,
    state: Mutex<State>,
    changed: Notify,
    cleanup_waiter: AtomicBool,
    _metadata: Credit,
}
struct Index {
    serial: u64,
    rows: [Option<Arc<Row>>; ROWS],
}
pub(crate) struct Ledger {
    index: Mutex<Index>,
}
impl Default for Ledger {
    fn default() -> Self {
        Self {
            index: Mutex::new(Index {
                serial: 0,
                rows: std::array::from_fn(|_| None),
            }),
        }
    }
}
/// Only the native task owns this ticket. Dropping after actor handoff retains a counted
/// uncertain purpose, even if no browser ever received input. An exact retry reuses that row.
pub(crate) struct Registration {
    ledger: Arc<Ledger>,
    row: Arc<Row>,
    attached: bool,
}
pub(crate) enum Begin {
    Factory(Registration),
    Existing(RowId),
}
/// Native-only access to one original action row. A MAC or copied RowId is only a lookup
/// hint: the native-selected grant allocation, executor, lease and complete purpose must all
/// match before any input, terminal history or cleanup operation is available. Keep the Row
/// itself alive so its original metadata payment covers a delayed response/cleanup owner.
///
/// This is row provenance, not current first-effect authority. The channel must still fence
/// actual closure/current selection under the original UI commit, and Root checks current
/// membership/epoch/deadlines before a protocol effect. Historical methods intentionally do
/// not upgrade a dead CallGrant: End must not prevent exact ACK/settlement recovery.
/// No Clone, public constructor, serialization or conversion from renderer metadata exists.
/// Repeated resolutions are possible: this is metadata ownership, not a once-only worker
/// capability. Actual physical/cleanup callers must bound their simultaneous handles within
/// their existing payments; work/observer/waiter admission keeps its original separate gates.
pub(crate) struct ResolvedRow {
    ledger: Arc<Ledger>,
    row: Arc<Row>,
}
impl ResolvedRow {
    /// Fixed metadata for an authenticated reply after a lost factory response. The ID is
    /// never authority: later operations must still resolve this same native provenance.
    pub(crate) fn id(&self) -> RowId {
        self.row.id
    }
    pub(crate) fn status(&self) -> Result<Status, VoiceError> {
        self.ledger.status(self.row.id)
    }
    pub(crate) fn reconcile(&self) -> Result<Status, VoiceError> {
        self.ledger.reconcile(self.row.id)
    }
    pub(crate) fn acknowledge(&self, receipt: &CryptoResultReceipt) -> Result<(), VoiceError> {
        self.ledger.acknowledge(self.row.id, receipt)
    }
    pub(crate) fn settle(&self) -> Result<Status, VoiceError> {
        self.ledger.settle(self.row.id)
    }
    /// Native-only closure after the original authenticated ClosePhase adapter established
    /// exact Session/RowId/purpose and retained-broker SDK/RPC drainage. This is not a renderer
    /// boolean or proof of Root settlement. No other consumer may infer those preconditions.
    pub(super) fn close_pending(&self) -> Result<Status, VoiceError> {
        self.ledger.close_pending(&self.row)
    }
    /// Historical provenance alone cannot restart data work after End. This early fence is
    /// not the channel's final admission or a replacement for Root's actual effect checks.
    pub(crate) fn work(&self) -> Result<Work, VoiceError> {
        self.ledger.work(self.row.id)
    }
    /// Requires prior authenticated browser closure; neither this handle nor a renderer's
    /// claim of drainage can establish that protocol precondition by itself.
    pub(crate) async fn wait_compacted(&self) -> Result<Status, VoiceError> {
        self.ledger.wait_compacted(self.row.id).await
    }
}

impl Ledger {
    /// Pure cleanup preflight under the caller's original UI commit. No new factory may
    /// enter until channel closure commits under that SAME mutex. Drained is monotonic;
    /// unknown rows do not assert SDK drainage, but no original action means there is no
    /// action Control path to strand. Pending and Live (even settled/ACKed) refuse closure.
    pub(super) fn channel_may_close(&self, selected: &SelectedHandoffOwner) -> bool {
        let index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        index.rows.iter().flatten().all(|row| {
            !row.key.matches(selected, row.key.purpose)
                || matches!(
                    row.state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .body,
                    Body::Drained | Body::ClosedPending
                )
        })
    }

    /// Accept only the actual bootstrap registry's last-owner proof, after all its locks
    /// have been released. The full original grant allocation/executor/lease must match;
    /// equal public labels in a replacement generation do not authorize removal of history.
    /// Remember this before attempting collection: delayed ResolvedRow/cleanup owners may
    /// legitimately keep the record alive after its one original channel witness is gone.
    pub(super) fn channel_collected(&self, collected: super::bootstrap::CollectedChannel) {
        {
            let index = self.index.lock().unwrap_or_else(|error| error.into_inner());
            for row in index.rows.iter().flatten() {
                if row.key.matches(collected.selected(), row.key.purpose) {
                    row.state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .session_collected = true;
                }
            }
        }
        // Selected owners, credits and weak control blocks may have nontrivial destructors.
        // In particular the channel witness MUST NOT drop while either index is locked.
        drop(collected);
    }

    /// Bounded history reclamation, never uncertain-history eviction. Phase/engine/receipt
    /// compaction and actual channel collection are separate prerequisites. An original live
    /// Session must retain its purpose even after FinalACK, otherwise a fresh Begin could
    /// allocate another pending factory on that same purpose. Dead leases cannot be revived.
    /// Pending (including a possibly published Root pair) and Live rows remain untouched.
    pub(super) fn collect_retired(&self) {
        let mut removed: [Option<Arc<Row>>; ROWS] = std::array::from_fn(|_| None);
        {
            let mut index = self.index.lock().unwrap_or_else(|error| error.into_inner());
            for (slot, retired) in index.rows.iter_mut().zip(&mut removed) {
                let reclaim = slot.as_ref().is_some_and(|row| {
                    if row.key.lease.is_live() || Arc::strong_count(row) != 1 {
                        return false;
                    }
                    let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
                    state.session_collected
                        && matches!(state.body, Body::Drained | Body::ClosedPending)
                });
                if reclaim {
                    *retired = slot.take();
                }
            }
        }
        // Never run original credit/lease/weak-grant destruction under native index/state
        // guards. The index serial deliberately survives collection and slot reuse.
        drop(removed);
    }

    /// Capacity ONLY after canonical header authentication. The opaque request fixes the
    /// exact header RowId; no unauthenticated/reparsed logical ID can trigger this lookup.
    /// Compare the original grant allocation/executor/lease, then return only its existing
    /// Phase, never an Action, engine, input, receipt or retry capability. No live-grant
    /// upgrade, Clock, new row/payment or worker mutation occurs here.
    ///
    /// Full Purpose is inside the encrypted body. After intake/decryption the dispatcher
    /// MUST match that full purpose and the SAME header RowId via resolve before operations.
    /// A same-owner wrong-purpose body can consume only its bounded prepaid intake before
    /// refusal; it cannot cause a Root effect. Closed Phase absence never proves drainage:
    /// exact historical terminal recovery uses a separately bounded Control operation.
    pub(super) fn intake_phase(
        &self,
        selected: &SelectedHandoffOwner,
        authenticated: &AuthenticatedRequest,
    ) -> Result<Arc<Phase>, VoiceError> {
        if authenticated.operation() != Operation::Result || authenticated.kind() != BodyKind::Data
        {
            return Err(VoiceError::Unauthorized);
        }
        let row = self.row(RowId::from_logical(authenticated.logical())?)?;
        if !row.key.matches(selected, row.key.purpose) {
            return Err(VoiceError::Unauthorized);
        }
        let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
        match &state.body {
            Body::Live(live) => Ok(live.phase.clone()),
            Body::Pending(_) | Body::ClosingPending | Body::ClosedPending | Body::Drained => {
                Err(VoiceError::Capacity)
            }
        }
    }
    /// Recover a lost factory reply even when its caller never received a RowId. Search only
    /// the finite paid index using the ORIGINAL selected grant/lease/executor/full purpose.
    /// This historical lookup requires no live grant/current selection, takes no data loan,
    /// and never calls `begin`, re-arms an idle Pending worker or runs a Clock callback.
    ///
    /// `None` means no retained matching row at this observation; it is not drainage evidence.
    /// A separately authorized retry must still use `begin` on the SAME original purpose:
    /// returning a Pending row here must not suppress its existing idle-worker retry path.
    pub(crate) fn lookup_original(
        self: &Arc<Self>,
        selected: &SelectedHandoffOwner,
        purpose: Purpose,
    ) -> Option<ResolvedRow> {
        let row = self
            .index
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .rows
            .iter()
            .flatten()
            .find(|row| row.key.matches(selected, purpose))
            .cloned()?;
        Some(ResolvedRow {
            ledger: self.clone(),
            row,
        })
    }
    /// Resolve against the ORIGINAL native-selected owner, not the current replacement and
    /// not strings from the request. Preserve access to exact historical cleanup after End;
    /// no grant upgrade, Clock callback, resource charge or data loan occurs in this lookup.
    pub(crate) fn resolve(
        self: &Arc<Self>,
        selected: &SelectedHandoffOwner,
        purpose: Purpose,
        id: RowId,
    ) -> Result<ResolvedRow, VoiceError> {
        let row = self.row(id)?;
        if !row.key.matches(selected, purpose) {
            return Err(VoiceError::Unauthorized);
        }
        Ok(ResolvedRow {
            ledger: self.clone(),
            row,
        })
    }
    pub(crate) fn begin(
        self: &Arc<Self>,
        selected: Arc<SelectedHandoffOwner>,
        purpose: Purpose,
    ) -> Result<Begin, VoiceError> {
        if !selected.lease().is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let mut index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(row) = index
            .rows
            .iter()
            .flatten()
            .find(|row| row.key.matches(&selected, purpose))
        {
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            if let Body::Pending(pending) = &mut state.body {
                if !pending.worker {
                    if pending.prepared.is_none() {
                        pending.prepared = Some(PreparedPhase::new(
                            selected.resources(),
                            selected.binding().scope,
                            selected.lease().clone(),
                        )?);
                    }
                    pending.worker = true;
                    return Ok(Begin::Factory(Registration {
                        ledger: self.clone(),
                        row: row.clone(),
                        attached: false,
                    }));
                }
            }
            if matches!(state.body, Body::ClosingPending) {
                return Err(VoiceError::Capacity);
            }
            return Ok(Begin::Existing(row.id));
        }
        let slot = index
            .rows
            .iter()
            .position(Option::is_none)
            .ok_or(VoiceError::Capacity)?;
        let serial = index.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        let resources = selected.resources();
        let scope = selected.binding().scope;
        // Both fixed records are paid BEFORE either factory. No callback-bearing Clock or
        // action method runs under Index/State. Resource accounting is a pure bounded fence.
        let metadata = resources.charge(scope.group, ROW_BYTES, ResourceKind::Retained)?;
        let prepared = PreparedPhase::new(resources, scope, selected.lease().clone())?;
        let row = Arc::new(Row {
            id: RowId { slot, serial },
            key: Key {
                grant: selected.grant_identity(),
                executor: selected.executor(),
                lease: selected.lease().clone(),
                purpose,
            },
            state: Mutex::new(State {
                session_collected: false,
                body: Body::Pending(Pending {
                    selected,
                    prepared: Some(prepared),
                    worker: true,
                    handed_off: false,
                }),
                status: Status {
                    pending: PendingStatus::Open,
                    action: None,
                    engine: CryptoResultStatus {
                        settled: false,
                        receipt: None,
                    },
                    acknowledged: false,
                    drained: false,
                },
            }),
            _metadata: metadata,
            changed: Notify::new(),
            cleanup_waiter: AtomicBool::new(false),
        });
        index.serial = serial;
        index.rows[slot] = Some(row.clone());
        Ok(Begin::Factory(Registration {
            ledger: self.clone(),
            row,
            attached: false,
        }))
    }
    fn row(&self, id: RowId) -> Result<Arc<Row>, VoiceError> {
        self.index
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .rows
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|row| row.id == id)
            .cloned()
            .ok_or(VoiceError::Unauthorized)
    }
    fn status(&self, id: RowId) -> Result<Status, VoiceError> {
        let row = self.row(id)?;
        let status = row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .status
            .clone();
        Ok(status)
    }
    /// Close only this original native preparation, never a potentially published Root pair.
    /// State excludes both a retrying Registration and another closer. Move actual owners
    /// out, release ALL native guards, destroy them, then publish completion. Observers and
    /// Begin see Closing/Capacity until destruction finishes; no early success on last drop.
    fn close_pending(&self, row: &Arc<Row>) -> Result<Status, VoiceError> {
        match self.prepare_pending_close(row)? {
            ClosePending::Existing(status) => Ok(status),
            ClosePending::Retire(retired) => {
                drop(retired);
                self.status(row.id)
            }
        }
    }
    fn prepare_pending_close(&self, row: &Arc<Row>) -> Result<ClosePending, VoiceError> {
        let pending = {
            // Old metadata may outlive removal of a demonstrably unstarted Registration.
            // Validate indexed identity before any mutation, including repeated closure.
            // Same Index -> State order as begin/Registration::drop; owners leave both guards.
            let index = self.index.lock().unwrap_or_else(|error| error.into_inner());
            if !index
                .rows
                .get(row.id.slot)
                .and_then(Option::as_ref)
                .is_some_and(|indexed| indexed.id == row.id && Arc::ptr_eq(indexed, row))
            {
                return Err(VoiceError::Unauthorized);
            }
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            match &state.body {
                Body::ClosedPending => return Ok(ClosePending::Existing(state.status.clone())),
                Body::ClosingPending => return Err(VoiceError::Capacity),
                Body::Pending(pending) if !pending.worker && state.status.action.is_none() => {}
                Body::Pending(_) => return Err(VoiceError::Capacity),
                Body::Live(_) | Body::Drained => return Err(VoiceError::Unauthorized),
            }
            state.status.pending = PendingStatus::Closing;
            let Body::Pending(pending) = std::mem::replace(&mut state.body, Body::ClosingPending)
            else {
                unreachable!("checked Pending")
            };
            pending
        };
        let actor_handoff = pending.handed_off;
        Ok(ClosePending::Retire(PendingRetirement {
            row: row.clone(),
            owners: Some(pending),
            actor_handoff,
        }))
    }
    /// Recover actual terminal/settlement metadata even after phase closure or a native
    /// worker lost its last refresh. This admits no data copy, engine, computation or first
    /// effect. A single observer is prepaid per row; callers coalesce/retry Capacity outside.
    fn reconcile(&self, id: RowId) -> Result<Status, VoiceError> {
        self.reconcile_inner(id, true)
    }
    fn reconcile_inner(&self, id: RowId, notify: bool) -> Result<Status, VoiceError> {
        let row = self.row(id)?;
        let Some(observer) = Self::observe(&row, notify)? else {
            return self.status(id);
        };
        // Original callback-bearing Clock/history observations run outside ALL native gates.
        let observed = observer.action().terminal_status();
        let status = {
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            merge_observed(&mut state.status, observed)?;
            state.status.clone()
        };
        drop(observer);
        Ok(status)
    }
    fn observe(row: &Arc<Row>, notify: bool) -> Result<Option<Observation>, VoiceError> {
        let action = {
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            let Body::Live(live) = &mut state.body else {
                return Ok(None);
            };
            if live.observer {
                return Err(VoiceError::Capacity);
            }
            live.observer = true;
            live.action.clone()
        };
        Ok(Some(Observation {
            row: row.clone(),
            action: Some(action),
            notify,
        }))
    }
    /// Fixed history ACK recovery needs no data-phase loan and remains available after End
    /// or a lost earlier ACK reply. It cannot acknowledge a different terminal transition.
    fn acknowledge(&self, id: RowId, receipt: &CryptoResultReceipt) -> Result<(), VoiceError> {
        let row = self.row(id)?;
        if let Some(observer) = Self::observe(&row, true)? {
            observer.action().acknowledge_result(receipt)?;
            let observed = observer.action().terminal_status();
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            merge_observed(&mut state.status, observed)?;
            state.status.acknowledged = true;
            // Drop native guard before Observation's actual Action owner/cleanup.
            drop(state);
            drop(observer);
            return Ok(());
        }
        let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.status.acknowledged
            && state
                .status
                .engine
                .receipt
                .as_ref()
                .is_some_and(|original| original.same_transition(receipt))
        {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
    /// Native cleanup after the selected executor's actual provider/result/RPC work drained.
    /// This is NOT a callable IPC assertion of drainage. The future authenticated control
    /// adapter must establish those preconditions before invoking irreversible settlement.
    fn settle(&self, id: RowId) -> Result<Status, VoiceError> {
        let row = self.row(id)?;
        if let Some(observer) = Self::observe(&row, true)? {
            observer.action().settle_engine();
            let observed = observer.action().terminal_status();
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            merge_observed(&mut state.status, observed)?;
            let status = state.status.clone();
            drop(state);
            drop(observer);
            Ok(status)
        } else {
            let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.status.drained {
                Ok(state.status.clone())
            } else {
                Err(VoiceError::Capacity)
            }
        }
    }
    /// One actual native data worker per row. The future physical channel must additionally
    /// serialize across sibling rows and independently pay fixed control/reply copies.
    fn work(&self, id: RowId) -> Result<Work, VoiceError> {
        let row = self.row(id)?;
        let owners = {
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            // End can win while this caller waits for Index/State. Check the ORIGINAL
            // lease only after those waits, before admitting a copy or publishing worker.
            // A later End still requires the final channel/UI/Root effect fences.
            if !row.key.lease.is_live() {
                return Err(VoiceError::Unauthorized);
            }
            let Body::Live(live) = &mut state.body else {
                return Err(VoiceError::Unauthorized);
            };
            if live.worker {
                return Err(VoiceError::Capacity);
            }
            // Phase copy admission is pure and never calls Root/Clock. It keeps even a completed
            // but uncollected native task in the actual phase-drain count.
            let custody = live.phase.copy(0)?;
            live.worker = true;
            WorkOwners {
                selected: live.selected.clone(),
                action: live.action.clone(),
                phase: live.phase.clone(),
                _custody: custody,
            }
        };
        Ok(Work {
            row,
            owners: Some(owners),
        })
    }
    /// Compact only after the original phase has actually returned its loan, all native work
    /// was collected, engine settlement is observed and any terminal receipt was acknowledged.
    /// The caller must first establish browser closure through the separate authenticated
    /// protocol; this method never accepts a renderer-provided "drained" boolean.
    fn compact(&self, id: RowId) -> Result<bool, VoiceError> {
        // Only wait_compacted's gated sole waiter may call this quiet path in production.
        // Another native caller could otherwise overlap an unnotified quiet observation.
        // The sole cleanup waiter must not wake itself merely by observing unchanged history.
        // External observation retirement still notifies readiness after releasing its fence.
        self.reconcile_inner(id, false)?;
        let row = self.row(id)?;
        let retired = {
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            if matches!(state.body, Body::Drained) {
                return Ok(true);
            }
            let Body::Live(live) = &state.body else {
                return Ok(false);
            };
            if live.worker
                || live.observer
                || !live.phase.is_drained()
                || !state.status.engine.settled
                || (state.status.engine.receipt.is_some() && !state.status.acknowledged)
            {
                return Ok(false);
            }
            state.status.drained = true;
            if let Some(receipt) = &mut state.status.engine.receipt {
                receipt.retired = true; // this closed phase can never admit another first effect
            }
            std::mem::replace(&mut state.body, Body::Drained)
        };
        // Native input/action/Workflow owners and affine worker grants must never destruct
        // beneath State/Index. The retained tombstone contains only fixed evidence and Weak.
        drop(retired);
        row.changed.notify_waiters();
        Ok(true)
    }
    /// Coalesce the sole process-owned native cleanup waiter. The caller first establishes
    /// authenticated browser closure and closes Phase. Native data drainage and metadata/ACK
    /// readiness are distinct events: observing the first must never lose the second wake.
    /// This future creates no retry timer, new factory, engine or variable data loan.
    /// ClosedPending returns explicit LOCAL preparation closure with drained=false and
    /// engine.settled=false, never an attached Phase or Root compaction receipt.
    async fn wait_compacted(&self, id: RowId) -> Result<Status, VoiceError> {
        let row = self.row(id)?;
        if row
            .cleanup_waiter
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(VoiceError::Capacity);
        }
        let _waiter = CleanupWaiter(row.clone());
        let phase = {
            let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            match &state.body {
                Body::Live(live) => Some(live.phase.clone()),
                Body::Drained => None,
                Body::Pending(_) | Body::ClosingPending => return Err(VoiceError::Capacity),
                Body::ClosedPending => return Ok(state.status.clone()),
            }
        };
        if let Some(phase) = phase {
            phase.wait_drained().await;
        }
        loop {
            let changed = row.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            match self.compact(id) {
                Ok(true) => return self.status(id),
                Ok(false) | Err(VoiceError::Capacity) => changed.await,
                Err(error) => return Err(error),
            }
        }
    }
}
#[cfg(test)]
pub(super) fn pending_close_layout_bytes() -> usize {
    std::mem::size_of::<ClosePending>() + std::mem::size_of::<PendingRetirement>()
}
impl Registration {
    pub(crate) fn id(&self) -> RowId {
        self.row.id
    }
    /// Call immediately BEFORE awaiting the actor. A lost/Stopped reply is not no-publication
    /// evidence. Once marked, Drop cannot clear the original row or give the next retry new ID.
    pub(crate) fn mark_handoff(&self) -> Result<(), VoiceError> {
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Body::Pending(pending) = &mut state.body else {
            return Err(VoiceError::Unauthorized);
        };
        pending.handed_off = true;
        Ok(())
    }
    /// PairReceipt must be the exact trusted native factory reply. Neither it nor matching
    /// metadata substitutes for the actual returned Action allocation or its current authority.
    pub(crate) fn attach(
        mut self,
        action: Arc<CryptoAction>,
        pair: PairReceipt,
        bytes: usize,
    ) -> Result<RowId, VoiceError> {
        // Receiving an actual factory output is itself publication evidence, even if a future
        // native caller mistakenly omitted mark_handoff before its await. Refusal below may
        // not turn that proven publication into a fresh uncounted purpose.
        self.mark_handoff()?;
        let selected = {
            let state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Body::Pending(pending) = &state.body else {
                return Err(VoiceError::Unauthorized);
            };
            pending.selected.clone()
        };
        let binding = action.protected_binding()?;
        let Purpose::Derive(selector) = self.row.key.purpose;
        if !selected.lease().is_live()
            || action.operation() != CryptoOperation::Derive
            || !action.belongs_to(selected.binding().scope, selected.lease())
            || !action.uses_resources(&selected.resources())
            || binding.local_join != selected.join()
            || binding.local_key != catcoms_sync::voice_v2::key_reference(&selected.point())
            || binding.peer != selector.peer
            || binding.peer_join != selector.join
            || binding.peer_key != selector.key_ref
            || binding.epoch != selector.epoch
            || binding.handshake != pair.handshake
            || pair.peer != selector.peer
            || binding.local_is_low != pair.local_is_low
        {
            return Err(VoiceError::Unauthorized);
        }
        let prepared = {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Body::Pending(pending) = &mut state.body else {
                return Err(VoiceError::Unauthorized);
            };
            pending.prepared.take().ok_or(VoiceError::Capacity)?
        };
        // Includes original Clock observation. It MUST run outside native registry gates.
        let phase = prepared.bind(&action, bytes)?;
        let retired = {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !selected.lease().is_live() || !matches!(state.body, Body::Pending(_)) {
                return Err(VoiceError::Unauthorized);
            }
            state.status.action = Some((action.serial(), action.version(), action.input_tag()));
            state.status.pending = PendingStatus::NotPending;
            std::mem::replace(
                &mut state.body,
                Body::Live(Live {
                    selected,
                    action,
                    phase,
                    worker: false,
                    observer: false,
                }),
            )
        };
        drop(retired);
        self.attached = true;
        // The index owns the actual Action/Phase before any caller receives this identity.
        Ok(self.row.id)
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if self.attached {
            return;
        }
        let removed = {
            // Same Index -> State order as begin. Publishing idle and removing an unstarted
            // row must be atomic with a retry, or that retry could acquire an orphaned ticket.
            let mut index = self
                .ledger
                .index
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Body::Pending(pending) = &mut state.body else {
                return;
            };
            pending.worker = false;
            let slot = &mut index.rows[self.row.id.slot];
            if !pending.handed_off
                && slot
                    .as_ref()
                    .is_some_and(|row| row.id == self.row.id && Arc::ptr_eq(row, &self.row))
            {
                slot.take()
            } else {
                None
            }
        };
        drop(removed);
    }
}
struct WorkOwners {
    selected: Arc<SelectedHandoffOwner>,
    action: Arc<CryptoAction>,
    phase: Arc<Phase>,
    // Actual objects drop before the final phase contribution.
    _custody: CopyCustody,
}
pub(crate) struct Work {
    row: Arc<Row>,
    owners: Option<WorkOwners>,
}
impl Work {
    pub(crate) fn action(&self) -> &Arc<CryptoAction> {
        &self.owners.as_ref().expect("live work").action
    }
    pub(crate) fn phase(&self) -> &Arc<Phase> {
        &self.owners.as_ref().expect("live work").phase
    }
    pub(crate) fn selected(&self) -> &SelectedHandoffOwner {
        &self.owners.as_ref().expect("live work").selected
    }
    /// Observe the actual Root action OUTSIDE State/Index; never accept serialized terminal
    /// claims. The single worker prevents stale observations from overwriting newer history.
    pub(crate) fn refresh(&self) -> Result<Status, VoiceError> {
        let observed = self.action().terminal_status();
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        merge_observed(&mut state.status, observed)?;
        let status = state.status.clone();
        drop(state);
        self.row.changed.notify_waiters();
        Ok(status)
    }
    pub(crate) fn acknowledge(&self, receipt: &CryptoResultReceipt) -> Result<(), VoiceError> {
        self.action().acknowledge_result(receipt)?;
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.status.acknowledged = true;
        drop(state);
        self.row.changed.notify_waiters();
        Ok(())
    }
}
/// Concurrent metadata observations can finish out of order. Preserve immutable terminal
/// identity and monotonic settlement/retirement; an older empty snapshot cannot erase proof.
fn merge_observed(status: &mut Status, observed: CryptoResultStatus) -> Result<(), VoiceError> {
    if let Some(receipt) = observed.receipt {
        if let Some(old) = &mut status.engine.receipt {
            if !old.same_transition(&receipt) {
                return Err(VoiceError::RetryMismatch);
            }
            old.retired |= receipt.retired;
        } else {
            status.engine.receipt = Some(receipt);
        }
    }
    status.engine.settled |= observed.settled;
    Ok(())
}
struct Observation {
    row: Arc<Row>,
    action: Option<Arc<CryptoAction>>,
    notify: bool,
}
impl Drop for Observation {
    fn drop(&mut self) {
        drop(self.action.take());
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Body::Live(live) = &mut state.body {
            live.observer = false;
        }
        drop(state);
        if self.notify {
            self.row.changed.notify_waiters();
        }
    }
}
impl Observation {
    fn action(&self) -> &CryptoAction {
        self.action.as_ref().expect("original observation")
    }
}
struct CleanupWaiter(Arc<Row>);
impl Drop for CleanupWaiter {
    fn drop(&mut self) {
        self.0.cleanup_waiter.store(false, Ordering::Release);
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        // JoinHandle availability is not collection. All real worker owners drain first,
        // outside State. Publish worker retirement BEFORE the final contribution can wake the
        // sole phase waiter; otherwise it could observe drained + worker=true, miss compaction
        // and never receive another notification. No real objects/work remain at that point.
        let WorkOwners {
            selected,
            action,
            phase,
            _custody: custody,
        } = self.owners.take().expect("one original worker");
        drop((selected, action, phase));
        {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Body::Live(live) = &mut state.body {
                live.worker = false;
            }
        }
        self.row.changed.notify_waiters();
        drop(custody);
    }
}

#[cfg(test)]
pub(super) mod tests;
