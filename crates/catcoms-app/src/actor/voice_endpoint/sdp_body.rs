//! Concrete paid SDP storage crossing the bounded Actor mailbox. No generic AsRef callback,
//! renderer descriptor, byte copy or raw Vec extraction is allowed at this boundary. The
//! original native publication guard travels with the same Buffer/Copy through actual RPC
//! collection; timeout, a lost observer or reading result metadata never proves drainage.
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::{Buffer, Copy};
use catcoms_sync::voice_v2::sdp::InitialSdpIntent;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fixed body/RPC graph allowance to include in the ORIGINAL Copy before constructing its
/// body. Actual buffer backing capacity and the native task/publication contribution are
/// additional, separately measured terms. This does not increase the 64KiB native phase.
pub const VOICE_SDP_RPC_METADATA_BYTES: usize = 4096;

// Model both Arc allocations including their counters and alignment. Re-audit with the
// pinned Rust/Tokio implementations on toolchain updates; allocator slack is not included.
#[repr(C)]
struct ArcAllocation<T> {
    strong: std::sync::atomic::AtomicUsize,
    weak: std::sync::atomic::AtomicUsize,
    value: T,
}
/// Common fixed RPC graph. Use voice_sdp_rpc_bytes with the ACTUAL calling future before
/// admission, so eager construction is covered too.
/// Buffer backing capacity and native task/publication storage are separately paid. The
/// Buffer/Copy inline value is already counted inside State; no extra payload copy exists.
pub const VOICE_SDP_RPC_FIXED_BYTES: usize = std::mem::size_of::<ArcAllocation<State>>()
    + std::mem::size_of::<ArcAllocation<Work>>()
    + std::mem::size_of::<super::Command>()
    + std::mem::size_of::<crate::AppCommand>()
    + std::mem::size_of::<VoiceSdpBody>()
    + std::mem::size_of::<VoiceSdpCompletion<Arc<CryptoAction>>>()
    + std::mem::size_of::<VoiceSdpCompletion<CryptoResultReceipt>>()
    + std::mem::size_of::<VoiceSdpCompletion<catcoms_sync::voice_v2::sdp::SdpOpenResult>>()
    + std::mem::size_of::<VoiceSdpPending<Arc<CryptoAction>>>()
    + std::mem::size_of::<VoiceSdpPending<CryptoResultReceipt>>()
    + std::mem::size_of::<VoiceSdpPending<catcoms_sync::voice_v2::sdp::SdpOpenResult>>()
    + 1024; // pinned Tokio oneshot allocation, envelope and channel bookkeeping

/// Common storage plus the larger of eager construction and the actual returned future.
/// These execution phases do not coexist: the constructor moves its single request pair
/// into that future before returning. Backing allocation/common channel graph remains
/// additive in both phases. Native's enclosing worker/publication needs separate payment.
pub const fn voice_sdp_rpc_bytes(future_bytes: usize) -> usize {
    let transient = if future_bytes > super::sdp_ops::RECORDED_CONSTRUCTION_BYTES {
        future_bytes
    } else {
        super::sdp_ops::RECORDED_CONSTRUCTION_BYTES
    };
    VOICE_SDP_RPC_FIXED_BYTES.saturating_add(transient)
}
const _: () = assert!(voice_sdp_rpc_bytes(0) <= VOICE_SDP_RPC_METADATA_BYTES);

