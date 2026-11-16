//! Ownership transfer only, not IPC or authority. The native caller prepays the actual buffer
//! and the fixed wrapper/actor/reply metadata, then pins that owner once outside all guards.
//! Bytes::from_owner caches its slice at construction; Root never calls an arbitrary AsRef
//! while holding protocol locks. Do not add public Clone, serialization, mutable access,
//! into_vec/BytesMut or a constructor from a renderer descriptor here.
use super::{CryptoResultReceipt, VoiceError};
use bytes::Bytes;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Fixed actor/result metadata contribution to reserve in the ORIGINAL native data Phase
/// before constructing a body. Payload allocation capacity is separately charged by its
/// actual owner. This includes the pinned owner's Box, envelope, oneshot and one completed
/// reply, not an unbounded producer queue, detached task, SDK memory or allocator overhead.
pub const VOICE_CONFIRMATION_BODY_METADATA_BYTES: usize = 4096;
const MAX_OWNER_BYTES: usize = 128;
const MAX_OWNER_ALIGN: usize = 64;
const MAX_RESULT_BYTES: usize = 528;

/// A bounded native-owned confirmation result. No byte buffer is copied on mailbox handoff.
/// Construct only from a trusted paid owner whose Drop releases its actual copy custody;
/// include the fixed metadata contribution above in that same owner's lifetime. There is no
/// wire/renderer constructor, and merely possessing this wrapper grants no Root authority.
pub struct VoiceConfirmationBody {
    state: Arc<BodyState>,
}
struct BodyState {
    busy: AtomicBool,
    // Actual owner/payment drops after the gate, including when the original caller is gone.
    bytes: Bytes,
}
impl std::fmt::Debug for VoiceConfirmationBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceConfirmationBody { .. }")
    }
}
impl VoiceConfirmationBody {
    /// Pin the actual paid owner and call its AsRef exactly once, HERE, outside native/Root
    /// guards. A refusal drops that owner here too. Owner inline layout is capped before any
    /// Box allocation; its referenced payload may not exceed the existing confirmation bound.
    /// A panicking AsRef is cleaned up by the pinned bytes implementation's ordinary Drop.
    pub fn from_owner<T: AsRef<[u8]> + Send + 'static>(owner: T) -> Result<Self, VoiceError> {
        if std::mem::size_of::<T>() > MAX_OWNER_BYTES || std::mem::align_of::<T>() > MAX_OWNER_ALIGN
        {
            return Err(VoiceError::Bounds);
        }
        let bytes = Bytes::from_owner(owner);
        if bytes.len() > MAX_RESULT_BYTES {
            return Err(VoiceError::Bounds);
        }
        Ok(Self {
            state: Arc::new(BodyState {
                busy: AtomicBool::new(false),
                bytes,
            }),
        })
    }
    /// One actor request OR completed/uncollected output per prepaid body. An unlimited set
    /// of concurrent callers may not reuse one4KiB contribution for many mailbox envelopes.
    /// Refusal is proven local before any request exists, preserving the exact body for retry.
    pub(super) fn claim(&self) -> Result<Flight, VoiceError> {
        self.state
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| VoiceError::Capacity)?;
        Ok(Flight {
            body: Self {
                state: self.state.clone(),
            },
        })
    }
    #[cfg(test)]
    fn bytes(&self) -> &[u8] {
        &self.state.bytes
    }
}
pub(super) struct Flight {
    body: VoiceConfirmationBody,
}
impl Flight {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.body.state.bytes
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        // Work and completion were collected before this guard can retire. Reopening this
        // gate is not a Phase/drain statement: original Body/copy owners still retain Bytes.
        self.body.state.busy.store(false, Ordering::Release);
    }
}

/// A completed actor reply still owns its actual body/copy payment. Result readiness is not
/// physical collection, and the returned receipt is never evidence of native Phase drainage.
pub struct VoiceConfirmationCompletion {
    result: Result<CryptoResultReceipt, VoiceError>,
    _body: Flight,
}
impl std::fmt::Debug for VoiceConfirmationCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceConfirmationCompletion { .. }")
    }
}
impl VoiceConfirmationCompletion {
    pub(super) fn new(result: Result<CryptoResultReceipt, VoiceError>, body: Flight) -> Self {
        Self {
            result,
            _body: body,
        }
    }
    /// Fixed historical outcome only; keep this Completion until the physical consumer has
    /// collected it. Dropping it does not establish that another original body owner drained.
    pub fn result(&self) -> Result<CryptoResultReceipt, VoiceError> {
        self.result.clone()
    }
}

