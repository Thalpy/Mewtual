//! Pre-key ordinary signing plans. Only the same live general verifier can issue one;
//! immutable Draft DATA and caller device labels alone cannot. Root's independent actual
//! Source/UI/native credential admission is STILL mandatory before the real key call.
use super::*;
use crate::chat::action::ActionDraftPlanClaim;
use crate::chat::{ActionDraft, OriginalActionDraftObservation};
use std::sync::{RwLockReadGuard, Weak};

struct PlanIdentity;
struct PlanFacts {
    draft: OriginalActionDraftObservation,
    action: ActionId,
    signer: [u8; 32],
    admission: crate::chat::owner_facts::OwnerFactId,
    group: Arc<GroupViewLife>,
    view: Arc<ViewLife>,
    open_origin: V2ProjectionOrigin,
}
enum PlanWork {
    Prepared,
    KeyEntered,
    RawCached([u8; 64]),
}

/// Non-Clone, one-owner exact plan. Neither dropping it nor losing a key return permits a
/// replacement plan for the same Draft. It retains no model, SDK, source sender or key.
pub struct V2OrdinarySigningPlan {
    facts: PlanFacts,
    work: PlanWork,
    identity: Arc<PlanIdentity>,
    _claim: ActionDraftPlanClaim,
}
/// Tiny original-plan identity DATA for the later private Sync packet; equal bytes or a new
/// selected Admission cannot adopt another plan. This Weak retains neither plan nor source.
#[derive(Clone)]
pub struct OriginalV2OrdinarySigningPlanObservation {
    identity: Weak<PlanIdentity>,
    draft: OriginalActionDraftObservation,
    action: ActionId,
    signer: [u8; 32],
    admission: crate::chat::owner_facts::OwnerFactId,
}
/// Actual original group reader plus final original-plan/Draft/OPEN negatives. Keep this
/// through cached-raw reuse OR consume it into one key entry. No I/O, pool wait, codec,
/// SDK lookup, allocation or await belongs after admission; only the bounded actual key call.
pub struct V2OrdinarySigningGuard<'a> {
    _facts: &'a PlanFacts,
    work: &'a mut PlanWork,
    draft: &'a ActionDraft,
    _reader: RwLockReadGuard<'a, ()>,
}
/// Consuming key-entry owner. Entry was marked BEFORE this escaped. Drop/unwind without
/// cache leaves the original Plan terminal KeyEntered, never eligible for another key call.
pub struct V2OrdinaryKeyCall<'a> {
    admitted: V2OrdinarySigningGuard<'a>,
}
redacted_debug!(
    V2OrdinarySigningPlan,
    OriginalV2OrdinarySigningPlanObservation,
    V2OrdinarySigningGuard<'_>,
    V2OrdinaryKeyCall<'_>
);

impl PlanFacts {
    fn require_draft(
        &self,
        draft: &ActionDraft,
        full_device_key: &[u8; 32],
    ) -> Result<(), ChatError> {
        draft.require_original_data(&self.draft)?;
        if self.action != draft.action_id_data()
            || self.signer != *draft.signer_data()
            || self.signer != *full_device_key
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    fn live(&self) -> Result<(), ChatError> {
        if self.group.live() && self.view.live() && self.open_origin.live() {
            Ok(())
        } else {
            Err(ChatError::Scope)
        }
    }
}
impl V2GeneralPrefixVerifier {
    /// Separate transient logical invoice for ONE unsigned-authority verification, outside
    /// writers, in either the signed issuer or the pre-key issuer. At most65 dependencies
    /// undergo real signature/delegation/ID checks SEQUENTIALLY; this does not reserve65
    /// simultaneous action clones. Four canonical-buffer allowances cover encoding growth
    /// and signature/ID scratch, plus small delegation encoders. Required-ID clone+insert
    /// can request128 slots for64 parents plus a missing target; retain the old64-slot growth
    /// allocation allowance too. Covered bases retain at most65 keys;256 logical backing
    /// bytes per possible key conservatively covers tree-node/control allocation policy.
    ///
    /// This is ADDITIONAL to ActionDraft::private_layout_bytes, Plan metadata, replay1MiB,
    /// caller-owned action/dependency evidence and crypto/SDK/provider internals. Actual
    /// Root must debit it independently, or explicitly prove/pay an exclusive phase in one
    /// real account. This name is not an actual payment, allocator/RSS promise or cap change.
    pub const fn ordinary_authorization_temporary_layout_bytes() -> usize {
        4 * MAX_ACTION_BYTES
            + 4 * MAX_GROUP_ID_BYTES
            + 2 * (MAX_PARENTS + 1) * std::mem::size_of::<ActionId>()
            + MAX_PARENTS * std::mem::size_of::<ActionId>()
            + (MAX_PARENTS + 1) * (std::mem::size_of::<BasisId>() + 256)
            + std::mem::size_of::<Vec<ActionId>>()
            + std::mem::size_of::<BTreeSet<BasisId>>()
            + 4 * std::mem::size_of::<blake3::Hasher>()
            + 8 * std::mem::size_of::<usize>()
    }

