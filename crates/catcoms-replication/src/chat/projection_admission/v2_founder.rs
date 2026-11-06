//! The first closed V2 host profile: a sole founder, epoch zero, repeated ordinary Creates.
//!
//! This is a crypto/lineage verifier, not the App's storage or native authorization boundary.
//! HOST trust inputs must come from the original root pin and actual selected save receipts,
//! independently of the records checked here. There is no initializer from a hot snapshot,
//! no automatic fork winner and no externally supplied next-model setter. A cold source must
//! replay the complete signed lineage under its newly paid original source/group lifetime.
use super::*;
use crate::chat::acceptance_tree::{
    verify_inclusion, PreparedAppend, TreeContext, TreeHash, TreeTip,
};
use crate::chat::authority::SemanticConstraint;
use crate::chat::owner_bodies::{AcceptanceRecord, AppendBody};
use crate::chat::owner_facts::OwnerFactId;
use crate::chat::owner_header::{
    OwnerHeaderId, OwnerStateCommitment, OwnerTenureId, SignedOwnerHeader,
};
use crate::chat::owner_profile::{AdmissionBody, SignedOwnerProfile};
use crate::chat::owner_state::transition::{ReplayBody, ReplayMath, StepData};
use std::collections::BTreeSet;

/// Explicit trusted-HOST genesis selection DATA. A matching self-signed record is not root
/// provenance. The actual adapter constructs this from its independent original immutable
/// pin and covering genesis receipt, never by asking incoming records which root to trust.
pub struct FounderGenesisAnchorData {
    pub group: Vec<u8>,
    pub founder: [u8; 32],
    pub profile: OwnerFactId,
    pub admission: OwnerFactId,
    pub tenure: OwnerTenureId,
    pub header: OwnerHeaderId,
    pub state: OwnerStateCommitment,
}
impl std::fmt::Debug for FounderGenesisAnchorData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderGenesisAnchorData")
            .finish_non_exhaustive()
    }
}

/// Closed trust configuration, not a persisted/current/root capability or decoder output.
pub struct V2FounderExpectation {
    anchor: FounderGenesisAnchorData,
}
impl std::fmt::Debug for V2FounderExpectation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2FounderExpectation")
            .finish_non_exhaustive()
    }
}
impl V2FounderExpectation {
    /// Make an explicit HOST trust decision. Authenticating its provenance is the adapter's
    /// obligation. This constructor deliberately does not accept a Profile as its expectation.
    pub fn from_trusted_host(anchor: FounderGenesisAnchorData) -> Result<Self, ChatError> {
        if anchor.group.is_empty() || anchor.group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        Ok(Self { anchor })
    }
}

/// Explicit HOST selection of an already saved successor. This is not a proof that App
/// storage selected it. Only the private actual Root adapter may supply this in production,
/// after its exact SavedB/current proof (or authenticated frozen cold-selector replay).
pub struct V2HostSelectionExpectation {
    header: OwnerHeaderId,
    state: OwnerStateCommitment,
}
impl std::fmt::Debug for V2HostSelectionExpectation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2HostSelectionExpectation")
            .finish_non_exhaustive()
    }
}
impl V2HostSelectionExpectation {
    pub fn from_trusted_host(header: OwnerHeaderId, state: OwnerStateCommitment) -> Self {
        Self { header, state }
    }
}

/// Bounded selected-prefix verifier. Retain its real source/payment in the Root host BEFORE
/// signing; a borrowed mathematical model alone cannot carry the pending transaction's debt.
/// Only current frontier state is resident. Historical leaves/headers live in the paid archive.
pub struct V2FounderPrefixVerifier {
    math: ReplayMath,
    founder: AdmissionBody,
    group: Arc<GroupViewLife>,
    origin: V2ProjectionOrigin,
    // A new verifier must not consume an earlier verifier's candidate, even with equal labels.
    identity: Arc<()>,
}
impl std::fmt::Debug for V2FounderPrefixVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2FounderPrefixVerifier")
            .finish_non_exhaustive()
    }
}

