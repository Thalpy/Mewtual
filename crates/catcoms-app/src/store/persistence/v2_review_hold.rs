//! Encrypted custody for exact removed-member Create bytes. A frame read
//! from this namespace grants no prior acceptance, visibility or decision.
use super::v2_shared_history::SharedHistoryScope;
use crate::store::{critical_namespaces::refused, frame, unframe};
use crate::AppError;
use catcoms_replication::chat::{
    cold_actions::OriginalActionDigest, owner_bodies::ReviewCaseId, owner_header::OwnerHeaderId,
    ActionId, V2GeneralPrefixVerifier, V2RemovedCreateEvidence, V2RemovedCreateRecord,
    MAX_REMOVED_CREATE_RECORD_BYTES,
};
use catcoms_rt::{CryptoRngCore, RequestCancellation};
use std::sync::{Arc, Mutex, Weak};
use zeroize::Zeroizing;

pub(in crate::store::persistence) mod disk;
pub use disk::SelectedHeldCase as NativeTextHeldCase;

const KEY_DOMAIN: &str = "catcoms/private-chat-v2-removed-create-vault-key/v1";
const LEAF_DOMAIN: &str = "catcoms/private-chat-v2-removed-create-leaf/v1";
pub(super) const MAX_FRAME: usize = MAX_REMOVED_CREATE_RECORD_BYTES + 40;

/// Exact immutable coordinates for a retained case. These are data for
/// reproof against the selected governing prefix before any decision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) struct HeldCase {
    pub(super) id: ReviewCaseId,
    pub(super) action: ActionId,
    pub(super) original: OriginalActionDigest,
    pub(super) removal: (OwnerHeaderId, u64),
}

/// Sealed incoming data is bound to the actual vault, group incarnation
/// and channel. Admission is intentionally per record: one removed tenure
/// can never reinterpret another's case even after the same device rejoins.
pub(in crate::store::persistence) struct HoldScope(SharedHistoryScope);

impl HoldScope {
    pub(super) fn new(scope: SharedHistoryScope) -> Self {
        Self(scope)
    }

    fn key(&self, vault_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
        let mut hash = blake3::Hasher::new_derive_key(KEY_DOMAIN);
        hash.update(vault_key);
        hash.update(&self.0.id());
        Zeroizing::new(*hash.finalize().as_bytes())
    }

    pub(super) fn partition_label(&self, vault_key: &[u8; 32]) -> [u8; 32] {
        blake3::derive_key(
            "catcoms/private-chat-v2-removed-create-directory/v1",
            &*self.key(vault_key),
        )
    }

    pub(super) fn file_label(&self, vault_key: &[u8; 32], case: ReviewCaseId) -> [u8; 32] {
        let key = Zeroizing::new(blake3::derive_key(LEAF_DOMAIN, &*self.key(vault_key)));
        *blake3::keyed_hash(&key, case.as_bytes()).as_bytes()
    }

    fn require_scope(&self, record: &V2RemovedCreateRecord<'_>) -> Result<HeldCase, AppError> {
        let target = record.target_data();
        let spec = target.spec();
        if spec.group.as_slice() != self.0.group
            || spec.channel != self.0.channel
            || spec.context.profile != self.0.profile
        {
            return Err(refused());
        }
        Ok(HeldCase {
            id: ReviewCaseId::from_target_data(target),
            action: spec.action,
            original: record.original_digest_data(),
            removal: record.removal_data(),
        })
    }

