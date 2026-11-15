//! Whole original Source cleanup, distinct from UI-lock key drainage.
//!
//! Native must retain the canonical Sources until this finite worker proves
//! completion, then dispose Sources/facade outside all guards. Dropping a worker
//! relinquishes its single capture slot, never the original bodies or evidence.
use super::*;
use catcoms_rt::NativeChatKeyDrain;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Weak;
const PREPARATIONS_CLOSED: usize = 1 << (usize::BITS - 1);
const MAX_PREPARATIONS: usize = 2;

/// Fee-only collection control. Neither capture acknowledgements nor a future
/// negative observer may retain the collection, an issuer or Physical here.
pub(super) struct Control {
    captured: AtomicBool,
    preparations: AtomicUsize,
    _payment: CatalogReadReservation,
}
impl Control {
    pub(super) fn new(payment: CatalogReadReservation) -> Self {
        Self {
            captured: AtomicBool::new(false),
            preparations: AtomicUsize::new(0),
            _payment: payment,
        }
    }
    pub(super) fn enter_preparation(self: &Arc<Self>) -> Option<PreparationEnd> {
        self.preparations
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                (value & PREPARATIONS_CLOSED == 0 && value < MAX_PREPARATIONS).then_some(value + 1)
            })
            .ok()?;
        Some(PreparationEnd(self.clone()))
    }
    pub(super) fn close_preparations(&self) {
        self.preparations
            .fetch_or(PREPARATIONS_CLOSED, Ordering::SeqCst);
    }
    fn preparations_drained(&self) -> bool {
        self.preparations.load(Ordering::SeqCst) == PREPARATIONS_CLOSED
    }
}

/// Admission and closure share one packed atomic: no delayed increment can
/// arrive behind CLOSED+zero. LAST after actual preparation custody leaves.
pub(super) struct PreparationEnd(Arc<Control>);
impl Drop for PreparationEnd {
    fn drop(&mut self) {
        let previous = self.0.preparations.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(previous & !PREPARATIONS_CLOSED != 0);
    }
}

pub(super) fn invoice<T: MeshTransport, R: CryptoRngCore>() -> usize {
    2 * std::mem::size_of::<NativeTextSourceRetirement<T, R>>()
        + 2 * std::mem::size_of::<NativeTextSourceRetirementPreparation<T, R>>()
        + std::mem::size_of::<Control>()
        // The Weak book can outlive Sources. Its real allocation header and
        // Mutex/Vec metadata stay covered by the capture's Control fee tail.
        + std::mem::size_of::<SourceBook<T, R>>() + 2 * std::mem::size_of::<usize>()
        + 4096 // fixed capture/ack and Arc allocation overlap
}

