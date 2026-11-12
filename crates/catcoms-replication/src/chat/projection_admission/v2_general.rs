//! General V2 ordinary authority, independently of owner certification availability.
//!
//! The caller's ORIGINAL mounted pin and selected-save proof supply the explicit HOST
//! expectations. Signed replay authenticates the complete governing prefix; it does not
//! turn downloaded self-signatures, an MLS roster, or negative lifetime owners into trust.
//! Ordinary actions need no new owner signature after their exact Admission is authenticated.
//! Native source selection, durable fork escrow and cold complete-prefix repair remain host
//! obligations. This separate implementation does not relax the Founder-only verifier.
use super::*;
use crate::chat::acceptance_tree::{verify_inclusion, TreeContext, TreeHash, TreeTip};
use crate::chat::authority::SemanticConstraint;
use crate::chat::owner_bodies::AcceptanceRecord;
use crate::chat::owner_header::{OwnerHeaderId, SignedOwnerHeader};
use crate::chat::owner_profile::AdmissionBody;
use crate::chat::owner_state::transition::{FrozenCutData, ReplayBody, ReplayMath, StepData};
use std::collections::BTreeSet;

// None of these diagnostic implementations expose action bodies, roster/key material,
// scope identities or contradiction payloads. Callers use explicit immutable DATA loans.
macro_rules! redacted_debug {
    ($($name:ty),+ $(,)?) => { $(impl std::fmt::Debug for $name {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct(stringify!($name)).finish_non_exhaustive()
        }
    })+ };
}
mod ordinary;
mod signing;
pub use ordinary::{V2GeneralAcceptanceSource, V2OrdinaryDependency};
pub use signing::{
    OriginalV2OrdinarySigningPlanObservation, V2OrdinaryKeyCall, V2OrdinarySigningGuard,
    V2OrdinarySigningPlan,
};

const MAX_ADMISSIONS: usize = crate::chat::owner_profile::MAX_CURRENT_MEMBERS as usize;

/// Independent original HOST trust decision, never inferred from a candidate Profile.
pub struct V2GeneralExpectation(V2FounderExpectation);
impl V2GeneralExpectation {
    pub fn from_trusted_host(anchor: FounderGenesisAnchorData) -> Result<Self, ChatError> {
        Ok(Self(V2FounderExpectation::from_trusted_host(anchor)?))
    }
}

struct AdmissionLife {
    closed: AtomicBool,
}
#[derive(Debug)]
struct AdmissionNegative {
    life: Arc<AdmissionLife>,
    original: V2ProjectionOrigin,
}
impl std::fmt::Debug for AdmissionLife {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2AdmissionLife").finish_non_exhaustive()
    }
}
impl V2OriginNegative for AdmissionNegative {
    fn is_live(&self) -> bool {
        !self.life.closed.load(Ordering::Acquire) && self.original.live()
    }
}
struct AdmissionCore {
    body: AdmissionBody,
    identity: Arc<()>,
    life: Arc<AdmissionLife>,
    open_origin: V2ProjectionOrigin,
}

/// Opaque actual selected Admission provenance. No decoder, arbitrary-body constructor or
/// fixture-witness adapter exists. Historical handles may outlive removal; they never reopen
/// their original incarnation. The archive/host separately pays each retained handle/body.
#[derive(Clone)]
pub struct V2AuthenticatedAdmission {
    core: Arc<AdmissionCore>,
}
impl std::fmt::Debug for V2AuthenticatedAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2AuthenticatedAdmission")
            .finish_non_exhaustive()
    }
}
impl V2AuthenticatedAdmission {
    pub fn admission_data(&self) -> &AdmissionBody {
        &self.core.body
    }
}

