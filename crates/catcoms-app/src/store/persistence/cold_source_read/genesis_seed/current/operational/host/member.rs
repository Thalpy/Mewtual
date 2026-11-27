//! ORDINARY-member authoring and signing lane of the whole original Host.
//!
//! This is the founder-as-ordinary-member vertical of the Chat V2 member path
//! (`docs/communication-recovery/CHAT-V2-ORDINARY-MEMBER-PATH.md`). The Profile founder is
//! also a member with its own founder Admission; its ordinary Create/reply, own Edit/Delete and
//! Join are signed here by the SAME retained MLS device of this host's whole original node,
//! under a Core `V2OrdinarySigningPlan` from a general verifier. Nothing here touches the
//! founder A/I/S/B certification lane, a governing Header, an owner Append, a channel secret,
//! a sender ratchet or the network. The output is a cached [`MemberSignedOriginal`]: DATA
//! custody that grants no save or publication right. Saving is the sibling `member_save`
//! driver's, which uses this lane only through its narrow save hooks: the stable outbox context
//! binding, the serialized save-source lifecycle (`admit_save_source`), exact saved-hit
//! reconciliation (`reconcile_saved`) and recovered-DATA adoption (`adopt_retained_original`).
//!
//! Trust and scope (why this is sound, and exactly where it stops):
//! - The general verifier's HOST expectation is the genesis anchor produced from the
//!   authenticated ORIGINAL pin (`SelectedV2TextHead::genesis_member_anchor_data`), never from
//!   a Profile/Header being verified.
//! - That producer, and every key cut below, prove under the Current group guard that the
//!   selected operational O is still the empty generation-1 promotion. While that holds the
//!   governing prefix is exactly genesis, so the verifier is current and the founder's
//!   Admission is the only current one. When O advances, signing refuses; later prefixes need
//!   a separate replay producer (the joined-member/governance owner).
//! - The lane is mutually exclusive with the founder Create lane on this host (the founder
//!   token lookup refuses while this lane exists, and this lane refuses while founder Create
//!   state exists), so two token namespaces never author for one request.
//!
//! Order for one request (one pending request per lane):
//! 1. Pure DATA first: if the Plan already holds a raw64, complete the original outside every
//!    writer before ANY positive gate, so a late lock/negative never strands it.
//! 2. Request fee reserved BEFORE copying the body/parents (borrowed inputs until then).
//! 3. Lane (once): fee, member-role domain, paid parent capture; under Current: capture the
//!    selected O, produce and bind the genesis anchor; outside writers: the general verifier.
//! 4. Clock/RNG drawn ONCE through the node's own seams into the request's draw cell, which
//!    also binds the request to that exact node. Retries never draw again.
//! 5. Immutable Draft (canonical parents), then ONE Plan for it with real dependencies.
//! 6. Key cut inside `with_v2_text_current` + group guard: source and selected-O proofs (IO)
//!    and the node's full source check run BEFORE Core `admit_signing`; after it only the
//!    node's allocation-free, SDK-free origin check and the one bounded key call run. The
//!    node caches raw64 in the Plan before returning; a retry reuses it with no key call.
//! 7. Completion outside writers into `MemberSignedOriginal::from_completed_plan`.
//!
//! The native/UI command credential is NOT checked here: like every other operational host
//! entry point this is Store-private and has no native dispatcher yet. The Store session lock
//! is enforced by `with_v2_text_current`. The primary's dispatcher must add its own command
//! generation check before calling in.
use super::super::owner::V2TextOwnerRef;
use super::member_save::{V2MemberSaveSource, V2MemberSaveSourceEnd};
use super::projection::{BoundV2TextFounderAnchor, V2TextProjectionDomain};
use super::*;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::persistence::v2_member_outbox::{
    canonical_member_parents, member_outbox_key, MemberDebtClass, MemberOutboxContextId,
    MemberSignedOriginal,
};
use crate::store::persistence::v2_member_transport::{FounderDebtOperation, FounderDebtOutcome};
use crate::store::persistence::v2_shared_history::disk::SavedSharedOriginal;
use crate::store::persistence::v2_text::{PreparedV2TextParentCapture, V2TextFounderAnchorData};
use crate::store::{NativeTextJoinedRowSelection, ServerStore};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE;
use catcoms_replication::chat::{
    ActionDraft, ActionId, ActionSpec, Body, ChatError, ChatLedger, LedgerLimits,
    OriginalV2OrdinarySigningPlanObservation, ProjectionView, Realm, SignedAction,
    V2AuthenticatedAdmission, V2FrozenAdmission, V2GeneralPrefixVerifier, V2LocalArchiveOriginal,
    V2OrdinaryDependency, V2OrdinarySigningPlan, MAX_ACTION_BYTES, MAX_GROUP_ID_BYTES, MAX_PARENTS,
    MAX_TEXT_BYTES,
};
use catcoms_rt::{
    CryptoRngCore, MeshTransport, NativeChatKeyEnds, NativeChatPreparedKeyCut,
    NativeChatSourceObservation, RequestCancellation,
};
use catcoms_sync::owner_v2::{V2MemberDraftInputs, MAX_V2_MEMBER_TRANSPORT_BYTES};
use catcoms_sync::{ClosedV2JoinedNode, PreparedV2JoinedPrefix};
use std::collections::VecDeque;
mod receive;
mod recovered;
use receive::FounderReceivedLedger;
pub(in crate::store) use recovered::{RecoveredMemberLane, RecoveredMemberSource};

/// Bounded live channel views per lane. Refuse beyond it; a view with a live Plan is never
/// evicted (that would revoke the Plan's channel lifetime under an in-flight request).
const MAX_MEMBER_CHANNELS: usize = 8;
/// Rolling positive cache for requests covered by Saved/SavedHit. Older saved-token
/// deduplication is the outbox's mandatory exact lookup before authoring.
const MAX_FINISHED_TOKENS: usize = 64;
/// Negative retirement is different: Absent lookup cannot remember cancellation. Never
/// evict these tokens during this lane incarnation. Reserve room before new work so an
/// admitted original can always be abandoned/forgotten. Normal saved traffic consumes no
/// permanent slot; restart-safe negative retirement needs a future durable record.
const MAX_UNSAVED_RETIREMENTS: usize = 64;
const FOUNDER_TRANSPORT_FEE: usize = 5 * MAX_V2_MEMBER_TRANSPORT_BYTES + 4096;

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn run_founder_member_debt(
        &mut self,
        layout: &V2CurrentLayoutStore,
        operation: FounderDebtOperation,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<FounderDebtOutcome, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        access
            .require(&self.origin)
            .map_err(|_| Refusal::Unavailable)?;
        if caller.is_cancelled() {
            return Err(Refusal::Cancelled);
        }
        let Some(governed) = self
            .governed_add
            .as_deref_mut()
            .filter(|add| add.installed.is_some())
        else {
            return match operation {
                FounderDebtOperation::Prepare => Ok(FounderDebtOutcome::Idle),
                FounderDebtOperation::Checkpoint(_) => Err(Refusal::Stale),
            };
        };
        let store = layout.checked_store().map_err(|_| Refusal::Unavailable)?;
        let lane = self.member.as_deref().ok_or(Refusal::Unavailable)?;
        lane.validate_accounts(store)
            .map_err(|_| Refusal::Unavailable)?;
        let verifier = lane.verifier.as_ref().ok_or(Refusal::Unavailable)?;
        check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
            .map_err(|_| Refusal::Stale)?;
        let fee = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or(Refusal::Unavailable)?
            .pending
            .pool
            .try_reserve(FOUNDER_TRANSPORT_FEE)
            .map_err(|_| Refusal::Busy)?;
        let mount = store
            .member_outbox_mount()
            .map_err(|_| Refusal::Unavailable)?;
        let context = super::member_save::member_outbox_context(
            &self
                .owner
                .borrow_current()
                .map_err(|_| Refusal::Unavailable)?,
            mount.capability().current_layout_data(),
        )
        .map_err(|_| Refusal::Unavailable)?;
        if lane.outbox_context().is_some_and(|id| id != context.id()) {
            return Err(Refusal::Stale);
        }
        let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
        match operation {
            FounderDebtOperation::Prepare => {
                let Some(batch) = mount
                    .read_publication_debt(store, context, &key, lane.admission, rng)
                    .map_err(|_| Refusal::Failed)?
                else {
                    return Ok(FounderDebtOutcome::Idle);
                };
                let item = &batch.items()[0];
                if item.class() != MemberDebtClass::Publishable {
                    return Err(Refusal::Stale);
                }
                let submission = self
                    .owner
                    .prepare_governed_member_submission(store, verifier, item)
                    .map_err(|_| Refusal::Failed)?;
                check_governed_member(&self.owner, governed, &lane.domain, verifier, store)
                    .map_err(|_| Refusal::Stale)?;
                if caller.is_cancelled() || !submission.is_current_cached() {
                    return Err(Refusal::Cancelled);
                }
                Ok(FounderDebtOutcome::Prepared {
                    submission,
                    batch,
                    fee,
                })
            }
            FounderDebtOperation::Checkpoint(batch) => {
                if caller.is_cancelled() {
                    return Err(Refusal::Cancelled);
                }
                mount
                    .checkpoint_publication_debt(store, context, &key, batch, rng, || {
                        !caller.is_cancelled()
                    })
                    .map(|saved| FounderDebtOutcome::Checkpointed(saved.front))
                    .map_err(|_| Refusal::Failed)
            }
        }
    }
}

