//! Default-closed current projection custody, separate from monotone historical semantics.
//!
//! Public lifetime owners create only negative custody. The bounded V2 founder verifier can
//! issue exact accepted Creates after full replay from an explicitly HOST-selected genesis;
//! the separate general verifier issues ordinary OPEN effects from actual selected
//! Admission/dependency provenance. Raw labels, snapshots and semantic Valid cannot
//! construct an admission token. Native root
//! provenance, selected storage receipts and UI authorization remain the actual host's job.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, TryLockError};

mod group;
mod guarded;
mod v2_founder;
mod v2_general;
mod v2_origin;
use group::GroupViewLife;
pub use group::{
    ProjectionGroup, ProjectionGroupCloseCompletion, ProjectionGroupView,
    ProjectionGroupViewRevoker,
};
pub use guarded::{GuardedCausalHeads, GuardedMessageProjection, GuardedProjection};
pub use v2_founder::{
    FounderGenesisAnchorData, V2FounderExpectation, V2FounderPrefixVerifier, V2FounderReplayCursor,
    V2HostSelectionExpectation, V2RetiredAppendData, V2VerifiedAppendCandidate,
};
pub use v2_general::{
    OriginalV2OrdinarySigningPlanObservation, V2AdmissionCloseCompletion, V2AuthenticatedAdmission,
    V2FrozenAdmission, V2GeneralAcceptanceSource, V2GeneralCandidate, V2GeneralExpectation,
    V2GeneralPrefixVerifier, V2GeneralRetiredData, V2KnownGoverningParent, V2OrdinaryDependency,
    V2OrdinaryKeyCall, V2OrdinarySigningGuard, V2OrdinarySigningPlan,
};
pub use v2_origin::{V2OriginNegative, V2ProjectionOrigin};

const OPEN: u8 = 0;
const PREPARING: u8 = 1;
const PENDING: u8 = 2;
const COMPLETE: u8 = 3;
const UNCOUNTED: u8 = 4;

/// Current effect certainty. Provisional effects observe their ORIGINAL admission's OPEN
/// lifetime; Proven effects do not become Held merely because that admission was removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionCertainty {
    Proven,
    Provisional,
}