    /// Only the exact selected issuer may supply a new hold. The mounted
    /// owner also checks independently known history under its write lane.
    pub(super) fn prepare(
        &self,
        vault_key: &[u8; 32],
        verifier: &V2GeneralPrefixVerifier,
        evidence: &V2RemovedCreateEvidence,
        original: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(HeldCase, Zeroizing<Vec<u8>>), AppError> {
        verifier
            .require_current_removed_create(evidence)
            .map_err(|_| refused())?;
        let plain = evidence
            .encode_record_data(original)
            .map_err(|_| refused())?;
        let prepared = self.seal(vault_key, &plain, rng)?;
        verifier
            .require_current_removed_create(evidence)
            .map_err(|_| refused())?;
        Ok(prepared)
    }

    fn seal(
        &self,
        vault_key: &[u8; 32],
        plain: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(HeldCase, Zeroizing<Vec<u8>>), AppError> {
        let record = V2RemovedCreateRecord::decode(plain).map_err(|_| refused())?;
        let identity = self.require_scope(&record)?;
        let sealed = catcoms_crypto::seal(&self.key(vault_key), plain, rng)?;
        Ok((identity, Zeroizing::new(frame(&sealed))))
    }

    pub(super) fn open(&self, vault_key: &[u8; 32], sealed: &[u8]) -> Result<OpenedHold, AppError> {
        if !(41..=MAX_FRAME).contains(&sealed.len()) {
            return Err(refused());
        }
        let plain = Zeroizing::new(catcoms_crypto::unseal(
            &self.key(vault_key),
            &unframe(sealed)?,
        )?);
        let record = V2RemovedCreateRecord::decode(&plain).map_err(|_| refused())?;
        let identity = self.require_scope(&record)?;
        Ok(OpenedHold { plain, identity })
    }
}

pub(super) struct OpenedHold {
    plain: Zeroizing<Vec<u8>>,
    pub(super) identity: HeldCase,
}

#[derive(Clone)]
pub(in crate::store) struct HeldAuditObservation {
    owner: Weak<Mutex<u64>>,
    expected: u64,
}

impl HeldAuditObservation {
    pub(super) fn capture(owner: &Arc<Mutex<u64>>) -> Result<Self, AppError> {
        let expected = *owner.try_lock().map_err(|_| refused())?;
        Ok(Self {
            owner: Arc::downgrade(owner),
            expected,
        })
    }

    fn with_current<T>(&self, read: impl FnOnce() -> T) -> Result<T, AppError> {
        let owner = self.owner.upgrade().ok_or_else(refused)?;
        let current = owner.try_lock().map_err(|_| refused())?;
        if *current != self.expected {
            return Err(refused());
        }
        Ok(read())
    }
}

/// Process-local continuation for one selected vault, channel and audit
/// revision. Native lends it only to the original authorized view.
#[derive(Clone)]
pub struct NativeTextHeldReviewContinuation {
    scope: SharedHistoryScope,
    after: [u8; 32],
    observation: HeldAuditObservation,
    signal: RequestCancellation,
}

impl NativeTextHeldReviewContinuation {
    pub(super) fn after_for(
        &self,
        scope: SharedHistoryScope,
        observation: &HeldAuditObservation,
    ) -> Result<[u8; 32], AppError> {
        if self.scope != scope
            || self.observation.expected != observation.expected
            || !self.observation.owner.ptr_eq(&observation.owner)
            || self.signal.is_cancelled()
        {
            return Err(refused());
        }
        Ok(self.after)
    }
}

/// Paid, read-only data from the original mounted founder. The negative
/// follows the selected membership and the caller. Native still orders its
/// final publication against lock and original registry-row replacement.
pub struct NativeTextHeldReviewPage {
    window: Option<(disk::SelectedHeldWindow, HeldAuditObservation)>,
    scope: SharedHistoryScope,
    signal: RequestCancellation,
}

catcoms_rt::redacted_debug! { NativeTextHeldReviewPage; NativeTextHeldReviewContinuation; }

impl NativeTextHeldReviewPage {
    pub(in crate::store) fn new(
        window: Option<(disk::SelectedHeldWindow, HeldAuditObservation)>,
        scope: SharedHistoryScope,
        signal: RequestCancellation,
    ) -> Self {
        Self {
            window,
            scope,
            signal,
        }
    }

    /// Recheck the original hold revision through the final synchronous
    /// release. This acquires a readiness check without performing file I/O.
    pub fn with_publication<T>(&self, publish: impl FnOnce() -> T) -> Result<T, AppError> {
        let check = || {
            if self.signal.is_cancelled() {
                return Err(refused());
            }
            Ok(publish())
        };
        match self.window.as_ref() {
            Some((_, observation)) => observation.with_current(check)?,
            None => check(),
        }
    }

    pub fn continuation(&self) -> Option<NativeTextHeldReviewContinuation> {
        let (page, observation) = self.window.as_ref()?;
        Some(NativeTextHeldReviewContinuation {
            scope: self.scope,
            after: page.next?,
            observation: observation.clone(),
            signal: self.signal.clone(),
        })
    }

    /// None means the hold namespace is unavailable. A complete empty page
    /// requires a selected namespace and an observed empty window.
    pub fn with_records<T>(
        &self,
        read: impl FnOnce(Option<&[NativeTextHeldCase]>, usize) -> T,
    ) -> Result<T, AppError> {
        let value = self.with_publication(|| match self.window.as_ref() {
            Some((page, _)) => read(Some(page.records.as_slice()), page.unavailable),
            None => read(None, 0),
        })?;
        if self.signal.is_cancelled() {
            return Err(refused());
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests;