fn check_governed_member<T: MeshTransport, R: CryptoRngCore>(
    owner: &V2TextOwnedCurrent<T, R>,
    governed: &mut super::governed_add::PendingGovernedAdd,
    domain: &V2TextProjectionDomain,
    verifier: &V2GeneralPrefixVerifier,
    store: &ServerStore,
) -> Result<(), AppError> {
    let original = owner.borrow_current()?;
    let resources = governed.parent.resources().clone();
    store.with_v2_text_current(&original, |admission| {
        let guard = store.lock_v2_text_current_group(admission, &resources)?;
        domain.require_current_source(admission)?;
        governed.require_selected_member_head(&guard, verifier.replay_data())
    })
}

const MEMBER_CHANNEL_LAYOUT_BYTES: usize = std::mem::size_of::<MemberChannel>() + 4 * 1024;
const WARM_FOUNDER_LANE_BYTES: usize = V2_MEMBER_LANE_BYTES
    + catcoms_sync::MAX_V2_GOVERNING_JOIN_STEPS
        * catcoms_replication::chat::V2FrozenAdmission::retained_layout_bytes();

/// Lane invoice, reserved BEFORE the lane/domain/capture/verifier allocations: the general
/// verifier's replay math and named metadata, the bounded channel ledgers/views and finished
/// tokens, and the lane layout. The member domain and parent capture pay their own fees.
/// A logical inventory, not an allocator/RSS measurement or proof that a configured cap fits.
pub(super) const V2_MEMBER_LANE_BYTES: usize = REPLAY_MATH_PRIVATE_INVOICE
    + V2GeneralPrefixVerifier::metadata_layout_bytes()
    + MAX_MEMBER_CHANNELS * MEMBER_CHANNEL_LAYOUT_BYTES
    + std::mem::size_of::<V2MemberLane>()
    + MAX_FINISHED_TOKENS * 32
    + MAX_UNSAVED_RETIREMENTS * 32
    + MAX_GROUP_ID_BYTES
    + 16 * 1024;

/// Request invoice, reserved BEFORE copying the body/parents: one Draft plus completion
/// overlap, Plan/observation metadata, the outside-writer authorization transient, the owned
/// request copy and the sorted dependency loans. Dependency SignedActions are borrowed from
/// the caller and paid by their owner. Same logical-inventory caveat as the lane.
pub(super) const V2_MEMBER_REQUEST_BYTES: usize = ActionDraft::private_layout_bytes()
    + V2OrdinarySigningPlan::metadata_layout_bytes()
    + V2GeneralPrefixVerifier::ordinary_authorization_temporary_layout_bytes()
    + std::mem::size_of::<PendingMemberRequest>()
    + std::mem::size_of::<MemberSignedOriginal>()
    + MAX_TEXT_BYTES
    + (MAX_PARENTS + 1)
        * (std::mem::size_of::<ActionId>()
            + std::mem::size_of::<V2OrdinaryDependency<'static>>()
            + std::mem::size_of::<V2AuthenticatedAdmission>()
            + std::mem::size_of::<usize>())
    + 4 * 1024;

/// One ordinary-member request, borrowed. Nothing is copied before the request fee.
///
/// `parents` are the request's EXACT original causal parents; the lane never derives them
/// from tips or ledger state. Create must name exactly its reply target (or nothing); Edit,
/// Delete and Join must name their target and may name further observed parents. The request
/// composer must persist the exact token, channel, body and parent list it sent: a retry
/// after restart with different parents is a conflict by design (member outbox lookup).
///
/// Public borrowed DATA for the native consumer of `prepare_member_save`; constructing one
/// grants nothing.
pub struct V2MemberSignRequest<'a> {
    pub token: [u8; 32],
    pub channel: u128,
    pub body: &'a Body,
    pub parents: &'a [ActionId],
    /// The exact required dependency set (parents plus Edit/Delete/Join target), in any
    /// order. Caller-owned DATA loans: Core verifies every signature, delegation and that each
    /// names a CURRENT Admission of this verifier. Unused extras or a missing one refuse.
    /// Native replies can load their exact Create from the original outbox or selected
    /// received archive. Native Edit and Delete can load their Create and one explicit
    /// original revision from the original local outbox when this slice is empty.
    pub dependencies: &'a [V2MemberDependency<'a>],
}

/// One dependency loan: an already-signed action and the Admission it was signed under.
pub struct V2MemberDependency<'a> {
    pub action: &'a SignedAction,
    pub admission: OwnerFactId,
}

impl std::fmt::Debug for V2MemberSignRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2MemberSignRequest")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2MemberDependency<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2MemberDependency").finish_non_exhaustive()
    }
}

/// Allocation-free set equality of a stored canonical list and a fresh request's list.
fn same_parent_set(canonical: &[ActionId], given: &[ActionId]) -> bool {
    given.len() <= MAX_PARENTS
        && given.iter().all(|p| canonical.binary_search(p).is_ok())
        && canonical.iter().all(|p| given.contains(p))
}

/// Shape bounds on the BORROWED request, applied on every call (first and retry) before any
/// copy, hash, fee or dependency resolution. Parents are bounded as supplied, before dedup:
/// at most 64 entries. Dependencies: at most the 64 parents plus one target.
fn require_request_shape(request: &V2MemberSignRequest<'_>) -> Result<(), AppError> {
    let text = match request.body {
        Body::Create { text, .. } | Body::Edit { text, .. } => text.len(),
        Body::Delete { .. } | Body::Join { .. } => 0,
    };
    if text > MAX_TEXT_BYTES
        || request.parents.len() > MAX_PARENTS
        || request.dependencies.len() > MAX_PARENTS + 1
    {
        return Err(refused());
    }
    Ok(())
}

/// What the request has produced so far. Never serialized; never authority.
enum SignedState {
    /// No completed original yet (draw/Draft/Plan/key work may be in progress).
    Pending,
    /// The exact original, completed outside writers from the Plan's cached raw64. Boxed:
    /// the custody is large next to the other states.
    Complete(Box<MemberSignedOriginal>),
    /// Moved to the outbox caller; only this exact ActionId under this Plan's Admission may
    /// come back. The request (and its fee) stays until SavedLocal or explicit abandonment.
    Lent(ActionId),
}

/// One request's whole custody. Retries reuse the SAME draw cell, Draft and Plan; a second
/// Draft for this request is never prepared. Field order is Drop order: the fee is LAST.
struct PendingMemberRequest {
    token: [u8; 32],
    channel: u128,
    body: Body,
    parents: Vec<ActionId>,
    inputs: V2MemberDraftInputs,
    draft: Option<ActionDraft>,
    plan: Option<V2OrdinarySigningPlan>,
    original: Option<OriginalV2OrdinarySigningPlanObservation>,
    signed: SignedState,
    /// Recovered-DATA custody only: the historical Admission of an original this lane ADOPTED
    /// from the Store's member-outbox cell (signed by an earlier host on the same Store). Such a
    /// request has no Draft or Plan, is always Lent, and can never be signed again.
    adopted: Option<OwnerFactId>,
    fee: CatalogReadReservation,
}

