//! Scoped final read admission. The actual ServerActor keeps ChannelSync exclusively
//! borrowed while native enters its own final fence; an earlier actor reply or intent
//! observation cannot substitute for this current Root admission.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoHandoffStatus, ObservedSdpInput, SdpInputRead};
use crate::voice_v2::helper::RoomAuthority;
use crate::voice_v2::runtime::PreparedLocalOrigin;

pub(crate) struct InputCurrent<'a> {
    pub(in crate::voice_v2) origin: PreparedLocalOrigin<'a>,
    pub(in crate::voice_v2) room: &'a RoomAuthority,
    pub(in crate::voice_v2) intent: &'a Arc<InitialSdpIntent>,
    pub(in crate::voice_v2) call: &'a CallLease,
    pub(in crate::voice_v2) watch: &'a crate::voice_v2::WatchLease,
    pub(in crate::voice_v2) pair: &'a CallLease,
}
impl InputCurrent<'_> {
    fn check(self) -> Result<(), VoiceError> {
        self.with_current(|| ())
    }
    /// Shared actual-Actor provenance for Input and engine admission. The engine's
    /// infallible permit insertion must run inside the original Group/Policy view,
    /// rather than after a reusable boolean currentness observation escapes it.
    pub(in crate::voice_v2) fn with_current(self, apply: impl FnOnce()) -> Result<(), VoiceError> {
        self.origin.with_origin(|wall, mono, origin| {
            if !self.room.lease.is_live()
                || !self.call.is_live()
                || !self.watch.is_live()
                || !self.pair.is_live()
                || !self.intent.current_at(wall, mono)
            {
                return Err(VoiceError::Unauthorized);
            }
            // Group/Policy were entered with the original peer and actual context.
            // The caller supplies only the already prepared infallible apply tail.
            // This creates no signed packet, traffic quota or RTC effect.
            origin.publish(apply)
        })
    }
}

enum State<'a> {
    History(CryptoHandoffStatus),
    Current {
        observed: ObservedSdpInput<'a>,
        current: Result<InputCurrent<'a>, VoiceError>,
    },
}
/// A single synchronous native admission opportunity. All original Clock callbacks run
/// before this value is lent; commit uses only try-locks and pure ownership checks. The
/// borrowed lifetime cannot escape the actual Root actor's exclusive ChannelSync call.
///
/// Native must bind its original Session/row/SDK/PC/funding and acquire its real UI
/// publication custody before commit. Its separately paid response retains the returned
/// affine Data owner until actual collection. This scoped value itself owns no capacity.
pub struct SdpInputAdmission<'a>(State<'a>);
redacted_debug!(SdpInputAdmission<'_>);
impl<'a> SdpInputAdmission<'a> {
    pub(crate) fn history(status: CryptoHandoffStatus) -> Self {
        Self(State::History(status))
    }
    pub(crate) fn current(
        observed: ObservedSdpInput<'a>,
        current: Result<InputCurrent<'a>, VoiceError>,
    ) -> Self {
        Self(State::Current { observed, current })
    }
    /// Terminal/settlement recovery wins over a now-stale current view. On a pending
    /// action, the SAME Engine fence spans current Group/Policy admission and input loan.
    /// No injected Clock, await, new allocation, engine permit or token mutation occurs.
    pub fn commit(self) -> Result<SdpInputRead, VoiceError> {
        match self.0 {
            State::History(status) => Ok(SdpInputRead::History(status)),
            State::Current { observed, current } => observed.commit(|| current?.check()),
        }
    }
}

/// Fixed owned/call-frame graph for the scoped read, additional to App's actual
/// envelope and native's independently paid publisher/output. No payload backing is
/// allocated: its original Action retains that existing separately paid allocation.
/// Inventory both history and pending preparations conservatively; all caps stay fixed.
pub const fn sdp_input_admission_bytes() -> usize {
    std::mem::size_of::<SdpInputAdmission<'static>>()
        + 2 * std::mem::size_of::<ObservedSdpInput<'static>>()
        + std::mem::size_of::<InputCurrent<'static>>()
        + std::mem::size_of::<Result<crate::voice_v2::Context, VoiceError>>()
        + std::mem::size_of::<Result<RoomAuthority, VoiceError>>()
        + std::mem::size_of::<Option<SdpInputRead>>()
        + std::mem::size_of::<SdpInputRead>()
        + std::mem::size_of::<CryptoHandoffStatus>()
        + std::mem::size_of::<PreparedLocalOrigin<'static>>()
        + 5 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 256 // fixed Clock samples, selectors, argument borrows and try-lock bookkeeping
}