    /// Validate unsigned shape/delegation/minimal REAL signed dependencies under this exact
    /// current Admission, then bind its original OPEN/group/channel/source to ONE Draft.
    /// This is not signature verification, a semantic witness, owner acceptance or native
    /// device authorization. The signed issuer still verifies the action's actual signature.
    pub fn prepare_ordinary_signing_plan(
        &self,
        draft: &ActionDraft,
        admission: &V2AuthenticatedAdmission,
        dependencies: &[V2OrdinaryDependency<'_>],
        view: &ProjectionView,
    ) -> Result<V2OrdinarySigningPlan, ChatError> {
        self.verify_unsigned_authority(
            draft.spec_data(),
            draft.signer_data(),
            admission,
            dependencies,
        )?;
        self.require_view_spec(draft.spec_data(), view)?;
        if !self
            .admissions
            .iter()
            .any(|a| Arc::ptr_eq(&a.core, &admission.core))
            || !admission.core.open_origin.live()
        {
            return Err(ChatError::Scope);
        }
        // Allocate only before the final negative/monotone claim. The host prepaid the new
        // Plan metadata and all Draft/spec/verification copies independently of replay1MiB.
        let identity = Arc::new(PlanIdentity);
        let facts = PlanFacts {
            draft: draft.observe_original(),
            action: draft.action_id_data(),
            signer: *draft.signer_data(),
            admission: admission.core.body.admission_id(),
            group: self.group.clone(),
            view: view.life.clone(),
            open_origin: admission.core.open_origin.clone(),
        };
        self.live()?;
        facts.live()?;
        let claim = draft.claim_signing_plan()?;
        Ok(V2OrdinarySigningPlan {
            facts,
            work: PlanWork::Prepared,
            identity,
            _claim: claim,
        })
    }
}
impl V2OrdinarySigningPlan {
    /// Original selected Admission DATA, including after closure. This does not
    /// establish current OPEN, native/source custody, save or publication rights.
    pub fn admission_id_data(&self) -> crate::chat::owner_facts::OwnerFactId {
        self.facts.admission
    }
    pub fn observe_original(&self) -> OriginalV2OrdinarySigningPlanObservation {
        OriginalV2OrdinarySigningPlanObservation {
            identity: Arc::downgrade(&self.identity),
            draft: self.facts.draft.clone(),
            action: self.facts.action,
            signer: self.facts.signer,
            admission: self.facts.admission,
        }
    }
    /// Historical exact-original DATA comparison, usable after Stop. Root/Sync must compare
    /// their original source/UI generation too before any positive cached-raw reuse or key.
    pub fn require_original_data(
        &self,
        original: &OriginalV2OrdinarySigningPlanObservation,
        draft: &ActionDraft,
    ) -> Result<(), ChatError> {
        if !Weak::ptr_eq(&original.identity, &Arc::downgrade(&self.identity))
            || original.action != self.facts.action
            || original.signer != self.facts.signer
            || original.admission != self.facts.admission
        {
            return Err(ChatError::Scope);
        }
        self.facts.draft.require_same_original(&original.draft)?;
        self.facts.require_draft(draft, draft.signer_data())
    }
    /// No model/signature revalidation or allocation occurs here. Acquire the SAME group
    /// reader, then repeat exact Draft/full key and original negative checks INSIDE it.
    /// Remove selection may proceed; its writer completion waits until this guard/raw cache
    /// drains. A caller must hold its separate real native/Source/UI guard as well.
    pub fn admit_signing<'a>(
        &'a mut self,
        draft: &'a ActionDraft,
        full_device_key: &[u8; 32],
    ) -> Result<V2OrdinarySigningGuard<'a>, ChatError> {
        self.facts.require_draft(draft, full_device_key)?;
        self.facts.live()?;
        let reader = self
            .facts
            .group
            .gate
            .try_read()
            .map_err(|_| ChatError::Scope)?;
        self.facts.require_draft(draft, full_device_key)?;
        self.facts.live()?;
        Ok(V2OrdinarySigningGuard {
            _facts: &self.facts,
            work: &mut self.work,
            draft,
            _reader: reader,
        })
    }
    /// Existing raw DATA even after source/OPEN closure. This never acquires new admission,
    /// enters a key, mutates a Plan or fabricates a missing result. Keep its actual packet fee.
    pub fn borrow_cached_raw64_data<'a>(
        &'a self,
        original: &OriginalV2OrdinarySigningPlanObservation,
        draft: &ActionDraft,
    ) -> Result<Option<&'a [u8; 64]>, ChatError> {
        self.require_original_data(original, draft)?;
        Ok(match &self.work {
            PlanWork::RawCached(raw) => Some(raw),
            _ => None,
        })
    }
    /// Pure historical completion OUTSIDE writers. Caller must cache the resulting whole
    /// action before late negatives. No key, origin renewal, SDK, encoding variant or issuer.
    pub fn complete_cached_raw64_data(
        &self,
        original: &OriginalV2OrdinarySigningPlanObservation,
        draft: &ActionDraft,
    ) -> Result<SignedAction, ChatError> {
        let raw = self
            .borrow_cached_raw64_data(original, draft)?
            .ok_or(ChatError::Dependency)?;
        draft.complete_raw64(*raw)
    }
    /// Additional named fixed Plan/identity/weak/guard/result metadata. Draft's separate
    /// complete_raw64 inventory, actual original admission/control tails, Root Source/UI,
    /// SDK/signer internals, ledger and outbox payments remain independent obligations.
    pub const fn metadata_layout_bytes() -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of::<PlanIdentity>()
            + 4 * std::mem::size_of::<OriginalV2OrdinarySigningPlanObservation>()
            + 2 * std::mem::size_of::<V2OrdinarySigningGuard<'static>>()
            + 2 * std::mem::size_of::<V2OrdinaryKeyCall<'static>>()
            + 12 * std::mem::size_of::<usize>()
    }
}
impl OriginalV2OrdinarySigningPlanObservation {
    pub fn require_same_original(&self, other: &Self) -> Result<(), ChatError> {
        if !Weak::ptr_eq(&self.identity, &other.identity)
            || self.action != other.action
            || self.signer != other.signer
            || self.admission != other.admission
        {
            return Err(ChatError::Scope);
        }
        self.draft.require_same_original(&other.draft)
    }
}
impl<'a> V2OrdinarySigningGuard<'a> {
    /// Full signer of the exact admitted Draft, already compared with the caller's full
    /// device key by `admit_signing`. DATA only: a Sync signer compares it with its retained
    /// key BEFORE `begin_key`, so a wrong-key/replacement device refuses while the Plan is
    /// still Prepared instead of consuming the one key entry. Grants no authority.
    pub fn signer_data(&self) -> &[u8; 32] {
        self.draft.signer_data()
    }
    /// The admitted Draft's immutable spec, as DATA. A Sync signer compares its presented
    /// time and nonce with the request's once-only draw cell BEFORE `begin_key`, so a Draft
    /// that did not come from that exact draw refuses without consuming the key entry.
    pub fn spec_data(&self) -> &crate::chat::ActionSpec {
        self.draft.spec_data()
    }
    pub fn cached_raw64_data(&self) -> Option<&[u8; 64]> {
        match &*self.work {
            PlanWork::RawCached(raw) => Some(raw),
            _ => None,
        }
    }
    /// Mark BEFORE the real key operation. A second entry, including after a lost/unwound
    /// return with no raw bytes, refuses. No caller success flag changes this phase.
    pub fn begin_key(self) -> Result<V2OrdinaryKeyCall<'a>, ChatError> {
        if !matches!(self.work, PlanWork::Prepared) {
            return Err(ChatError::AuthorityConflict);
        }
        *self.work = PlanWork::KeyEntered;
        Ok(V2OrdinaryKeyCall { admitted: self })
    }
}
impl<'a> V2OrdinaryKeyCall<'a> {
    pub fn signing_bytes_data(&self) -> &[u8] {
        self.admitted.draft.signing_bytes_data()
    }
    /// Infallibly retain the raw result BEFORE any callback return/postcheck. The caller
    /// keeps the returned admitted reader until its outer owning packet cache is also safe.
    /// Raw bytes remain unverified DATA until pure outside complete_raw64 succeeds.
    pub fn cache_raw64(self, raw: [u8; 64]) -> V2OrdinarySigningGuard<'a> {
        *self.admitted.work = PlanWork::RawCached(raw);
        self.admitted
    }
}