impl PendingMemberRequest {
    fn prepare(store: &ServerStore, request: &V2MemberSignRequest<'_>) -> Result<Self, AppError> {
        // Shape checks on borrowed DATA: nothing is copied or hashed before the fee.
        require_request_shape(request)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(V2_MEMBER_REQUEST_BYTES)?;
        fee.validate(pool, V2_MEMBER_REQUEST_BYTES)?;
        let parents = canonical_member_parents(request.body, request.parents)?;
        Ok(Self {
            token: request.token,
            channel: request.channel,
            body: request.body.clone(),
            parents,
            inputs: V2MemberDraftInputs::new(),
            draft: None,
            plan: None,
            original: None,
            signed: SignedState::Pending,
            adopted: None,
            fee,
        })
    }

    /// Adopt a retained original (recovered-DATA custody): the request DATA is copied from the
    /// original's OWN signed spec under the request fee, which must already be canonical. The
    /// request starts Lent with that exact ActionId; no Draft, Plan, Clock, RNG or key exists.
    fn adopt(
        store: &ServerStore,
        token: [u8; 32],
        original: &MemberSignedOriginal,
    ) -> Result<Self, AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(V2_MEMBER_REQUEST_BYTES)?;
        fee.validate(pool, V2_MEMBER_REQUEST_BYTES)?;
        let spec = original.action().spec();
        let parents = canonical_member_parents(&spec.body, &spec.parents)?;
        if parents != spec.parents {
            return Err(refused());
        }
        Ok(Self {
            token,
            channel: spec.realm.channel(),
            body: spec.body.clone(),
            parents,
            inputs: V2MemberDraftInputs::new(),
            draft: None,
            plan: None,
            original: None,
            signed: SignedState::Lent(original.action().id()),
            adopted: Some(original.admission()),
            fee,
        })
    }

    /// The historical Admission this request's original was signed under: the Plan's, or the
    /// adopted original's. DATA only; never proof that the Admission is still current.
    fn historical_admission(&self) -> Option<OwnerFactId> {
        match (&self.plan, self.adopted) {
            (Some(plan), None) => Some(plan.admission_id_data()),
            (None, Some(adopted)) => Some(adopted),
            _ => None,
        }
    }

    /// Same token must mean the same request. A changed channel, body or parent set is a
    /// conflict; the original custody is left untouched.
    fn require_same_request(&self, request: &V2MemberSignRequest<'_>) -> Result<(), AppError> {
        if self.token != request.token
            || self.channel != request.channel
            || self.body != *request.body
            || !same_parent_set(&self.parents, request.parents)
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Pure DATA: complete the original from the Plan's cached raw64, if any, OUTSIDE every
    /// writer. No key, Clock, RNG, source or positive gate. Legal after Stop/lock.
    fn complete_cached(&mut self) {
        if !matches!(self.signed, SignedState::Pending) {
            return;
        }
        let (Some(plan), Some(original), Some(draft)) = (&self.plan, &self.original, &self.draft)
        else {
            return;
        };
        if !matches!(plan.borrow_cached_raw64_data(original, draft), Ok(Some(_))) {
            return;
        }
        if let Ok(value) = MemberSignedOriginal::from_completed_plan(plan, original, draft) {
            self.signed = SignedState::Complete(Box::new(value));
        }
    }

    fn has_cached_raw(&self) -> bool {
        match (&self.plan, &self.original, &self.draft) {
            (Some(plan), Some(original), Some(draft)) => {
                matches!(plan.borrow_cached_raw64_data(original, draft), Ok(Some(_)))
            }
            _ => false,
        }
    }

    fn validate_accounts(&self, store: &ServerStore) -> Result<(), AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.fee.validate(pool, V2_MEMBER_REQUEST_BYTES)
    }

    fn prepare_draft(
        &mut self,
        group: &[u8],
        signer: [u8; 32],
        own: &V2AuthenticatedAdmission,
    ) -> Result<(), AppError> {
        if own.admission_data().spec().subject != signer {
            return Err(refused());
        }
        let (presented_ms, nonce) = self.inputs.drawn_data().ok_or_else(refused)?;
        let realm = Realm::new(group, self.channel).map_err(|_| refused())?;
        let labels = own
            .admission_data()
            .ordinary_labels(&realm)
            .map_err(|_| refused())?;
        let spec = ActionSpec {
            realm,
            basis: labels.basis,
            grant: labels.grant,
            nonce,
            presented_ms,
            parents: self.parents.clone(),
            body: self.body.clone(),
        };
        self.draft = Some(ActionDraft::new(spec, signer).map_err(|_| refused())?);
        Ok(())
    }

    fn sign_and_complete(
        &mut self,
        sign: impl FnOnce(
            &V2MemberDraftInputs,
            &ActionDraft,
            &mut V2OrdinarySigningPlan,
        ) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let result = if self.has_cached_raw() {
            Ok(())
        } else {
            sign(
                &self.inputs,
                self.draft.as_ref().ok_or_else(refused)?,
                self.plan.as_mut().ok_or_else(refused)?,
            )
        };
        self.complete_cached();
        result?;
        if matches!(self.signed, SignedState::Pending) {
            return Err(refused());
        }
        Ok(())
    }

    fn prepare_plan(
        &mut self,
        verifier: &V2GeneralPrefixVerifier,
        own: &V2AuthenticatedAdmission,
        dependencies: &[V2MemberDependency<'_>],
        (selected_reply, retired): (Option<&SavedSharedOriginal>, &[V2FrozenAdmission]),
        view: &ProjectionView,
    ) -> Result<(), AppError> {
        let draft = self.draft.as_ref().ok_or_else(refused)?;
        // Resolve each dependency's Admission in this verifier before Core
        // checks the real signature, delegation and exact minimal set.
        let mut resolved = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            let admission = match verifier.admission(dependency.admission) {
                Ok(current) => Some(current),
                Err(ChatError::Dependency)
                    if selected_reply.is_some_and(|selected| {
                        selected.admission == dependency.admission
                            && std::ptr::eq(selected.original.action(), dependency.action)
                    }) =>
                {
                    None
                }
                Err(_) => return Err(refused()),
            };
            resolved.push((dependency.action, admission));
        }
        resolved.sort_by_cached_key(|(action, _)| action.id());
        let dependencies = resolved
            .iter()
            .map(|(action, admission)| {
                if let Some(admission) = admission {
                    return Ok(V2OrdinaryDependency::new(action, admission));
                }
                let selected = selected_reply.ok_or_else(refused)?;
                let proven = V2LocalArchiveOriginal::from_trusted_local_selection(
                    action,
                    selected.admission,
                    selected.original.original_bytes(),
                )
                .map_err(|_| refused())?;
                verifier
                    .local_archive_reply_parent(proven, retired)
                    .map_err(|_| refused())
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let plan = verifier
            .prepare_ordinary_signing_plan(draft, own, &dependencies, view)
            .map_err(|_| refused())?;
        self.original = Some(plan.observe_original());
        self.plan = Some(plan);
        Ok(())
    }
}

/// One channel's ledger bound to THIS lane's own group view. The ledger stays empty here
/// (projection install is a separate consumer); it exists so the Plan binds the exact
/// channel-view lifetime of this member domain. It does not follow the real chat projection.
struct MemberChannel {
    view: ProjectionView,
    _ledger: ChatLedger,
    channel: u128,
}

/// The whole ordinary-member lane. Field order is Drop order: the request (Plan/Draft) first,
/// then channel views, then the verifier (whose Drop closes this lane's own group), then the
/// anchor/capture, the save-source lifecycle, the member-role domain, and the lane fee LAST.
pub(super) struct V2MemberLane {
    requests: MemberRequests,
    channels: Vec<MemberChannel>,
    received: Option<FounderReceivedLedger>,
    pub(super) retired_history: Vec<catcoms_replication::chat::V2FrozenAdmission>,
    verifier: Option<V2GeneralPrefixVerifier>,
    bound: Option<BoundV2TextFounderAnchor>,
    anchor: Option<V2TextFounderAnchorData>,
    capture: PreparedV2TextParentCapture,
    /// The serialized save-source lifecycle. Its `close` is part of [`Self::cancel`] and its
    /// drain part of [`Self::try_closed`]: an admitted save job keeps this lane's close pending
    /// through its IO. Its body holds the member domain's own paid negative, not this lane.
    save_source: V2MemberSaveSource,
    /// The stable member-outbox context this lane's saves are bound to, once the save driver
    /// derived it. A cache/comparison only: the driver derives it from historical DATA itself.
    outbox_context: Option<MemberOutboxContextId>,
    domain: Arc<V2TextProjectionDomain>,
    group: Vec<u8>,
    /// This source's own Admission (the founder's genesis Admission in this vertical).
    admission: OwnerFactId,
    fee: CatalogReadReservation,
}

/// Request custody for a single ordinary-member source. It retains the exact
/// signed original or its loan and never obtains a key, a new Source or IO.
pub(super) struct MemberRequests {
    request: Option<Box<PendingMemberRequest>>,
    finished: VecDeque<[u8; 32]>,
    retired_unsaved: Vec<[u8; 32]>,
}

fn member_channel_view<'a>(
    channels: &'a mut Vec<MemberChannel>,
    group: &[u8],
    channel: u128,
    projection: &catcoms_replication::chat::ProjectionGroupView,
) -> Result<&'a ProjectionView, AppError> {
    if let Some(index) = channels.iter().position(|view| view.channel == channel) {
        return Ok(&channels[index].view);
    }
    if channels.len() >= MAX_MEMBER_CHANNELS {
        return Err(refused());
    }
    let realm = Realm::new(group, channel).map_err(|_| refused())?;
    let mut ledger = ChatLedger::new(
        realm,
        LedgerLimits {
            max_actions: 1,
            max_action_bytes: MAX_ACTION_BYTES,
        },
    )
    .map_err(|_| refused())?;
    let mut view = ledger
        .begin_projection_view_in(projection)
        .map_err(|_| refused())?;
    if !view.try_ready() {
        return Err(refused());
    }
    channels.push(MemberChannel {
        view,
        _ledger: ledger,
        channel,
    });
    Ok(&channels.last().ok_or_else(refused)?.view)
}