/// One actual ledger's shared publication barrier. ALL replacement views share it, so a third
/// view cannot pass an older reader by replacing an intermediate still-pending view. Neither
/// the barrier nor a view identity is reconstructed from a realm label after restore.
pub(super) struct ProjectionState {
    gate: Arc<RwLock<()>>,
    current: Option<Arc<ViewLife>>,
    realm: RealmId,
}
impl ProjectionState {
    pub(super) fn new(realm: RealmId) -> Self {
        Self {
            gate: Arc::new(RwLock::new(())),
            current: None,
            realm,
        }
    }
    fn begin(&mut self, group: Option<Arc<GroupViewLife>>) -> ProjectionView {
        let previous = self.current.as_ref().map(ViewLife::request_close);
        let life = Arc::new(ViewLife {
            gate: self.gate.clone(),
            group,
            realm: self.realm,
            revision: AtomicU64::new(1),
            cancelled: AtomicBool::new(false),
            pending: AtomicUsize::new(if previous.is_some() { 1 } else { 0 }),
            closed: AtomicBool::new(false),
            #[cfg(test)]
            mutation_hook: std::sync::Mutex::new(None),
        });
        self.current = Some(life.clone());
        ProjectionView { life, previous }
    }
    pub(super) fn cancel(&self) {
        if let Some(view) = &self.current {
            view.cancel();
        }
    }
    pub(super) fn lease(&self) -> Result<ProjectionLease, ChatError> {
        let view = self.current.as_ref().ok_or(ChatError::Scope)?;
        ProjectionLease::capture(view)
    }
}
impl Drop for ProjectionState {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct ViewLife {
    gate: Arc<RwLock<()>>,
    // Immutable ORIGINAL group identity; never looked up anew after body/actor work.
    group: Option<Arc<GroupViewLife>>,
    realm: RealmId,
    revision: AtomicU64,
    cancelled: AtomicBool,
    pending: AtomicUsize,
    closed: AtomicBool,
    #[cfg(test)]
    mutation_hook: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl ViewLife {
    fn bump(&self) -> Result<(), ChatError> {
        if self
            .revision
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .is_err()
        {
            self.cancelled.store(true, Ordering::Release);
            return Err(ChatError::Capacity);
        }
        Ok(())
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        // Exhaustion is a permanent negative fence, never a wrapping generation.
        let _ = self.bump();
    }
    fn live(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire)
            && !self.gate.is_poisoned()
            && self.pending.load(Ordering::Acquire) == 0
            && self.group.as_ref().is_none_or(|group| group.live())
    }
    fn request_close(this: &Arc<Self>) -> ProjectionCloseCompletion {
        this.cancel();
        ProjectionCloseCompletion {
            target: CloseTarget::View(this.clone()),
        }
    }
}

/// Negative owner of one original view. Drop cancels future admission WITHOUT acquiring the
/// publication barrier. In particular a synchronous publication callback may own/drop this
/// value, including during unwind; waiting in Drop would deadlock behind its own reader.
pub struct ProjectionView {
    life: Arc<ViewLife>,
    previous: Option<ProjectionCloseCompletion>,
}
impl std::fmt::Debug for ProjectionView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionView").finish_non_exhaustive()
    }
}
impl ProjectionView {
    /// Retain original negative custody before waiting for outer actor/UI locks. This handle
    /// never keeps the view open and cannot select a replacement view with matching labels.
    pub fn revoker(&self) -> ProjectionViewRevoker {
        ProjectionViewRevoker {
            life: self.life.clone(),
        }
    }
    /// Poll replacement drain without blocking an actor. New publication remains pending
    /// until the old view's actual shared writer probe succeeds. A cancelled view never opens.
    pub fn try_ready(&mut self) -> bool {
        if self.life.cancelled.load(Ordering::Acquire) {
            return false;
        }
        if let Some(previous) = &mut self.previous {
            if !previous.try_complete() {
                return false;
            }
            self.previous = None;
            if self
                .life
                .pending
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                .is_err()
            {
                self.life.cancel();
                return false;
            }
        }
        self.life.live()
    }
    /// Begin explicit view closure. A returned completion is not an acknowledgement that an
    /// already admitted publication has finished; retain/poll it before reporting Closed.
    pub fn close(self) -> ProjectionCloseCompletion {
        ViewLife::request_close(&self.life)
    }

    /// Create ONLY original negative OPEN custody. Public raw labels cannot produce a token.
    /// A2 with identical labels cannot renew A1 because its life has a different Arc identity.
    pub fn open_admission(
        &self,
        profile: owner_facts::OwnerFactId,
        admission: owner_facts::OwnerFactId,
        subject: [u8; 32],
    ) -> Result<ProjectionOpenAdmission, ChatError> {
        if !self.life.live() {
            return Err(ChatError::Scope);
        }
        Ok(ProjectionOpenAdmission {
            life: Arc::new(OpenLife {
                view: self.life.clone(),
                profile,
                admission,
                subject,
                phase: AtomicU8::new(OPEN),
                completions: AtomicUsize::new(0),
                #[cfg(test)]
                preparing_hook: std::sync::Mutex::new(None),
            }),
        })
    }
}
impl Drop for ProjectionView {
    fn drop(&mut self) {
        self.life.cancel();
    }
}

#[derive(Clone)]
pub struct ProjectionViewRevoker {
    life: Arc<ViewLife>,
}
impl std::fmt::Debug for ProjectionViewRevoker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionViewRevoker")
            .finish_non_exhaustive()
    }
}
impl ProjectionViewRevoker {
    pub fn close(&self) -> ProjectionCloseCompletion {
        ViewLife::request_close(&self.life)
    }
}

