//! Whole original Source cleanup, distinct from UI-lock key drainage.
//!
//! Native must retain the canonical Sources until this finite worker proves
//! completion, then dispose Sources/facade outside all guards. Dropping a worker
//! relinquishes its single capture slot, never the original bodies or evidence.
use super::*;
use catcoms_rt::NativeChatKeyDrain;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fee-only collection control. Neither capture acknowledgements nor a future
/// negative observer may retain the collection, an issuer or Physical here.
pub(super) struct Control {
    captured: AtomicBool,
    _payment: CatalogReadReservation,
}
impl Control {
    pub(super) fn new(payment: CatalogReadReservation) -> Self {
        Self {
            captured: AtomicBool::new(false),
            _payment: payment,
        }
    }
}

pub(super) fn invoice<T: MeshTransport, R: CryptoRngCore>() -> usize {
    2 * std::mem::size_of::<NativeTextSourceRetirement<T, R>>()
        + std::mem::size_of::<Control>()
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextSourceRetirementStatus {
    /// Retry the SAME canonical collection. Neither a timer nor an idle key
    /// counter proves SDK/body/network destruction.
    Pending,
    /// All captured original rows passed the complete conjunction. The caller
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

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSources<T, R> {
    /// Close original admission first, then acquire the one prepaid work slot.
    /// This finite capture does not wait or destroy any affine owner. Call only
    /// outside native/Root guards; close_admission takes the short entry mutex.
    pub fn prepare_retirement(
        &self,
    ) -> Result<NativeTextSourceRetirement<T, R>, NativeTextSourceRetirementRefusal> {
        use NativeTextSourceRetirementRefusal as Error;
        self.close_admission();
        self.retirement
            .captured
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| Error::Busy)?;
        // Ack exists before any fallible lock/capture. Local rows leave before
        // Ack on refusal/unwind; no capacity or issuer gets dropped in a writer.
        let ack = CaptureAck(self.retirement.clone());
        let mut rows = std::array::from_fn(|_| None);
        {
            let entries = self.entries.try_lock().map_err(|_| Error::Busy)?;
            for (target, original) in rows.iter_mut().zip(entries.iter()) {
                *target = Some(Row {
                    cell: original.runtime.clone(),
                    keys: original.owner.observe_retirement(),
                });
            }
        }
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
        let mut pending = false;
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