/// The joined member uses the same request and save-source custody as the
/// founder. Its verifier and SDK stay in their original selected owners.
pub(in crate::store) struct JoinedMemberLane {
    pub(super) requests: MemberRequests,
    channels: Vec<MemberChannel>,
    save_source: V2MemberSaveSource,
    row: NativeTextJoinedRowSelection,
    fee: CatalogReadReservation,
}

/// A warm node loan and its checked selected Core prefix. Root retains the
/// surrounding physical and native admission through work on this borrow.
pub(in crate::store) struct JoinedMemberSource<'a, T: MeshTransport, R: CryptoRngCore> {
    pub(in crate::store) node: &'a mut ClosedV2JoinedNode<T, R>,
    pub(in crate::store) prefix: &'a PreparedV2JoinedPrefix,
    pub(in crate::store) view: &'a catcoms_replication::chat::ProjectionGroupView,
    pub(in crate::store) native: &'a NativeChatSourceObservation,
    pub(in crate::store) cut: &'a mut NativeChatPreparedKeyCut,
    pub(in crate::store) ends: &'a mut NativeChatKeyEnds,
}

impl JoinedMemberLane {
    pub(in crate::store) fn require_quiescent_for_add(&self) -> Result<(), AppError> {
        if self.requests.request.is_some() {
            return Err(refused());
        }
        self.save_source.require_open_idle()
    }

    pub(in crate::store) fn new(
        store: &ServerStore,
        row: NativeTextJoinedRowSelection,
    ) -> Result<Self, AppError> {
        if !row.is_current_cached() {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(V2_MEMBER_LANE_BYTES)?;
        fee.validate(pool, V2_MEMBER_LANE_BYTES)?;
        let save_source = V2MemberSaveSource::prepare_joined(pool, row.clone())?;
        Ok(Self {
            requests: MemberRequests::new(),
            channels: Vec::with_capacity(MAX_MEMBER_CHANNELS),
            save_source,
            row,
            fee,
        })
    }

    pub(super) fn outbox_context(&self) -> MemberOutboxContextId {
        MemberOutboxContextId::from_bytes(self.row.member_outbox_context_id_data())
    }

    pub(super) fn require_original_row(
        &self,
        row: &NativeTextJoinedRowSelection,
    ) -> Result<(), AppError> {
        if !self.row.same_original(row) || !self.row.is_current_cached() {
            return Err(refused());
        }
        Ok(())
    }

    pub(super) fn require_retained_original(
        &self,
        token: [u8; 32],
        original: &MemberSignedOriginal,
    ) -> Result<(), AppError> {
        if !self.row.is_current_cached() {
            return Err(refused());
        }
        self.requests
            .require_lent(token, original.action().id(), original.admission())?;
        Ok(())
    }

    pub(super) fn admit_save_source<T: MeshTransport, R: CryptoRngCore>(
        &self,
        facts: crate::store::persistence::v2_member_outbox::MemberAppendAdmissionFacts,
        node: &ClosedV2JoinedNode<T, R>,
    ) -> Result<V2MemberSaveSourceEnd, AppError> {
        if facts.context != self.outbox_context() || !self.row.is_current_cached() {
            return Err(refused());
        }
        let pending =
            self.requests
                .require_lent(*facts.token.as_bytes(), facts.action, facts.admission)?;
        let plan = pending.plan.as_ref().ok_or_else(refused)?;
        let _fence = node.admit_member_save(plan).map_err(|_| refused())?;
        self.save_source.issue()
    }

    pub(super) fn retry_request<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        (request, selected_reply): (&V2MemberSignRequest<'_>, Option<&SavedSharedOriginal>),
        source: &mut JoinedMemberSource<'_, T, R>,
    ) -> Result<(), AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.fee.validate(pool, V2_MEMBER_LANE_BYTES)?;
        if !self.row.is_current_cached() {
            return Err(refused());
        }
        if self.requests.retry_completed(request)? {
            return Ok(());
        }
        self.requests.ensure_pending(store, request)?;
        let pending = self.requests.request.as_deref_mut().ok_or_else(refused)?;
        pending.validate_accounts(store)?;
        let verifier = source
            .prefix
            .selected_member_verifier()
            .map_err(|_| refused())?;
        let own = verifier
            .admission(source.prefix.own_admission_data())
            .map_err(|_| refused())?;
        let group = &source.prefix.initial_profile_data().spec().group;
        if pending.draft.is_none() && pending.inputs.drawn_data().is_none() {
            source
                .node
                .draw_member_draft_inputs(&mut pending.inputs)
                .map_err(|_| refused())?;
        }
        if pending.draft.is_none() {
            let signer = source.node.member_signer_data().map_err(|_| refused())?;
            pending.prepare_draft(group, signer, &own)?;
        }
        if pending.plan.is_none() {
            let view =
                member_channel_view(&mut self.channels, group, pending.channel, source.view)?;
            pending.prepare_plan(
                verifier,
                &own,
                request.dependencies,
                (selected_reply, source.prefix.retired_history_data()),
                view,
            )?;
        }
        pending.sign_and_complete(|inputs, draft, plan| {
            source
                .node
                .sign_member_action_native(
                    inputs,
                    draft,
                    plan,
                    source.native,
                    source.cut,
                    source.ends,
                )
                .map(drop)
                .map_err(|_| refused())
        })
    }
}

impl V2MemberLane {
    pub(super) fn governed_add_verifier(
        &mut self,
    ) -> Result<&mut V2GeneralPrefixVerifier, AppError> {
        self.require_quiescent_for_add()?;
        self.verifier.as_mut().ok_or_else(refused)
    }

    pub(super) fn governed_add_replay(
        &self,
    ) -> Result<&catcoms_replication::chat::owner_state::transition::ReplayMath, AppError> {
        self.require_quiescent_for_add()?;
        Ok(self.verifier.as_ref().ok_or_else(refused)?.replay_data())
    }

    pub(super) fn require_quiescent_for_add(&self) -> Result<(), AppError> {
        if self.requests.request.is_some() {
            return Err(refused());
        }
        self.save_source.require_open_idle()
    }

