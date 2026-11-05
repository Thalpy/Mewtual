//! Default-closed current projection custody, separate from monotone historical semantics.
//!
//! No production positive issuer exists here. Public owners create only negative lifetimes;
//! raw labels, snapshots and semantic Valid cannot construct an admission token. The later
//! governing verifier must own issuance after its actual complete current-evidence checks.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, TryLockError};

mod guarded;
pub use guarded::{GuardedCausalHeads, GuardedMessageProjection, GuardedProjection};

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
    pub(super) fn begin(&mut self) -> ProjectionView {
        let previous = self.current.as_ref().map(ViewLife::request_close);
        let life = Arc::new(ViewLife {
            gate: self.gate.clone(),
            realm: self.realm,
            revision: AtomicU64::new(1),
            cancelled: AtomicBool::new(false),
            pending: AtomicUsize::new(if previous.is_some() { 1 } else { 0 }),
            closed: AtomicBool::new(false),
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
    realm: RealmId,
    revision: AtomicU64,
    cancelled: AtomicBool,
    pending: AtomicUsize,
    closed: AtomicBool,
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
}
impl ProjectionLease {
    fn capture(view: &Arc<ViewLife>) -> Result<Self, ChatError> {
        let lease = Self {
            view: view.clone(),
            revision: view.revision.load(Ordering::Acquire),
            open: Vec::new(),
        };
        lease.validate()?;
        Ok(lease)
    }
    pub(super) fn validate(&self) -> Result<(), ChatError> {
        if !self.view.live()
            || self.view.revision.load(Ordering::Acquire) != self.revision
            || self.open.iter().any(|scope| !scope.live())
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    pub(super) fn observe(&mut self, token: &ProjectionAdmissionToken) {
        if let Some(scope) = &token.open {
            self.open.push(scope.clone());
        }
    }
    pub(super) fn publish<T>(&self, callback: impl FnOnce() -> T) -> Result<T, ChatError> {
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
    #[cfg(test)]
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
struct EvidenceContext {
    profile: owner_facts::OwnerFactId,
    admission: owner_facts::OwnerFactId,
}

/// Opaque exact current-effect permission. There is intentionally NO production constructor,
/// decoder, public field setter or status-bool adapter. The later current-governance verifier
/// must construct it inside this chat boundary after checking complete original dependencies.
/// Its view binding is identity/support, not issuance revision: unrelated OPEN closure must not
/// revoke accepted/reviewed support when the next original projection lease is captured.
#[derive(Clone)]
pub struct ProjectionAdmissionToken {
    binding: ActionBinding,
    context: EvidenceContext,
    view: Arc<ViewLife>,
    open: Option<Arc<OpenLife>>,
    proof: owner_facts::OwnerFactId,
    source: Arc<()>,
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
            && match (&self.open, &other.open) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
    }
    fn certainty(&self) -> ProjectionCertainty {
        if self.open.is_some() {
            ProjectionCertainty::Provisional
        } else {
            ProjectionCertainty::Proven
        }
    }
    fn live_for(&self, view: &Arc<ViewLife>, id: ActionId, action: &SignedAction) -> bool {
        Arc::ptr_eq(&self.view, view)
            && view.live()
            && self.binding.realm == view.realm
            && self.binding.action == id
            && self.binding.create == action.spec().body.create().unwrap_or(id)
            && self.binding.signer == *action.signer()
            && self.binding.grant == action.spec().grant
            && self.binding.basis == action.spec().basis
            && self.open.as_ref().is_none_or(|open| {
                open.live()
                    && Arc::ptr_eq(&open.view, view)
                    && open.profile == self.context.profile
                    && open.admission == self.context.admission
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
    /// Begin an original default-closed view. It issues no positive tokens. A replacement
    /// remains pending until its returned owner's try_ready completes the old drain barrier.
    pub fn begin_projection_view(&mut self) -> ProjectionView {
        self.projection.begin()
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
        let mut capsules = Vec::new();
        let mut left = budget;
        match entry.action.spec().body {
            Body::Create { .. } => {
                for (delete_id, delete) in &self.entries {
                    left = left.checked_sub(1).ok_or(ChatError::WorkLimit)?;
                    if delete.prior_delete.is_none()
                        && matches!(delete.action.spec().body, Body::Delete { create } if create == id)
                    {
                        if let Some(capsule) = self.eligible_capsule(*delete_id, &view, Some(token))
                        {
                            capsules.push((*delete_id, capsule));
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
        Some(PriorDeleteCapsule {
            delete: delete_token.binding,
            delete_context: delete_token.context,
            create: create_token.binding,
            create_context: create_token.context,
            source: delete_token.source.clone(),
            proof: delete_token.proof,
        })
    }

    /// Clear/replace only an opaque exact-capsule complete-current closure. Each independent
    /// Delete retains its own obligation. Current eligible Proven support is re-capsuled, so
    /// resolving an old source cannot leave an effective Delete unprotected at next proof loss.
    pub fn resolve_projection_delete(
        &mut self,
        resolution: &CompleteProjectionResolution,
    ) -> Result<(), ChatError> {
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