enum TcpDisposition {
    // Recorded only by examining the genuine present Runtime. Missing Runtime
    // or missing retirement metadata is UNKNOWN, never this negative proof.
    NotPrepared,
    Tracked(catcoms_net::NativeTcpDrain),
}
impl TcpDisposition {
    fn drained(&self) -> bool {
        match self {
            Self::NotPrepared => true,
            Self::Tracked(original) => original.is_drained(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destruction {
    NotDestroyed,
    Returned,
    Unwound,
}

/// Canonical metadata stays in the SAME cell after a lost worker/reply. Its TCP
/// observation is fee-only; it cannot keep a transport or Physical alive.
pub(super) struct BodyState {
    sdk_closed: bool,
    tcp: Option<TcpDisposition>,
    destroyed: Destruction,
}
impl BodyState {
    pub(super) fn new() -> Self {
        Self {
            sdk_closed: false,
            tcp: None,
            destroyed: Destruction::NotDestroyed,
        }
    }
    fn complete(&self) -> bool {
        self.sdk_closed
            && self.destroyed != Destruction::NotDestroyed
            && self.tcp.as_ref().is_some_and(TcpDisposition::drained)
    }
}

struct Row<T: MeshTransport, R: CryptoRngCore> {
    cell: Arc<RuntimeCell<T, R>>,
    keys: NativeChatKeyDrain,
}
struct CaptureAck(Arc<Control>);
impl Drop for CaptureAck {
    fn drop(&mut self) {
        self.0.captured.store(false, Ordering::SeqCst);
    }
}

/// A bounded WORK capture, not a metadata-only drain: its cells can still own
/// original SDK/physical resources. Native must keep its canonical Sources too.
/// Never capture, run or destroy this value while holding native/Root writers.
pub struct NativeTextSourceRetirement<T: MeshTransport, R: CryptoRngCore> {
    rows: [Option<Row<T, R>>; MAX_SOURCES],
    // LAST: another worker cannot enter until all old cell/key pins are gone.
    _ack: CaptureAck,
}
/// Guard-safe negative capture. It has not yet visited/closed individual RT
/// SourceOwners. Native must keep the SAME canonical Sources until finish and
/// whole drainage; a successful capture alone establishes no SDK/key shutdown.
pub struct NativeTextSourceRetirementPreparation<T: MeshTransport, R: CryptoRngCore> {
    book: Weak<SourceBook<T, R>>,
    #[cfg(test)]
    weak_drop_order: Option<WeakDropOrder>,
    // Slot acknowledgement follows real Weak/cell work. Busy has no slot but
    // still owns the unconditional fee-only tail below.
    ack: Option<CaptureAck>,
    // LAST for EVERY capture, including Busy/Unknown. The original weak-book
    // allocation must not outlive the actual CONTROL reservation paying it.
    _control: Arc<Control>,
}

// Field immediately AFTER the actual Weak observes the exact debit's Drop
// flag, including partial-move error returns. It is absent in production.
#[cfg(test)]
struct WeakDropOrder {
    refunded: Arc<AtomicBool>,
    observed: Arc<AtomicBool>,
    covered: Arc<AtomicBool>,
}
#[cfg(test)]
impl Drop for WeakDropOrder {
    fn drop(&mut self) {
        self.covered
            .store(!self.refunded.load(Ordering::SeqCst), Ordering::SeqCst);
        self.observed.store(true, Ordering::SeqCst);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextSourceRetirementStatus {
    /// Retry the SAME canonical collection. Neither a timer nor an idle key
    /// counter proves SDK/body/network destruction.
    Pending,
    /// All admitted preparations retired and original rows passed the complete
    /// conjunction. No-End refusal metadata may remain. The caller
    /// must still destroy the original Sources/facade outside its own guards.
    Drained,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextSourceRetirementRefusal {
    Busy,
    /// Missing/contradictory original evidence. A fresh collection, mount or
    /// empty body cannot repair this by impersonating an earlier consumer.
    Unknown,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for NativeTextSourceRetirement<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextSourceRetirement { .. }")
    }
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug
    for NativeTextSourceRetirementPreparation<T, R>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTextSourceRetirementPreparation { .. }")
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSources<T, R> {
    /// Close original admission first, then acquire the one prepaid work slot.
    /// This finite capture does not wait or destroy any affine owner. Call only
    /// outside native/Root guards; close_admission takes the short entry mutex.
    pub fn prepare_retirement(
        &self,
    ) -> Result<NativeTextSourceRetirement<T, R>, NativeTextSourceRetirementRefusal> {
        self.capture_retirement().finish()
    }

    /// Atomic/Weak-only, infallible owning capture under a short native guard.
    /// Its caller has already fenced the actual UI generation. No entries lock,
    /// RT owner/issuer lookup, payment release, SDK call or IO occurs here.
    pub fn capture_retirement(&self) -> NativeTextSourceRetirementPreparation<T, R> {
        self.scope.close_first();
        self.retirement.close_preparations();
        let ack = self
            .retirement
            .captured
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| CaptureAck(self.retirement.clone()));
        NativeTextSourceRetirementPreparation {
            book: Arc::downgrade(&self.entries),
            #[cfg(test)]
            weak_drop_order: None,
            ack,
            _control: self.retirement.clone(),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSourceRetirementPreparation<T, R> {
    /// Visit the SAME original book outside all native/Root guards. Close every
    /// original source counter before returning body work. Slot custody moves
    /// directly to the worker; lost finish/reply cannot briefly admit a second.
    pub fn finish(
        mut self,
    ) -> Result<NativeTextSourceRetirement<T, R>, NativeTextSourceRetirementRefusal> {
        use NativeTextSourceRetirementRefusal as Error;
        if self.ack.is_none() {
            return Err(Error::Busy);
        }
        let mut rows = std::array::from_fn(|_| None);
        let book = self.book.upgrade().ok_or(Error::Unknown)?;
        {
            let entries = book.try_lock().map_err(|_| Error::Busy)?;
            for original in entries.iter() {
                original.owner.close();
            }
            for (target, original) in rows.iter_mut().zip(entries.iter()) {
                *target = Some(Row {
                    cell: original.runtime.clone(),
                    keys: original.owner.observe_retirement(),
                });
            }
        }
        drop(book); // may be the final issuer book, always outside its writer
                    // Only successful, infallible handoff takes the slot from self. On
                    // every earlier error self drops Weak first, then slot/control last.
        let ack = self.ack.take().expect("original retirement capture slot");
        Ok(NativeTextSourceRetirement { rows, _ack: ack })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSourceRetirement<T, R> {
    /// Make one bounded pass over original rows. SDK/transport destruction and
    /// any admitted finite SDK close work execute outside every control writer.
    /// Pending preserves the same partial SDK state for another pass; there is
    /// no timeout, replacement constructor, general refund or ambiguous retry.
    pub fn try_retire(
        &mut self,
    ) -> Result<NativeTextSourceRetirementStatus, NativeTextSourceRetirementRefusal> {
        let mut pending = !self._ack.0.preparations_drained();
        for row in self.rows.iter().flatten() {
            if !row.try_retire()? {
                pending = true;
            }
        }
        Ok(if pending {
            NativeTextSourceRetirementStatus::Pending
        } else {
            NativeTextSourceRetirementStatus::Drained
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Row<T, R> {
    fn try_retire(&self) -> Result<bool, NativeTextSourceRetirementRefusal> {
        use NativeTextSourceRetirementRefusal as Error;
        // This negative remains distinct from worker presence and body Drop.
        // It must be closed even if no key primitive was ever entered.
        let keys_drained = self.keys.is_drained();
        let body = {
            let mut state = match self.cell.state.try_lock() {
                Ok(state) => state,
                Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(Error::Unknown),
            };
            state.retiring = true;
            if state.poisoned {
                return Err(Error::Unknown);
            }
            if !keys_drained || state.running {
                return Ok(false);
            }
            if state.retirement.destroyed != Destruction::NotDestroyed {
                if state.body.is_some()
                    || !state.retirement.sdk_closed
                    || state.retirement.tcp.is_none()
                {
                    return Err(Error::Unknown);
                }
                return Ok(state.retirement.complete());
            }
            // A missing body without its actual destruction envelope is never
            // success, even if an old task failed and every counter is idle.
            let body = state.body.take().ok_or(Error::Unknown)?;
            state.running = true;
            body
        };
        let mut loan = RuntimeLoan {
            body: Some(body),
            cell: self.cell.clone(),
            recorded: true,
        };
        if let Some(driver) = loan.body.as_mut().and_then(|body| body.driver.as_mut()) {
            // Whole driver closes owner, projection and member domains. In
            // particular an admitted member save End still keeps this Busy.
            if driver.close_and_drain().is_err() {
                return Ok(false); // exact mutated driver restored by Loan Drop
            }
        }
        let body = loan.body.take().expect("original retirement loan");
        let tcp = body
            .network
            .as_ref()
            .and_then(|network| network.observe_tcp())
            .map(TcpDisposition::Tracked)
            .unwrap_or(TcpDisposition::NotPrepared);
        // Arm the destruction envelope BEFORE acquiring its metadata writer.
        // Field order proves every actual Runtime field Drop has completed
        // before acknowledgment, including a destructor unwinding that Drop.
        let destruction = DestroyBody {
            _body: body,
            _after: BodyDestroyed(self.cell.clone()),
        };
        {
            let mut state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
            state.retirement.sdk_closed = true;
            // This can only be empty: a whole body is borrowed exactly once at
            // a time and destruction is terminal. Never drop an old TCP tail
            // or its final fee under this writer.
            debug_assert!(state.retirement.tcp.is_none());
            state.retirement.tcp = Some(tcp);
        }
        drop(destruction); // also drops an unused prearmed TCP preparation
        drop(loan); // running=false only AFTER actual whole Runtime destruction
        let state = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(!state.poisoned
            && !state.running
            && state.body.is_none()
            && self.keys.is_drained()
            && state.retirement.complete())
    }
}

// Deliberately no custom Drop: Rust field-drop glue runs _body FIRST and _after
// LAST, including unwinding a body field destructor. An outer worker guard or
// RuntimeLoan Drop alone cannot manufacture this evidence.
struct DestroyBody<T: MeshTransport, R: CryptoRngCore> {
    _body: Runtime<T, R>,
    _after: BodyDestroyed<T, R>,
}
struct BodyDestroyed<T: MeshTransport, R: CryptoRngCore>(Arc<RuntimeCell<T, R>>);
impl<T: MeshTransport, R: CryptoRngCore> Drop for BodyDestroyed<T, R> {
    fn drop(&mut self) {
        {
            let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
            state.retirement.destroyed = if std::thread::panicking() {
                Destruction::Unwound
            } else {
                Destruction::Returned
            };
        }
        self.0.returned.notify_waiters();
    }
}

#[cfg(test)]
pub(super) mod tests;