/// ONE known governing parent and at most TWO valid semantic children. This escrow is
/// produced only by actual selected replay. Retain it in the paid archive after advancement;
/// no cache of unknown parents or recursive whole-prefix owner chain is created.
pub struct V2KnownGoverningParent {
    identity: Arc<()>,
    parent: SignedOwnerHeader,
    first: Option<SignedOwnerHeader>,
    second: Option<SignedOwnerHeader>,
}
impl std::fmt::Debug for V2KnownGoverningParent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2KnownGoverningParent")
            .finish_non_exhaustive()
    }
}
impl V2KnownGoverningParent {
    pub fn parent_data(&self) -> &SignedOwnerHeader {
        &self.parent
    }
    pub fn contradiction_data(&self) -> Option<(&SignedOwnerHeader, &SignedOwnerHeader)> {
        Some((self.first.as_ref()?, self.second.as_ref()?))
    }
    fn observe(&mut self, header: &SignedOwnerHeader) -> Result<bool, ChatError> {
        // Verify EVERY signature representation before CID comparison. A bad same-CID
        // representation and a valid foreign/unknown-parent header cannot poison this source.
        header.verify_successor_of(&self.parent)?;
        if let Some(first) = &self.first {
            if first.id() != header.id() {
                if self.second.is_none() {
                    self.second = Some(header.clone());
                }
                return Ok(true);
            }
        } else {
            self.first = Some(header.clone());
        }
        Ok(self.second.is_some())
    }
}

/// Original admission close debt. Its monotone close has already happened; this object
/// reports drain only after probing the SAME actual group's publication writer. Abandoning
/// it cannot reopen anything, but is not an acknowledgment that old callbacks finished.
#[must_use = "retain and poll the original admission publication drain"]
pub struct V2AdmissionCloseCompletion {
    group: Arc<GroupViewLife>,
    life: Arc<AdmissionLife>,
    complete: bool,
}
impl V2AdmissionCloseCompletion {
    pub fn try_complete(&mut self) -> bool {
        if self.complete {
            return true;
        }
        if !self.life.closed.load(Ordering::Acquire) {
            return false;
        }
        // A completed drain of the SAME original group already covers all of
        // this Admission's readers. Re-probing the shared flat gate would make
        // old debt wait forever behind unrelated replacement-view traffic.
        if self.group.has_completed_drain() {
            self.complete = true;
            return true;
        }
        self.complete = match self.group.gate.try_write() {
            Ok(_writer) => true,
            Err(TryLockError::WouldBlock) => false,
            Err(TryLockError::Poisoned(error)) => {
                drop(error.into_inner());
                true
            }
        };
        self.complete
    }
}

