//! One original group-wide NEGATIVE lifetime, shared across otherwise independent channels.
//!
//! This is not a governing-state verifier: a caller cannot turn group bytes, a new view or a
//! completed drain into Profile/root/tenure authority. The V2 founder verifier binds its
//! explicit HOST-selected genesis and original source to this lifetime before issuing
//! supported channel tokens. Other governing profiles remain separately gated.
use super::*;

pub(super) struct GroupViewLife {
    pub(super) group: Arc<[u8]>,
    pub(super) gate: Arc<RwLock<()>>,
    cancelled: AtomicBool,
    pending: AtomicBool,
    closed: AtomicBool,
}
impl GroupViewLife {
    pub(super) fn live(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire)
            && !self.pending.load(Ordering::Acquire)
            && !self.gate.is_poisoned()
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    fn request_close(this: &Arc<Self>) -> ProjectionGroupCloseCompletion {
        this.cancel();
        ProjectionGroupCloseCompletion { life: this.clone() }
    }
}

/// Original mounted group's publication domain. A domain's replacements share ONE flat actual
/// barrier; neither a restored group label nor another domain with equal bytes can renew it.
/// Allocation is bounded per domain/view, but the future host must pay and bound their count.
pub struct ProjectionGroup {
    group: Arc<[u8]>,
    gate: Arc<RwLock<()>>,
    current: Option<Arc<GroupViewLife>>,
}
impl std::fmt::Debug for ProjectionGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionGroup").finish_non_exhaustive()
    }
}
impl ProjectionGroup {
    /// Only a bounded exact group identity, never a root/admission or positive capability.
    pub fn new(group: &[u8]) -> Result<Self, ChatError> {
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        Ok(Self {
            group: Arc::from(group),
            gate: Arc::new(RwLock::new(())),
            current: None,
        })
    }
    /// Immediately cancel every channel in the old group view, without enumerating them.
    /// The new view is pending until try_ready proves the original shared gate drained. A
    /// third view cannot bypass first-view callbacks through an intermediate pending view.
    pub fn begin_view(&mut self) -> ProjectionGroupView {
        let previous = self.current.as_ref().map(GroupViewLife::request_close);
        let life = Arc::new(GroupViewLife {
            group: self.group.clone(),
            gate: self.gate.clone(),
            cancelled: AtomicBool::new(false),
            pending: AtomicBool::new(previous.is_some()),
            closed: AtomicBool::new(false),
        });
        self.current = Some(life.clone());
        ProjectionGroupView { life, previous }
    }
}
impl Drop for ProjectionGroup {
    fn drop(&mut self) {
        if let Some(life) = &self.current {
            life.cancel();
        }
    }
}

/// Negative owner of one original governing-view lifetime. Drop cancels future group admission
/// in O(1), never waits for publication and never reports that admitted work has finished.
pub struct ProjectionGroupView {
    pub(super) life: Arc<GroupViewLife>,
    previous: Option<ProjectionGroupCloseCompletion>,
}
impl std::fmt::Debug for ProjectionGroupView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionGroupView")
            .finish_non_exhaustive()
    }
}
impl ProjectionGroupView {
    /// Capture this exact negative owner BEFORE waiting for an actor/native response. Looking
    /// up a newer group view after the response arrives cannot preserve original custody.
    pub fn revoker(&self) -> ProjectionGroupViewRevoker {
        ProjectionGroupViewRevoker {
            life: self.life.clone(),
        }
    }
    pub fn try_ready(&mut self) -> bool {
        if self.life.cancelled.load(Ordering::Acquire) {
            return false;
        }
        if let Some(previous) = &mut self.previous {
            if !previous.try_complete() {
                return false;
            }
            self.previous = None;
            self.life.pending.store(false, Ordering::Release);
        }
        self.life.live()
    }
    pub fn close(self) -> ProjectionGroupCloseCompletion {
        GroupViewLife::request_close(&self.life)
    }
}
impl Drop for ProjectionGroupView {
    fn drop(&mut self) {
        self.life.cancel();
    }
}

/// Clonable original cancellation custody, not an owner that keeps the view open.
#[derive(Clone)]
pub struct ProjectionGroupViewRevoker {
    life: Arc<GroupViewLife>,
}
impl std::fmt::Debug for ProjectionGroupViewRevoker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionGroupViewRevoker")
            .finish_non_exhaustive()
    }
}
impl ProjectionGroupViewRevoker {
    pub fn close(&self) -> ProjectionGroupCloseCompletion {
        GroupViewLife::request_close(&self.life)
    }
}

/// Explicit original group-close debt. Actual callbacks AND positive token/capsule mutations
/// hold its shared reader. Only this gate's exclusive probe can establish their completed drain.
#[must_use = "retain and poll the original group completion before acknowledging closure"]
pub struct ProjectionGroupCloseCompletion {
    life: Arc<GroupViewLife>,
}
impl std::fmt::Debug for ProjectionGroupCloseCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionGroupCloseCompletion")
            .finish_non_exhaustive()
    }
}
impl ProjectionGroupCloseCompletion {
    pub fn try_complete(&mut self) -> bool {
        if self.life.closed.load(Ordering::Acquire) {
            return true;
        }
        let _writer = match self.life.gate.try_write() {
            Ok(writer) => writer,
            Err(TryLockError::WouldBlock) => return false,
            Err(TryLockError::Poisoned(error)) => {
                // Poison forbids new positive work, but still carries the ACTUAL exclusive
                // guard and can therefore establish negative drain. Never clear the poison.
                self.life.cancel();
                error.into_inner()
            }
        };
        self.life.closed.store(true, Ordering::Release);
        true
    }
}

impl ChatLedger {
    /// Bind this channel to the caller's exact original group view, before reading bodies or
    /// awaiting work. Check group/liveness BEFORE replacing any existing channel view. This
    /// creates only negative custody; the future issuer must separately verify governing data.
    pub fn begin_projection_view_in(
        &mut self,
        group: &ProjectionGroupView,
    ) -> Result<ProjectionView, ChatError> {
        if group.life.group.as_ref() != self.realm.group() || !group.life.live() {
            return Err(ChatError::Scope);
        }
        Ok(self.projection.begin(Some(group.life.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisoned_group_barrier_proves_only_negative_drain_and_never_new_readiness() {
        let mut group = ProjectionGroup::new(b"poisoned-group").unwrap();
        let view = group.begin_view();
        let gate = group.gate.clone();
        assert!(std::panic::catch_unwind(move || {
            let _writer = gate.write().unwrap();
            panic!("poison actual original group writer");
        })
        .is_err());
        let mut closed = view.close();
        assert!(
            closed.try_complete(),
            "actual poisoned exclusive guard still proves drain"
        );
        let mut replacement = group.begin_view();
        assert!(!replacement.try_ready());
        assert!(!replacement.life.live());
    }

    #[test]
    fn group_identity_bound_is_checked_before_domain_allocation() {
        assert!(matches!(ProjectionGroup::new(&[]), Err(ChatError::Bound)));
        assert!(ProjectionGroup::new(&[1; MAX_GROUP_ID_BYTES]).is_ok());
        assert!(matches!(
            ProjectionGroup::new(&[1; MAX_GROUP_ID_BYTES + 1]),
            Err(ChatError::Bound)
        ));
    }
}
