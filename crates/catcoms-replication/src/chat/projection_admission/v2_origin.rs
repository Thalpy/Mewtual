//! Original host-source revocation for V2 effects, independent of historical semantics.
//!
//! An origin is negative custody, not a root, selected prefix, membership proof or token
//! issuer. Root supplies its actual source/scope/writer observations after independent
//! physical verification. Neither equal labels nor a newly allocated observer renews an
//! old token or a retained result. Publication must recheck this original object.
use super::*;

/// A host's original cached negative observations. Implementations must be bounded,
/// nonblocking and side-effect free: no filesystem, SDK, pool or host-lock operations
/// belong inside Core's publication readers. The host pays the concrete observer and
/// its escaping weak tails; this trait cannot authorize a positive effect.
/// A false observation is permanent for that ORIGINAL source. Core latches it rather than
/// allowing a later mount/scope observation to renew old tokens or captured plaintext.
pub trait V2OriginNegative: std::fmt::Debug + Send + Sync {
    fn is_live(&self) -> bool;
}

struct OriginLife {
    stopped: AtomicBool,
    negative: Arc<dyn V2OriginNegative>,
}

/// One sender-free negative identity retained by the certified V2 source and its results.
/// Creating this value grants no semantic or projection authority. Only the full pinned
/// V2 prefix verifier may attach it to an otherwise certified exact accepted Create.
#[derive(Clone)]
pub struct V2ProjectionOrigin {
    life: Arc<OriginLife>,
}
impl std::fmt::Debug for V2ProjectionOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2ProjectionOrigin").finish_non_exhaustive()
    }
}
impl V2ProjectionOrigin {
    /// Root must fund both this fixed allocation and its concrete observer independently
    /// of source/model/SDK fees. It must retain no strong Store, SDK or actor sender here.
    pub fn new(negative: Arc<dyn V2OriginNegative>) -> Self {
        Self {
            life: Arc::new(OriginLife {
                stopped: AtomicBool::new(false),
                negative,
            }),
        }
    }

    /// Cancel future admission immediately. This is not an acknowledgment that an already
    /// admitted publication callback drained. Actual host close must also join the original
    /// group/view publication completion before reporting that stronger result.
    pub fn cancel(&self) {
        self.life.stopped.store(true, Ordering::Release);
    }

    pub(super) fn live(&self) -> bool {
        if self.life.stopped.load(Ordering::Acquire) {
            return false;
        }
        if !self.life.negative.is_live() {
            self.life.stopped.store(true, Ordering::Release);
            return false;
        }
        // A concurrent explicit cancel/negative must not be missed after observer work.
        !self.life.stopped.load(Ordering::Acquire)
    }

    pub(super) fn same_original(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.life, &other.life)
    }

    /// Fixed logical allocation only. The erased concrete observer, ownership invoice,
    /// caller's result vectors and allocator/OS costs are separate host responsibilities.
    pub const fn retained_layout_bytes() -> usize {
        std::mem::size_of::<OriginLife>() + 2 * std::mem::size_of::<usize>()
    }
}
