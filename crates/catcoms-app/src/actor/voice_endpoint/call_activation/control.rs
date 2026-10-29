//! Original Actor control without retaining its command channel in native history.
//!
//! Tokio WeakSender is logically weak but physically owns Arc<Chan>; it is NOT a
//! tombstone. Keep it only on the actual Actor task. Native retains a real std::Weak
//! to this small independently funded owner, whose body drops channel references at
//! strong-zero. No raw Tokio weak or strong owner may enter an Envelope/Publisher.
use super::*;
use catcoms_sync::voice_v2::call_activation::ActivationInstancePin;
use std::sync::{Mutex, Weak};

pub const VOICE_ACTIVATION_CONTROL_BYTES: usize = 4096;
struct Funding {
    _credit: catcoms_sync::voice_v2::resources::Credit,
}
pub(super) struct Owner {
    pub(super) sender: mpsc::WeakSender<Envelope>,
    pin: ActivationInstancePin,
    // Only final open-check + Permit.send, never callbacks, allocation, Clock,
    // Root teardown, waits or destruction. ActorInbox::drop closes it before Rx.
    pub(super) send: Mutex<bool>,
}
pub(super) struct PaidOwner {
    pub(super) owner: Arc<Owner>,
    // LAST: Sender/owner body, header and final weak allocation stay sponsored.
    _funding: Arc<Funding>,
}
/// Original trusted routing control; not membership, call/profile or Root-return
/// authority. Every clone pairs the SAME plain payer with its real weak tombstone.
#[derive(Clone)]
pub struct VoiceActivationControl {
    owner: Weak<Owner>,
    pin: ActivationInstancePin,
    pub(super) trace: Trace,
    funding: Arc<Funding>,
}
impl std::fmt::Debug for VoiceActivationControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceActivationControl { .. }")
    }
}
impl VoiceActivationControl {
    pub fn is_same(&self, other: &Self) -> bool {
        self.owner.ptr_eq(&other.owner) && self.pin.is_same(&other.pin)
    }
    pub fn with_trace(&self, trace: u64) -> Self {
        Self {
            trace: Trace(trace),
            ..self.clone()
        }
    }
    pub(super) fn matches(&self, original: &ActivationObserver) -> bool {
        self.pin.matches(original)
    }
    pub(super) fn upgrade(&self) -> Option<PaidOwner> {
        Some(PaidOwner {
            owner: self.owner.upgrade()?,
            _funding: self.funding.clone(),
        })
    }
}

/// Same original inert Root pair plus the actual Actor's negative routing control.
/// Dropping a lost prepare reply revokes the pair; it cannot publish Root selection.
#[derive(Debug)]
pub struct PreparedVoiceCall {
    prepared: PreparedAdmittedCall,
    control: VoiceActivationControl,
}
impl PreparedVoiceCall {
    pub fn grant(&self) -> &Arc<CallGrant> {
        self.prepared.grant()
    }
    pub fn observer(&self) -> ActivationObserver {
        self.prepared.observer()
    }
    pub fn control(&self) -> VoiceActivationControl {
        self.control.clone()
    }
    pub fn into_parts(self) -> (PreparedAdmittedCall, VoiceActivationControl) {
        (self.prepared, self.control)
    }
}

