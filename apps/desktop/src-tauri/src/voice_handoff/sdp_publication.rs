//! Native lifetime custody for original SDP work, separate from the drained confirmation
//! Workflow. A queued Actor command must keep publication ordered against UI lock even if
//! its submitting task disappears. This wrapper grants no crypto/RTC authority: native final
//! admission and Root current-owner checks still apply independently.
use super::channel::{PhysicalHold, Session};
use catcoms_sync::voice_v2::chain::sdp_handoff::TaskCopy;
use catcoms_sync::voice_v2::sdp::InitialSdpIntent;
use catcoms_sync::voice_v2::VoiceError;
use std::sync::{Arc, OnceLock};

// Measure actual detached-worker/custodian/future storage before worker entry. Body/RPC
// metadata has a separate ORIGINAL Copy. Both come from the SAME 64KiB phase, not a new pool.
pub(super) const TASK_BYTES: usize = 4096;

pub(super) struct Publication {
    commit: OnceLock<tokio::sync::OwnedMutexGuard<()>>,
    _physical: PhysicalHold,
    // LAST. TaskCopy cannot adopt bytes, retain a keepalive or expose its underlying Copy.
    // Thus Publication cannot retain itself through its own funding owner.
    _payment: TaskCopy,
}
impl std::fmt::Debug for Publication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpPublication { .. }")
    }
}
impl Publication {
    /// After body/completion collection, wait WITHOUT Clock for the actual bound Actor
    /// sender's remaining Copy/keepalive/credit to retire. The same original worker and
    /// physical position exclude new same-purpose IO; TaskCopy itself remains retained.
    pub(super) async fn wait_other_io_collected(&self) -> Result<(), VoiceError> {
        self._payment.wait_other_native_io_collected().await
    }
    /// Outside every native/Root/Phase guard, BEFORE constructing a detached future or
    /// lending custody to an Actor body. Both inputs are affine original native owners.
    pub(super) fn preparing(
        physical: PhysicalHold,
        session: &Arc<Session>,
        intent: &Arc<InitialSdpIntent>,
        payment: TaskCopy,
    ) -> Result<Self, VoiceError> {
        let owner = Self::retaining(physical, payment);
        owner.validate(session, intent)?;
        Ok(owner)
    }
    /// Infallible custody construction for an ordered caller guard. It grants no
    /// permission; validate must succeed before native final admission or Arc sharing.
    /// Keeping the complete owner on the caller's guard also orders every refusal Drop
    /// AFTER Registration, rather than returning physical idle from an inner constructor.
    pub(super) fn retaining(physical: PhysicalHold, payment: TaskCopy) -> Self {
        Self {
            commit: OnceLock::new(),
            _physical: physical,
            _payment: payment,
        }
    }
    pub(super) fn validate(
        &self,
        session: &Arc<Session>,
        intent: &Arc<InitialSdpIntent>,
    ) -> Result<(), VoiceError> {
        if !self._physical.belongs_to(session) || !self._payment.uses_intent(intent) {
            return Err(VoiceError::Unauthorized);
        }
        if !self._payment.covers_work(TASK_BYTES) {
            return Err(VoiceError::Capacity);
        }
        Ok(())
    }
    /// Lifetime custody is shared before queueing with NO UI guard. Only the actual
    /// final native callback installs the admitted guard after its short guards release.
    /// The original Worker, body/RPC sender and output retain this same owner thereafter.
    pub(super) fn admit(&self, commit: tokio::sync::OwnedMutexGuard<()>) {
        assert!(
            self.commit.set(commit).is_ok(),
            "one original SDP publication admission"
        );
    }
}

// Arc counters, alignment, and the whole inline custodian are paid before Arc allocation.
#[repr(C)]
struct ArcAllocation<T> {
    strong: std::sync::atomic::AtomicUsize,
    weak: std::sync::atomic::AtomicUsize,
    value: T,
}
pub(super) const FIXED_BYTES: usize = std::mem::size_of::<ArcAllocation<Publication>>();
const _: () = assert!(FIXED_BYTES <= TASK_BYTES);
