//! ORDINARY-member authoring and signing lane of the whole original Host.
//!
//! This is the founder-as-ordinary-member vertical of the Chat V2 member path
//! (`docs/communication-recovery/CHAT-V2-ORDINARY-MEMBER-PATH.md`). The Profile founder is
//! also a member with its own founder Admission; its ordinary Create/reply, own Edit/Delete and
//! Join are signed here by the SAME retained MLS device of this host's whole original node,
//! under a Core `V2OrdinarySigningPlan` from a general verifier. Nothing here touches the
//! founder A/I/S/B certification lane, a governing Header, an owner Append, a channel secret,
//! a sender ratchet, the network or the member outbox. The output is a cached
//! [`MemberSignedOriginal`]: DATA custody that grants no save or publication right.
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
use super::projection::{BoundV2TextFounderAnchor, V2TextProjectionDomain};
use super::*;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::persistence::v2_member_outbox::MemberSignedOriginal;
use crate::store::persistence::v2_text::{PreparedV2TextParentCapture, V2TextFounderAnchorData};
use crate::store::ServerStore;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE;
use catcoms_replication::chat::{
    ActionDraft, ActionId, ActionSpec, Body, ChatLedger, LedgerLimits,
    OriginalV2OrdinarySigningPlanObservation, ProjectionView, Realm, SignedAction,
    V2AuthenticatedAdmission, V2GeneralPrefixVerifier, V2OrdinaryDependency, V2OrdinarySigningPlan,
    MAX_ACTION_BYTES, MAX_GROUP_ID_BYTES, MAX_PARENTS, MAX_TEXT_BYTES,
};
use catcoms_rt::{CryptoRngCore, MeshTransport};
use catcoms_sync::owner_v2::V2MemberDraftInputs;
use std::collections::VecDeque;

/// Bounded live channel views per lane. Refuse beyond it; a view with a live Plan is never
/// evicted (that would revoke the Plan's channel lifetime under an in-flight request).
const MAX_MEMBER_CHANNELS: usize = 8;
/// Tokens finished (saved or abandoned) in this host lifetime that may not author again.
/// Bounded; cross-restart and older-token dedupe is the member outbox's exact token lookup,
/// which the request composer must run before authoring.
const MAX_FINISHED_TOKENS: usize = 64;

const MEMBER_CHANNEL_LAYOUT_BYTES: usize = std::mem::size_of::<MemberChannel>() + 4 * 1024;

/// Lane invoice, reserved BEFORE the lane/domain/capture/verifier allocations: the general
/// verifier's replay math and named metadata, the bounded channel ledgers/views and finished
/// tokens, and the lane layout. The member domain and parent capture pay their own fees.
/// A logical inventory, not an allocator/RSS measurement or proof that a configured cap fits.
pub(super) const V2_MEMBER_LANE_BYTES: usize = REPLAY_MATH_PRIVATE_INVOICE
    + V2GeneralPrefixVerifier::metadata_layout_bytes()
    + MAX_MEMBER_CHANNELS * MEMBER_CHANNEL_LAYOUT_BYTES
    + std::mem::size_of::<V2MemberLane>()
    + MAX_FINISHED_TOKENS * 32
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
pub(in crate::store) struct V2MemberSignRequest<'a> {
    pub(in crate::store) token: [u8; 32],
    pub(in crate::store) channel: u128,
    pub(in crate::store) body: &'a Body,
    pub(in crate::store) parents: &'a [ActionId],
    /// The exact required dependency set (parents plus Edit/Delete/Join target), in any
    /// order. Caller-owned DATA loans: Core verifies every signature, delegation and that each
    /// names a CURRENT Admission of this verifier. Unused extras or a missing one refuse.
    pub(in crate::store) dependencies: &'a [V2MemberDependency<'a>],
}

/// One dependency loan: an already-signed action and the Admission it was signed under.
pub(in crate::store) struct V2MemberDependency<'a> {
    pub(in crate::store) action: &'a SignedAction,
    pub(in crate::store) admission: OwnerFactId,
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

