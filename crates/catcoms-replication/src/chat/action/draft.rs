//! Immutable unsigned action DATA. This is neither a historical authority witness nor a
//! signing/publication permit. Shared action/v1 shape/encoding helpers keep this path byte
//! identical to SignedAction; no dummy signature or alternative serialized grammar exists.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};

struct DraftLife {
    // Monotone: one exact Draft can acquire only one Plan, including after that Plan drops.
    // Lost key work must never acquire a fresh plan and silently invoke the signer again.
    planned: AtomicBool,
}

/// An owned, validated canonical unsigned action and full signer. Construction authenticates
/// no Admission, source, membership or rights. There is no mutable spec/bytes accessor, Clone,
/// wire decoder or constructor from a claimed 'verified' status.
pub struct ActionDraft {
    spec: ActionSpec,
    signer: [u8; 32],
    semantic: Vec<u8>,
    id: ActionId,
    identity: Arc<DraftLife>,
}

/// Tiny historical exact-original comparison DATA. Weak custody cannot retain a Draft, its
/// text or a signing permission. Equal canonical bytes from another Draft do not adopt it.
#[derive(Clone)]
pub struct OriginalActionDraftObservation {
    identity: Weak<DraftLife>,
    id: ActionId,
    signer: [u8; 32],
}

// The claim is deliberately not public API. Its ownership proves only that THIS draft's
// monotone planning slot was consumed; it cannot create authority or expose a raw signer.
pub(in crate::chat) struct ActionDraftPlanClaim {
    _identity: Arc<DraftLife>,
}

impl ActionDraft {
    pub fn new(spec: ActionSpec, signer: [u8; 32]) -> Result<Self, ChatError> {
        // Public ActionSpec DATA may carry huge spare backing despite small valid lengths.
        // Refuse that custody BEFORE canonical allocation or a Plan claim. These are Draft-
        // only memory checks: shared SignedAction validation/wire compatibility stays exact.
        if spec.parents.capacity() > MAX_PARENTS
            || matches!(&spec.body, Body::Create { text, .. } | Body::Edit { text, .. }
                if text.capacity() > MAX_TEXT_BYTES)
        {
            return Err(ChatError::Bound);
        }
        validate_spec(&spec)?;
        let semantic = semantic_spec(&spec, &signer);
        if semantic
            .len()
            .checked_add(72)
            .is_none_or(|n| n > MAX_ACTION_BYTES)
            || semantic.capacity() > 2 * MAX_ACTION_BYTES
        {
            return Err(ChatError::Bound);
        }
        let mut hash = blake3::Hasher::new();
        hash.update(ID_DOMAIN);
        hash.update(&semantic);
        Ok(Self {
            spec,
            signer,
            id: ActionId(*hash.finalize().as_bytes()),
            semantic,
            identity: Arc::new(DraftLife {
                planned: AtomicBool::new(false),
            }),
        })
    }
    pub fn spec_data(&self) -> &ActionSpec {
        &self.spec
    }
    pub fn signer_data(&self) -> &[u8; 32] {
        &self.signer
    }
    pub fn action_id_data(&self) -> ActionId {
        self.id
    }
    pub fn signing_bytes_data(&self) -> &[u8] {
        &self.semantic
    }
    pub fn observe_original(&self) -> OriginalActionDraftObservation {
        OriginalActionDraftObservation {
            identity: Arc::downgrade(&self.identity),
            id: self.id,
            signer: self.signer,
        }
    }
    pub fn require_original_data(
        &self,
        original: &OriginalActionDraftObservation,
    ) -> Result<(), ChatError> {
        if !Weak::ptr_eq(&original.identity, &Arc::downgrade(&self.identity))
            || original.id != self.id
            || original.signer != self.signer
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    pub(in crate::chat) fn claim_signing_plan(&self) -> Result<ActionDraftPlanClaim, ChatError> {
        self.identity
            .planned
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ChatError::AuthorityConflict)?;
        Ok(ActionDraftPlanClaim {
            _identity: self.identity.clone(),
        })
    }
    /// Pure DATA materialization OUTSIDE all writers. This callback has no key, SDK or host
    /// access: it returns only the already retained raw64. The existing implementation
    /// independently verifies the real signature over the unchanged action/v1 bytes.
    /// Failure preserves the SAME immutable Draft/raw; caller caches success before late
    /// checks. This remains legal historical DATA after Stop and grants no renewed rights.
    pub fn complete_raw64(&self, raw: [u8; 64]) -> Result<SignedAction, ChatError> {
        SignedAction::sign_using(self.spec.clone(), self.signer, |bytes| {
            if bytes != self.semantic.as_slice() {
                return Err(ChatError::Malformed);
            }
            Ok(raw)
        })
    }
    /// Separate named requested inventory for ONE Draft plus complete_raw64 overlap. The
    /// real host reserves it BEFORE spec/text/bytes/identity allocation. It is additional
    /// to replay1MiB, source/SDK/plan/ledger/outbox costs; no existing cap or payer is implied.
    pub const fn private_layout_bytes() -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of::<SignedAction>()
            + std::mem::size_of::<ActionSpec>()
            + std::mem::size_of::<DraftLife>()
            + 2 * std::mem::size_of::<OriginalActionDraftObservation>()
            + 6 * MAX_ACTION_BYTES
            + 2 * MAX_TEXT_BYTES
            + 2 * MAX_PARENTS * std::mem::size_of::<ActionId>()
            + 3 * MAX_GROUP_ID_BYTES
            + 12 * std::mem::size_of::<usize>()
    }
}
impl OriginalActionDraftObservation {
    pub fn require_same_original(&self, other: &Self) -> Result<(), ChatError> {
        if Weak::ptr_eq(&self.identity, &other.identity)
            && self.id == other.id
            && self.signer == other.signer
        {
            Ok(())
        } else {
            Err(ChatError::Scope)
        }
    }
}
impl std::fmt::Debug for ActionDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionDraft").finish_non_exhaustive()
    }
}
impl std::fmt::Debug for OriginalActionDraftObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalActionDraftObservation")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