pub struct VoiceSdpBody {
    state: Arc<State>,
}
struct State {
    busy: AtomicBool,
    range: Range<usize>,
    intent: Arc<InitialSdpIntent>,
    // LAST: bytes zeroize/deallocate before their keepalive reference and credit return.
    // A worker retaining publication has a separate TaskCopy; Phase drainage waits for ALL
    // of them. No external mutable view survives transfer.
    buffer: Buffer,
}
impl std::fmt::Debug for VoiceSdpBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSdpBody { .. }")
    }
}
/// Proven local construction refusal. The caller receives the SAME original paid Buffer,
/// including any publication keepalive, so Capacity cannot discard its only sealed result.
pub struct VoiceSdpBodyRefusal {
    error: VoiceError,
    buffer: Buffer,
}
impl std::fmt::Debug for VoiceSdpBodyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSdpBodyRefusal { .. }")
    }
}
impl VoiceSdpBodyRefusal {
    pub fn into_parts(self) -> (VoiceError, Buffer) {
        (self.error, self.buffer)
    }
}
impl VoiceSdpBody {
    /// Validate every bound and the original phase identity before allocating an Arc. The
    /// phase may already be closed for history recovery; identity is never fresh authority.
    /// Native must authenticate the full Session/row/purpose and choose this actual intent.
    // A proven local refusal returns the SAME paid Buffer inline. Boxing this error would
    // allocate unbudgeted storage precisely on a capacity-refusal path, or lose those bytes.
    #[allow(clippy::result_large_err)]
    pub fn from_buffer(
        buffer: Buffer,
        range: Range<usize>,
        intent: Arc<InitialSdpIntent>,
    ) -> Result<Self, VoiceSdpBodyRefusal> {
        let error = if range.start > range.end
            || range.end > buffer.bytes().len()
            || range.end - range.start > catcoms_sync::voice_v2::MAX_SDP_PLAINTEXT + 16
        {
            Some(VoiceError::Bounds)
        } else if !buffer.uses_intent(&intent) {
            Some(VoiceError::Unauthorized)
        } else if buffer
            .capacity()
            .checked_add(VOICE_SDP_RPC_METADATA_BYTES)
            .is_none_or(|bytes| !buffer.covers_work(bytes))
        {
            Some(VoiceError::Capacity)
        } else {
            None
        };
        if let Some(error) = error {
            return Err(VoiceSdpBodyRefusal { error, buffer });
        }
        Ok(Self {
            state: Arc::new(State {
                busy: AtomicBool::new(false),
                range,
                intent,
                buffer,
            }),
        })
    }
    /// Native attaches its actual admitted UI/physical publication custodian only while
    /// this fresh body remains unique, before any RPC claim. A separate TaskCopy funds that
    /// custodian, which must NEVER retain this Body or attach itself to its own payment.
    /// This is lifetime custody only; Root still validates every current protocol owner.
    pub fn attach_publication(
        &mut self,
        custody: catcoms_rt::SharedRequestKeepalive,
    ) -> Result<(), VoiceError> {
        let state = Arc::get_mut(&mut self.state).ok_or(VoiceError::Capacity)?;
        state.buffer.attach_keepalive(custody)
    }
    pub(super) fn claim(
        &self,
        intent: &Arc<InitialSdpIntent>,
        maximum: usize,
    ) -> Result<Flight, VoiceError> {
        if !Arc::ptr_eq(&self.state.intent, intent) || !self.state.buffer.has_keepalive() {
            return Err(VoiceError::Unauthorized);
        }
        if self.state.range.len() > maximum {
            return Err(VoiceError::Bounds);
        }
        self.state
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| VoiceError::Capacity)?;
        Ok(Flight {
            state: self.state.clone(),
        })
    }
}
pub(super) struct Flight {
    state: Arc<State>,
}
impl Flight {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.state.buffer.bytes()[self.state.range.clone()]
    }
    pub(super) fn uses_intent(&self, intent: &Arc<InitialSdpIntent>) -> bool {
        Arc::ptr_eq(&self.state.intent, intent) && self.state.buffer.has_keepalive()
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        self.state.busy.store(false, Ordering::Release);
    }
}

/// One actual mailbox request and its pending/completed graph. Shared by sdp_rpc through
/// both channel halves; a synchronous receiver cannot refund a still-active sender.
pub(super) enum Work {
    Body(Flight),
    Metadata(Copy),
}
impl Work {
    pub(super) fn uses_intent(&self, intent: &Arc<InitialSdpIntent>) -> bool {
        match self {
            Self::Body(body) => body.uses_intent(intent),
            Self::Metadata(copy) => {
                copy.uses_intent(intent)
                    && copy.has_keepalive()
                    && copy.covers_work(VOICE_SDP_RPC_METADATA_BYTES)
            }
        }
    }
    pub(super) fn bytes(&self) -> Result<&[u8], VoiceError> {
        match self {
            Self::Body(body) => Ok(body.bytes()),
            Self::Metadata(_) => Err(VoiceError::Unauthorized),
        }
    }
}
/// Actual collection owner, not a plaintext Promise or a fresh effect grant. Root's result
/// metadata can be borrowed while this same work owner keeps its original native bytes paid.
pub struct VoiceSdpCompletion<T> {
    result: Result<T, VoiceError>,
    _work: Arc<Work>,
}
impl<T> std::fmt::Debug for VoiceSdpCompletion<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSdpCompletion { .. }")
    }
}
impl<T> VoiceSdpCompletion<T> {
    pub(super) fn new(result: Result<T, VoiceError>, work: Arc<Work>) -> Self {
        Self {
            result,
            _work: work,
        }
    }
    pub fn result(&self) -> Result<&T, VoiceError> {
        self.result.as_ref().map_err(|error| *error)
    }
}
pub struct VoiceSdpPending<T>(
    pub(super) sdp_rpc::Pending<Result<VoiceSdpCompletion<T>, VoiceError>, Work>,
);
impl<T> std::fmt::Debug for VoiceSdpPending<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceSdpPending { .. }")
    }
}
impl<T> VoiceSdpPending<T> {
    pub async fn complete(self) -> Result<VoiceSdpCompletion<T>, VoiceEndpointError> {
        self.0
            .receive()
            .await
            .map_err(|_| VoiceEndpointError::Stopped)?
            .map_err(VoiceEndpointError::Protocol)
    }
}
