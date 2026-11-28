//! Evidence for withholding and checking a reviewed signed Create from a
//! closed admission. This grants no inclusion or publication authority.
use super::*;
use crate::chat::cold_actions::OriginalActionDigest;
use crate::chat::join_bindings::GenesisAnchorBindingData;
use crate::chat::owner_bodies::{
    AcceptanceSpec, ReviewCaseId, ReviewDecision, ReviewStreamEntry, ReviewStreamRecord,
    MAX_ACCEPTANCE_RECORD_BYTES,
};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const RECORD_DOMAIN: &[u8] = b"mewtual/chat/removed-create-hold/v1\0";
/// Plaintext record ceiling. Charge the authenticated ciphertext, decoded
/// verification scratch and all overlapping copies separately.
pub const MAX_REMOVED_CREATE_RECORD_BYTES: usize = 4
    + RECORD_DOMAIN.len()
    + 4
    + MAX_ACCEPTANCE_RECORD_BYTES
    + 36
    + 8
    + 36
    + 4
    + crate::chat::MAX_ACTION_BYTES;

pub struct V2RemovedCreateEvidence {
    target: AcceptanceRecord,
    original: OriginalActionDigest,
    removal: (OwnerHeaderId, u64),
    waiting_for_reply_parent: Option<ActionId>,
    issuer: std::sync::Weak<()>,
    selected_sequence: u64,
}

redacted_debug!(V2RemovedCreateEvidence);

impl V2RemovedCreateEvidence {
    pub(super) fn derive(
        action: &SignedAction,
        original: &[u8],
        frozen: &V2FrozenAdmission,
        issuer: &V2GeneralPrefixVerifier,
        waiting_for_reply_parent: Option<ActionId>,
    ) -> Result<Self, ChatError> {
        let spec = action.spec();
        Ok(Self {
            target: AcceptanceRecord::new(AcceptanceSpec {
                group: spec.realm.group().to_vec(),
                context: frozen.cut.tip.context,
                channel: spec.realm.channel(),
                action: action.id(),
                subject: *action.signer(),
                grant: spec.grant,
                basis: spec.basis,
                create: None,
            })?,
            original: OriginalActionDigest::for_original_bytes(original),
            removal: frozen.removal,
            waiting_for_reply_parent,
            issuer: Arc::downgrade(&issuer.identity),
            selected_sequence: issuer.selected_sequence.load(Ordering::Acquire),
        })
    }

    pub fn target_data(&self) -> &AcceptanceRecord {
        &self.target
    }

    /// Layout allowance for the opaque evidence and its owned coordinates.
    pub const fn retained_layout_bytes() -> usize {
        std::mem::size_of::<Self>() + MAX_ACCEPTANCE_RECORD_BYTES + 1024
    }

    /// A signed reply can be held before its exact parent is locally valid.
    /// This grants no right to fetch or release a post. Reverify against the
    /// selected parent before deciding the case.
    pub fn waiting_for_reply_parent_data(&self) -> Option<ActionId> {
        self.waiting_for_reply_parent
    }

    /// Semantic duplicates address one case. A later decision must bind
    /// the exact retained signature bytes as well as this case identity.
    pub fn case_data(&self) -> ReviewCaseId {
        ReviewCaseId::from_target_data(&self.target)
    }

    pub fn original_digest_data(&self) -> OriginalActionDigest {
        self.original
    }

    /// Exact selected Remove Header and its governing MLS epoch.
    pub fn removal_data(&self) -> (OwnerHeaderId, u64) {
        self.removal
    }

    /// Check the exact incoming bytes before looking up a retained case.
    /// A semantic duplicate can have another signature and must first
    /// satisfy the evidence for its own incoming envelope.
    pub fn require_original_data(&self, original: &[u8]) -> Result<(), ChatError> {
        if original.is_empty()
            || original.len() > crate::chat::MAX_ACTION_BYTES
            || OriginalActionDigest::for_original_bytes(original) != self.original
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }

    /// Encode the exact original. Temporary dependency state is deliberately
    /// omitted so a missing parent never changes the case's durable identity.
    /// The store must still bind the selected root and the actual mount.
    pub fn encode_record_data(&self, original: &[u8]) -> Result<Zeroizing<Vec<u8>>, ChatError> {
        self.require_original_data(original)?;
        let target = self.target.encode();
        let mut frame = Encoder::with_capacity(
            MAX_REMOVED_CREATE_RECORD_BYTES
                - MAX_ACCEPTANCE_RECORD_BYTES
                - crate::chat::MAX_ACTION_BYTES
                + target.len()
                + original.len(),
        );
        frame.put_bytes(RECORD_DOMAIN)?;
        frame.put_bytes(&target)?;
        frame.put_bytes(self.removal.0.as_bytes())?;
        frame.put_u64(self.removal.1);
        frame.put_bytes(self.original.as_bytes())?;
        frame.put_bytes(original)?;
        Ok(Zeroizing::new(frame.finish()))
    }
}