struct OpenLife {
    view: Arc<ViewLife>,
    profile: owner_facts::OwnerFactId,
    admission: owner_facts::OwnerFactId,
    subject: [u8; 32],
    phase: AtomicU8,
    completions: AtomicUsize,
    #[cfg(test)]
    preparing_hook: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl OpenLife {
    fn live(&self) -> bool {
        self.phase.load(Ordering::Acquire) == OPEN
    }
    /// Called only after the SAME original view's opaque completed barrier or an actual
    /// exclusive writer probe. Never infer completion from another view/current labels.
    fn complete_after_drain(&self) -> bool {
        let phase = self.phase.load(Ordering::Acquire);
        if phase == PREPARING || phase == OPEN {
            return false;
        }
        if phase == COMPLETE {
            return true;
        }
        if self
            .phase
            .compare_exchange(phase, COMPLETE, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            if phase == PENDING
                && self
                    .view
                    .pending
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                    .is_err()
            {
                self.view.cancel();
            }
            true
        } else {
            self.phase.load(Ordering::Acquire) == COMPLETE
        }
    }
    fn request_close(this: &Arc<Self>) -> ProjectionCloseCompletion {
        if this
            .completions
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .is_err()
        {
            this.view.cancel();
            // No counter wrapped; this handle cannot decrement an unregistered completion.
            return ProjectionCloseCompletion {
                target: CloseTarget::View(this.view.clone()),
            };
        }
        if this
            .phase
            .compare_exchange(OPEN, PREPARING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            // Once OPEN has closed, an unwind MUST cancel the whole view rather than abandon
            // an unregistered barrier. No callback, allocation, I/O or await belongs here.
            struct Preparing<'a> {
                scope: &'a OpenLife,
                charged: bool,
                armed: bool,
            }
            impl Drop for Preparing<'_> {
                fn drop(&mut self) {
                    if self.armed {
                        // Publish a retryable original negative phase, not a forever
                        // PREPARING latch. This unwound close returned no completion handle.
                        let _ = self.scope.phase.compare_exchange(
                            PREPARING,
                            if self.charged { PENDING } else { UNCOUNTED },
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        let _ = self.scope.completions.fetch_update(
                            Ordering::AcqRel,
                            Ordering::Acquire,
                            |n| n.checked_sub(1),
                        );
                        self.scope.view.cancel();
                    }
                }
            }
            let mut preparing = Preparing {
                scope: this,
                charged: false,
                armed: true,
            };
            #[cfg(test)]
            {
                let hook = this.preparing_hook.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            let counted = this
                .view
                .pending
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
                .is_ok();
            preparing.charged = counted;
            if !counted {
                this.view.cancel();
            }
            let _ = this.view.bump();
            this.phase
                .store(if counted { PENDING } else { UNCOUNTED }, Ordering::Release);
            preparing.armed = false;
        }
        ProjectionCloseCompletion {
            target: CloseTarget::Admission(this.clone()),
        }
    }
}

/// Negative original-admission owner. Retained revokers/tokens never keep it OPEN after Drop.
pub struct ProjectionOpenAdmission {
    life: Arc<OpenLife>,
}
impl std::fmt::Debug for ProjectionOpenAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionOpenAdmission")
            .finish_non_exhaustive()
    }
}
impl ProjectionOpenAdmission {
    pub fn revoker(&self) -> ProjectionAdmissionRevoker {
        ProjectionAdmissionRevoker {
            life: self.life.clone(),
        }
    }
    pub fn close(self) -> ProjectionCloseCompletion {
        OpenLife::request_close(&self.life)
    }
}
impl Drop for ProjectionOpenAdmission {
    fn drop(&mut self) {
        if self
            .life
            .phase
            .compare_exchange(OPEN, UNCOUNTED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            // Abandonment promises no drain completion. Conservatively close the whole view.
            self.life.view.cancel();
        }
    }
}
#[derive(Clone)]
pub struct ProjectionAdmissionRevoker {
    life: Arc<OpenLife>,
}
impl std::fmt::Debug for ProjectionAdmissionRevoker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionAdmissionRevoker")
            .finish_non_exhaustive()
    }
}
impl ProjectionAdmissionRevoker {
    pub fn close(&self) -> ProjectionCloseCompletion {
        OpenLife::request_close(&self.life)
    }
}

