//! Affine payment for one native action snapshot RPC and its actual output. The caller
//! cannot release the backing-Vec payment by abandoning an admitted Actor request.
use super::*;

/// Fixed complete snapshot contribution:32 Arc slots, owner/command/oneshot/output metadata.
/// The Actor crate pins its actual wrapper inventory against this allowance. No Clone,
/// serialization or extraction of the paid Vec exists; original Actions pay their own data.
pub struct CryptoActionSnapshotPermit {
    scope: Scope,
    custody: Arc<SnapshotCustody>,
}
struct SnapshotCustody {
    waiter: std::sync::atomic::AtomicBool,
    // This payment follows BOTH the RPC receiver and the command/output. Actor stop can
    // drop the latter first; caller loss can drop the former first. Neither is drainage.
    credit: Credit,
}
/// Only one original Actor RPC waiter may share a permit's payment. This is not a second
/// permit, a cloneable work grant, or a way to extract actions from the paid output.
pub struct CryptoActionSnapshotWaiter {
    custody: Arc<SnapshotCustody>,
}
impl Drop for CryptoActionSnapshotWaiter {
    fn drop(&mut self) {
        self.custody
            .waiter
            .store(false, std::sync::atomic::Ordering::Release);
    }
}
impl CryptoActionSnapshotPermit {
    pub const MEMORY_BYTES: usize = 4096;
    /// Concrete shared owner including the Arc control block, for downstream layout guards.
    pub const SHARED_OWNER_BYTES: usize =
        std::mem::size_of::<SnapshotCustody>() + 2 * std::mem::size_of::<usize>();
    pub fn new(resources: &Arc<Resources>, scope: Scope) -> Result<Self, VoiceError> {
        let credit = resources.charge(scope.group, Self::MEMORY_BYTES, ResourceKind::Retained)?;
        Ok(Self {
            scope,
            custody: Arc::new(SnapshotCustody {
                waiter: std::sync::atomic::AtomicBool::new(false),
                credit,
            }),
        })
    }
    /// Trusted Actor adapter calls this BEFORE creating the oneshot or enqueueing work.
    /// Keep it until the actual receiver is collected, including Actor-stop/error paths.
    /// Affine permit ownership and this gate bound the complete graph to one RPC waiter.
    pub fn rpc_waiter(&self) -> Result<CryptoActionSnapshotWaiter, VoiceError> {
        self.custody
            .waiter
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| VoiceError::Capacity)?;
        Ok(CryptoActionSnapshotWaiter {
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
        mut actions: Result<Vec<Arc<CryptoAction>>, VoiceError>,
    ) -> CryptoActionSnapshot {
        if actions
            .as_ref()
            .is_ok_and(|actions| actions.len() > 4 || actions.capacity() > ACTIONS)
        {
            // Drop any actual buffer before changing the observation; the SAME original
            // permit still funds the fixed refusal output until physical collection.
            actions = Err(VoiceError::Bounds);
        }
        CryptoActionSnapshot {
            actions,
            permit: self,
        }
    }
}

/// Non-dequeueing original-action observation, not an engine or RTC grant. Its affine
/// original payment follows the buffer through a late completed reply and actual collection.
pub struct CryptoActionSnapshot {
    actions: Result<Vec<Arc<CryptoAction>>, VoiceError>,
    // MUST drop after the actual Vec and its original action handles.
    permit: CryptoActionSnapshotPermit,
}
impl CryptoActionSnapshot {
    pub fn result(&self) -> Result<&[Arc<CryptoAction>], VoiceError> {
        self.actions
            .as_ref()
            .map(Vec::as_slice)
            .map_err(|error| *error)
    }
    /// Reuse this SAME metadata payment only AFTER its actual Vec and action snapshot
    /// owners are destroyed. No execution/attempt/deadline allowance is returned or renewed.
    pub fn return_permit(self) -> CryptoActionSnapshotPermit {
        let Self { actions, permit } = self;
        drop(actions);
        permit
    }
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.actions.as_ref().map_or(0, Vec::capacity)
    }
}
redacted_debug!(
    CryptoActionSnapshotPermit,
    CryptoActionSnapshotWaiter,
    CryptoActionSnapshot
);
