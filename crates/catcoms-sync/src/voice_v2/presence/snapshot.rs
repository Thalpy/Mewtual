//! Affine custody for a native participant's proven-peer observation. This pays the bounded
//! handle vector and RPC metadata, not a new presence proof or a renderer authorization.
use super::*;

/// The existing per-group Presence limit is64, shared by all watchers/retained observations.
/// Every handle owns its original proof payment; this separate4KiB owner pays the vector,
/// command, oneshot and waiter through actual collection, including stop/caller loss.
pub struct PeerSnapshotPermit {
    scope: Scope,
    custody: Arc<Custody>,
}
struct Custody {
    waiter: std::sync::atomic::AtomicBool,
    credit: Credit,
}
/// A single actual RPC receiver may share the permit's existing payment. This cannot create
/// another snapshot or release a queued command/output by dropping its observation future.
pub struct PeerSnapshotWaiter {
    custody: Arc<Custody>,
}
impl Drop for PeerSnapshotWaiter {
    fn drop(&mut self) {
        self.custody
            .waiter
            .store(false, std::sync::atomic::Ordering::Release);
    }
}
impl PeerSnapshotPermit {
    pub const MEMORY_BYTES: usize = 4096;
    pub const MAX_PEERS: usize = 64;
    pub const SHARED_OWNER_BYTES: usize =
        std::mem::size_of::<Custody>() + 2 * std::mem::size_of::<usize>();

    /// Reserve before creating the oneshot or submitting a native Actor command. The exact
    /// live Actor must independently validate this pool/scope and the original call grant.
    pub fn new(resources: &Arc<Resources>, scope: Scope) -> Result<Self, VoiceError> {
        let credit = resources.charge(scope.group, Self::MEMORY_BYTES, ResourceKind::Retained)?;
        Ok(Self {
            scope,
            custody: Arc::new(Custody {
                waiter: std::sync::atomic::AtomicBool::new(false),
                credit,
            }),
        })
    }
    pub fn rpc_waiter(&self) -> Result<PeerSnapshotWaiter, VoiceError> {
        self.custody
            .waiter
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| VoiceError::Capacity)?;
        Ok(PeerSnapshotWaiter {
            custody: self.custody.clone(),
        })
    }
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }
    pub(crate) fn resources(&self) -> &Arc<Resources> {
        self.custody.credit.resources()
    }
    pub(crate) fn complete(
        self,
        mut peers: Result<(u64, Vec<FreshPeer>), VoiceError>,
    ) -> PeerSnapshot {
        if peers.as_ref().is_ok_and(|(_, peers)| {
            peers.len() > Self::MAX_PEERS || peers.capacity() > Self::MAX_PEERS
        }) {
            // The actual vector/proof handles drop while this SAME payment is still held.
            // Never truncate silently or expose an over-budget buffer to the native pump.
            peers = Err(VoiceError::Bounds);
        }
        PeerSnapshot {
            peers,
            permit: self,
        }
    }
}

/// Exact native observation, neither serializable nor cloneable. Borrow proof handles while
/// this original payment lives; factory admission still rechecks their current authority.
pub struct PeerSnapshot {
    peers: Result<(u64, Vec<FreshPeer>), VoiceError>,
    permit: PeerSnapshotPermit, // Drops after the actual Vec and proof handles.
}
impl PeerSnapshot {
    /// The epoch is this native Actor observation, not the announcement's possibly older
    /// epoch. It selects a candidate only: later pair publication rechecks the current epoch.
    pub fn result(&self) -> Result<(u64, &[FreshPeer]), VoiceError> {
        self.peers
            .as_ref()
            .map(|(epoch, peers)| (*epoch, peers.as_slice()))
            .map_err(|error| *error)
    }
    /// Reuse only the same metadata allowance, after the previous actual buffer is gone.
    /// This grants no fresh execution/deadline/attempt allowance and creates no work.
    pub fn return_permit(self) -> PeerSnapshotPermit {
        let Self { peers, permit } = self;
        drop(peers);
        permit
    }
}
redacted_debug!(PeerSnapshotPermit, PeerSnapshotWaiter, PeerSnapshot);