enum CloseTarget {
    View(Arc<ViewLife>),
    Admission(Arc<OpenLife>),
}
/// An explicit nonblocking drain obligation. Drop never waits. Abandoning the last pending
/// admission completion cancels its whole view; it cannot silently reopen fresh publications.
#[must_use = "retain and poll this completion before acknowledging projection closure"]
pub struct ProjectionCloseCompletion {
    target: CloseTarget,
}
impl std::fmt::Debug for ProjectionCloseCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionCloseCompletion")
            .finish_non_exhaustive()
    }
}
impl ProjectionCloseCompletion {
    pub fn try_complete(&mut self) -> bool {
        let view = match &self.target {
            CloseTarget::View(view) if view.closed.load(Ordering::Acquire) => return true,
            CloseTarget::View(view) => view,
            CloseTarget::Admission(scope) => match scope.phase.load(Ordering::Acquire) {
                COMPLETE => return true,
                PREPARING | OPEN => return false,
                _ if scope.view.closed.load(Ordering::Acquire) => {
                    return scope.complete_after_drain()
                }
                _ => &scope.view,
            },
        };
        let _writer = match view.gate.try_write() {
            Ok(writer) => writer,
            Err(TryLockError::WouldBlock) => return false,
            Err(TryLockError::Poisoned(error)) => {
                view.cancel();
                // The poisoned result still contains the actual exclusive barrier guard.
                error.into_inner()
            }
        };
        match &self.target {
            CloseTarget::View(view) => view.closed.store(true, Ordering::Release),
            CloseTarget::Admission(scope) => {
                return scope.complete_after_drain();
            }
        }
        true
    }
}
impl Drop for ProjectionCloseCompletion {
    fn drop(&mut self) {
        if let CloseTarget::Admission(scope) = &self.target {
            if scope
                .completions
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                == Ok(1)
                && scope.phase.load(Ordering::Acquire) != COMPLETE
            {
                scope.view.cancel();
            }
        }
    }
}