#[cfg(test)]
pub(super) fn metadata_layout(caller_future: usize) -> usize {
    // bytes1.12 Owned<T> is repr(C), including padding before/after the owner's value.
    let owner_box = std::alloc::Layout::new::<std::sync::atomic::AtomicUsize>()
        .extend(std::alloc::Layout::from_size_align(MAX_OWNER_BYTES, MAX_OWNER_ALIGN).unwrap())
        .unwrap()
        .0
        .pad_to_align()
        .size();
    owner_box
        + caller_future
        + std::mem::size_of::<BodyState>()
        + 2 * std::mem::size_of::<usize>()
        + 2 * std::mem::size_of::<Flight>()
        + std::mem::size_of::<crate::AppCommand>()
        + std::mem::size_of::<VoiceConfirmationBody>()
        + std::mem::size_of::<VoiceConfirmationCompletion>()
        + std::mem::size_of::<
            tokio::sync::oneshot::Receiver<Result<VoiceConfirmationCompletion, VoiceError>>,
        >()
        + 1024 // pinned Tokio oneshot allocation/envelope bookkeeping; no detached task
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    struct Owner {
        bytes: Vec<u8>,
        calls: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }
    impl AsRef<[u8]> for Owner {
        fn as_ref(&self) -> &[u8] {
            self.calls.fetch_add(1, Ordering::SeqCst);
            &self.bytes
        }
    }
    impl Drop for Owner {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn owner(bytes: usize) -> (Owner, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        (
            Owner {
                bytes: vec![7; bytes],
                calls: calls.clone(),
                drops: drops.clone(),
            },
            calls,
            drops,
        )
    }
    #[test]
    fn actor_and_completed_reply_retain_the_same_bytes_without_another_owner_callback() {
        let (owner, calls, drops) = owner(MAX_RESULT_BYTES);
        let pointer = owner.bytes.as_ptr();
        let body = VoiceConfirmationBody::from_owner(owner).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(body.bytes().as_ptr(), pointer);
        let command = body.claim().unwrap();
        drop(body);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(command.bytes().as_ptr(), pointer);
        let completion = VoiceConfirmationCompletion::new(Err(VoiceError::Capacity), command);
        assert_eq!(completion.result(), Err(VoiceError::Capacity));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(completion);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn oversize_result_refuses_and_releases_the_original_owner_once() {
        let (owner, calls, drops) = owner(MAX_RESULT_BYTES + 1);
        assert_eq!(
            VoiceConfirmationBody::from_owner(owner).err(),
            Some(VoiceError::Bounds)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn one_body_cannot_fund_multiple_queued_or_completed_actor_owners() {
        let (owner, calls, drops) = owner(32);
        let body = VoiceConfirmationBody::from_owner(owner).unwrap();
        let command = body.claim().unwrap();
        assert!(matches!(body.claim(), Err(VoiceError::Capacity)));
        let completion = VoiceConfirmationCompletion::new(Err(VoiceError::Capacity), command);
        assert!(matches!(body.claim(), Err(VoiceError::Capacity)));
        drop(completion);
        let retry = body.claim().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(body);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(retry);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn oversize_owner_layout_refuses_before_its_callback_or_box_is_created() {
        struct Large([u8; MAX_OWNER_BYTES + 1]);
        impl AsRef<[u8]> for Large {
            fn as_ref(&self) -> &[u8] {
                panic!("oversize layout must refuse before AsRef")
            }
        }
        let large = Large([0; MAX_OWNER_BYTES + 1]);
        assert_eq!(large.0.len(), MAX_OWNER_BYTES + 1);
        assert_eq!(
            VoiceConfirmationBody::from_owner(large).err(),
            Some(VoiceError::Bounds)
        );
    }
    #[test]
    fn concrete_actor_body_and_reply_metadata_fit_the_fixed_phase_contribution() {
        let bytes = metadata_layout(0);
        assert!(
            bytes <= VOICE_CONFIRMATION_BODY_METADATA_BYTES,
            "actor body metadata requires {bytes}"
        );
    }
    #[test]
    fn excessive_alignment_alone_refuses_before_the_owner_callback() {
        #[repr(align(128))]
        struct Aligned([u8; 1]);
        impl AsRef<[u8]> for Aligned {
            fn as_ref(&self) -> &[u8] {
                panic!("alignment must refuse before AsRef")
            }
        }
        let owner = Aligned([0]);
        assert_eq!(owner.0, [0]);
        assert_eq!(std::mem::size_of::<Aligned>(), MAX_OWNER_BYTES);
        assert_eq!(
            VoiceConfirmationBody::from_owner(owner).err(),
            Some(VoiceError::Bounds)
        );
    }
    #[test]
    fn a_panicking_initial_owner_callback_drops_the_actual_owner_exactly_once() {
        struct Panics(Arc<AtomicUsize>);
        impl AsRef<[u8]> for Panics {
            fn as_ref(&self) -> &[u8] {
                panic!("injected owner callback")
            }
        }
        impl Drop for Panics {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = Panics(drops.clone());
        assert!(std::panic::catch_unwind(|| VoiceConfirmationBody::from_owner(owner)).is_err());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