/// Data borrowed from one bounded plaintext hold frame. Decoding checks exact
/// action bytes and signature. It grants no trust in the recorded Remove or
/// Admission, and does not assert that a decision has been made.
pub struct V2RemovedCreateRecord<'a> {
    target: AcceptanceRecord,
    removal: (OwnerHeaderId, u64),
    original: OriginalActionDigest,
    bytes: &'a [u8],
}

redacted_debug!(V2RemovedCreateRecord<'_>);

impl<'a> V2RemovedCreateRecord<'a> {
    pub fn decode(frame: &'a [u8]) -> Result<Self, ChatError> {
        if frame.len() > MAX_REMOVED_CREATE_RECORD_BYTES {
            return Err(ChatError::Bound);
        }
        let mut input = Decoder::new(frame);
        if input.get_bytes()? != RECORD_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let target = AcceptanceRecord::decode(input.get_bytes()?)?;
        let header = OwnerHeaderId::from_bytes(
            input
                .get_bytes()?
                .try_into()
                .map_err(|_| ChatError::Malformed)?,
        );
        let removal = (header, input.get_u64()?);
        let original = OriginalActionDigest::from_bytes(
            input
                .get_bytes()?
                .try_into()
                .map_err(|_| ChatError::Malformed)?,
        );
        let bytes = input.get_bytes()?;
        input.finish()?;
        let action = SignedAction::decode(bytes)?;
        if !matches!(action.spec().body, Body::Create { .. }) {
            return Err(ChatError::Permission);
        }
        if OriginalActionDigest::for_original_bytes(bytes) != original {
            return Err(ChatError::Scope);
        }
        target.matches_action(&action)?;
        Ok(Self {
            target,
            removal,
            original,
            bytes,
        })
    }

    pub fn original_bytes_data(&self) -> &'a [u8] {
        self.bytes
    }

    /// Digest checked against the exact original and its signature on decode.
    pub fn original_digest_data(&self) -> OriginalActionDigest {
        self.original
    }

    pub fn target_data(&self) -> &AcceptanceRecord {
        &self.target
    }

    pub fn removal_data(&self) -> (OwnerHeaderId, u64) {
        self.removal
    }

    /// Cold callers first obtain new evidence from the authenticated selected
    /// prefix. Exact equality binds record data but does not admit an effect.
    pub fn require_evidence(&self, original: &V2RemovedCreateEvidence) -> Result<(), ChatError> {
        if self.target != original.target
            || self.removal != original.removal
            || self.original != original.original
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
}

/// The checked approval either needs its exact reply parent or is ready
/// for the host's separate receipt selection.
pub enum V2ReviewedCreateCheck<'a> {
    /// The authenticated reply is still missing its exact verified parent.
    WaitingForReplyParent(ActionId),
    /// The exact record passed the current verifier and parent replay.
    Ready(V2CheckedReviewedCreate<'a>),
}

/// Transient evidence for the exact borrowed approval. The host must still
/// prove durable review selection and cover its own receipt before use.
pub struct V2CheckedReviewedCreate<'a> {
    record: &'a ReviewStreamRecord<'a>,
    // Keep the exact verified ancestry immutable until the receipt edge.
    _parent_ledger: &'a ChatLedger,
    parent_view: &'a ProjectionView,
    issuer: std::sync::Weak<()>,
    selected_sequence: u64,
}

redacted_debug!(V2ReviewedCreateCheck<'_>, V2CheckedReviewedCreate<'_>);

impl<'a> V2CheckedReviewedCreate<'a> {
    pub fn record_data(&self) -> &'a ReviewStreamRecord<'a> {
        self.record
    }

    /// Layout allowance for the check. The borrowed record, parent replay
    /// and temporary verification have separate fees.
    pub const fn retained_layout_bytes() -> usize {
        std::mem::size_of::<Self>()
    }
}

