//! Borrowed custody for a subordinate saver while the original admission writer is held.
//!
//! Re-entering `with_chat_admission_storage` from initialization would deadlock on that same
//! writer. This permit carries the actual guard lifetime instead. It does not mint a Current
//! binding, prove historical membership or authorize publication. Only trusted Store code may
//! consume it; wire/native scalar fields cannot construct it.
use super::*;
use std::marker::PhantomData;
use std::rc::Rc;

/// An already-held original admission writer. The higher-ranked callback lifetime prevents
/// escape, and the `Rc` marker prevents sending even a borrowed permit to another worker. Do
/// not add Clone, serialization, a public constructor or an async callback convenience API.
pub(crate) struct ChatAdmissionStoragePermit<'writer> {
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    key: &'writer ChatAdmissionKey,
    domain: [u8; 32],
    nonce: [u8; 32],
    revision: u64,
    owner_identity: &'writer Arc<()>,
    observer: &'writer RequestCancellation,
    mode: ChatAdmissionStorageMode,
    _not_send: PhantomData<Rc<()>>,
}
impl std::fmt::Debug for ChatAdmissionStoragePermit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatAdmissionStoragePermit")
            .finish_non_exhaustive()
    }
}

impl<'writer> ChatAdmissionStoragePermit<'writer> {
    // These constructors are private to the admission implementation. Callers cannot turn
    // a decoded nonce/domain or an arbitrary Store reference into writer custody.
    pub(super) fn initializing(
        store: &'writer ServerStore,
        runtime: &'writer Runtime,
        preparation: &'writer ChatAdmissionPreparation,
    ) -> Self {
        Self {
            store,
            runtime,
            key: &preparation.key,
            domain: preparation.domain,
            nonce: preparation.record.nonce,
            revision: preparation.record.revision,
            owner_identity: &preparation.owner_identity,
            observer: &preparation.admission,
            mode: ChatAdmissionStorageMode::Initializing,
            _not_send: PhantomData,
        }
    }

    pub(super) fn current(
        store: &'writer ServerStore,
        runtime: &'writer Runtime,
        binding: &'writer ChatAdmissionBinding,
    ) -> Self {
        Self {
            store,
            runtime,
            key: &binding.key,
            domain: binding.domain,
            nonce: binding.nonce,
            revision: binding.revision,
            owner_identity: &binding.owner_identity,
            observer: &binding.publication,
            mode: ChatAdmissionStorageMode::Current,
            _not_send: PhantomData,
        }
    }

    pub(crate) fn store(&self) -> &'writer ServerStore {
        self.store
    }
    pub(crate) fn group(&self) -> &[u8] {
        self.key.group()
    }
    pub(crate) fn signer(&self) -> &[u8; 32] {
        self.key.signer()
    }
    pub(crate) fn domain(&self) -> [u8; 32] {
        self.domain
    }
    pub(crate) fn nonce(&self) -> [u8; 32] {
        self.nonce
    }
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn owner_identity(&self) -> &Arc<()> {
        self.owner_identity
    }
    /// Original weak observation only. This cannot retain its owner or promote Initializing.
    pub(crate) fn observer(&self) -> RequestCancellation {
        self.observer.clone()
    }
    pub(crate) fn mode(&self) -> ChatAdmissionStorageMode {
        self.mode
    }

    /// Recheck AFTER taking a subordinate head writer and immediately before synchronous I/O.
    /// This checks the actual physical Store and the exact group/full signer/source owner. A
    /// negative revoker takes neither writer, so the original observer must also be reread.
    /// Once this final read admits an original write, revocation cannot recall that write; the
    /// held admission writer still serializes its completion before durable retirement.
    pub(crate) fn validate_original(
        &self,
        store: &ServerStore,
        group: &[u8],
        signer: &[u8; 32],
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.store(), store)
            || self.group() != group
            || self.signer() != signer
            || self.domain() != store.chat_vault_domain()?
            || !self.runtime.managed
            || self.runtime.dirty
        {
            return Err(refused());
        }
        let row = self
            .runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(self.key)
            .ok_or_else(refused)?;
        let phase = match self.mode() {
            ChatAdmissionStorageMode::Initializing => Phase::Initializing,
            ChatAdmissionStorageMode::Current => Phase::Current,
        };
        let owner = self.runtime.owners.get(self.key).ok_or_else(refused)?;
        if row.phase != phase
            || row.nonce != self.nonce()
            || row.revision != self.revision()
            || owner.nonce != self.nonce()
            || owner.revision != self.revision()
            || !Arc::ptr_eq(&owner.life.identity(), self.owner_identity())
            || self.observer().is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }
}