pub(super) struct ProjectionLease {
    view: Arc<ViewLife>,
    revision: u64,
    open: Vec<Arc<OpenLife>>,
    // Original V2 sources survive result computation and are rechecked under the
    // actual publication readers. Watching only the channel view misses independent
    // Source/scope/writer revocation while that view object is still alive.
    v2_origins: Vec<V2ProjectionOrigin>,
}
impl ProjectionLease {
    fn capture(view: &Arc<ViewLife>) -> Result<Self, ChatError> {
        let lease = Self {
            view: view.clone(),
            revision: view.revision.load(Ordering::Acquire),
            open: Vec::new(),
            v2_origins: Vec::new(),
        };
        lease.validate()?;
        Ok(lease)
    }
    pub(super) fn validate(&self) -> Result<(), ChatError> {
        if !self.view.live()
            || self.view.revision.load(Ordering::Acquire) != self.revision
            || self.open.iter().any(|scope| !scope.live())
            || self.v2_origins.iter().any(|origin| !origin.live())
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    pub(super) fn observe(&mut self, token: &ProjectionAdmissionToken) {
        if let Some(scope) = &token.open {
            self.open.push(scope.clone());
        }
        if let Some(origin) = token.support.v2_origin() {
            if !self.v2_origins.iter().any(|old| old.same_original(origin)) {
                self.v2_origins.push(origin.clone());
            }
        }
    }
    pub(super) fn publish<T>(&self, callback: impl FnOnce() -> T) -> Result<T, ChatError> {
        // Refuse already-cancelled callers before taking either reader. Repeated obsolete
        // replies must not compete with the explicit negative drain probe. Recheck INSIDE
        // both guards too: cancellation may race this preliminary observation.
        self.validate()?;
        // Fixed native commit order: outer UI -> registry -> original group -> ledger. Never
        // acquire the group after the ledger, and retain both readers through actual reply.
        let _group_reader = self
            .view
            .group
            .as_ref()
            .map(|group| group.gate.try_read())
            .transpose()
            .map_err(|_| ChatError::Scope)?;
        let _reader = self.view.gate.try_read().map_err(|_| ChatError::Scope)?;
        self.validate()?;
        // No post-callback refusal: this reply was already admitted. Explicit completion must
        // wait for this guard even if the callback cancels/drops its original view or ledger.
        Ok(callback())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActionBinding {
    realm: RealmId,
    action: ActionId,
    create: ActionId,
    signer: [u8; 32],
    grant: GrantId,
    basis: BasisId,
}
impl ActionBinding {
    fn of(id: ActionId, action: &SignedAction) -> Self {
        Self {
            realm: action.spec().realm.id(),
            action: id,
            create: action.spec().body.create().unwrap_or(id),
            signer: *action.signer(),
            grant: action.spec().grant,
            basis: action.spec().basis,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvidenceContext {
    // The old positive projection issuer exists only in private signed fixtures. Keep
    // that test capability absent from production rather than suppressing dead-code lint.
    #[cfg(test)]
    LegacyV1 {
        profile: owner_facts::OwnerFactId,
        admission: owner_facts::OwnerFactId,
    },
    V2Ordinary {
        profile: owner_facts::OwnerFactId,
        admission: owner_facts::OwnerFactId,
        tenure: owner_header::OwnerTenureId,
    },
    V2Founder {
        profile: owner_facts::OwnerFactId,
        admission: owner_facts::OwnerFactId,
        tenure: owner_header::OwnerTenureId,
    },
}
impl EvidenceContext {
    fn profile(self) -> owner_facts::OwnerFactId {
        match self {
            #[cfg(test)]
            Self::LegacyV1 { profile, .. } => profile,
            Self::V2Founder { profile, .. } | Self::V2Ordinary { profile, .. } => profile,
        }
    }
    fn admission(self) -> owner_facts::OwnerFactId {
        match self {
            #[cfg(test)]
            Self::LegacyV1 { admission, .. } => admission,
            Self::V2Founder { admission, .. } | Self::V2Ordinary { admission, .. } => admission,
        }
    }
    fn is_legacy(self) -> bool {
        match self {
            #[cfg(test)]
            Self::LegacyV1 { .. } => true,
            Self::V2Founder { .. } | Self::V2Ordinary { .. } => false,
        }
    }
}

/// Versioned support prevents v2 admission labels or acceptance records from being
/// interpreted as v1 fact proofs. V2's original negative is retained by result leases.
#[derive(Clone)]
enum ProjectionSupport {
    #[cfg(test)]
    Legacy {
        proof: owner_facts::OwnerFactId,
        source: Arc<()>,
    },
    // General OPEN uses the private verified incarnation's original negative. A raw
    // public ProjectionOpenAdmission cannot create this support. Accepted support retains
    // only the original source, independently of later OPEN removal.
    GeneralOpen {
        origin: V2ProjectionOrigin,
    },
    GeneralAccepted {
        record: owner_bodies::AcceptanceRecordId,
        leaf_index: u64,
        origin: V2ProjectionOrigin,
    },
    FounderCreate {
        // Supporting tip headers can advance without changing this accepted effect.
        // Exact signed witnesses remain in the real archive, not in effect identity.
        record: owner_bodies::AcceptanceRecordId,
        leaf_index: u64,
        origin: V2ProjectionOrigin,
    },
}
impl ProjectionSupport {
    fn v2_origin(&self) -> Option<&V2ProjectionOrigin> {
        match self {
            #[cfg(test)]
            Self::Legacy { .. } => None,
            Self::FounderCreate { origin, .. }
            | Self::GeneralOpen { origin }
            | Self::GeneralAccepted { origin, .. } => Some(origin),
        }
    }
    fn live(&self) -> bool {
        match self {
            #[cfg(test)]
            Self::Legacy { .. } => true,
            Self::FounderCreate { origin, .. }
            | Self::GeneralOpen { origin }
            | Self::GeneralAccepted { origin, .. } => origin.live(),
        }
    }
    fn same_effect(&self, other: &Self) -> bool {
        match (self, other) {
            // Preserve v1's normalized effect equivalence across valid fact/signature
            // representatives. The exact legacy delete capsule keeps its own source.
            #[cfg(test)]
            (Self::Legacy { .. }, Self::Legacy { .. }) => true,
            (
                Self::FounderCreate {
                    record: ar,
                    leaf_index: ai,
                    origin: ao,
                },
                Self::FounderCreate {
                    record: br,
                    leaf_index: bi,
                    origin: bo,
                },
            ) => ar == br && ai == bi && ao.same_original(bo),
            (Self::GeneralOpen { origin: a }, Self::GeneralOpen { origin: b }) => {
                a.same_original(b)
            }
            (
                Self::GeneralAccepted {
                    record: ar,
                    leaf_index: ai,
                    origin: ao,
                },
                Self::GeneralAccepted {
                    record: br,
                    leaf_index: bi,
                    origin: bo,
                },
            ) => ar == br && ai == bi && ao.same_original(bo),
            _ => false,
        }
    }
    fn legacy(&self) -> Option<(&Arc<()>, owner_facts::OwnerFactId)> {
        match self {
            #[cfg(test)]
            Self::Legacy { source, proof } => Some((source, *proof)),
            Self::FounderCreate { .. }
            | Self::GeneralOpen { .. }
            | Self::GeneralAccepted { .. } => None,
        }
    }
}

/// Opaque exact current-effect permission. There is no decoder, public field setter or
/// status-bool adapter. The V2 founder verifier issues only accepted Create effects inside
/// this boundary after checking exact signed lineage, delegated semantics and inclusion.
/// The separate general verifier issues Provisional ordinary effects and non-Delete LOG
/// inclusion. Exact Proven V2 Delete custody/Review remain separately closed.
/// Its view binding is identity/support, not issuance revision: unrelated OPEN closure must not
/// revoke accepted/reviewed support when the next original projection lease is captured.
#[derive(Clone)]
pub struct ProjectionAdmissionToken {
    binding: ActionBinding,
    context: EvidenceContext,
    view: Arc<ViewLife>,
    open: Option<Arc<OpenLife>>,
    support: ProjectionSupport,
}
impl std::fmt::Debug for ProjectionAdmissionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectionAdmissionToken")
            .finish_non_exhaustive()
    }
}
impl ProjectionAdmissionToken {
    fn equivalent_effect(&self, other: &Self) -> bool {
        self.binding == other.binding
            && self.context == other.context
            && Arc::ptr_eq(&self.view, &other.view)
            && self.support.same_effect(&other.support)
            && match (&self.open, &other.open) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
    }
    fn certainty(&self) -> ProjectionCertainty {
        if self.open.is_some() || matches!(self.support, ProjectionSupport::GeneralOpen { .. }) {
            ProjectionCertainty::Provisional
        } else {
            ProjectionCertainty::Proven
        }
    }
    fn live_for(&self, view: &Arc<ViewLife>, id: ActionId, action: &SignedAction) -> bool {
        Arc::ptr_eq(&self.view, view)
            && view.live()
            && self.support.live()
            && self.binding.realm == view.realm
            && self.binding.action == id
            && self.binding.create == action.spec().body.create().unwrap_or(id)
            && self.binding.signer == *action.signer()
            && self.binding.grant == action.spec().grant
            && self.binding.basis == action.spec().basis
            && self.open.as_ref().is_none_or(|open| {
                self.support.legacy().is_some()
                    && open.live()
                    && Arc::ptr_eq(&open.view, view)
                    && open.profile == self.context.profile()
                    && open.admission == self.context.admission()
                    && open.subject == self.binding.signer
            })
    }
}

/// One fixed negative slot on EACH Delete entry, never a replica-local rendered flag. It
/// remains through view cancellation and does not become an approved tombstone in another view.
/// Exact runtime source identity cannot be reconstructed by a snapshot/status decoder.
#[derive(Clone)]
pub(super) struct PriorDeleteCapsule {
    delete: ActionBinding,
    delete_context: EvidenceContext,
    create: ActionBinding,
    create_context: EvidenceContext,
    source: Arc<()>,
    proof: owner_facts::OwnerFactId,
}
impl PriorDeleteCapsule {
    fn same(&self, other: &Self) -> bool {
        self.delete == other.delete
            && self.delete_context == other.delete_context
            && self.create == other.create
            && self.create_context == other.create_context
            && self.proof == other.proof
            && Arc::ptr_eq(&self.source, &other.source)
    }
}

/// Opaque complete-current-effect closure for ONE exact prior capsule/target. No production
/// constructor exists until the real governance consumer proves that complete closure. A lone
/// signed Reject, missing proof, raw action or rendered status cannot clear negative custody.
pub struct CompleteProjectionResolution {
    capsule: PriorDeleteCapsule,
    view: Arc<ViewLife>,
    revision: u64,
}
impl std::fmt::Debug for CompleteProjectionResolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompleteProjectionResolution")
            .finish_non_exhaustive()
    }
}

impl ChatLedger {
    /// Begin an UNBOUND core-only default-closed view. It issues no positive tokens. A future
    /// production issuer MUST refuse unbound views and use begin_projection_view_in with its
    /// original verified governing source. A replacement
    /// remains pending until its returned owner's try_ready completes the old drain barrier.
    pub fn begin_projection_view(&mut self) -> ProjectionView {
        self.projection.begin(None)
    }

    /// Install only opaque current support for already Valid semantic evidence. This never
    /// authorizes historical semantics and cannot cache a token on Pending/Invalid for later
    /// unguarded activation. Positive changes use the actual writer BEFORE revision/mutation;
    /// Busy leaves both retained entries and original leases unchanged.
    pub fn admit_projection(
        &mut self,
        token: &ProjectionAdmissionToken,
        budget: usize,
    ) -> Result<(), ChatError> {
        let group = self
            .projection
            .current
            .as_ref()
            .and_then(|view| view.group.clone());
        if group.as_ref().is_some_and(|group| !group.live()) {
            return Err(ChatError::Scope);
        }
        let _group_reader = group
            .as_ref()
            .map(|group| group.gate.try_read())
            .transpose()
            .map_err(|_| ChatError::Scope)?;
        let gate = self.projection.gate.clone();
        let _writer = gate.try_write().map_err(|_| ChatError::Scope)?;
        let view = self
            .projection
            .current
            .as_ref()
            .ok_or(ChatError::Scope)?
            .clone();
        let id = token.binding.action;
        let entry = self.entries.get(&id).ok_or(ChatError::Unknown)?;
        if entry.state != ActionState::Valid || !token.live_for(&view, id, &entry.action) {
            return Err(ChatError::Authority);
        }
        if let Some(previous) = &entry.projection {
            if Arc::ptr_eq(&previous.view, &view)
                && (previous.context != token.context
                    || (previous.certainty() == ProjectionCertainty::Proven
                        && token.certainty() != ProjectionCertainty::Proven))
            {
                return Err(ChatError::AuthorityConflict);
            }
            // No rescan is required for an unchanged effect. Every eligible Delete is minted
            // at its own installation or at a genuinely new Create admission; scopes never
            // reopen, and complete resolution re-capsules still-current Proven support.
            if previous.equivalent_effect(token) {
                return Ok(());
            }
        }
        // Prepare ALL new capsules before mutation. Installing the Create after its Deletes
        // must not produce a proof-loss window or a partial scan labelled complete. A Proven
        // Delete needs an admitted Create, which may itself still be Provisional.
        let original_revision = view.revision.load(Ordering::Acquire);
        let mut capsules = Vec::new();
        let mut left = budget;
        match entry.action.spec().body {
            Body::Create { .. } => {
                for delete_id in self.message_action_ids(id) {
                    left = left.checked_sub(1).ok_or(ChatError::WorkLimit)?;
                    let delete = &self.entries[&delete_id];
                    if delete.prior_delete.is_none()
                        && matches!(delete.action.spec().body, Body::Delete { create } if create == id)
                    {
                        if let Some(capsule) = self.eligible_capsule(delete_id, &view, Some(token))
                        {
                            capsules.push((delete_id, capsule));
                        }
                    }
                }
            }
            Body::Delete { .. } if entry.prior_delete.is_none() => {
                left.checked_sub(1).ok_or(ChatError::WorkLimit)?;
                if let Some(capsule) = self.eligible_capsule(id, &view, Some(token)) {
                    capsules.push((id, capsule));
                }
            }
            _ => {}
        }
        // Capsule preparation may inspect many own-message candidates. If negative custody
        // won during that preflight, refuse BEFORE committing any positive metadata. After
        // this final admission the already prepared capsules must not be re-evaluated against
        // a later cancellation: both real guards retain the operation until mutation ends.
        if !token.live_for(&view, id, &entry.action)
            || view.revision.load(Ordering::Acquire) != original_revision
        {
            return Err(ChatError::Scope);
        }
        #[cfg(test)]
        if let Some(hook) = view.mutation_hook.lock().unwrap().take() {
            hook();
        }
        view.bump()?;
        self.entries
            .get_mut(&id)
            .ok_or(ChatError::Unknown)?
            .projection = Some(token.clone());
        for (delete, capsule) in capsules {
            self.entries
                .get_mut(&delete)
                .ok_or(ChatError::Unknown)?
                .prior_delete = Some(capsule);
        }
        Ok(())
    }

    fn eligible_capsule(
        &self,
        delete_id: ActionId,
        view: &Arc<ViewLife>,
        replacing: Option<&ProjectionAdmissionToken>,
    ) -> Option<PriorDeleteCapsule> {
        let delete = self.entries.get(&delete_id)?;
        let Body::Delete { create } = delete.action.spec().body else {
            return None;
        };
        let original = self.entries.get(&create)?;
        if delete.state != ActionState::Valid
            || original.state != ActionState::Valid
            || !matches!(original.action.spec().body, Body::Create { .. })
        {
            return None;
        }
        let delete_token = candidate_token(delete_id, delete, replacing)?;
        let create_token = candidate_token(create, original, replacing)?;
        if delete_token.certainty() != ProjectionCertainty::Proven
            || !delete_token.live_for(view, delete_id, &delete.action)
            || !create_token.live_for(view, create, &original.action)
        {
            return None;
        }
        let (source, proof) = delete_token.support.legacy()?;
        if create_token.support.legacy().is_none()
            || !delete_token.context.is_legacy()
            || !create_token.context.is_legacy()
        {
            return None;
        };
        Some(PriorDeleteCapsule {
            delete: delete_token.binding,
            delete_context: delete_token.context,
            create: create_token.binding,
            create_context: create_token.context,
            source: source.clone(),
            proof,
        })
    }

    /// Clear/replace only an opaque exact-capsule complete-current closure. Each independent
    /// Delete retains its own obligation. Current eligible Proven support is re-capsuled, so
    /// resolving an old source cannot leave an effective Delete unprotected at next proof loss.
    pub fn resolve_projection_delete(
        &mut self,
        resolution: &CompleteProjectionResolution,
    ) -> Result<(), ChatError> {
        let group = self
            .projection
            .current
            .as_ref()
            .and_then(|view| view.group.clone());
        if group.as_ref().is_some_and(|group| !group.live()) {
            return Err(ChatError::Scope);
        }
        let _group_reader = group
            .as_ref()
            .map(|group| group.gate.try_read())
            .transpose()
            .map_err(|_| ChatError::Scope)?;
        let gate = self.projection.gate.clone();
        let _writer = gate.try_write().map_err(|_| ChatError::Scope)?;
        let view = self.projection.current.as_ref().ok_or(ChatError::Scope)?;
        if !Arc::ptr_eq(view, &resolution.view)
            || !view.live()
            || view.revision.load(Ordering::Acquire) != resolution.revision
        {
            return Err(ChatError::Scope);
        }
        let id = resolution.capsule.delete.action;
        let entry = self.entries.get(&id).ok_or(ChatError::Unknown)?;
        if entry
            .prior_delete
            .as_ref()
            .is_none_or(|capsule| !capsule.same(&resolution.capsule))
        {
            return Err(ChatError::Authority);
        }
        let replacement = self.eligible_capsule(id, view, None);
        if !view.live() || view.revision.load(Ordering::Acquire) != resolution.revision {
            return Err(ChatError::Scope);
        }
        #[cfg(test)]
        if let Some(hook) = view.mutation_hook.lock().unwrap().take() {
            hook();
        }
        view.bump()?;
        self.entries
            .get_mut(&id)
            .ok_or(ChatError::Unknown)?
            .prior_delete = replacement;
        Ok(())
    }
}

fn candidate_token<'a>(
    id: ActionId,
    entry: &'a super::ledger::Entry,
    replacing: Option<&'a ProjectionAdmissionToken>,
) -> Option<&'a ProjectionAdmissionToken> {
    replacing
        .filter(|token| token.binding.action == id)
        .or(entry.projection.as_ref())
}

#[cfg(test)]
mod tests;
