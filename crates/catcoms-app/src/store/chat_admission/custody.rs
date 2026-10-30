//! Private runtime custody construction. It grants no historical membership authority.
use super::*;

impl ServerStore {
    fn custody_from_owner(
        &self,
        key: &ChatAdmissionKey,
        record: &Record,
        life: &Arc<revocation::Owner>,
    ) -> Result<ChatAdmissionPreparation, AppError> {
        if !life.live() || record.phase == Phase::Retired {
            return Err(refused());
        }
        let mount = self.chat_mount.observer();
        let original = life.observer();
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
            owner_identity: life.identity(),
            revoker: ChatAdmissionRevoker(life.handle()),
        })
    }

    pub(super) fn reservation_locked(
        &self,
        runtime: &mut Runtime,
        key: &ChatAdmissionKey,
        record: &Record,
    ) -> Result<ChatAdmissionPreparation, AppError> {
        if record.phase == Phase::Retired {
            return Err(refused());
        }
        if runtime
            .owners
            .get(key)
            .is_some_and(|owner| owner.nonce != record.nonce || owner.revision != record.revision)
        {
            if let Some(old) = runtime.owners.remove(key) {
                old.life.revoke();
            }
        }
        let owner = runtime
            .owners
            .entry(key.clone())
            .or_insert_with(|| AdmissionOwner {
                nonce: record.nonce,
                revision: record.revision,
                life: revocation::Owner::new(),
            });
        // Never replace a negatively revoked matching owner with a new live publisher. Only
        // exact durable retirement followed by a genuine new admission can advance lineage.
        self.custody_from_owner(key, record, &owner.life)
    }

    pub(super) fn new_reservation(
        &self,
        key: &ChatAdmissionKey,
        record: &Record,
    ) -> Result<(ChatAdmissionPreparation, AdmissionOwner), AppError> {
        let owner = AdmissionOwner {
            nonce: record.nonce,
            revision: record.revision,
            life: revocation::Owner::new(),
        };
        let preparation = self.custody_from_owner(key, record, &owner.life)?;
        Ok((preparation, owner))
    }
}
