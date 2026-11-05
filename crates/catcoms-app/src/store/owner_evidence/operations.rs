//! Critical operation-format intent and automatic pre-cleaner wrapper recovery.
//!
//! This does not create an owner pin, Current admission, operation or public protocol profile.
//! The fixed intent is mandatory metadata, distinct from every user-history/canonical namespace.
use super::*;
use catcoms_storage::VaultFormat;

const TAG: &str = "catcoms/chat-owner-operations-format/v1";
const FILE: &str = "chat-owner-operations.bin";
const STAGE: &str = "chat-owner-operations.replace.tmp";
const MAX_PLAIN: usize = 128;
const MAX_FRAMED: usize = MAX_PLAIN + 40;

#[derive(Clone, Copy)]
enum IntentSlot {
    Current,
    Stage,
}
impl IntentSlot {
    fn name(self) -> &'static str {
        match self {
            Self::Current => FILE,
            Self::Stage => STAGE,
        }
    }
}

fn refused() -> AppError {
    AppError::Invalid("owner operation storage is unavailable or unsupported".into())
}

/// An actual authenticated fixed intent on the ORIGINAL mounted Store, never a portable
/// authoring/format capability. Files remain open for their real durability barriers. No raw
/// constructor or public success boolean can make a legacy/missing inventory acceptable.
pub(in crate::store) struct ValidatedOwnerOperationsIntent {
    mount: Arc<()>,
    original: RequestCancellation,
    current: Option<File>,
    stage: Option<File>,
}
impl std::fmt::Debug for ValidatedOwnerOperationsIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedOwnerOperationsIntent")
            .finish_non_exhaustive()
    }
}
impl ValidatedOwnerOperationsIntent {
    pub(in crate::store) fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.mount, &store.registry_mount()) || self.original.is_cancelled() {
            return Err(refused());
        }
        Ok(())
    }

    /// Re-sync actual known files before promoting/discarding a fixed activation stage. Both
    /// files already validated to the SAME immutable tag/domain; unknown bytes never reach here.
    fn settle(&self, store: &ServerStore) -> Result<(), AppError> {
        self.validate_original(store)?;
        for file in self.current.iter().chain(self.stage.iter()) {
            file.sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
        }
        store
            .sync_own_vault_root()
            .map_err(|error| AppError::Io(error.to_string()))?;
        if self.stage.is_some() {
            self.validate_original(store)?;
            if self.current.is_some() {
                fs::remove_file(store.dir.join(STAGE))
                    .map_err(|error| AppError::Io(error.to_string()))?;
            } else {
                fs::rename(store.dir.join(STAGE), store.dir.join(FILE))
                    .map_err(|error| AppError::Io(error.to_string()))?;
            }
            store
                .sync_owner_operations_after_rename()
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
        }
        self.validate_original(store)
    }
}