/// Opaque whole derived candidate. No caller next-model setter or Clone exists. The actual
/// HOST must select the exact header/state before consuming this same candidate once.
pub struct V2GeneralCandidate {
    identity: Arc<()>,
    parent: OwnerHeaderId,
    step: StepData,
    admissions: Vec<V2AuthenticatedAdmission>,
    removed: Option<V2AuthenticatedAdmission>,
    next_parent: V2KnownGoverningParent,
    _claim: CandidateClaim,
}
struct CandidateClaim(Arc<AtomicBool>);
impl Drop for CandidateClaim {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl V2GeneralCandidate {
    pub fn header_data(&self) -> &SignedOwnerHeader {
        self.step.next.header()
    }
    pub fn image_data(&self) -> &[u8] {
        self.step.next.image()
    }
}

/// Whole old math/escrow/output owners for outside-writer disposal or paid archive handoff.
/// Keeping a cut is NOT itself proof that an arbitrary late action was included in that cut.
pub struct V2GeneralRetiredData {
    math: ReplayMath,
    parent: Option<V2KnownGoverningParent>,
    admissions: Vec<V2AuthenticatedAdmission>,
    append: Option<crate::chat::acceptance_tree::PreparedAppend>,
    frozen: Option<V2FrozenAdmission>,
    closed: Option<V2AdmissionCloseCompletion>,
    // A next candidate cannot create a THIRD model while this actual old math is held.
    // Drop this whole old owner outside writers after taking separately paid archive DATA.
    _claim: CandidateClaim,
}
impl V2GeneralRetiredData {
    pub fn parent_data(&self) -> &ReplayMath {
        &self.math
    }
    pub fn known_parent_data_mut(&mut self) -> Option<&mut V2KnownGoverningParent> {
        self.parent.as_mut()
    }
    pub fn take_known_parent_data(&mut self) -> Option<V2KnownGoverningParent> {
        self.parent.take()
    }
    pub fn frozen_data(&self) -> Option<&V2FrozenAdmission> {
        self.frozen.as_ref()
    }
    pub fn take_close_completion(&mut self) -> Option<V2AdmissionCloseCompletion> {
        self.closed.take()
    }
    /// The caller may move this opaque actual frozen proof into its independently paid
    /// archive. There is no constructor from a TreeTip, count, roster or removal label.
    pub fn take_frozen_data(&mut self) -> Option<V2FrozenAdmission> {
        self.frozen.take()
    }
    pub fn append_data(&self) -> Option<&crate::chat::acceptance_tree::PreparedAppend> {
        self.append.as_ref()
    }
    pub fn retained_admission_count_data(&self) -> usize {
        self.admissions.len()
    }
}

/// Exact whole cut derived by the real selected Remove. It retains the original
/// admission/subject/tip-header/frontier DATA for paid archive handoff, never a whole
/// prior model or source chain. The fixed frontier has no heap allocation; retaining
/// it preserves the complete replay output instead of reconstructing omitted fields.
/// Accepted-action verification must still check exact inclusion.
pub struct V2FrozenAdmission {
    admission: V2AuthenticatedAdmission,
    cut: FrozenCutData,
}

/// General selected governing prefix. At most 64 CURRENT Admission owners are retained;
/// removed historical handles/cuts belong to the separately bounded paid archive. No
/// wall-clock, transport epoch, membership boolean or owner private key enters issuance.
pub struct V2GeneralPrefixVerifier {
    math: ReplayMath,
    admissions: Vec<V2AuthenticatedAdmission>,
    known: V2KnownGoverningParent,
    identity: Arc<()>,
    group: Arc<GroupViewLife>,
    origin: V2ProjectionOrigin,
    fork_close: Option<ProjectionGroupCloseCompletion>,
    candidate_live: Arc<AtomicBool>,
}
impl Drop for V2GeneralPrefixVerifier {
    fn drop(&mut self) {
        // Abandoning the selected governor is proof loss, not a replacement trust decision.
        // O(1) cancellation takes no publication writer and acknowledges no completed drain.
        drop(GroupViewLife::request_close(&self.group));
    }
}
redacted_debug!(
    V2GeneralExpectation,
    V2AdmissionCloseCompletion,
    V2GeneralCandidate,
    V2GeneralRetiredData,
    V2FrozenAdmission,
    V2GeneralPrefixVerifier
);

impl V2GeneralPrefixVerifier {
    /// Metadata ADDITIONAL to the existing complete ReplayMath1MiB invoice, ordinary action
    /// buffers/ledger slots, host observers and archive escrow. This is a named layout only,
    /// not a paid quota or a claim that the Founder/Root metadata allowance covers this type.
    pub const fn metadata_layout_bytes() -> usize {
        2 * std::mem::size_of::<Self>()
            + 2 * MAX_ADMISSIONS
                * (std::mem::size_of::<V2AuthenticatedAdmission>()
                    + std::mem::size_of::<AdmissionCore>()
                    + std::mem::size_of::<AdmissionLife>()
                    + std::mem::size_of::<AdmissionNegative>()
                    + V2ProjectionOrigin::retained_layout_bytes()
                    + crate::chat::owner_profile::MAX_ADMISSION_BYTES
                    + 16 * std::mem::size_of::<usize>())
            + 4 * crate::chat::owner_header::MAX_HEADER_BYTES
            + 2 * std::mem::size_of::<V2GeneralCandidate>()
            + 2 * std::mem::size_of::<V2GeneralRetiredData>()
            + 2 * (std::mem::size_of::<AtomicBool>() + 4 * std::mem::size_of::<usize>())
    }