/// One opaque, already-derived candidate. No Clone, decoder, mutable next-model or public
/// constructor exists. Physical staging borrows exact outputs before the real SavedB receipt;
/// installing consumes this SAME candidate once and does not repeat allocating replay math.
pub struct V2VerifiedAppendCandidate {
    identity: Arc<()>,
    origin: V2ProjectionOrigin,
    parent: OwnerHeaderId,
    step: StepData,
}
impl std::fmt::Debug for V2VerifiedAppendCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2VerifiedAppendCandidate")
            .finish_non_exhaustive()
    }
}
impl V2VerifiedAppendCandidate {
    pub fn header_data(&self) -> &SignedOwnerHeader {
        self.step.next.header()
    }
    pub fn image_data(&self) -> &[u8] {
        self.step.next.image()
    }
    pub fn append_data(&self) -> &PreparedAppend {
        // Only derive_verified_append constructs this type, using ReplayBody::Append.
        self.step
            .append
            .as_ref()
            .expect("verified append owns its derived tree output")
    }
}

/// Actual old model and append output, returned for destruction OUTSIDE all Root/storage
/// writers. The original resident and append payments must cover both until that drain.
pub struct V2RetiredAppendData {
    parent: ReplayMath,
    append: Option<PreparedAppend>,
}
impl std::fmt::Debug for V2RetiredAppendData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2RetiredAppendData")
            .finish_non_exhaustive()
    }
}
impl V2RetiredAppendData {
    /// Borrowed inert DATA for the host's exact cleanup/accounting, never an install API.
    pub fn parent_data(&self) -> &ReplayMath {
        &self.parent
    }
    pub fn append_data(&self) -> Option<&PreparedAppend> {
        self.append.as_ref()
    }
}

impl V2FounderPrefixVerifier {
    /// Authenticate COMPLETE genesis from independent HOST expectations. The supplied group
    /// owner is negative-only: it cannot replace the root expectation or signed replay.
    /// The caller funds the complete ReplayMath invoice and original observer/metadata first.
    pub fn from_genesis(
        expectation: V2FounderExpectation,
        profile: &SignedOwnerProfile,
        founder: &AdmissionBody,
        header: &SignedOwnerHeader,
        image: &[u8],
        group: &ProjectionGroupView,
        origin: V2ProjectionOrigin,
    ) -> Result<Self, ChatError> {
        let a = &expectation.anchor;
        if !origin.live()
            || !group.life.live()
            || group.life.group.as_ref() != a.group.as_slice()
            || profile.spec().group != a.group
            || profile.spec().founder != a.founder
            || profile.id() != a.profile
            || founder.admission_id() != a.admission
            || profile.tenure() != a.tenure
            || header.id() != a.header
            || header.spec().state != a.state
        {
            return Err(ChatError::Scope);
        }
        // This checks each actual signature, exact founder/genesis bindings, epoch zero,
        // empty acceptance tree, complete result image and the single original founder slot.
        let math = ReplayMath::from_genesis(profile, founder, header, image)?;
        if !origin.live() || !group.life.live() {
            return Err(ChatError::Scope);
        }
        Ok(Self {
            math,
            founder: founder.clone(),
            group: group.life.clone(),
            origin,
            identity: Arc::new(()),
        })
    }

    fn live(&self) -> Result<(), ChatError> {
        if self.origin.live() && self.group.live() {
            Ok(())
        } else {
            Err(ChatError::Scope)
        }
    }

    /// Public planning DATA only. It cannot be changed, adopted or converted to an admission
    /// by the caller. The actual node must independently enforce its original key/source.
    pub fn replay_data(&self) -> &ReplayMath {
        &self.math
    }

    /// Derive exactly once, outside global writers. This profile accepts only the original
    /// founder's Append; no Admission/Remove/Review/epoch transition can enter through it.
    pub fn derive_verified_append(
        &self,
        header: &SignedOwnerHeader,
        body: &AppendBody,
        image: &[u8],
    ) -> Result<V2VerifiedAppendCandidate, ChatError> {
        self.live()?;
        if body.spec().old_tip.context != self.context() {
            return Err(ChatError::Scope);
        }
        let step = self
            .math
            .derive_successor(header, ReplayBody::Append(body), image)?;
        self.live()?;
        Ok(V2VerifiedAppendCandidate {
            identity: self.identity.clone(),
            origin: self.origin.clone(),
            parent: self.math.header().id(),
            step,
        })
    }