    /// Reuse this lane's one verifier and actual selected G1 parent. No second
    /// prefix/domain is constructed for the invite producer. This phase does
    /// not sign or perform IO writes; key admission requires its later journal.
    pub(super) fn require_invite_parent(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        governed: Option<&mut super::governed_add::PendingGovernedAdd>,
    ) -> Result<(), AppError> {
        self.with_invite_parent(store, owner, governed, |_, _| Ok(()))
    }
    /// The isolated invitation journal stays under the same actual Current4
    /// and selected G1 proof. It mutates no Head/O/H/WAL/index object and never
    /// escapes this synchronous original RuntimeLoan as detached work.
    pub(super) fn with_invite_parent<U>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        governed: Option<&mut super::governed_add::PendingGovernedAdd>,
        work: impl FnOnce(
            &crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard<'_, '_>,
            Option<&V2GeneralPrefixVerifier>,
        ) -> Result<U, AppError>,
    ) -> Result<U, AppError> {
        self.validate_accounts(store)?;
        self.retry_verifier(store, owner)?;
        let verifier = self.verifier.as_ref().ok_or_else(refused)?;
        let admission = verifier.admission(self.admission).map_err(|_| refused())?;
        let original = owner.source().data();
        if !admission.is_open_data()
            || admission.admission_data().spec().subject != original.context.spec_data().signer
        {
            return Err(refused());
        }
        if let Some(governed) = governed {
            self.require_quiescent_for_add()?;
            let resources = governed.parent.resources().clone();
            return store.with_v2_text_current(owner, |current| {
                let guard = store.lock_v2_text_current_group(current, &resources)?;
                self.domain.require_current_source(current)?;
                governed.require_selected_member_head(&guard, verifier.replay_data())?;
                let result = work(&guard, Some(verifier))?;
                governed.require_selected_member_head(&guard, verifier.replay_data())?;
                self.domain.require_current_source(current)?;
                Ok(result)
            });
        }
        if verifier.replay_data().mls_epoch_data() != 0
            || verifier.replay_data().header().id() != original.header.id()
        {
            return Err(refused());
        }
        let selected = self.capture.selected_data().ok_or_else(refused)?;
        let resources = self.capture.resources().clone();
        store.with_v2_text_current(owner, |current| {
            let guard = store.lock_v2_text_current_group(current, &resources)?;
            self.domain.require_current_source(current)?;
            guard.require_governed_native_invite_parent()?;
            selected.validate_idle_genesis_member_under_current_guard(&guard)?;
            let result = work(&guard, None)?;
            guard.require_governed_native_invite_parent()?;
            selected.validate_idle_genesis_member_under_current_guard(&guard)?;
            self.domain.require_current_source(current)?;
            Ok(result)
        })
    }

    pub(super) fn selected_invite_verifier(&self) -> Result<&V2GeneralPrefixVerifier, AppError> {
        self.verifier.as_ref().ok_or_else(refused)
    }
    fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        observer: &V2TextSourceNegativeObserver,
    ) -> Result<Self, AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(WARM_FOUNDER_LANE_BYTES)?;
        fee.validate(pool, WARM_FOUNDER_LANE_BYTES)?;
        // The member domain pays its own metadata and repeats original negatives across its
        // waits. Its role forbids minting the founder verifier on it, and vice versa.
        let domain = Arc::new(V2TextProjectionDomain::prepare_member(
            store, owner, observer,
        )?);
        // Independently paid: an issued end can outlive this lane while its job drains.
        let save_source = V2MemberSaveSource::prepare(pool, domain.negative())?;
        let capture = PreparedV2TextParentCapture::prepare(store, owner.source())?;
        let genesis = owner.source().data();
        let group_ref = genesis.context.spec_data().group;
        if group_ref.is_empty() || group_ref.len() > MAX_GROUP_ID_BYTES {
            return Err(refused());
        }
        let mut group = Vec::with_capacity(group_ref.len());
        group.extend_from_slice(group_ref);
        Ok(Self {
            requests: MemberRequests::new(),
            channels: Vec::with_capacity(MAX_MEMBER_CHANNELS),
            received: None,
            retired_history: Vec::with_capacity(catcoms_sync::MAX_V2_GOVERNING_JOIN_STEPS),
            verifier: None,
            bound: None,
            anchor: None,
            capture,
            save_source,
            outbox_context: None,
            domain,
            group,
            admission: genesis.admission.admission_id(),
            fee,
        })
    }

    fn validate_accounts(&self, store: &ServerStore) -> Result<(), AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.fee.validate(pool, WARM_FOUNDER_LANE_BYTES)?;
        self.domain.validate_accounts(store)?;
        if let Some(request) = self.requests.request.as_deref() {
            request.validate_accounts(store)?;
        }
        Ok(())
    }

    /// Negative only: future key admissions AND save-source issuance refuse immediately. Never
    /// an acknowledgment that already admitted work has finished; `try_closed` polls the real
    /// gates.
    pub(super) fn cancel(&self) {
        self.domain.cancel();
        self.save_source.close();
    }
    /// The lane's real close completion: the member domain's group barrier AND every issued
    /// save-source end dropped (its job finished its IO and cleanup). Never blocks.
    pub(super) fn try_closed(&self) -> bool {
        self.domain.try_closed() && self.save_source.drained()
    }

    /// Save-source lifecycle, for regressions only.
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(super) fn save_source_for_test(&self) -> &V2MemberSaveSource {
        &self.save_source
    }

    /// The bound stable outbox context, if the save driver bound one. DATA only.
    pub(super) fn outbox_context(&self) -> Option<MemberOutboxContextId> {
        self.outbox_context
    }

    /// Bind (once) the stable outbox context this lane's saves use. A different context is
    /// refused: one lane saves into exactly one member outbox.
    pub(super) fn bind_outbox_context(
        &mut self,
        context: MemberOutboxContextId,
    ) -> Result<(), AppError> {
        match self.outbox_context {
            Some(bound) if bound != context => Err(refused()),
            _ => {
                self.outbox_context = Some(context);
                Ok(())
            }
        }
    }

    /// FINAL-EDGE source admission (no IO, no allocation): issue this lane's save-source end
    /// for exactly these facts. Refuses unless the stable context is this lane's own, the
    /// pending request is `token` Lent with exactly `action`, `admission` is that request's
    /// historical Admission (its Plan's, or the adopted original's), and the member domain's
    /// original source negative is live. Historical labels alone are not present authority;
    /// the later joined-member source must additionally enforce that Admission's actual
    /// current lifetime here.
    pub(super) fn admit_save_source(
        &self,
        context: MemberOutboxContextId,
        token: [u8; 32],
        action: ActionId,
        admission: OwnerFactId,
    ) -> Result<V2MemberSaveSourceEnd, AppError> {
        if self.outbox_context != Some(context) {
            return Err(refused());
        }
        self.requests.require_lent(token, action, admission)?;
        self.domain.validate_negative()?;
        // The serialized lifecycle re-checks the same negative under its own lock.
        self.save_source.issue()
    }

    /// Exact saved-hit reconciliation: the member outbox holds `saved` for `token`. Acts ONLY
    /// on this lane's pending request for exactly that token and never touches other work.
    /// - no such request: `Ok(None)` (the receipt stands on its own);
    /// - Lent: retire it as saved; `Some(true)` when the lent original is the stored one,
    ///   `Some(false)` when the store holds an EARLIER original of the same request;
    /// - Pending or Complete: complete any cached raw as DATA first, then retire it as saved,
    ///   but only when `spec` (the stored original's own signed spec) is this exact request.
    ///   The in-memory original, if any, is retired; only the stored entry may be published.
    pub(super) fn reconcile_saved(
        &mut self,
        token: [u8; 32],
        saved: ActionId,
        spec: Option<&ActionSpec>,
    ) -> Result<Option<bool>, AppError> {
        self.requests.reconcile_saved(token, saved, spec)
    }

    /// Whether this lane's member source can still admit anything: the member domain's
    /// original source negative is live and the lane was not cancelled. Cached negatives only.
    pub(super) fn require_live_source(&self) -> Result<(), AppError> {
        self.domain.validate_negative()
    }

    /// Recovered-DATA custody across host replacement: adopt a retained original from the
    /// Store's member-outbox cell as this lane's Lent request, so the next final edge can issue
    /// a source end for it without signing again. Idempotent for the request it already holds.
    /// Refuses while another request is pending, for a token this lane already finished, and
    /// for an original that is not this lane's own device, group and source Admission or does
    /// not verify. Requiring the lane's OWN Admission keeps the final-edge Admission check from
    /// being circular: the adopted label must equal this source's Admission, not merely itself.
    pub(super) fn adopt_retained_original(
        &mut self,
        store: &ServerStore,
        token: [u8; 32],
        original: &MemberSignedOriginal,
        signer: [u8; 32],
    ) -> Result<(), AppError> {
        let action = original.action();
        if let Some(pending) = self.requests.request.as_deref() {
            return match pending.signed {
                SignedState::Lent(lent) if pending.token == token && lent == action.id() => Ok(()),
                _ => Err(refused()),
            };
        }
        if !self.requests.may_start(token)
            || action.signer() != &signer
            || action.spec().realm.group() != self.group.as_slice()
            || original.admission() != self.admission
            || !action.verify()
        {
            return Err(refused());
        }
        self.validate_accounts(store)?;
        self.requests.request = Some(Box::new(PendingMemberRequest::adopt(
            store, token, original,
        )?));
        Ok(())
    }

    /// Anchor and verifier, once. The anchor comes from the authenticated original pin and
    /// the selected generation-1 O under the Current group guard; the verifier is built
    /// OUTSIDE every writer under the lane's paid model invoice.
    fn retry_verifier(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
    ) -> Result<(), AppError> {
        if self.verifier.is_some() {
            return Ok(());
        }
        if self.bound.is_none() {
            let resources = self.capture.resources().clone();
            let capture = &mut self.capture;
            let domain = &self.domain;
            let anchor = &mut self.anchor;
            let bound = &mut self.bound;
            store.with_v2_text_current(owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                domain.require_current_source(admission)?;
                let selected = capture.capture(&guard)?;
                if anchor.is_none() {
                    *anchor = Some(selected.genesis_member_anchor_data(&guard)?);
                }
                domain.bind_anchor_into(&guard, anchor, bound)
            })?;
        }
        let genesis = owner.source().data();
        let verifier = self.domain.general_verifier_from_genesis(
            self.bound.as_ref().ok_or_else(refused)?,
            genesis.profile,
            genesis.admission,
            genesis.header,
            genesis.state,
        )?;
        self.verifier = Some(verifier);
        Ok(())
    }

    /// Index of this channel's live view, binding a new one (bounded) if needed.
    fn ensure_channel(&mut self, channel: u128) -> Result<usize, AppError> {
        if let Some(index) = self.channels.iter().position(|c| c.channel == channel) {
            return Ok(index);
        }
        if self.channels.len() >= MAX_MEMBER_CHANNELS {
            return Err(refused());
        }
        let realm = Realm::new(&self.group, channel).map_err(|_| refused())?;
        let mut ledger = ChatLedger::new(
            realm,
            LedgerLimits {
                max_actions: 1,
                max_action_bytes: MAX_ACTION_BYTES,
            },
        )
        .map_err(|_| refused())?;
        let mut view = self.domain.begin_channel_view(&mut ledger)?;
        if !view.try_ready() {
            return Err(refused());
        }
        self.channels.push(MemberChannel {
            view,
            _ledger: ledger,
            channel,
        });
        Ok(self.channels.len() - 1)
    }

    fn retry_request<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &mut V2TextOwnedCurrent<T, R>,
        governed_add: Option<&mut super::governed_add::PendingGovernedAdd>,
        (request, selected_reply): (&V2MemberSignRequest<'_>, Option<&SavedSharedOriginal>),
        mut access: FounderAccess<'_>,
    ) -> Result<(), AppError> {
        if self.requests.retry_completed(request)? {
            return Ok(());
        }
        self.validate_accounts(store)?;
        self.requests.ensure_pending(store, request)?;
        {
            let owner_ref = owner.borrow_current()?;
            self.retry_verifier(store, &owner_ref)?;
        }
        let channel = self
            .requests
            .request
            .as_deref()
            .ok_or_else(refused)?
            .channel;
        let channel_index = self.ensure_channel(channel)?;

        let Self {
            requests,
            channels,
            verifier,
            capture,
            domain,
            group,
            admission,
            retired_history,
            ..
        } = self;
        let pending = requests.request.as_deref_mut().ok_or_else(refused)?;
        let verifier = verifier.as_ref().ok_or_else(refused)?;

        // ONE Clock read and ONE nonce through the original node's own seams. No owner loan
        // is alive across this mutable node call; the cell caches each value first.
        if pending.draft.is_none() && pending.inputs.drawn_data().is_none() {
            owner
                .draw_ordinary_draft_inputs(store, &mut pending.inputs)
                .map_err(|_| refused())?;
        }
        if pending.draft.is_none() {
            let signer = owner
                .ordinary_member_signer_data(store)
                .map_err(|_| refused())?;
            let own = verifier.admission(*admission).map_err(|_| refused())?;
            pending.prepare_draft(group, signer, &own)?;
        }
        if pending.plan.is_none() {
            let own = verifier.admission(*admission).map_err(|_| refused())?;
            pending.prepare_plan(
                verifier,
                &own,
                request.dependencies,
                (selected_reply, retired_history),
                &channels[channel_index].view,
            )?;
        }

        if !pending.has_cached_raw() {
            let installed = governed_add.filter(|add| add.installed.is_some());
            let resources = installed
                .as_ref()
                .map_or_else(|| capture.resources(), |add| add.parent.resources())
                .clone();
            let shared: &V2TextOwnedCurrent<T, R> = owner;
            let owner_ref = shared.borrow_current()?;
            let PendingMemberRequest {
                inputs,
                draft,
                plan,
                ..
            } = &mut *pending;
            let draft = draft.as_ref().ok_or_else(refused)?;
            let plan = plan.as_mut().ok_or_else(refused)?;
            let inputs: &V2MemberDraftInputs = inputs;
            let domain: &V2TextProjectionDomain = domain;
            let result = store.with_v2_text_current(&owner_ref, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                domain.require_current_source(admission)?;
                if let Some(installed) = installed {
                    installed.require_selected_member_head(&guard, verifier.replay_data())?;
                } else {
                    capture
                        .selected_data()
                        .ok_or_else(refused)?
                        .validate_idle_genesis_member_under_current_guard(&guard)?;
                }
                let signer = shared
                    .ordinary_member_signer_data(store)
                    .map_err(|_| refused())?;
                let signing = plan.admit_signing(draft, &signer).map_err(|_| refused())?;
                #[cfg(all(test, windows))]
                note_member_sign_dispatch();
                let signed = match &mut access {
                    FounderAccess::Library => shared.sign_ordinary_admitted(inputs, signing),
                    FounderAccess::Native(keys) => {
                        let (original, cut, ends) = keys.member_entry();
                        shared.sign_ordinary_admitted_native(inputs, signing, original, cut, ends)
                    }
                }
                .map_err(|_| refused())?;
                // The raw64 is already in the Plan. A close requested here (lock/Remove)
                // must still wait for this admitted reader; see the regression cut.
                #[cfg(all(test, windows))]
                cancel_domain_during_admitted_sign(domain);
                #[cfg(all(test, windows))]
                cancel_owner_during_admitted_sign(shared);
                drop(signed);
                drop(guard);
                Ok(())
            });
            // The non-owning source view has no guard or End; its loan ends at the last use above.
            // Cache the completed original BEFORE reporting any late negative.
            pending.complete_cached();
            result?;
        }
        pending.complete_cached();
        if matches!(pending.signed, SignedState::Pending) {
            // A raw that does not complete under the exact Draft/full key is terminal for
            // this request: never a second key call or a replacement Draft.
            return Err(refused());
        }
        Ok(())
    }
}