    /// Verify full signed genesis under an independent original HOST pin and original
    /// negative group/source custody. These negative owners supply no positive trust.
    pub fn from_genesis(
        expectation: V2GeneralExpectation,
        profile: &crate::chat::owner_profile::SignedOwnerProfile,
        founder: &AdmissionBody,
        header: &SignedOwnerHeader,
        image: &[u8],
        group: &ProjectionGroupView,
        origin: V2ProjectionOrigin,
    ) -> Result<Self, ChatError> {
        if !origin.live() || !group.life.live() || group.life.group.as_ref() != profile.spec().group
        {
            return Err(ChatError::Scope);
        }
        let math =
            v2_founder::verified_genesis_math(&expectation.0, profile, founder, header, image)?;
        let identity = Arc::new(());
        let mut admissions = Vec::with_capacity(MAX_ADMISSIONS);
        admissions.push(Self::new_admission(founder, &identity, &origin));
        let known = V2KnownGoverningParent {
            identity: identity.clone(),
            parent: header.clone(),
            first: None,
            second: None,
        };
        if !origin.live() || !group.life.live() {
            return Err(ChatError::Scope);
        }
        Ok(Self {
            math,
            admissions,
            known,
            identity,
            group: group.life.clone(),
            origin,
            fork_close: None,
            candidate_live: Arc::new(AtomicBool::new(false)),
        })
    }
    fn new_admission(
        body: &AdmissionBody,
        identity: &Arc<()>,
        origin: &V2ProjectionOrigin,
    ) -> V2AuthenticatedAdmission {
        let life = Arc::new(AdmissionLife {
            closed: AtomicBool::new(false),
        });
        let open_origin = V2ProjectionOrigin::new(Arc::new(AdmissionNegative {
            life: life.clone(),
            original: origin.clone(),
        }));
        V2AuthenticatedAdmission {
            core: Arc::new(AdmissionCore {
                body: body.clone(),
                identity: identity.clone(),
                life,
                open_origin,
            }),
        }
    }
    fn live(&self) -> Result<(), ChatError> {
        if self.origin.live() && self.group.live() && self.fork_close.is_none() {
            Ok(())
        } else {
            Err(ChatError::Scope)
        }
    }
    /// Public mathematical planning DATA only. There is no adopting or mutable getter.
    pub fn replay_data(&self) -> &ReplayMath {
        &self.math
    }
    pub fn admission(
        &self,
        id: crate::chat::owner_facts::OwnerFactId,
    ) -> Result<V2AuthenticatedAdmission, ChatError> {
        self.live()?;
        self.admissions
            .iter()
            .find(|a| a.core.body.admission_id() == id)
            .cloned()
            .ok_or(ChatError::Dependency)
    }
    pub fn current_contradiction_data(&self) -> Option<(&SignedOwnerHeader, &SignedOwnerHeader)> {
        self.known.contradiction_data()
    }
    pub fn fork_close_completion_mut(&mut self) -> Option<&mut ProjectionGroupCloseCompletion> {
        self.fork_close.as_mut()
    }
    fn fence_fork(&mut self) {
        // Both actual valid signed siblings are already cached BEFORE this irreversible
        // original-view close. Never choose arrival order/CID or clear historical ledger data.
        if self.fork_close.is_none() {
            self.fork_close = Some(GroupViewLife::request_close(&self.group));
        }
    }
    /// Negative known-parent processing precedes body availability. The opaque parent may
    /// come from an earlier actual selected step; a caller-supplied parsed header cannot
    /// assert that it belonged to this original verified prefix.
    pub fn observe_archived_header(
        &mut self,
        parent: &mut V2KnownGoverningParent,
        header: &SignedOwnerHeader,
    ) -> Result<(), ChatError> {
        if !Arc::ptr_eq(&self.identity, &parent.identity) {
            return Err(ChatError::Scope);
        }
        if parent.observe(header)? {
            self.fence_fork();
            return Err(ChatError::AuthorityConflict);
        }
        self.live()
    }
    pub fn observe_current_header(&mut self, header: &SignedOwnerHeader) -> Result<(), ChatError> {
        if self.known.observe(header)? {
            self.fence_fork();
            return Err(ChatError::AuthorityConflict);
        }
        self.live()
    }
    /// Real Add/Append/Remove replay. The first signed header remains cached on missing/bad
    /// body, and a different valid sibling fences the view BEFORE attempting either body.
    pub fn derive_successor(
        &mut self,
        header: &SignedOwnerHeader,
        body: ReplayBody<'_>,
        image: &[u8],
    ) -> Result<V2GeneralCandidate, ChatError> {
        self.observe_current_header(header)?;
        // Structurally at most one complete candidate per original verifier. Signature
        // contradiction processing still runs first while an earlier candidate is held.
        self.candidate_live
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ChatError::Capacity)?;
        let claim = CandidateClaim(self.candidate_live.clone());
        let admission_body = match &body {
            ReplayBody::Admission(a) => Some(*a),
            _ => None,
        };
        let step = self.math.derive_successor(header, body, image)?;
        let mut admissions = Vec::with_capacity(MAX_ADMISSIONS);
        let mut removed = None;
        for old in &self.admissions {
            if step.next.slot(old.core.body.id()).is_some() {
                admissions.push(old.clone());
            } else {
                removed = Some(old.clone());
            }
        }
        if let Some(body) = admission_body {
            admissions.push(Self::new_admission(body, &self.identity, &self.origin));
        }
        if admissions.len() != step.next.slots().len() || admissions.len() > MAX_ADMISSIONS {
            return Err(ChatError::Dependency);
        }
        let next_parent = V2KnownGoverningParent {
            identity: self.identity.clone(),
            parent: header.clone(),
            first: None,
            second: None,
        };
        self.live()?;
        Ok(V2GeneralCandidate {
            identity: self.identity.clone(),
            parent: self.math.header().id(),
            step,
            admissions,
            removed,
            next_parent,
            _claim: claim,
        })
    }
    /// Consume only this exact candidate after a separate actual HOST selected-save proof.
    /// All fallible predicates precede take. Removal closes its original incarnation before
    /// return; every old owner/cut and explicit close debt returns whole for outside disposal.
    pub fn advance_host_selected(
        &mut self,
        selected: &V2HostSelectionExpectation,
        candidate: &mut Option<V2GeneralCandidate>,
    ) -> Result<V2GeneralRetiredData, ChatError> {
        self.live()?;
        let next = candidate.as_ref().ok_or(ChatError::Dependency)?;
        if !Arc::ptr_eq(&self.identity, &next.identity)
            || next.parent != self.math.header().id()
            || !selected.matches_data(next.step.next.header())
        {
            return Err(ChatError::Scope);
        }
        let closed = next.removed.as_ref().map(|a| V2AdmissionCloseCompletion {
            group: self.group.clone(),
            life: a.core.life.clone(),
            complete: false,
        });
        let frozen_admission = match (&next.removed, &next.step.removal) {
            (Some(admission), Some(cut)) if admission.core.body.id() == cut.admission => {
                Some(admission.clone())
            }
            (None, None) => None,
            _ => return Err(ChatError::Dependency),
        };
        self.live()?;
        let next = candidate.take().expect("checked same general candidate");
        // Every fallible predicate precedes take. Move the SAME original whole cut
        // into returned archive custody; copying only its tip would discard the
        // tip-header/subject/frontier that Root must durably retain. This is fixed
        // DATA, not a new allocation or a caller-reconstructed acceptance witness.
        let frozen = frozen_admission
            .zip(next.step.removal)
            .map(|(admission, cut)| V2FrozenAdmission { admission, cut });
        if let Some(removed) = &next.removed {
            removed.core.life.closed.store(true, Ordering::Release);
        }
        let math = std::mem::replace(&mut self.math, next.step.next);
        let parent = std::mem::replace(&mut self.known, next.next_parent);
        let admissions = std::mem::replace(&mut self.admissions, next.admissions);
        Ok(V2GeneralRetiredData {
            math,
            parent: Some(parent),
            admissions,
            append: next.step.append,
            frozen,
            closed,
            _claim: next._claim,
        })
    }
}

#[cfg(test)]
mod tests;