impl V2GeneralPrefixVerifier {
    /// The host supplies the actual selected public ordinal and case tip, and
    /// proves the signed membership anchor in its selected ancestry. Core
    /// checks their coordinates against the exact record and live verifier.
    /// Use a paid replay ledger with verified selected ordinary ancestry.
    /// This check leaves that ledger and its view unchanged.
    pub fn check_selected_reviewed_create<'a>(
        &self,
        (pin, membership): (&FounderGenesisAnchorData, (OwnerHeaderId, u64)),
        (selected, record): (&ReviewStreamEntry, &'a ReviewStreamRecord<'a>),
        author: &V2FrozenAdmission,
        (ledger, view): (&'a ChatLedger, &'a ProjectionView),
        retired_dependencies: &[&V2FrozenAdmission],
    ) -> Result<V2ReviewedCreateCheck<'a>, ChatError> {
        let position = selected.position_data();
        if selected != record.entry_data()
            || (position.membership_header, position.membership_epoch) != membership
            || position.membership_epoch > self.math.mls_epoch_data()
            || pin.group != self.math.profile().spec().group
        {
            return Err(ChatError::Scope);
        }
        let step = selected.case_step_data().ok_or(ChatError::Dependency)?;
        if step.choice_data() != ReviewDecision::Approve {
            return Err(ChatError::Permission);
        }
        self.require_join_genesis_data(
            GenesisAnchorBindingData {
                group: pin.group.clone(),
                founder: pin.founder,
                profile: *pin.profile.as_bytes(),
                admission: *pin.admission.as_bytes(),
                tenure: *pin.tenure.as_bytes(),
                header: *pin.header.as_bytes(),
                state: *pin.state.as_bytes(),
            }
            .commitment_data()?,
        )?;
        selected.verify_decision_for_pin(pin, record.decision_data())?;
        let held = record.held_data();
        let evidence = self.verify_removed_create_for_review(
            held.original_bytes_data(),
            author,
            ledger,
            view,
            retired_dependencies,
        )?;
        held.require_evidence(&evidence)?;
        self.require_current_removed_create(&evidence)?;
        if !view.life.live() {
            return Err(ChatError::Scope);
        }
        if let Some(parent) = evidence.waiting_for_reply_parent_data() {
            return Ok(V2ReviewedCreateCheck::WaitingForReplyParent(parent));
        }
        record.decision_data().require_evidence_data(&evidence)?;
        Ok(V2ReviewedCreateCheck::Ready(V2CheckedReviewedCreate {
            record,
            _parent_ledger: ledger,
            parent_view: view,
            issuer: evidence.issuer,
            selected_sequence: evidence.selected_sequence,
        }))
    }

    /// At the receipt edge, recheck the original issuer, parent view and
    /// exact header. The host separately rechecks its durable selectors.
    pub fn require_current_reviewed_create(
        &self,
        checked: &V2CheckedReviewedCreate<'_>,
        selected: &ReviewStreamEntry,
    ) -> Result<(), ChatError> {
        if selected != checked.record.entry_data() || !checked.parent_view.life.live() {
            return Err(ChatError::Scope);
        }
        self.require_current_review_issuer(&checked.issuer, checked.selected_sequence)
    }

    /// Reprove a retained case's signed original against the current selected
    /// historical author and Remove. This read does not assess reply ancestry,
    /// prove covered storage selection, or return publication authority.
    pub fn verify_removed_create_record_for_audit(
        &self,
        record: &V2RemovedCreateRecord<'_>,
        author: &V2FrozenAdmission,
    ) -> Result<SignedAction, ChatError> {
        let removal = self.removed_review_coordinates(author)?;
        if record.removal_data() != removal
            || record.target_data().spec().context != author.cut.tip.context
        {
            return Err(ChatError::Scope);
        }
        let action = SignedAction::decode(record.original_bytes_data())?;
        author
            .admission
            .core
            .body
            .matches_action_delegation(&action)?;
        self.live()?;
        Ok(action)
    }

    /// Recheck a transient withholding result at the same selected issuer.
    /// Reloaded or advanced hosts reverify the original evidence instead.
    pub fn require_current_removed_create(
        &self,
        evidence: &V2RemovedCreateEvidence,
    ) -> Result<(), ChatError> {
        self.require_current_review_issuer(&evidence.issuer, evidence.selected_sequence)
    }

    fn require_current_review_issuer(
        &self,
        issuer: &std::sync::Weak<()>,
        selected_sequence: u64,
    ) -> Result<(), ChatError> {
        self.live()?;
        if !issuer.ptr_eq(&Arc::downgrade(&self.identity))
            || selected_sequence != self.selected_sequence.load(Ordering::Acquire)
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
}
