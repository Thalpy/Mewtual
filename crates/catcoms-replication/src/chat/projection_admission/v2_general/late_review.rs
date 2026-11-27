//! Evidence for withholding one exact signed Create from a closed admission.
//! This never proves tree inclusion, releases a post or makes it visible.
use super::*;
use crate::chat::cold_actions::OriginalActionDigest;
use crate::chat::owner_bodies::{AcceptanceSpec, ReviewCaseId, MAX_ACCEPTANCE_RECORD_BYTES};
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

    /// Encode the exact original. Temporary dependency state is deliberately
    /// omitted so a missing parent never changes the case's durable identity.
    /// The store must still bind the selected root and the actual mount.
    pub fn encode_record_data(&self, original: &[u8]) -> Result<Zeroizing<Vec<u8>>, ChatError> {
        if original.is_empty()
            || original.len() > crate::chat::MAX_ACTION_BYTES
            || OriginalActionDigest::for_original_bytes(original) != self.original
        {
            return Err(ChatError::Scope);
        }
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

impl V2GeneralPrefixVerifier {
    /// Recheck a transient withholding result at the same selected issuer.
    /// Reloaded or advanced hosts reverify the original evidence instead.
    pub fn require_current_removed_create(
        &self,
        evidence: &V2RemovedCreateEvidence,
    ) -> Result<(), ChatError> {
        self.live()?;
        if !evidence.issuer.ptr_eq(&Arc::downgrade(&self.identity))
            || evidence.selected_sequence != self.selected_sequence.load(Ordering::Acquire)
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
}