/// Called after physical-session exclusion but BEFORE open_or_create could generate a new DEK.
/// symlink_metadata distinguishes genuine absence from broken links/permission/unsupported state.
pub(in crate::store) fn refuse_orphaned_intent_before_vault_creation(
    dir: &Path,
) -> Result<(), AppError> {
    match fs::symlink_metadata(dir.join("vault.bin")) {
        Ok(metadata) if metadata.file_type().is_file() => return Ok(()),
        // In particular, a dangling symlink is PRESENT. The lower legacy vault API's exists()
        // probe would follow it, see absence and create a new DEK, orphaning retained ciphertext.
        Ok(_) => return Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(refused()),
    }
    for name in [FILE, STAGE] {
        match fs::symlink_metadata(dir.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    Ok(())
}

impl ServerStore {
    /// Original classified wrapper + fixed intent decide constructor ordering. A pending intent
    /// forces mandatory EXISTING inventories even with v3/legacy metadata; no fake v4 label is
    /// installed and no root cleaner runs before this branch. Entirely absent intent uses the
    /// unchanged older constructor, except v5 must refuse its missing mandatory record.
    pub(in crate::store) fn open_owner_operations_gate(
        &self,
        actual_format: VaultFormat,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let Some(intent) = self.inspect_owner_operations_intent()? else {
            if actual_format == VaultFormat::OwnerOperations {
                return Err(refused());
            }
            self.open_chat_admission_inventory(actual_format)?;
            self.open_owner_evidence_inventory(actual_format)?;
            return Ok(());
        };
        self.open_chat_admissions_for_operations_recovery(actual_format, &intent)?;
        self.open_owner_pins_for_operations_recovery(&intent)?;
        intent.settle(self)?;
        if actual_format != VaultFormat::OwnerOperations {
            self.activate_owner_operations_wrapper(passphrase, rng, &intent)?;
        }
        // An already authenticated v5 needs no extra Argon2/rewrap. Its managed root barrier
        // happened in open_classified; mandatory inventories and actual marker barriers just ran.
        intent.validate_original(self)
    }

    /// Consume the pre-exposure Store. v3's legitimate absent owner table is initialized only
    /// through the real existing v4 transition, BEFORE any intent stage exists. Every failure
    /// drops/revokes the original mount instead of returning a borrowed half-upgraded Store.
    pub fn into_owner_operations_store(
        self,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        let store = self.into_owner_evidence_store(passphrase, rng)?;
        let original = store.chat_mount.observer();
        let intent = match store.inspect_owner_operations_intent()? {
            Some(intent) => intent,
            None => {
                if store.chat_vault_format()? == VaultFormat::OwnerOperations
                    || original.is_cancelled()
                {
                    return Err(refused());
                }
                let mut encoder = Encoder::with_capacity(MAX_PLAIN);
                encoder.put_str(TAG).map_err(|_| refused())?;
                encoder
                    .put_bytes(&store.chat_vault_domain()?)
                    .map_err(|_| refused())?;
                let plain = Zeroizing::new(encoder.finish());
                if plain.len() > MAX_PLAIN {
                    return Err(refused());
                }
                let sealed = seal(&store.keys.db_key()?, &plain, rng)?;
                let framed = frame(&sealed);
                if framed.len() > MAX_FRAMED || original.is_cancelled() {
                    return Err(refused());
                }
                store.write_owner_operations_intent(&framed)?;
                store
                    .inspect_owner_operations_intent()?
                    .ok_or_else(refused)?
            }
        };
        intent.settle(&store)?;
        store.activate_owner_operations_wrapper(passphrase, rng, &intent)?;
        if original.is_cancelled() {
            return Err(refused());
        }
        Ok(store)
    }

    fn activate_owner_operations_wrapper(
        &self,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        intent: &ValidatedOwnerOperationsIntent,
    ) -> Result<(), AppError> {
        intent.validate_original(self)?;
        map_vault_passphrase_change(catcoms_storage::vault::activate_owner_operations_vault(
            &self.dir, passphrase, rng,
        ))?;
        // This opaque result authenticates the SAME actual wrapper/keys after its real barrier.
        // Never infer the format from a subsequent unauthenticated header read or caller bool.
        let opened = open_or_create_vault_classified(&self.dir, passphrase, rng)?;
        self.confirm_owner_operations_format(opened, intent)
    }

    fn inspect_owner_operations_intent(
        &self,
    ) -> Result<Option<ValidatedOwnerOperationsIntent>, AppError> {
        let current = self.read_owner_operations_intent_file(IntentSlot::Current)?;
        let stage = self.read_owner_operations_intent_file(IntentSlot::Stage)?;
        if current.is_none() && stage.is_none() {
            return Ok(None);
        }
        Ok(Some(ValidatedOwnerOperationsIntent {
            mount: self.registry_mount(),
            original: self.chat_mount.observer(),
            current,
            stage,
        }))
    }

    fn read_owner_operations_intent_file(
        &self,
        slot: IntentSlot,
    ) -> Result<Option<File>, AppError> {
        let path = self.dir.join(slot.name());
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(metadata)
                if metadata.file_type().is_file() && metadata.len() <= MAX_FRAMED as u64 => {}
            _ => return Err(refused()),
        }
        // Windows FlushFileBuffers needs a write-capable handle. No truncate/create option is
        // used: this opens the SAME already existing fixed marker for read and later sync.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| AppError::Io(error.to_string()))?;
        let mut raw = Zeroizing::new(Vec::with_capacity(MAX_FRAMED + 1));
        (&mut file)
            .take((MAX_FRAMED + 1) as u64)
            .read_to_end(&mut raw)
            .map_err(|error| AppError::Io(error.to_string()))?;
        if raw.len() > MAX_FRAMED {
            return Err(refused());
        }
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&raw)?)?);
        if plain.len() > MAX_PLAIN {
            return Err(refused());
        }
        let mut decoder = Decoder::new(&plain);
        if decoder.get_str().map_err(|_| refused())? != TAG
            || decoder.get_bytes().map_err(|_| refused())? != self.chat_vault_domain()?
        {
            return Err(refused());
        }
        decoder.finish().map_err(|_| refused())?;
        Ok(Some(file))
    }

    /// Fixed create-new metadata stage only. It is preserved on any failure; constructor repair
    /// authenticates/syncs it before adoption. This never writes a history/owner canonical path.
    fn write_owner_operations_intent(&self, bytes: &[u8]) -> Result<(), AppError> {
        if bytes.len() > MAX_FRAMED || self.chat_mount.observer().is_cancelled() {
            return Err(refused());
        }
        let staged = self.dir.join(STAGE);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|error| AppError::Io(error.to_string()))?;
        file.write_all(bytes)
            .map_err(|error| AppError::Io(error.to_string()))?;
        file.sync_all()
            .map_err(|error| AppError::Io(error.to_string()))?;
        drop(file);
        #[cfg(test)]
        let fault = self
            .owner_evidence
            .operations_marker_fault
            .load(std::sync::atomic::Ordering::SeqCst);
        #[cfg(test)]
        if fault == 1 {
            fs::remove_file(&staged).expect("remove actual marker stage before rename");
        }
        #[cfg(test)]
        if fault == 3 {
            return Err(AppError::Io(
                "injected loss after actual intent stage sync".into(),
            ));
        }
        fs::rename(&staged, self.dir.join(FILE))
            .map_err(|error| AppError::Io(error.to_string()))?;
        self.sync_owner_operations_after_rename()
            .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))
    }
}

#[cfg(test)]
mod tests;
