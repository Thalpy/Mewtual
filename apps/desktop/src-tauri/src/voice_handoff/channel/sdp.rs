//! Capacity-only permit for one immutable authenticated SDP header. It is not a row,
//! action, engine, publication or RTC grant. Generic optional-Phase intake cannot use it.
use super::*;
use catcoms_app::VOICE_SDP_RPC_METADATA_BYTES;
use catcoms_sync::voice_v2::chain::sdp_handoff::{self, NativeRowFunding, NativeRowPurpose};
use catcoms_sync::voice_v2::sdp::InitialSdpIntent;
use catcoms_sync::voice_v2::{Scope, VoiceError};

/// Private native provenance retained from the exact indexed parent/row lookup. The
/// constructor still checks actual objects, rather than accepting an asserted validity bit.
pub(in crate::voice_handoff) struct Permit {
    phase: Arc<sdp_handoff::Phase>,
    intent: Arc<InitialSdpIntent>,
    funding: Arc<NativeRowFunding>,
    session: Arc<SessionIdentity>,
    scope: Scope,
    // Complete canonical header, including direction, sequence and reserved bytes. A
    // permit for a prior physical request cannot be transplanted onto another flight.
    request: Header,
    clock: Arc<dyn catcoms_rt::Clock>,
    position: Arc<PhysicalPosition>,
}
impl Permit {
    /// Call outside native fences: the fixed fee is recovered/paid before any body
    /// allocation. Only the native ledger should resolve these actual original owners.
    /// Header MAC alone never supplies them, and equal labels cannot substitute a Phase.
    pub(in crate::voice_handoff) fn original_begin(
        authenticated: &AuthenticatedRequest,
        session: Arc<SessionIdentity>,
        phase: Arc<sdp_handoff::Phase>,
        intent: Arc<InitialSdpIntent>,
        scope: Scope,
        clock: Arc<dyn catcoms_rt::Clock>,
    ) -> Result<Self, VoiceError> {
        if !authenticated.belongs_to(&session)
            || authenticated.operation() != Operation::SdpSealBegin
            || authenticated.kind() != BodyKind::Sdp
            || !super::super::requests::sdp_action::wire_shape(
                authenticated.operation(),
                authenticated.kind(),
                authenticated.logical(),
                authenticated.length(),
            )
            || !intent.uses_phase(&phase)
            || !phase.effect().is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        intent.remaining_at(clock.now_ms(), clock.monotonic_ms())?;
        let funding = phase.native_history(NativeRowPurpose::Seal)?;
        Ok(Self {
            phase,
            intent,
            funding,
            session,
            scope,
            request: authenticated.request,
            clock,
            position: authenticated.worker.position.clone(),
        })
    }
    /// No body allocation or counter debit precedes the full original allocation check.
    /// Reserve actual backing bytes AND the later Actor/RPC metadata in one original Copy;
    /// it moves unchanged into VoiceSdpBody when the native action adapter is integrated.
    pub(super) fn allocate(
        self,
        authenticated: &AuthenticatedRequest,
    ) -> Result<sdp_handoff::Buffer, Refused> {
        let original = &authenticated.worker.position.session;
        if !authenticated.belongs_to(&self.session)
            || !Arc::ptr_eq(&authenticated.worker.position, &self.position)
            || authenticated.request != self.request
            || authenticated.kind() != BodyKind::Sdp
            || !self.intent.uses_phase(&self.phase)
            || !self.phase.uses(&original.resources, self.scope)
            || self.scope.group != original.group
            || !self.funding.uses_phase(&self.phase)
            || self.funding.purpose() != NativeRowPurpose::Seal
        {
            return Err(Refused::Binding);
        }
        self.intent
            .remaining_at(self.clock.now_ms(), self.clock.monotonic_ms())
            .map_err(map)?;
        let bytes = self
            .request
            .length()
            .checked_add(VOICE_SDP_RPC_METADATA_BYTES)
            .ok_or(Refused::Bounds)?;
        let mut copy = self.phase.copy(bytes).map_err(map)?;
        // CLOSED and this concrete counter's independent16-Copy bound are checked before
        // allocating the Vec. A local refusal must leave both physical nonces untouched.
        copy.bind_native_history(self.funding).map_err(map)?;
        // A contended reserve or injected original Clock can revoke this owner. Recheck
        // outside guards immediately before allocating; refusal drops the actual Copy.
        self.intent
            .remaining_at(self.clock.now_ms(), self.clock.monotonic_ms())
            .map_err(map)?;
        copy.adopt(vec![0; self.request.length()]).map_err(map)
    }
}
fn map(error: VoiceError) -> Refused {
    if error == VoiceError::Capacity {
        Refused::Capacity
    } else {
        Refused::Binding
    }
}
