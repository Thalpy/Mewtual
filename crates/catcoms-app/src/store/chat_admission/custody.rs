//! Private runtime custody construction. It grants no historical membership authority.
use super::*;

impl ServerStore {
    fn custody_from_owner(
        &self,
        key: &ChatAdmissionKey,
        record: &Record,
        owner: &AdmissionOwner,
    ) -> Result<ChatAdmissionPreparation, AppError> {
        if !owner.life.live() || record.phase == Phase::Retired {
            return Err(refused());
        }
        let mount = self.chat_mount.observer();
        let original = owner.life.observer_with_custody(owner.custody.clone());
        let admission = original
            .clone()
            .with_authority(Arc::new(AdmissionAuthority {
                mount: mount.clone(),
                admission: original,
            }));
        Ok(ChatAdmissionPreparation {
            key: key.clone(),
            record: record.clone(),
            domain: self.chat_vault_domain()?,
            mount_identity: self.chat_mount.identity.clone(),
            mount,
            admission,
            owner_identity: owner.life.identity(),
            revoker: ChatAdmissionRevoker(owner.life.handle_with_custody(owner.custody.clone())),
        })
    }

    pub(super) fn reservation_locked(
        &self,
        runtime: &mut Runtime,
        key: &ChatAdmissionKey,
        record: &Record,
        retired_custody: &mut Option<SharedRequestKeepalive>,
    ) -> Result<ChatAdmissionPreparation, AppError> {
        if record.phase == Phase::Retired {
            return Err(refused());
        }
        if runtime
            .owners
            .get(key)
            .is_some_and(|owner| owner.nonce != record.nonce || owner.revision != record.revision)
        {
            if retired_custody.is_some() {
                // Check BEFORE removing/revoking anything. Each actual outer transaction
                // replaces at most one owner and drains this holder outside its writer.
                return Err(refused());
            }
            if let Some(old) = runtime.owners.remove(key) {
                detach_retired_owner(old, retired_custody);
            }
        }
        let owner = runtime
            .owners
            .entry(key.clone())
            .or_insert_with(|| AdmissionOwner {
                nonce: record.nonce,
                revision: record.revision,
                life: revocation::Owner::new(),
                custody: None,
            });
        // Never replace a negatively revoked matching owner with a new live publisher. Only
        // exact durable retirement followed by a genuine new admission can advance lineage.
        self.custody_from_owner(key, record, owner)
    }

    pub(super) fn new_reservation(
        &self,
        key: &ChatAdmissionKey,
        record: &Record,
    ) -> Result<(ChatAdmissionPreparation, AdmissionOwner), AppError> {
        self.new_reservation_from_custody(key, record, None)
    }

    /// Resource-only setup for an actual V2 saved-artifact caller. The receipt is never a
    /// scope/source proof by itself; concrete issuers validate it before invoking this helper.
    pub(super) fn new_reservation_with_custody(
        &self,
        key: &ChatAdmissionKey,
        record: &Record,
        custody: SharedRequestKeepalive,
    ) -> Result<(ChatAdmissionPreparation, AdmissionOwner), AppError> {
        self.new_reservation_from_custody(key, record, Some(custody))
    }

    fn new_reservation_from_custody(
        &self,
        key: &ChatAdmissionKey,
        record: &Record,
        custody: Option<SharedRequestKeepalive>,
    ) -> Result<(ChatAdmissionPreparation, AdmissionOwner), AppError> {
        let owner = AdmissionOwner {
            nonce: record.nonce,
            revision: record.revision,
            life: revocation::Owner::new(),
            custody,
        };
        // The actual caller retains its original receipt across setup errors, until AFTER
        // its admission writer unlocks. Runtime owns this same payment only after transfer.
        let preparation = self.custody_from_owner(key, record, &owner)?;
        Ok((preparation, owner))
    }

    /// Bind ONLY an already validated exact live owner. Unlike the legacy restore helper,
    /// this cannot create a sender, evict a mismatch or sample another identity. The actual
    /// V2 caller repeats its source/head/writer proof separately before outward handoff.
    pub(super) fn bind_owned_chat_admission_locked(
        &self,
        runtime: &Runtime,
        key: &ChatAdmissionKey,
        nonce: [u8; 32],
        revision: u64,
        owner_identity: &Arc<()>,
    ) -> Result<ChatAdmissionBinding, AppError> {
        if !runtime.managed || runtime.dirty || !self.chat_mount.life.live() {
            return Err(refused());
        }
        let row = runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .get(key)
            .ok_or_else(refused)?;
        let owner = runtime.owners.get(key).ok_or_else(refused)?;
        if row.phase != Phase::Current
            || row.nonce != nonce
            || row.revision != revision
            || owner.nonce != nonce
            || owner.revision != revision
            || !Arc::ptr_eq(&owner.life.identity(), owner_identity)
        {
            return Err(refused());
        }
        let custody = self.custody_from_owner(key, row, owner)?;
        if custody.admission.is_cancelled() {
            return Err(refused());
        }
        Ok(ChatAdmissionBinding {
            key: custody.key,
            nonce,
            revision,
            domain: custody.domain,
            mount_identity: custody.mount_identity,
            mount: custody.mount,
            publication: custody.admission,
            owner_identity: custody.owner_identity,
            revoker: custody.revoker,
            owner_writer_identity: None,
        })
    }
}

/// The caller has checked its outside-writer holder is vacant BEFORE any mutation. Preserve
/// old cancellation/Sender timing: revoke and drop the life NOW, detach ONLY the resource.
/// No receipt in this custody may own that sender, a Store/key/session or a source graph.
pub(super) fn detach_retired_owner(
    owner: AdmissionOwner,
    retired_custody: &mut Option<SharedRequestKeepalive>,
) {
    let AdmissionOwner { life, custody, .. } = owner;
    life.revoke();
    drop(life);
    if custody.is_some() {
        // Every actual caller checks vacancy before ownership changes; no fallible work or
        // last-resource Drop is hidden between removal and parking this already-owned Arc.
        *retired_custody = custody;
    }
}