impl MemberRequests {
    fn new() -> Self {
        Self {
            request: None,
            finished: VecDeque::with_capacity(MAX_FINISHED_TOKENS),
            retired_unsaved: Vec::with_capacity(MAX_UNSAVED_RETIREMENTS),
        }
    }

    fn may_start(&self, token: [u8; 32]) -> bool {
        !self.finished.contains(&token)
            && !self.retired_unsaved.contains(&token)
            && self.retired_unsaved.len() < MAX_UNSAVED_RETIREMENTS
    }

    fn ensure_pending(
        &mut self,
        store: &ServerStore,
        request: &V2MemberSignRequest<'_>,
    ) -> Result<(), AppError> {
        if self.request.is_none() {
            self.request = Some(Box::new(PendingMemberRequest::prepare(store, request)?));
        }
        Ok(())
    }

    /// Pure DATA completion and exact retry. Bounds every call before reading
    /// retained data. An occupied slot belongs to its one original request.
    fn retry_completed(&mut self, request: &V2MemberSignRequest<'_>) -> Result<bool, AppError> {
        require_request_shape(request)?;
        let Some(pending) = self.request.as_deref_mut() else {
            return self
                .may_start(request.token)
                .then_some(false)
                .ok_or_else(refused);
        };
        pending.complete_cached();
        pending.require_same_request(request)?;
        Ok(!matches!(pending.signed, SignedState::Pending))
    }

