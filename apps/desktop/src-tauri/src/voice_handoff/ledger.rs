//! Process-owned registration for the bounded application handoff. Reserve before either native
//! pair/derivation factory, retain the native task independently of its invoke waiter, and attach
//! before exposing input. No renderer descriptor can create a SelectedHandoffOwner or Action.
//!
//! This is deliberately a finite pilot: pending, live and drained rows share 32 positions. A
//! live owner cannot evict old uncertain history to make space. Original physical/native
//! prepayments and authenticated browser drainage remain independent admission/cleanup gates.
use super::channel::{AuthenticatedRequest, BodyKind, Operation, SessionIdentity};
use super::custody::{CloseMode, ConfirmationStatus, CopyCustody, Phase, PreparedPhase};
use crate::voice_authority::SelectedHandoffOwner;
use catcoms_sync::voice_v2::crypto_actions::{
    CryptoAction, CryptoHandoffStatus, CryptoOperation, CryptoResultReceipt, CryptoResultStatus,
};
use catcoms_sync::voice_v2::presence::{PairReceipt, PairSelector};
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind};
use catcoms_sync::voice_v2::{CallGrant, CallLease, VoiceError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Notify;

const ROWS: usize = 32;
// A counted Pending now retains an actual Root publication through local attachment
// refusal. Pay for that additional owner and the complete native wrapper graph before
// factory admission. The32-row,512KiB group and2MiB process ceilings are unchanged.
const ROW_BYTES: usize = 2304;
mod children;
mod continuation;
mod key_session;
mod ready;
mod sdk;
mod sdp;
mod sdp_intake;
mod sdp_rows;
pub(super) use key_session::KeySessionReceipt;
pub(in crate::voice_handoff) use ready::{ReadyContinuation, ReadyProgress};
pub(super) use sdk::Receipt as SdkReceipt;
pub(super) use sdp::Receipt as SdpReceipt;
#[cfg(test)]
pub(in crate::voice_handoff) use sdp_rows::Begin as SdpBegin;
pub(in crate::voice_handoff) use sdp_rows::{
    Registration as SdpRegistration, SubmissionOwners as SdpSubmissionOwners,
};
mod pc;
pub(super) use pc::{Receipt as PcReceipt, State as PcState};
mod plan;
mod projection;
mod transition;
pub(super) use children::ConfirmationPurpose;
pub(super) use continuation::Continuation;
use plan::Plan;
pub(super) use projection::{WorkMetadata, WorkProjection};
pub(super) use transition::TransitionReceipt;

#[cfg(test)]
pub(super) fn retirement_layout_bytes() -> usize {
    plan::RETIREMENT_MEMORY_BYTES
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    Confirmation {
        parent: RowId,
        request: ConfirmationPurpose,
    },
    Sdp {
        parent: RowId,
        purpose: catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose,
    },
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
    // Child -> original parent only. Never install the reverse strong relationship.
    // The parent's independently paid row survives compaction and cannot be GC'd while
    // a descendant still needs its opaque plan or historical identity.
    parent: Option<Arc<Row>>,
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
    // Stored BEFORE callback-bearing loan/plan preparation. An uncertain caller or busy
    // previous loan must never turn this publication into another Actor factory request.
    published: Option<Arc<CryptoAction>>,
    worker: bool,
    handed_off: bool,
}
/// Own the actual local preparation between Closing and Closed. Cancellation/unwind cannot
/// publish closure before owners drop. Production retires this synchronously outside native
/// guards; deterministic tests can retain the SAME ticket at this actual transition boundary.
struct PendingRetirement {
    row: Arc<Row>,
    owners: Option<Pending>,
}
impl Drop for PendingRetirement {
    fn drop(&mut self) {
        // The moved Pending remains the sole source of this historical flag. Keeping a
        // second copy in the ticket needlessly enlarged every paid retirement position.
        // Snapshot before destruction, but publish Closed only AFTER all original owners
        // have dropped outside the native guards. No retry can mutate a Closing Pending.
        let owners = self
            .owners
            .take()
            .expect("original pending retirement owners");
        let actor_handoff = owners.handed_off;
        drop(owners);
        {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            debug_assert!(matches!(state.body, Body::ClosingPending));
            state.body = Body::ClosedPending;
            state.status.pending = PendingStatus::Closed { actor_handoff };
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
    // The existing paid Phase's fixed graph covers the independent identity allocation.
    // Drop delivery BEFORE phase on every abandonment/unwind, including whole-ledger Drop.
    // Compaction transfers this SAME delivery into Body::Drained before dropping Live.
    final_delivery: Option<FinalDelivery>,
    selected: Arc<SelectedHandoffOwner>,
    action: Arc<CryptoAction>,
    phase: Arc<Phase>,
    plan: Option<Plan>,
    worker: bool,
    observer: bool,
}
enum Body {
    Pending(Pending),
    ClosingPending,
    ClosedPending,
    Live(Live),
    Drained(
        Option<FinalDelivery>,
        Option<Plan>,
        Option<Arc<CryptoAction>>,
    ),
    SdpPending(sdp_rows::Pending),
    SdpLive(sdp_rows::Live),
    SdpDrained(sdp_rows::Drained),
}
struct State {
    body: Body,
    status: Status,
    // Final-intent compaction keeps the SAME already-paid Phase/witness, but no Action,
    // input or variable loan. Ordinary continuation does not retain it. This must survive
    // caller loss and local compaction until actual Root retirement, not weak/missing-row
    // inference. Phase's own original2KiB credit continues to fund its retained allocation.
    final_phase: Option<Arc<Phase>>,
    // Monotonic native-only proof. It is independent of action/Phase drainage and cannot be
    // inferred from a browser FinalACK, timeout, lease revocation or copied selected fields.
    session_collected: bool,
    // Fixed original role/message metadata survives input/Phase compaction. The role is
    // claimed atomically with publication; an outgoing message may be captured later,
    // outside native gates. End/settlement may already have removed its actual payload;
    // failed description must still retain the known original publication.
    child: Option<children::ChildMetadata>,
}
// Original-Session intent precedes local/Root close. The token retains NO Session or key.
// Live/Drained reuse enum storage occupied by the larger Pending preparation. The SAME
// original Row payment covers the full token allocation through delivery Drop, including
// Plan-final after ordinary Return when there is no Phase left. Layout guards include it.
struct FinalDelivery {
    session: Arc<SessionIdentity>,
    kind: FinalKind,
    issued: Option<[u8; 32]>,
    acknowledged: bool,
}
/// Immutable terminal meaning. An ordinary Phase Return is never upgraded to Retire;
/// Plan retirement instead closes its separately retained original control capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum FinalKind {
    Phase = 1,
    Plan = 2,
}
/// Fixed original-row final delivery metadata, separate from the 242-byte history schema.
/// It proves neither channel collection nor SDK erasure. Only native issuance can create it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FinalReceipt {
    pub row: RowId,
    pub action: super::requests::Action,
    pub kind: FinalKind,
    pub proof: Option<[u8; 32]>,
    pub acknowledged: bool,
}
impl State {
    /// Reuse the existing mutually exclusive Pending/Live/Drained storage. These original
    /// fixed handles survive compaction without enlarging every row's variable owner graph.
    fn plan(&self) -> Option<&Plan> {
        match &self.body {
            Body::Live(live) => live.plan.as_ref(),
            Body::Drained(_, plan, _) => plan.as_ref(),
            _ => None,
        }
    }
    fn plan_mut(&mut self) -> Option<&mut Plan> {
        match &mut self.body {
            Body::Live(live) => live.plan.as_mut(),
            Body::Drained(_, plan, _) => plan.as_mut(),
            _ => None,
        }
    }
    fn final_delivery(&self) -> Option<&FinalDelivery> {
        match &self.body {
            Body::Live(live) => live.final_delivery.as_ref(),
            Body::Drained(delivery, ..) => delivery.as_ref(),
            _ => None,
        }
    }
    fn final_delivery_mut(&mut self) -> Option<&mut FinalDelivery> {
        match &mut self.body {
            Body::Live(live) => live.final_delivery.as_mut(),
            Body::Drained(delivery, ..) => delivery.as_mut(),
            _ => None,
        }
    }
}
struct Row {
    id: RowId,
    key: Key,
    state: Mutex<State>,
    changed: Notify,
    cleanup_waiter: AtomicBool,
    _metadata: RowPayment,
}
/// Independently retained fixed funding. An SDP row never borrows the retired confirmation
/// Workflow or keeps its shared Phase alive merely to pay historical metadata. The SAME
/// fee also remains in each SDP body variant, while this final field pays Row destruction.
enum RowPayment {
    Ordinary(Credit),
    Sdp(Arc<catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowFunding>),
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
    /// Hold original indexed ownership through a metadata mutation. Slot+serial alone is
    /// insufficient for a retained orphan; native allocation equality is checked as well.
    fn indexed(&self) -> Result<std::sync::MutexGuard<'_, Index>, VoiceError> {
        let index = self.ledger.index.lock().unwrap_or_else(|e| e.into_inner());
        if !index
            .rows
            .get(self.row.id.slot)
            .and_then(Option::as_ref)
            .is_some_and(|row| row.id == self.row.id && Arc::ptr_eq(row, &self.row))
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(index)
    }
    /// Authenticated adapter only, after exact browser action SDK/result settlement. Admit
    /// local closure without taking Work/copies which would wait on their own destruction.
    /// Root retirement and original loan destruction happen only after Row guard release.
    pub(super) fn close_phase(
        &self,
        session: &Arc<SessionIdentity>,
        mode: CloseMode,
    ) -> Result<(), VoiceError> {
        if self.row.key.parent.is_some() && mode == CloseMode::Retire {
            // Sibling phases borrow the original parent plan. Only that parent may own
            // whole-plan control or its final proof; a child can only Return its loan.
            return Err(VoiceError::Unauthorized);
        }
        self.reconcile()?;
        let closing = {
            let _index = self.indexed()?;
            let row = &self.row;
            let mut state = row.state.lock().unwrap_or_else(|e| e.into_inner());
            let phase = match &state.body {
                Body::Live(live) if !live.worker && !live.observer => live.phase.clone(),
                Body::Live(_) => return Err(VoiceError::Capacity),
                Body::Drained(..) if mode == CloseMode::Retire => {
                    state.final_phase.clone().ok_or(VoiceError::Unauthorized)?
                }
                Body::Drained(..)
                    if state.final_phase.is_none() && state.final_delivery().is_none() =>
                {
                    return Ok(())
                }
                // Ordinary compaction already returned its only capability. A terminal
                // request can never relabel it, nor can ClosedPending close Root's plan.
                _ => return Err(VoiceError::Unauthorized),
            };
            if !state.status.engine.settled
                || (state.status.engine.receipt.is_some() && !state.status.acknowledged)
            {
                return Err(VoiceError::Capacity);
            }
            if let Some(delivery) = state.final_delivery() {
                if mode != CloseMode::Retire
                    || delivery.kind != FinalKind::Phase
                    || !Arc::ptr_eq(&delivery.session, session)
                {
                    return Err(VoiceError::Unauthorized);
                }
            } else if matches!(state.body, Body::Drained(..)) {
                return Err(VoiceError::Unauthorized);
            }
            let closing = phase.begin_close(mode)?;
            if mode == CloseMode::Retire && state.final_delivery().is_none() {
                // The authenticated original Session is physically alive now. A witness
                // for an equal-selected foreign Session received before this intent cannot
                // prove that this one was collected. Future evidence must match this token.
                state.session_collected = false;
                let Body::Live(live) = &mut state.body else {
                    unreachable!("new intent on original Live only")
                };
                live.final_delivery = Some(FinalDelivery {
                    session: session.clone(),
                    kind: FinalKind::Phase,
                    issued: None,
                    acknowledged: false,
                });
            }
            closing
        };
        // Dropping this actual original ticket may retire Root. Never do so under State.
        drop(closing);
        Ok(())
    }

    /// Issue once only AFTER actual local compaction and original variable Workflow
    /// retirement. Repeated requests recover the same stored proof; there is no Clock,
    /// factory, grant upgrade, variable owner or new allocation even under full pressure.
    pub(super) fn issue_final(
        &self,
        session: &Arc<SessionIdentity>,
    ) -> Result<FinalReceipt, VoiceError> {
        use sha2::{Digest, Sha256};
        let _index = self.indexed()?;
        let row = &self.row;
        let mut state = row.state.lock().unwrap_or_else(|e| e.into_inner());
        let delivery = state.final_delivery().ok_or(VoiceError::Unauthorized)?;
        if !Arc::ptr_eq(&delivery.session, session) {
            return Err(VoiceError::Unauthorized);
        }
        let (serial, version, input) = state.status.action.ok_or(VoiceError::Unauthorized)?;
        let action = super::requests::Action {
            serial,
            version,
            input_tag: input,
        };
        if !matches!(state.body, Body::Drained(..))
            || match delivery.kind {
                FinalKind::Phase => state.final_phase.is_none(),
                FinalKind::Plan => !state.plan().is_some_and(Plan::is_drained),
            }
            || !Ledger::final_retirement_ready(&state)
        {
            return Ok(FinalReceipt {
                row: row.id,
                action,
                kind: delivery.kind,
                proof: None,
                acknowledged: false,
            });
        }
        let proof = if let Some(proof) = delivery.issued {
            proof
        } else {
            let Purpose::Derive(selector) = row.key.purpose else {
                return Err(VoiceError::Unauthorized);
            };
            let mut hash = Sha256::new();
            hash.update(b"Mewtual native voice final retirement v1\0");
            hash.update(session.0);
            hash.update(row.id.logical());
            hash.update([0]); // closed Purpose::Derive tag
            hash.update(selector.peer.as_bytes());
            hash.update(selector.join);
            hash.update(selector.key_ref);
            hash.update(selector.epoch.to_be_bytes());
            hash.update(serial.to_be_bytes());
            hash.update(version.to_be_bytes());
            hash.update(input);
            hash.update([delivery.kind as u8]); // distinct immutable terminal meaning
            hash.finalize().into()
        };
        let delivery = state.final_delivery_mut().expect("checked original intent");
        delivery.issued = Some(proof);
        Ok(FinalReceipt {
            row: row.id,
            action,
            kind: delivery.kind,
            proof: Some(proof),
            acknowledged: delivery.acknowledged,
        })
    }

    /// ACK is observation only, usable after End/lock and an ACK reply loss. Even a guessed
    /// correct digest refuses before native issuance. It never manufactures Root retirement
    /// or actual physical channel collection, and never removes the original purpose fence.
    pub(super) fn acknowledge_final(
        &self,
        session: &Arc<SessionIdentity>,
        proof: [u8; 32],
    ) -> Result<FinalReceipt, VoiceError> {
        let _index = self.indexed()?;
        let row = &self.row;
        let mut state = row.state.lock().unwrap_or_else(|e| e.into_inner());
        let (serial, version, input_tag) = state.status.action.ok_or(VoiceError::Unauthorized)?;
        let delivery = state.final_delivery_mut().ok_or(VoiceError::Unauthorized)?;
        if !Arc::ptr_eq(&delivery.session, session) || delivery.issued != Some(proof) {
            return Err(VoiceError::Unauthorized);
        }
        delivery.acknowledged = true;
        Ok(FinalReceipt {
            row: row.id,
            action: super::requests::Action {
                serial,
                version,
                input_tag,
            },
            kind: delivery.kind,
            proof: Some(proof),
            acknowledged: true,
        })
    }

    /// One bounded non-waiting compaction visit for the serial authenticated cleanup driver.
    /// Share the SAME sole-waiter gate as wait_compacted; do not admit a quiet observer in
    /// parallel or park a physical request waiting for the Root's non-notifying snapshot.
    pub(super) fn try_compact(&self) -> Result<bool, VoiceError> {
        let _index = self.indexed()?;
        if self
            .row
            .cleanup_waiter
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(VoiceError::Capacity);
        }
        let waiter = CleanupWaiter(self.row.clone());
        drop(_index);
        let result = self.ledger.compact(self.row.id);
        drop(waiter);
        result
    }
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
    /// Native-only snapshot of the ORIGINAL final-plan witness, including after local
    /// compaction. No grant/Clock/task/allocation or Root callback; this is not final wire
    /// issuance or a wakeup. Unknown rows remain errors, never retirement evidence.
    pub(super) fn confirmation_status(&self) -> Result<ConfirmationStatus, VoiceError> {
        let row = self.ledger.row(self.row.id)?;
        let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(match &state.body {
            Body::Live(live) => live.phase.confirmation_status(),
            Body::Drained(..) => state
                .final_phase
                .as_ref()
                .map_or(ConfirmationStatus::NotRequested, |phase| {
                    phase.confirmation_status()
                }),
            Body::Pending(_) | Body::ClosingPending | Body::ClosedPending => {
                ConfirmationStatus::NotRequested
            }
            Body::SdpPending(_) | Body::SdpLive(_) | Body::SdpDrained(_) => {
                return Err(VoiceError::Unauthorized)
            }
        })
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
    /// Opaque ACTUAL retirement fence for cancellation/drop-order regressions. Exposes no
    /// mutable row state and changes no production synchronization or drop behavior.
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_hold_work_retirement(&self) -> impl Drop + '_ {
        self.row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
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
    /// action Control path to strand. Pending and Live (even settled/ACKed) refuse closure;
    /// An attached parent always requires exact issued/ACKed final delivery, even if native
    /// End already drained its Root plan. Otherwise an early close-first call could discard
    /// the only Control route before the browser recovered that original final proof.
    pub(super) fn channel_may_close(&self, selected: &SelectedHandoffOwner) -> bool {
        let index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        index.rows.iter().flatten().all(|row| {
            if !row.key.matches(selected, row.key.purpose) {
                return true;
            }
            let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(drained) = sdp_rows::locally_drained(&state) {
                return drained;
            }
            matches!(state.body, Body::Drained(..) | Body::ClosedPending)
                && Self::final_retirement_ready(&state)
                && ((state.final_phase.is_none()
                    && state.final_delivery().is_none()
                    && (row.key.parent.is_some() || state.status.action.is_none()))
                    || state
                        .final_delivery()
                        .is_some_and(|delivery| delivery.issued.is_some() && delivery.acknowledged))
        })
    }

    /// Local compaction/physical session closure cannot discard a requested final proof.
    /// Ordinary continuation and explicit ClosedPending require no invented Root witness.
    /// The retained cleanup driver must recheck this snapshot: Root has no notification
    /// into Row's existing Notify, so a prior Pending observation is not a final answer.
    fn final_retirement_ready(state: &State) -> bool {
        state.plan().is_none_or(Plan::is_drained) && Self::root_retirement_ready(state)
    }
    // Independent dead-owner GC has actual physical collection and a dead original lease.
    // It may discard an abandoned Claim, never fabricate Release/final issuance/ACK. Normal
    // finalization MUST continue using final_retirement_ready, including its Release gate.
    fn root_retirement_ready(state: &State) -> bool {
        if let Some(drained) = sdp_rows::locally_drained(state) {
            return drained;
        }
        state.plan().is_none_or(Plan::root_drained)
            && state
                .plan()
                .and_then(|plan| plan.transition.as_ref())
                .and_then(|claim| claim.key.as_ref())
                .and_then(|key| key.sdp.as_ref())
                .is_none_or(sdp::Claim::whole_drained)
            && state
                .final_phase
                .as_ref()
                .is_none_or(|phase| phase.confirmation_status() == ConfirmationStatus::Drained)
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
                    let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
                    if state.final_delivery().is_none_or(|delivery| {
                        collected
                            .identity()
                            .is_some_and(|identity| Arc::ptr_eq(identity, &delivery.session))
                    }) && sdp_rows::session(&state).is_none_or(|session| {
                        collected
                            .identity()
                            .is_some_and(|actual| Arc::ptr_eq(actual, session))
                    }) && state
                        .plan()
                        .and_then(|plan| plan.transition.as_ref())
                        .is_none_or(|claim| {
                            collected
                                .identity()
                                .is_some_and(|identity| Arc::ptr_eq(identity, &claim.session))
                        })
                    {
                        state.session_collected = true;
                    }
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
    /// Pending (including a possibly published Root pair), Live and final-unready rows remain
    /// untouched. A later bounded collection hook observes Root readiness without a new wake.
    pub(super) fn collect_retired(&self) {
        self.close_retired_sdp();
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
                        && (matches!(state.body, Body::Drained(..) | Body::ClosedPending)
                            || sdp_rows::locally_drained(&state) == Some(true))
                        && Self::root_retirement_ready(&state)
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
            Body::Pending(_)
            | Body::ClosingPending
            | Body::ClosedPending
            | Body::Drained(..)
            | Body::SdpPending(_)
            | Body::SdpLive(_)
            | Body::SdpDrained(_) => Err(VoiceError::Capacity),
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

    /// Native scheduler scan of original idle Derive preparation. Discovery expiry cannot
    /// erase the need to recover an admitted publication. Copy only this exact selector;
    /// no new epoch/proof, factory, lease upgrade, Clock or deadline is introduced here.
    pub(super) fn pending_derivation(
        &self,
        selected: &SelectedHandoffOwner,
        cursor: &mut u8,
    ) -> Option<PairSelector> {
        let index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        for offset in 0..ROWS {
            let slot = (usize::from(*cursor) + offset) % ROWS;
            let Some(row) = &index.rows[slot] else {
                continue;
            };
            let Purpose::Derive(selector) = row.key.purpose else {
                continue;
            };
            if !row.key.matches(selected, Purpose::Derive(selector)) {
                continue;
            }
            let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            if matches!(&state.body, Body::Pending(pending) if !pending.worker) {
                *cursor = ((slot + 1) % ROWS) as u8;
                return Some(selector);
            }
        }
        None
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
    /// Resolve the authenticated full parent identity using its actual stored Derive
    /// purpose. SDP controls cannot supply a second selector or redirect a child row.
    pub(super) fn resolve_sdp_parent(
        self: &Arc<Self>,
        selected: &SelectedHandoffOwner,
        id: RowId,
    ) -> Result<ResolvedRow, VoiceError> {
        let row = self.row(id)?;
        if row.key.parent.is_some()
            || !matches!(row.key.purpose, Purpose::Derive(_))
            || !row.key.matches(selected, row.key.purpose)
        {
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
        if !matches!(purpose, Purpose::Derive(_)) {
            return Err(VoiceError::Unauthorized); // children require the original-parent fence
        }
        if !selected.lease().is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let mut index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        self.begin_indexed(selected, purpose, None, &mut index)
    }
    /// Index is held by the original parent reservation gate for children. Pure resource
    /// accounting may run here; no Root/Clock call or affine destruction may be introduced.
    fn begin_indexed(
        self: &Arc<Self>,
        selected: Arc<SelectedHandoffOwner>,
        purpose: Purpose,
        parent: Option<&Arc<Row>>,
        index: &mut Index,
    ) -> Result<Begin, VoiceError> {
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
                parent: parent.cloned(),
            },
            state: Mutex::new(State {
                final_phase: None,
                session_collected: false,
                child: None,
                body: Body::Pending(Pending {
                    selected,
                    prepared: Some(prepared),
                    published: None,
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
            _metadata: RowPayment::Ordinary(metadata),
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
                Body::Live(_)
                | Body::Drained(..)
                | Body::SdpPending(_)
                | Body::SdpLive(_)
                | Body::SdpDrained(_) => return Err(VoiceError::Unauthorized),
            }
            state.status.pending = PendingStatus::Closing;
            let Body::Pending(pending) = std::mem::replace(&mut state.body, Body::ClosingPending)
            else {
                unreachable!("checked Pending")
            };
            pending
        };
        Ok(ClosePending::Retire(PendingRetirement {
            row: row.clone(),
            owners: Some(pending),
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
        let observed = observer.action().handoff_status();
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
            match &mut state.body {
                Body::Live(live) => {
                    if live.observer {
                        return Err(VoiceError::Capacity);
                    }
                    live.observer = true;
                    live.action.clone()
                }
                Body::SdpPending(p) if p.published.is_some() => {
                    if p.observer {
                        return Err(VoiceError::Capacity);
                    }
                    p.observer = true;
                    p.published
                        .as_ref()
                        .expect("guarded published action")
                        .clone()
                }
                Body::SdpLive(live) => {
                    if live.observer {
                        return Err(VoiceError::Capacity);
                    }
                    live.observer = true;
                    live.action.clone()
                }
                _ => return Ok(None),
            }
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
            let observed = observer.action().handoff_status();
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
            let observed = observer.action().handoff_status();
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
    /// protocol; this method never accepts a renderer-provided "drained" boolean. Local
    /// compaction does NOT wait for full Root retirement: it preserves the original final
    /// Phase proof instead. Channel closure and history GC check that separate snapshot.
    fn compact(&self, id: RowId) -> Result<bool, VoiceError> {
        // Only wait_compacted/try_compact's gated sole visitor may use this quiet path.
        // Another native caller could otherwise overlap an unnotified quiet observation.
        // The sole cleanup waiter must not wake itself merely by observing unchanged history.
        // External observation retirement still notifies readiness after releasing its fence.
        self.reconcile_inner(id, false)?;
        let row = self.row(id)?;
        let retired = {
            let mut state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            if matches!(state.body, Body::Drained(..)) {
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
            // Retain before detaching Live, under the established Row -> Phase order.
            // This is an infallible Arc increment, not a new allocation or witness copy.
            // Do NOT require Root-ready first: dropping Live below may itself release the
            // last actual Root owner. No callback or destructor runs under these guards.
            debug_assert!(state.final_phase.is_none());
            let final_phase = (live.phase.confirmation_status()
                != ConfirmationStatus::NotRequested)
                .then(|| live.phase.clone());
            state.final_phase = final_phase;
            state.status.drained = true;
            if let Some(receipt) = &mut state.status.engine.receipt {
                receipt.retired = true; // this closed phase can never admit another first effect
            }
            let mut retired = std::mem::replace(&mut state.body, Body::ClosedPending);
            let Body::Live(live) = &mut retired else {
                unreachable!("checked original Live")
            };
            // A first response factory may run AFTER ordinary Open Return/compaction.
            // Keep its ACTUAL settled/ACKed trigger, never reconstruct an Action from
            // copied fields. Settlement has already released input/engine/lender; the
            // original Root record continues to pay for this fixed historical owner.
            let response_trigger = (row.key.parent.is_some()
                && live.action.operation() == CryptoOperation::DecryptConfirmation
                && state.status.engine.receipt.as_ref().is_some_and(|receipt| {
                    receipt.outcome == catcoms_sync::voice_v2::crypto_actions::CryptoOutcome::Opened
                }))
            .then(|| live.action.clone());
            state.body = Body::Drained(
                live.final_delivery.take(),
                live.plan.take(),
                response_trigger,
            );
            retired
        };
        // Native input/action/Workflow owners and affine worker grants must never destruct
        // beneath State/Index. Compacted history retains fixed evidence/provenance and,
        // for final intent, the SAME paid Phase. An accepted incoming child also retains
        // its actual settled trigger Action, whose input/engine/lender already drained.
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
                Body::Drained(..) => None,
                Body::Pending(_) | Body::ClosingPending => return Err(VoiceError::Capacity),
                Body::ClosedPending => return Ok(state.status.clone()),
                Body::SdpPending(_) | Body::SdpLive(_) | Body::SdpDrained(_) => {
                    return Err(VoiceError::Unauthorized)
                }
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
        self,
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
        let Purpose::Derive(selector) = self.row.key.purpose else {
            return Err(VoiceError::Unauthorized);
        };
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
        {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Body::Pending(pending) = &mut state.body else {
                return Err(VoiceError::Unauthorized);
            };
            if let Some(original) = &pending.published {
                if !Arc::ptr_eq(original, &action) {
                    return Err(VoiceError::RetryMismatch);
                }
            } else {
                pending.published = Some(action.clone());
            }
        }
        // The counted row now owns the actual publication before ANY Clock-bearing plan
        // capture or loan attempt. Losing this caller can only leave that original Pending.
        self.attach_published(bytes)
    }
    /// A stored publication is historical ownership, never permission to run another
    /// factory. First-effect/current-selection fences still belong to the outer adapter.
    pub(crate) fn has_publication(&self) -> Result<bool, VoiceError> {
        let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        match &state.body {
            Body::Pending(pending) => Ok(pending.published.is_some()),
            _ => Err(VoiceError::Unauthorized),
        }
    }
    /// Attach the SAME stored action, without another Actor request, nonce or metadata
    /// charge. A prior local refusal leaves its original fixed preparation in the row.
    pub(crate) fn attach_published(mut self, bytes: usize) -> Result<RowId, VoiceError> {
        let (selected, action) = {
            let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            let Body::Pending(pending) = &state.body else {
                return Err(VoiceError::Unauthorized);
            };
            (
                pending.selected.clone(),
                pending
                    .published
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?
                    .clone(),
            )
        };
        if !selected.lease().is_live() {
            return Err(VoiceError::Unauthorized);
        }
        // Capture from the actual pending Root Action BEFORE its first engine/Return.
        // The callback-bearing observation runs outside all native gates. Refusal retains
        // the actual Action in Pending, including when End wins in that Clock callback.
        let plan = if self.row.key.parent.is_some() {
            self.require_child_action(&action)?;
            self.describe_child(&action)?;
            None
        } else {
            Some(Plan::capture(&action)?)
        };
        let prepared = {
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            let Body::Pending(pending) = &mut state.body else {
                return Err(VoiceError::Unauthorized);
            };
            pending.prepared.take().ok_or(VoiceError::Capacity)?
        };
        // Includes original Clock observation. It MUST run outside native registry gates.
        let phase = match prepared.try_bind(&action, bytes) {
            Ok(phase) => phase,
            Err((error, prepared)) => {
                // Registration's worker excludes close/retry while the preparation leaves
                // State. Restore it before returning; its actual destruction is never run
                // below a native guard. No ambiguous network/engine attempt is refunded.
                {
                    let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
                    let Body::Pending(pending) = &mut state.body else {
                        unreachable!("registration owns this Pending worker");
                    };
                    debug_assert!(pending.prepared.is_none());
                    pending.prepared = Some(prepared);
                }
                return Err(error);
            }
        };
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
                    plan,
                    worker: false,
                    observer: false,
                    final_delivery: None,
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
    // This actual contribution also retains the SAME Phase; no duplicate Arc is needed.
    // Actual objects drop before the final phase contribution.
    _custody: CopyCustody,
}
pub(crate) struct Work {
    row: Arc<Row>,
    owners: Option<WorkOwners>,
}
impl Work {
    pub(super) fn id(&self) -> RowId {
        self.row.id
    }
    pub(crate) fn action(&self) -> &Arc<CryptoAction> {
        &self.owners.as_ref().expect("live work").action
    }
    pub(crate) fn phase(&self) -> &Arc<Phase> {
        self.owners.as_ref().expect("live work")._custody.phase()
    }
    pub(crate) fn selected(&self) -> &SelectedHandoffOwner {
        &self.owners.as_ref().expect("live work").selected
    }
    /// Observe the actual Root action OUTSIDE State/Index; never accept serialized terminal
    /// claims. The single worker prevents stale observations from overwriting newer history.
    pub(crate) fn refresh(&self) -> Result<Status, VoiceError> {
        let observed = self.action().handoff_status();
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
fn merge_observed(status: &mut Status, observed: CryptoHandoffStatus) -> Result<(), VoiceError> {
    if let Some(receipt) = observed.engine.receipt {
        if let Some(old) = &mut status.engine.receipt {
            if !old.same_transition(&receipt) {
                return Err(VoiceError::RetryMismatch);
            }
            old.retired |= receipt.retired;
        } else {
            status.engine.receipt = Some(receipt);
        }
    }
    status.engine.settled |= observed.engine.settled;
    // Root reports this actual Engine bit under the SAME fence as its terminal identity.
    // Retired/settled/missing input are never substitutes, and stale snapshots cannot erase it.
    status.acknowledged |= observed.acknowledged;
    Ok(())
}
struct Observation {
    row: Arc<Row>,
    action: Option<Arc<CryptoAction>>,
    notify: bool,
}
/// Fixed synchronous reconciliation graph, independently of an async worker's inline
/// frame. Native Row/Root records already pay their persistent cached state; callers must
/// also pay this actual observer, Root snapshot and transient merge/returned status copies.
/// No variable input/provider buffer or callback can be hidden behind this fixed inventory.
pub(super) fn reconciliation_layout_bytes() -> usize {
    std::mem::size_of::<Observation>()
        + std::mem::size_of::<CryptoHandoffStatus>()
        + 2 * std::mem::size_of::<Status>()
        + std::mem::size_of::<CryptoResultReceipt>()
        + 128 // nested guards/scalar/borrowed arguments; no SDK or variable allocation
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
        if let Body::SdpPending(p) = &mut state.body {
            p.observer = false;
        }
        if let Body::SdpLive(live) = &mut state.body {
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
            _custody: custody,
        } = self.owners.take().expect("one original worker");
        drop((selected, action));
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