/// The ONE parent canonicalisation for a member request: sort and dedup, refuse more than
/// MAX_PARENTS, and a Create must name exactly its reply target. The member outbox's lookup
/// request DATA must be built from the signed spec (which this produced), so digests agree.
fn canonical_member_parents(body: &Body, parents: &[ActionId]) -> Result<Vec<ActionId>, AppError> {
    if parents.len() > MAX_PARENTS {
        return Err(refused());
    }
    let mut canonical = parents.to_vec();
    canonical.sort();
    canonical.dedup();
    if let Body::Create { reply_to, .. } = body {
        if canonical.as_slice() != reply_to.as_slice() {
            return Err(refused());
        }
    }
    // Exact capacity: ActionDraft refuses spare parent backing above MAX_PARENTS.
    canonical.shrink_to_fit();
    Ok(canonical)
}

/// Allocation-free set equality of a stored canonical list and a fresh request's list.
fn same_parent_set(canonical: &[ActionId], given: &[ActionId]) -> bool {
    given.len() <= MAX_PARENTS
        && given.iter().all(|p| canonical.binary_search(p).is_ok())
        && canonical.iter().all(|p| given.contains(p))
}

/// What the request has produced so far. Never serialized; never authority.
enum SignedState {
    /// No completed original yet (draw/Draft/Plan/key work may be in progress).
    Pending,
    /// The exact original, completed outside writers from the Plan's cached raw64.
    Complete(MemberSignedOriginal),
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
    fee: CatalogReadReservation,
}

impl PendingMemberRequest {
    fn prepare(store: &ServerStore, request: &V2MemberSignRequest<'_>) -> Result<Self, AppError> {
        // Shape checks on borrowed DATA: nothing is copied or hashed before the fee.
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
            fee,
        })
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
            self.signed = SignedState::Complete(value);
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
/// anchor/capture, the member-role domain, and the lane fee LAST.
pub(super) struct V2MemberLane {
    request: Option<Box<PendingMemberRequest>>,
    finished: VecDeque<[u8; 32]>,
    channels: Vec<MemberChannel>,
    verifier: Option<V2GeneralPrefixVerifier>,
    bound: Option<BoundV2TextFounderAnchor>,
    anchor: Option<V2TextFounderAnchorData>,
    capture: PreparedV2TextParentCapture,
    domain: Arc<V2TextProjectionDomain>,
    group: Vec<u8>,
    /// This source's own Admission (the founder's genesis Admission in this vertical).
    admission: OwnerFactId,
    fee: CatalogReadReservation,
}