    fn require_lent(
        &self,
        token: [u8; 32],
        action: ActionId,
        admission: OwnerFactId,
    ) -> Result<&PendingMemberRequest, AppError> {
        let pending = self
            .request
            .as_deref()
            .filter(|pending| pending.token == token)
            .ok_or_else(refused)?;
        if !matches!(pending.signed, SignedState::Lent(lent) if lent == action)
            || pending.historical_admission() != Some(admission)
        {
            return Err(refused());
        }
        Ok(pending)
    }

    pub(super) fn reconcile_saved(
        &mut self,
        token: [u8; 32],
        saved: ActionId,
        spec: Option<&ActionSpec>,
    ) -> Result<Option<bool>, AppError> {
        let Some(pending) = self
            .request
            .as_deref_mut()
            .filter(|pending| pending.token == token)
        else {
            return Ok(None);
        };
        pending.complete_cached();
        if !matches!(pending.signed, SignedState::Lent(_)) {
            let spec = spec.ok_or_else(refused)?;
            if pending.channel != spec.realm.channel()
                || pending.body != spec.body
                || pending.parents != spec.parents
            {
                return Err(refused());
            }
        }
        self.finish(token, FinishMode::SavedHit(saved)).map(Some)
    }

    pub(super) fn take_signed(
        &mut self,
        token: [u8; 32],
    ) -> Result<MemberSignedOriginal, AppError> {
        let pending = self
            .request
            .as_deref_mut()
            .filter(|pending| pending.token == token)
            .ok_or_else(refused)?;
        pending.complete_cached();
        match std::mem::replace(&mut pending.signed, SignedState::Pending) {
            SignedState::Complete(value) => {
                pending.signed = SignedState::Lent(value.action().id());
                Ok(*value)
            }
            other => {
                pending.signed = other;
                Err(refused())
            }
        }
    }

    /// Byte-exact: the same token, the exact lent ActionId, the Plan's Admission AND the
    /// Plan's own cached raw64 signature. An ActionId alone does not cover the signature.
    #[expect(
        clippy::result_large_err,
        reason = "A refused restore returns the same signed original and its payment instead of allocating or destroying it."
    )]
    pub(super) fn restore_signed(
        &mut self,
        token: [u8; 32],
        value: MemberSignedOriginal,
    ) -> Result<(), (MemberSignedOriginal, AppError)> {
        let Some(pending) = self
            .request
            .as_deref_mut()
            .filter(|pending| pending.token == token)
        else {
            return Err((value, refused()));
        };
        let admission = pending.plan.as_ref().map(|plan| plan.admission_id_data());
        let raw = match (&pending.plan, &pending.original, &pending.draft) {
            (Some(plan), Some(original), Some(draft)) => plan
                .borrow_cached_raw64_data(original, draft)
                .ok()
                .flatten()
                .copied(),
            _ => None,
        };
        match pending.signed {
            SignedState::Lent(id)
                if id == value.action().id()
                    && admission == Some(value.admission())
                    && raw.as_ref() == Some(value.action().signature()) =>
            {
                pending.signed = SignedState::Complete(Box::new(value));
                Ok(())
            }
            _ => Err((value, refused())),
        }
    }

    /// Retire the request and its fee. Unsaved negatives are permanent for this lane;
    /// saved tokens rely on exact durable lookup once their positive cache entry expires.
    /// Returns, for `Saved`, whether the outbox's saved original is the one this lane lent.
    pub(super) fn finish(&mut self, token: [u8; 32], mode: FinishMode) -> Result<bool, AppError> {
        let pending = self
            .request
            .as_deref_mut()
            .filter(|pending| pending.token == token)
            .ok_or_else(refused)?;
        // A raw cached by an interrupted cut becomes Complete before classification, so the
        // state below never hides a completed original behind `Pending`.
        pending.complete_cached();
        let same = match (&pending.signed, mode) {
            (SignedState::Lent(lent), FinishMode::Saved(saved) | FinishMode::SavedHit(saved)) => {
                *lent == saved
            }
            // An exact stored hit retires an in-memory original that never left this lane.
            (SignedState::Pending | SignedState::Complete(_), FinishMode::SavedHit(_)) => false,
            (SignedState::Lent(_), FinishMode::ForgetLent) => false,
            // Discarding an original that never left process memory publishes nothing. A
            // Pending request may hold a terminal unverifiable raw; that is dropped too.
            (SignedState::Pending | SignedState::Complete(_), FinishMode::Abandon) => false,
            _ => return Err(refused()),
        };
        if matches!(mode, FinishMode::Abandon | FinishMode::ForgetLent) {
            // New Pending/adopted requests reserve this slot before any key/RNG entry.
            // Final retirement cannot fail for pressure or allocate another buffer.
            debug_assert!(self.retired_unsaved.len() < MAX_UNSAVED_RETIREMENTS);
            self.retired_unsaved.push(token);
        } else {
            if self.finished.len() == MAX_FINISHED_TOKENS {
                self.finished.pop_front();
            }
            self.finished.push_back(token);
        }
        // Outside every writer: the Plan, Draft, cells and request fee drain here.
        drop(self.request.take());
        Ok(same)
    }
}

