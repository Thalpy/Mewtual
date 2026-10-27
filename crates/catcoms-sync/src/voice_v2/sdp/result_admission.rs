//! A borrowed final Result tail. All signatures, token plans, delivery candidates and
//! retired owners remain in the enclosing actual Actor call, outside native guards.
use super::VoiceError;
use crate::voice_v2::crypto_actions::CryptoAction;
use std::sync::Arc;

/// Exact selected output domain, in addition to the actual Action allocation. A closed
/// AEAD failure is never interchangeable with a successful byte result, even empty bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdpResultIdentity {
    Bytes([u8; 32]),
    AuthenticationFailed,
}

/// One synchronous final Result opportunity from the exclusively borrowed Actor.
/// Clock, hashing, signing and all fallible preparation finish before this loan. Native
/// may try its UI/Coordinator/route/index/row fences and call commit there. No owned
/// preparation is moved into this value, so refusal or unwind cannot destroy it beneath
/// those fences. Native UI custody must be lexical: release it before returning the hook.
///
/// A terminal retry may succeed without lending this value at all. Neither this loan nor
/// the returned receipt settles SDK work, acknowledges a result or permits fresh RTC use.
pub struct SdpResultAdmission<'a> {
    action: &'a Arc<CryptoAction>,
    identity: SdpResultIdentity,
    wall: u64,
    mono: u64,
    commit: &'a mut dyn FnMut() -> Result<(), VoiceError>,
}
impl<'a> SdpResultAdmission<'a> {
    pub(crate) fn new(
        action: &'a Arc<CryptoAction>,
        identity: SdpResultIdentity,
        wall: u64,
        mono: u64,
        commit: &'a mut dyn FnMut() -> Result<(), VoiceError>,
    ) -> Self {
        Self {
            action,
            identity,
            wall,
            mono,
            commit,
        }
    }
    /// Pure provenance before any effect. Equal labels or another same-intent Action
    /// cannot replace the actual registered allocation and its exact selected output.
    pub fn matches(&self, action: &Arc<CryptoAction>, identity: SdpResultIdentity) -> bool {
        Arc::ptr_eq(self.action, action) && self.identity == identity
    }
    /// Captured original Actor samples; this accessor invokes no Clock callback.
    pub fn observed_time(&self) -> (u64, u64) {
        (self.wall, self.mono)
    }
    /// Consume the one final tail. Only try-locks, pure current-owner observations and
    /// the original prepared infallible publication may occur; no await or Root reentry.
    pub fn commit(self) -> Result<(), VoiceError> {
        (self.commit)()
    }
}
impl std::fmt::Debug for SdpResultAdmission<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpResultAdmission { .. }")
    }
}

/// Root wrapper storage which can exist without an admitted engine, or survive its
/// preparation guard: current ChannelSync context/room/member snapshot, returned result,
/// historical observation, immutable input/PC/pair references and closure bookkeeping.
/// App composes this ADDITIVELY with its still-live sender/channel graph. Original
/// Action/delivery/history allocations and payload backing retain their existing fees.
pub const fn sdp_result_rpc_frame_bytes() -> usize {
    std::mem::size_of::<crate::voice_v2::Context>()
        + std::mem::size_of::<crate::voice_v2::helper::RoomAuthority>()
        + std::mem::size_of::<crate::voice_v2::packets::Authority<'static>>()
        + std::mem::size_of::<[Option<catcoms_crypto::DeviceId>; 4]>()
        + 12 * std::mem::size_of::<Arc<()>>() // actual call/watch/clock/input/PC/pair handles
        + std::mem::size_of::<crate::voice_v2::crypto_actions::ObservedSdpResult<'static>>()
        + std::mem::size_of::<super::SdpOpenResult>()
        + 2 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 128 // current selectors, return discriminants and historical callback borrows
        + sdp_result_hash_frame_bytes()
}

/// Actual fixed SHA256/digest wrappers used by bounded output-tag preparation,
/// including early historical recovery before any engine preparation loan exists.
/// Sequential hash calls reuse this bound; they never own two digest graphs together.
pub const fn sdp_result_hash_frame_bytes() -> usize {
    std::mem::size_of::<sha2::Sha256>()
        + std::mem::size_of::<sha2::digest::Output<sha2::Sha256>>()
        + 2 * std::mem::size_of::<[u8; 32]>()
        + 128 // digest trait/helper state, fixed finalization and returned-tag wrappers
}

/// Pending-only Root work, independently backed by the ORIGINAL admitted engine.
/// ResultPreparation is acquired before this work and outlives every owned scratch /
/// signature / token-plan destructor. The shared RPC-paid outer frame is above.
///
/// The four wire-sized terms conservatively cover the returned signed ciphertext clone
/// and the largest unsigned/header/child/joined/outer encoding stage (including Vec
/// growth). The extra2KiB covers fixed encoding headers/growth at small inputs. This is
/// Root's current preparation peak, not proof that a future browser SDK fits alongside it.
/// Native/browser execution must retain and compose its separately paid actual buffers.
pub fn sdp_result_preparation_bytes(aad: usize, ciphertext: usize) -> Option<usize> {
    use crate::voice_v2::{crypto_actions as actions, packets, runtime};
    let shared = std::mem::size_of::<Option<actions::TerminalResult>>()
        + std::mem::size_of::<Option<Result<actions::CryptoResultReceipt, VoiceError>>>()
        + std::mem::size_of::<Option<actions::ObservedSdpResult<'static>>>()
        + std::mem::size_of::<Option<runtime::PreparedLocalOrigin<'static>>>()
        + std::mem::size_of::<actions::ResultPreparation<'static>>()
        + std::mem::size_of::<SdpResultAdmission<'static>>()
        + 4 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 192; // borrowed closure captures, final samples and try-lock result bookkeeping
    let seal = std::mem::size_of::<Option<packets::PreparedSealedPublication<'static>>>()
        + std::mem::size_of::<Option<crate::voice_v2::SignedObject>>();
    let open = std::mem::size_of::<Option<packets::ValidatedInbound>>()
        + std::mem::size_of::<Option<(packets::CommittedPacket, packets::ConsumedIncoming)>>()
        + std::mem::size_of::<Option<Arc<super::SdpDelivery>>>();
    shared
        .checked_add(seal.max(open))?
        .checked_add(aad.checked_add(ciphertext)?.checked_mul(4)?)?
        .checked_add(2048)
}