    /// Install the SAME already-derived next model only after the actual host selected it.
    /// Every fallible predicate runs before taking the Option. On refusal the candidate stays
    /// owned, unchanged and retryable; an ambiguous saved result never authorizes a new child.
    /// Cache the advanced phase infallibly before dropping the returned old-model debt.
    pub fn advance_host_selected(
        &mut self,
        selected: &V2HostSelectionExpectation,
        candidate: &mut Option<V2VerifiedAppendCandidate>,
    ) -> Result<V2RetiredAppendData, ChatError> {
        self.live()?;
        let next = candidate.as_ref().ok_or(ChatError::Dependency)?;
        if !Arc::ptr_eq(&self.identity, &next.identity)
            || !self.origin.same_original(&next.origin)
            || self.math.header().id() != next.parent
            || selected.header != next.step.next.header().id()
            || selected.state != next.step.next.header().spec().state
        {
            return Err(ChatError::Scope);
        }
        // No hooks, allocation, callbacks, further source reads or fallible work from take
        // through installation. A close admitted later must drain the host's actual work.
        let next = candidate.take().expect("checked occupied candidate");
        let parent = std::mem::replace(&mut self.math, next.step.next);
        Ok(V2RetiredAppendData {
            parent,
            append: next.step.append,
        })
    }

    fn context(&self) -> TreeContext {
        TreeContext {
            profile: self.math.profile().id(),
            admission: self.founder.admission_id(),
        }
    }

    /// Verify historical semantic authority for ONE exact supported Create. This witness is
    /// monotone and intentionally has no source cancellation lifetime. It does not establish
    /// acceptance or release Held projection, and cloning it cannot grant Edit/Delete/Join.
    pub fn verify_create_semantics(
        &self,
        action: &SignedAction,
    ) -> Result<VerifiedAuthorityBasis, ChatError> {
        self.live()?;
        self.founder.matches_action_delegation(action)?;
        if !action.spec().parents.is_empty()
            || !matches!(action.spec().body, Body::Create { reply_to: None, .. })
        {
            return Err(ChatError::Authority);
        }
        Ok(VerifiedAuthorityBasis {
            realm: action.spec().realm.id(),
            basis: action.spec().basis,
            grant: action.spec().grant,
            subject: *action.signer(),
            rights: Rights::CREATE,
            covered: BTreeSet::new(),
            constraint: SemanticConstraint::V2FounderCreate {
                action: action.id(),
                profile: self.math.profile().id(),
                admission: self.founder.admission_id(),
                tenure: self.math.profile().tenure(),
            },
        })
    }

    /// Mint exact Proven Create support using inclusion under the COMPLETE authenticated
    /// current founder tip. Raw records, caller tips/counts, A/I/S and matching labels refuse.
    /// The target channel must share this ORIGINAL group owner, not merely equal group bytes.
    /// Tokens and retained result leases continue to observe the original source negative.
    pub fn accepted_create(
        &self,
        action: &SignedAction,
        record: &AcceptanceRecord,
        index: u64,
        siblings: &[TreeHash],
        view: &ProjectionView,
    ) -> Result<ProjectionAdmissionToken, ChatError> {
        self.verify_create_semantics(action)?;
        if !view.life.live()
            || view.life.realm != action.spec().realm.id()
            || !view
                .life
                .group
                .as_ref()
                .is_some_and(|g| Arc::ptr_eq(g, &self.group))
            || record.spec().context != self.context()
        {
            return Err(ChatError::Scope);
        }
        record.matches_action(action)?;
        let slot = self
            .math
            .slot(self.founder.id())
            .ok_or(ChatError::Dependency)?;
        let tip = TreeTip {
            context: self.context(),
            count: slot.image.accepted_count,
            root: slot.image.accepted_root,
        };
        verify_inclusion(tip, record.leaf(), index, siblings).map_err(|_| ChatError::Authority)?;
        self.live()?;
        if !view.life.live() {
            return Err(ChatError::Scope);
        }
        Ok(ProjectionAdmissionToken {
            binding: ActionBinding::of(action.id(), action),
            context: EvidenceContext::V2Founder {
                profile: self.math.profile().id(),
                admission: self.founder.admission_id(),
                tenure: self.math.profile().tenure(),
            },
            view: view.life.clone(),
            open: None,
            support: ProjectionSupport::V2FounderCreate {
                record: record.id(),
                leaf_index: index,
                origin: self.origin.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests;