/// How a request leaves the lane. Never serialized.
#[derive(Clone, Copy)]
pub(super) enum FinishMode {
    /// The outbox reported SavedLocal/AlreadySaved; the receipt's ActionId.
    Saved(ActionId),
    /// An exact authoritative outbox hit for this token (any lane state); the stored ActionId.
    SavedHit(ActionId),
    /// No original was handed out (Pending) or it never left this lane (Complete).
    Abandon,
    /// The lent value was lost and the outbox lookup proved Absent.
    ForgetLent,
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Author and sign ONE ordinary-member request as the founder acting as a member, or
    /// return the cached result for the exact same request. No founder Header/Append, save,
    /// projection install or publication happens here; `Ok` means the exact signed original
    /// is cached in this host for [`Self::take_member_signed_original`].
    ///
    /// Refuses (never authors) when: the host has not completed its empty promotion, founder
    /// Create state exists on this host, another token is pending, the token was abandoned
    /// or forgotten (or remains in the positive saved cache), negative-retirement capacity
    /// is exhausted, the original selected head cannot be reproved, or the source/lock
    /// closed. A completed original stays retrievable after lock (historical DATA).
    #[cfg(all(test, windows))]
    pub(in crate::store) fn retry_sign_member_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2MemberSignRequest<'_>,
    ) -> Result<(), AppError> {
        self.retry_sign_member_request_with_access(layout, (request, None), FounderAccess::Library)
    }
    pub(in crate::store) fn retry_sign_member_request_with_access(
        &mut self,
        layout: &V2CurrentLayoutStore,
        (request, selected_reply): (V2MemberSignRequest<'_>, Option<&SavedSharedOriginal>),
        access: FounderAccess<'_>,
    ) -> Result<(), AppError> {
        access.require(&self.origin)?;
        // A malformed request never creates a lane (which would exclude founder Create).
        require_request_shape(&request)?;
        // Pure DATA completion and exact retry BEFORE any positive gate.
        if let Some(lane) = self.member.as_deref_mut() {
            if lane.requests.retry_completed(&request)? {
                return Ok(());
            }
        }
        if self.selected.is_none()
            || self.lookup.is_some()
            || self.request_preparation.is_some()
            || self.resident.is_some()
            || self.next_request_preparation.is_some()
        {
            return Err(refused());
        }
        let store = layout.checked_store()?;
        if let Some(lane) = self.member.as_deref_mut() {
            return lane.retry_request(
                store,
                &mut self.owner,
                self.governed_add.as_deref_mut(),
                (&request, selected_reply),
                access,
            );
        }
        // A NEW lane is installed only once its anchor is bound and its verifier exists. If
        // it fails earlier (for example the anchor refuses beside a crash-left founder WAL),
        // nothing was drawn or signed: the draw follows the verifier. Dropping it here keeps
        // the founder lane available, so this host is not wedged with neither lane able to
        // make progress. The dropped domain minted no verifier and closes on Drop.
        let mut lane = {
            let owner = self.owner.borrow_current()?;
            let observer = self.observer.as_ref().ok_or_else(refused)?;
            V2MemberLane::prepare(store, &owner, observer)?
        };
        let result = lane.retry_request(
            store,
            &mut self.owner,
            None,
            (&request, selected_reply),
            access,
        );
        if lane.verifier.is_some() {
            self.member = Some(Box::new(lane));
        }
        result
    }

    /// Move the exact completed original out for the member outbox's `prepare_append`. The
    /// request stays (Lent) with its fee; no new Draft can be authored for this token.
    pub(in crate::store) fn take_member_signed_original(
        &mut self,
        token: [u8; 32],
    ) -> Result<MemberSignedOriginal, AppError> {
        self.member
            .as_deref_mut()
            .ok_or_else(refused)?
            .requests
            .take_signed(token)
    }

    /// Put back the SAME original after an outbox refusal (prepare/admit refused, or an
    /// uncertain run whose lookup proved absence). Anything else is returned whole.
    #[expect(
        clippy::result_large_err,
        reason = "A refused restore must return the same whole signed original without an allocation at this recovery boundary."
    )]
    pub(in crate::store) fn restore_member_signed_original(
        &mut self,
        token: [u8; 32],
        value: MemberSignedOriginal,
    ) -> Result<(), (MemberSignedOriginal, AppError)> {
        match self.member.as_deref_mut() {
            Some(lane) => lane.requests.restore_signed(token, value),
            None => Err((value, refused())),
        }
    }

    /// The outbox reported SavedLocal or AlreadySaved for this token: release the request and
    /// its fee. The durable exact lookup prevents this token authoring again after the
    /// rolling positive cache evicts it; callers MUST perform that lookup. `saved_action` is the
    /// receipt's ActionId. Returns `true` when that is the original this lane lent. `false`
    /// means AlreadySaved named an EARLIER original (for example the same request signed
    /// under another nonce before a restart): the caller must drop its lent copy and never
    /// publish it; the outbox's stored entry is the only original that will be published.
    pub(in crate::store) fn finish_member_request_saved(
        &mut self,
        token: [u8; 32],
        saved_action: ActionId,
    ) -> Result<bool, AppError> {
        self.member
            .as_deref_mut()
            .ok_or_else(refused)?
            .requests
            .finish(token, FinishMode::Saved(saved_action))
    }

    /// Drop a request whose original was never written by the outbox, retiring the token
    /// for this host lifetime. The host cannot see outbox outcomes, so the CALLER must only
    /// abandon a Complete (restored) original after one of these:
    /// - `prepare_append` refused with TokenConflict, Foreign or IndexExhausted; or
    /// - an exact outbox lookup with the original request DATA returned Absent.
    ///
    /// It must NOT abandon after: `run()` Uncertain (recover, then look up: Hit means
    /// `finish_member_request_saved(receipt.action)`, Absent means restore or abandon);
    /// DuplicateAction (that ActionId is retained under another token, possibly submitted or
    /// Held; reconcile its existing receipt/debt and report the invariant breach, never a
    /// fresh unsaved action); AlreadySaved (use finish); Busy, Stale (including
    /// an admit refusal), NeedsRecovery, Incomplete, Payment or a `prepare_append` Failed read
    /// error (nothing stored; keep the original, retry);
    /// Pressure (keep it and retry; dropping it instead discards the user's message and the
    /// UI must then say "not saved"). A Pending request (refused, terminal or uncertain key
    /// work, nothing completed) may always be abandoned.
    pub(in crate::store) fn abandon_member_request(
        &mut self,
        token: [u8; 32],
    ) -> Result<(), AppError> {
        self.member
            .as_deref_mut()
            .ok_or_else(refused)?
            .requests
            .finish(token, FinishMode::Abandon)
            .map(|_| ())
    }

    /// The lent original was lost (a dropped outbox preparation or an unwind) AND the member
    /// outbox's exact token lookup proved Absent: retire the request so the lane is not
    /// wedged. Without that Absent proof the caller must not call this; the token is retired.
    pub(in crate::store) fn forget_lent_member_request(
        &mut self,
        token: [u8; 32],
    ) -> Result<(), AppError> {
        self.member
            .as_deref_mut()
            .ok_or_else(refused)?
            .requests
            .finish(token, FinishMode::ForgetLent)
            .map(|_| ())
    }

    /// The member lane for saving a RETAINED original (recovered-DATA custody), created with
    /// the same founder-lane exclusion as authoring when this host has none (a replacement host
    /// on the same Store). Unlike authoring, a new lane is installed without a verifier: it
    /// holds the adopted lent work and signs nothing until a later authoring call builds its
    /// verifier under the usual gates.
    pub(super) fn ensure_member_lane(
        &mut self,
        layout: &V2CurrentLayoutStore,
    ) -> Result<&mut V2MemberLane, AppError> {
        if self.member.is_none() {
            if self.selected.is_none()
                || self.lookup.is_some()
                || self.request_preparation.is_some()
                || self.resident.is_some()
                || self.next_request_preparation.is_some()
            {
                return Err(refused());
            }
            let store = layout.checked_store()?;
            let lane = {
                let owner = self.owner.borrow_current()?;
                let observer = self.observer.as_ref().ok_or_else(refused)?;
                V2MemberLane::prepare(store, &owner, observer)?
            };
            self.member = Some(Box::new(lane));
        }
        self.member.as_deref_mut().ok_or_else(refused)
    }
}

// Regression observations only: they count the real Root->node signer dispatch and close the
// member domain at the real post-raw cut while Core's reader is held. They grant nothing.
#[cfg(all(test, windows))]
thread_local! {
    static SIGN_DISPATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CANCEL_DURING_SIGN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CLOSED_DURING_SIGN: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static CANCEL_OWNER_DURING_SIGN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Revoke the whole original owner at the post-raw cut, so `with_v2_text_current`'s final
/// source check fails AFTER the raw is cached: the late-negative regression.
#[cfg(all(test, windows))]
fn cancel_owner_during_admitted_sign<T, R>(owner: &V2TextOwnedCurrent<T, R>) {
    if CANCEL_OWNER_DURING_SIGN.with(|armed| armed.replace(false)) {
        owner.cancel_original();
    }
}
#[cfg(all(test, windows))]
pub(in crate::store) fn arm_owner_cancel_during_sign_for_test() {
    CANCEL_OWNER_DURING_SIGN.with(|armed| armed.set(true));
}
#[cfg(all(test, windows))]
fn note_member_sign_dispatch() {
    SIGN_DISPATCHES.with(|n| n.set(n.get() + 1));
}
#[cfg(all(test, windows))]
fn cancel_domain_during_admitted_sign(domain: &V2TextProjectionDomain) {
    if CANCEL_DURING_SIGN.with(|armed| armed.replace(false)) {
        domain.cancel();
        CLOSED_DURING_SIGN.with(|closed| closed.set(Some(domain.try_closed())));
    }
}
#[cfg(all(test, windows))]
pub(in crate::store) fn member_sign_dispatches_for_test() -> usize {
    SIGN_DISPATCHES.with(std::cell::Cell::get)
}
/// Read-only access to the existing actual dispatch counter for cross-boundary
/// native-runtime regressions. No signer, source or native permission escapes.
#[cfg(all(test, windows))]
impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn native_member_sign_dispatches_for_test() -> usize {
        member_sign_dispatches_for_test()
    }
}
/// Arm a member-domain close at the post-raw cut of the NEXT admitted key operation. Read
/// whether that close had drained at the cut (it must not: Core's reader is still held)
/// with `member_close_drained_during_sign_for_test`.
#[cfg(all(test, windows))]
pub(in crate::store) fn arm_member_close_during_sign_for_test() {
    CLOSED_DURING_SIGN.with(|closed| closed.set(None));
    CANCEL_DURING_SIGN.with(|armed| armed.set(true));
}
#[cfg(all(test, windows))]
pub(in crate::store) fn member_close_drained_during_sign_for_test() -> Option<bool> {
    CLOSED_DURING_SIGN.with(std::cell::Cell::get)
}