impl V2MemberLane {
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
        let fee = pool.try_reserve(V2_MEMBER_LANE_BYTES)?;
        fee.validate(pool, V2_MEMBER_LANE_BYTES)?;
        // The member domain pays its own metadata and repeats original negatives across its
        // waits. Its role forbids minting the founder verifier on it, and vice versa.
        let domain = Arc::new(V2TextProjectionDomain::prepare_member(
            store, owner, observer,
        )?);
        let capture = PreparedV2TextParentCapture::prepare(store, owner.source())?;
        let genesis = owner.source().data();
        let group_ref = genesis.context.spec_data().group;
        if group_ref.is_empty() || group_ref.len() > MAX_GROUP_ID_BYTES {
            return Err(refused());
        }
        let mut group = Vec::with_capacity(group_ref.len());
        group.extend_from_slice(group_ref);
        Ok(Self {
            request: None,
            finished: VecDeque::with_capacity(MAX_FINISHED_TOKENS),
            channels: Vec::with_capacity(MAX_MEMBER_CHANNELS),
            verifier: None,
            bound: None,
            anchor: None,
            capture,
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
        self.fee.validate(pool, V2_MEMBER_LANE_BYTES)?;
        self.domain.validate_accounts(store)?;
        if let Some(request) = self.request.as_deref() {
            request.validate_accounts(store)?;
        }
        Ok(())
    }

    /// Negative only: future admissions refuse immediately. Never an acknowledgment that an
    /// already-admitted key operation has finished; `try_closed` polls the real gate.
    pub(super) fn cancel(&self) {
        self.domain.cancel();
    }
    pub(super) fn try_closed(&self) -> bool {
        self.domain.try_closed()
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

    /// Pure DATA completion plus exact-retry classification, with no positive gate.
    /// `Ok(true)`: this same request already holds (or lent) its completed original.
    fn retry_completed(&mut self, request: &V2MemberSignRequest<'_>) -> Result<bool, AppError> {
        let Some(pending) = self.request.as_deref_mut() else {
            if self.finished.contains(&request.token) {
                return Err(refused());
            }
            return Ok(false);
        };
        pending.complete_cached();
        if pending.token != request.token {
            // One pending request per lane: a different token waits for this one to finish.
            return Err(refused());
        }
        pending.require_same_request(request)?;
        Ok(!matches!(pending.signed, SignedState::Pending))
    }

    fn retry_request<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &mut V2TextOwnedCurrent<T, R>,
        request: &V2MemberSignRequest<'_>,
    ) -> Result<(), AppError> {
        if self.retry_completed(request)? {
            return Ok(());
        }
        self.validate_accounts(store)?;
        if self.request.is_none() {
            self.request = Some(Box::new(PendingMemberRequest::prepare(store, request)?));
        }
        {
            let owner_ref = owner.borrow_current()?;
            self.retry_verifier(store, &owner_ref)?;
        }
        let channel = self.request.as_deref().ok_or_else(refused)?.channel;
        let channel_index = self.ensure_channel(channel)?;

        let Self {
            request: slot,
            channels,
            verifier,
            capture,
            domain,
            group,
            admission,
            ..
        } = self;
        let pending = slot.as_deref_mut().ok_or_else(refused)?;
        let verifier = verifier.as_ref().ok_or_else(refused)?;

        // ONE Clock read and ONE nonce through the original node's own seams. No owner loan
        // is alive across this mutable node call; the cell caches each value first.
        if pending.draft.is_none() && pending.inputs.drawn_data().is_none() {
            owner
                .draw_ordinary_draft_inputs(store, &mut pending.inputs)
                .map_err(|_| refused())?;
        }
        if pending.draft.is_none() {
            let (presented_ms, nonce) = pending.inputs.drawn_data().ok_or_else(refused)?;
            let signer = owner
                .ordinary_member_signer_data(store)
                .map_err(|_| refused())?;
            let own = verifier.admission(*admission).map_err(|_| refused())?;
            if own.admission_data().spec().subject != signer {
                return Err(refused());
            }
            let realm = Realm::new(group, pending.channel).map_err(|_| refused())?;
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
                parents: pending.parents.clone(),
                body: pending.body.clone(),
            };
            pending.draft = Some(ActionDraft::new(spec, signer).map_err(|_| refused())?);
        }
        if pending.plan.is_none() {
            let draft = pending.draft.as_ref().ok_or_else(refused)?;
            let own = verifier.admission(*admission).map_err(|_| refused())?;
            // Resolve every dependency's Admission in THIS verifier; Core checks each real
            // signature/delegation and the exact minimal set. Order = Core's sorted set.
            let mut resolved: Vec<(&SignedAction, V2AuthenticatedAdmission)> =
                Vec::with_capacity(request.dependencies.len());
            for dependency in request.dependencies {
                resolved.push((
                    dependency.action,
                    verifier
                        .admission(dependency.admission)
                        .map_err(|_| refused())?,
                ));
            }
            resolved.sort_by_key(|(action, _)| action.id());
            let dependencies: Vec<V2OrdinaryDependency<'_>> = resolved
                .iter()
                .map(|(action, admission)| V2OrdinaryDependency::new(action, admission))
                .collect();
            let plan = verifier
                .prepare_ordinary_signing_plan(
                    draft,
                    &own,
                    &dependencies,
                    &channels[channel_index].view,
                )
                .map_err(|_| refused())?;
            pending.original = Some(plan.observe_original());
            pending.plan = Some(plan);
        }

        if !pending.has_cached_raw() {
            let selected = capture.selected_data().ok_or_else(refused)?;
            let resources = capture.resources().clone();
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
                // IO-bearing proof that the SAME generation-1 O is still selected, and the
                // node's full (SDK-consulting) source check: both BEFORE Core admission.
                selected.validate_under_current_guard(&guard)?;
                let signer = shared
                    .ordinary_member_signer_data(store)
                    .map_err(|_| refused())?;
                let signing = plan.admit_signing(draft, &signer).map_err(|_| refused())?;
                #[cfg(all(test, windows))]
                note_member_sign_dispatch();
                let signed = shared
                    .sign_ordinary_admitted(store, inputs, signing)
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
            drop(owner_ref);
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

    fn take_signed(&mut self, token: [u8; 32]) -> Result<MemberSignedOriginal, AppError> {
        let pending = self
            .request
            .as_deref_mut()
            .filter(|pending| pending.token == token)
            .ok_or_else(refused)?;
        pending.complete_cached();
        match std::mem::replace(&mut pending.signed, SignedState::Pending) {
            SignedState::Complete(value) => {
                pending.signed = SignedState::Lent(value.action().id());
                Ok(value)
            }
            other => {
                pending.signed = other;
                Err(refused())
            }
        }
    }

    fn restore_signed(
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
        match pending.signed {
            SignedState::Lent(id)
                if id == value.action().id() && admission == Some(value.admission()) =>
            {
                pending.signed = SignedState::Complete(value);
                Ok(())
            }
            _ => Err((value, refused())),
        }
    }

    fn finish(&mut self, token: [u8; 32], require_lent: bool) -> Result<(), AppError> {
        let pending = self
            .request
            .as_deref()
            .filter(|pending| pending.token == token)
            .ok_or_else(refused)?;
        let allowed = match pending.signed {
            SignedState::Lent(_) => require_lent,
            // Abandonment never discards a completed signed original by accident.
            SignedState::Pending => !require_lent,
            SignedState::Complete(_) => false,
        };
        if !allowed {
            return Err(refused());
        }
        if self.finished.len() == MAX_FINISHED_TOKENS {
            self.finished.pop_front();
        }
        self.finished.push_back(token);
        // Outside every writer: the Plan, Draft, cells and request fee drain here.
        drop(self.request.take());
        Ok(())
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// Author and sign ONE ordinary-member request as the founder acting as a member, or
    /// return the cached result for the exact same request. No founder Header/Append, save,
    /// projection install or publication happens here; `Ok` means the exact signed original
    /// is cached in this host for [`Self::take_member_signed_original`].
    ///
    /// Refuses (never authors) when: the host has not completed its empty promotion, founder
    /// Create state exists on this host, another token is pending, the token finished earlier
    /// in this host lifetime, the selected O is no longer generation 1, or the source/lock
    /// closed. A completed original stays retrievable after lock (historical DATA).
    pub(in crate::store) fn retry_sign_member_request(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2MemberSignRequest<'_>,
    ) -> Result<(), AppError> {
        // Pure DATA completion and exact retry BEFORE any positive gate.
        if let Some(lane) = self.member.as_deref_mut() {
            if lane.retry_completed(&request)? {
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
        if self.member.is_none() {
            let lane = {
                let owner = self.owner.borrow_current()?;
                let observer = self.observer.as_ref().ok_or_else(refused)?;
                V2MemberLane::prepare(store, &owner, observer)?
            };
            self.member = Some(Box::new(lane));
        }
        self.member
            .as_deref_mut()
            .ok_or_else(refused)?
            .retry_request(store, &mut self.owner, &request)
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
            .take_signed(token)
    }

    /// Put back the SAME original after an outbox refusal (prepare/admit refused, or an
    /// uncertain run whose lookup proved absence). Anything else is returned whole.
    pub(in crate::store) fn restore_member_signed_original(
        &mut self,
        token: [u8; 32],
        value: MemberSignedOriginal,
    ) -> Result<(), (MemberSignedOriginal, AppError)> {
        match self.member.as_deref_mut() {
            Some(lane) => lane.restore_signed(token, value),
            None => Err((value, refused())),
        }
    }

    /// The outbox reported SavedLocal (or AlreadySaved) for the lent original: release the
    /// request and its fee; the token cannot author again in this host lifetime.
    pub(in crate::store) fn finish_member_request_saved(
        &mut self,
        token: [u8; 32],
    ) -> Result<(), AppError> {
        self.member
            .as_deref_mut()
            .ok_or_else(refused)?
            .finish(token, true)
    }

    /// Drop a request that has no completed original (refused, terminal or uncertain key
    /// work). The token is retired for this host lifetime; it is never re-authored here.
    pub(in crate::store) fn abandon_member_request(
        &mut self,
        token: [u8; 32],
    ) -> Result<(), AppError> {
        self.member
            .as_deref_mut()
            .ok_or_else(refused)?
            .finish(token, false)
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
