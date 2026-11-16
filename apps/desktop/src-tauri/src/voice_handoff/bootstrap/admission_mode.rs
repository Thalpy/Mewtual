//! First immutable admission-mode fence, in the original paid bootstrap Row.
//! This pin is not a Root registration or a successful capacity reservation. Until
//! canonical invocation/reclamation is connected, only ordinal1 may be selected;
//! falsely acknowledging advancement would let callers forget uncollected work.
use super::*;
use crate::voice_handoff::requests::negotiation_admission::{Kind, Request, REPLY_BYTES};

pub(super) struct AdmissionPin {
    generation: u64,
    cancelled_through: u64,
}
impl AdmissionPin {
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled_through != 0
    }
}

/// The observation owns no additional heap graph. Row's fixed10KiB pays its new
/// inline pin; the physical Control guard must count this actual borrowed frame.
pub(in crate::voice_handoff) fn observation_layout_bytes() -> usize {
    std::mem::size_of::<std::sync::MutexGuard<'_, State>>()
        + std::mem::size_of::<Request>()
        + REPLY_BYTES
        + 64 // exact closed/pin borrows, generation/floor encoding and return wrappers
}

impl Route {
    /// Authenticate the physical request before calling. Observe is noncreating;
    /// Register and Cancel both select sticky admission before the FIRST setup ACK.
    /// No Actor, Root, Clock, engine or capacity factory is called while holding State.
    pub(in crate::voice_handoff) fn admission_mode(
        &self,
        request: Request,
    ) -> Result<[u8; REPLY_BYTES], Refused> {
        // Do not apply a negative floor before validating its exact native ordinal.
        // In particular MAX must not destroy a Session's future progression.
        if request.generation != 1 {
            return Err(Refused::Bounds);
        }
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let closed = state.closed || state.effects_closed || !self.row.selected.lease().is_live();
        if state.admission.is_none() && request.kind != Kind::Observe {
            if closed || state.pending || state.browser_setup.is_none() {
                // Already ACKed pilot Sessions can never be upgraded in place.
                return Err(Refused::Binding);
            }
            state.admission = Some(AdmissionPin {
                generation: 1,
                cancelled_through: 0,
            });
        }
        if let Some(pin) = &mut state.admission {
            if request.kind == Kind::Cancel {
                pin.cancelled_through = pin.cancelled_through.max(request.generation);
            }
        }
        if request.kind == Kind::Cancel && state.admission.is_some() {
            // Whole-mode cancellation fences all current/future Slot attachment.
            // This is negative only: the mode reply never certifies reclamation.
            state.admission_slot.close();
        }
        let mut snapshot = [0; REPLY_BYTES];
        snapshot[0] = 10;
        snapshot[1] = if closed {
            3 // Closed; no positive admission can follow.
        } else if let Some(pin) = &state.admission {
            if pin.cancelled_through == pin.generation {
                2
            } else {
                1
            }
        } else {
            0 // Unselected, never a pilot/Ready permission.
        };
        if let Some(pin) = &state.admission {
            snapshot[2..10].copy_from_slice(&pin.generation.to_be_bytes());
            snapshot[10..].copy_from_slice(&pin.cancelled_through.to_be_bytes());
        }
        drop(state);
        if request.kind == Kind::Cancel {
            self.row.admission_changed.notify_waiters();
        }
        Ok(snapshot)
    }

    /// Repeat at the FINAL native pilot publication fence, not just in maintenance.
    /// Negative/history cleanup uses the ordinary Route gates and remains available.
    pub(in crate::voice_handoff) fn require_pilot_open(&self) -> Result<(), Refused> {
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed
            || state.effects_closed
            || state.pending
            // ACK is monotone. A successful pilot check requires ACK completed;
            // first mode selection requires it still outstanding. Thus even a
            // retained pre-ACK producer cannot pass, then race a successful pin.
            || state.browser_setup.is_some()
            || state.admission.is_some()
            || !self.row.selected.lease().is_live()
        {
            return Err(Refused::Binding);
        }
        Ok(())
    }
}