pub(in crate::actor) struct ActorControl {
    // This raw physical weak is task-local only. It does not increment logical
    // Sender count, so dropping ordinary handles still lets recv(None) terminate.
    seed: Option<mpsc::WeakSender<Envelope>>,
    owner: Option<PaidOwner>,
}
impl ActorControl {
    fn new(seed: mpsc::WeakSender<Envelope>) -> Self {
        Self {
            seed: Some(seed),
            owner: None,
        }
    }
    pub(super) fn wrap(
        &mut self,
        prepared: PreparedAdmittedCall,
        trace: Trace,
    ) -> Result<PreparedVoiceCall, VoiceError> {
        let original = prepared.observer();
        let pin = original.instance_pin();
        if let Some(owner) = &self.owner {
            if !owner.owner.pin.is_same(&pin) {
                return Err(VoiceError::Scope);
            }
        } else {
            let seed = self.seed.as_ref().ok_or(VoiceError::Unauthorized)?;
            if control_layout_bytes() > VOICE_ACTIVATION_CONTROL_BYTES {
                return Err(VoiceError::Capacity);
            }
            // Reserve BEFORE either Arc allocation, not retrospectively after
            // exposing a weak observer or registering a native pending call.
            let credit = original.reserve_retained(VOICE_ACTIVATION_CONTROL_BYTES)?;
            let funding = Arc::new(Funding { _credit: credit });
            self.owner = Some(PaidOwner {
                owner: Arc::new(Owner {
                    sender: seed.clone(),
                    pin,
                    send: Mutex::new(true),
                }),
                _funding: funding,
            });
        }
        let owner = self.owner.as_ref().expect("original paid owner");
        let control = VoiceActivationControl {
            owner: Arc::downgrade(&owner.owner),
            pin: owner.owner.pin.clone(),
            trace,
            funding: owner._funding.clone(),
        };
        Ok(PreparedVoiceCall { prepared, control })
    }
    pub(in crate::actor) fn close(&self) {
        if let Some(owner) = &self.owner {
            *owner.owner.send.lock().unwrap_or_else(|e| e.into_inner()) = false;
        }
    }
    fn detach(&mut self) {
        self.close(); // mutex guard has returned before any last destructor
        drop(self.owner.take());
        drop(self.seed.take());
    }
}
impl Drop for ActorControl {
    fn drop(&mut self) {
        self.detach();
    }
}

/// Capture as ONE aggregate in the actual spawned Actor future. Its explicit Drop
/// and field order close the SAME send fence before receiver destruction on abort,
/// panic and an unpolled future, not just the normal terminal command branches.
pub(in crate::actor) struct ActorInbox {
    control: ActorControl,
    receiver: mpsc::Receiver<Envelope>,
}
impl ActorInbox {
    pub(in crate::actor) fn new(
        receiver: mpsc::Receiver<Envelope>,
        sender: &mpsc::Sender<Envelope>,
    ) -> Self {
        Self {
            control: ActorControl::new(sender.downgrade()),
            receiver,
        }
    }
    pub(in crate::actor) fn control(&mut self) -> &mut ActorControl {
        &mut self.control
    }
    pub(in crate::actor) fn close_activation(&mut self) {
        // Normal shutdown retains THIS closed receiver to join older pre-close
        // permits. Detach the new-family owner and raw seed before Root retirement
        // or blocked Closed delivery; that separate receiver lifetime must not
        // keep native history's routing control alive. Actual argument owners keep
        // their own SAME payment until they return.
        self.control.detach();
    }
    pub(in crate::actor) async fn recv(&mut self) -> Option<Envelope> {
        self.receiver.recv().await
    }
    pub(in crate::actor) fn close(&mut self) {
        self.control.close();
        self.receiver.close();
    }
    pub(in crate::actor) fn try_recv(&mut self) -> Result<Envelope, mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}
impl Drop for ActorInbox {
    fn drop(&mut self) {
        self.control.detach();
        // Receiver is the LAST field and drops only after the fence is closed and
        // the anchor/raw seed have actually dropped, outside that mutex.
    }
}

/// Full fixed owner/header/weak tail and aggregate, independently of every App
/// RPC and native Cell. Live channel backing belongs to its actual Actor/Receiver;
/// a surviving native stdWeak cannot retain those body fields at strong-zero.
pub fn control_layout_bytes() -> usize {
    #[repr(C, align(2))]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    std::mem::size_of::<ArcAllocation<Owner>>()
        + std::mem::size_of::<ArcAllocation<Funding>>()
        + std::mem::size_of::<ArcAllocation<()>>()
        + std::mem::size_of::<ActorInbox>()
        + std::mem::size_of::<PaidOwner>()
        + 4 * std::mem::size_of::<VoiceActivationControl>()
        + 256
}

#[cfg(test)]
mod tests;
