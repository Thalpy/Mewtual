//! Original canonical-head custody and exact saved-candidate identity.
use super::*;
use catcoms_rt::RequestAuthority;

pub(super) struct HeadCell {
    pub(super) key: [u8; 32],
    identity: Arc<()>,
    state: Mutex<HeadState>,
}
#[derive(Default)]
struct HeadState {
    current: Option<Meta>,
    pending: Option<Meta>,
    owner: Option<Weak<WriterLife>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Meta {
    pub(super) generation: u64,
    pub(super) head: [u8; 32],
    pub(super) snapshot: [u8; 32],
    pub(super) attempt: [u8; 32],
    pub(super) profile: OwnerFactId,
}
impl HeadCell {
    pub(super) fn new(key: [u8; 32]) -> Self {
        Self {
            key,
            identity: Arc::new(()),
            state: Mutex::new(HeadState::default()),
        }
    }
    pub(super) fn read_stamp(&self) -> Result<Option<Meta>, AppError> {
        let head = self.lock()?;
        if head.state.pending.is_some() {
            return Err(refused());
        }
        Ok(head.state.current.clone())
    }

    // A disk entry may reserve a namespace, but cannot manufacture a verified head/writer.
    pub(super) fn lock(&self) -> Result<OwnerHeadWriteGuard<'_>, AppError> {
        Ok(OwnerHeadWriteGuard {
            cell: self,
            state: self.state.lock().map_err(|_| refused())?,
        })
    }
}

/// A real already-held per-logical-head writer, after the admission permit. Non-Send guard;
/// no caller-created labels or logical-key lookup can stand in for this original custody.
pub(crate) struct OwnerHeadWriteGuard<'writer> {
    cell: &'writer HeadCell,
    state: MutexGuard<'writer, HeadState>,
}
impl OwnerHeadWriteGuard<'_> {
    /// Negative-only first-initialization classification. A live private creator, possible
    /// terminal or installed head must be preserved even if no canonical file is visible yet.
    pub(super) fn is_unexposed_and_unowned(&self) -> bool {
        self.state.current.is_none()
            && self.state.pending.is_none()
            && self.state.owner.as_ref().and_then(Weak::upgrade).is_none()
    }

    pub(in crate::store) fn identifier(
        &self,
        permit: &ChatAdmissionStoragePermit<'_>,
    ) -> Result<[u8; 32], AppError> {
        if self.cell.key != key(permit.group(), permit.signer()) {
            return Err(refused());
        }
        Ok(self.cell.key)
    }
}
impl std::fmt::Debug for OwnerHeadWriteGuard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerHeadWriteGuard")
            .finish_non_exhaustive()
    }
}

/// Sole revoking owner of a canonical runtime writer. Observers retain only a dummy allocation
/// plus watch receiver, never this Sender. The actual OwnedOwnerServer/task must own this value
/// through retirement; dropping an old writer cancels queued chat even inside the same scope.
struct WriterLife {
    identity: Arc<()>,
    sender: watch::Sender<bool>,
}

struct WriterScope {
    group: Vec<u8>,
    signer: [u8; 32],
    domain: [u8; 32],
    nonce: [u8; 32],
    identity: Arc<()>,
    observer: RequestCancellation,
}
impl WriterScope {
    fn preparation(scope: &ChatAdmissionPreparation) -> Self {
        Self {
            group: scope.group().to_vec(),
            signer: *scope.signer(),
            domain: scope.domain(),
            nonce: scope.nonce(),
            identity: scope.owner_identity(),
            observer: scope.reservation_observer(),
        }
    }
    fn current(scope: &ChatAdmissionBinding) -> Self {
        Self {
            group: scope.group().to_vec(),
            signer: *scope.signer(),
            domain: scope.domain(),
            nonce: scope.nonce(),
            identity: scope.owner_identity(),
            observer: scope.reservation_observer(),
        }
    }
}

pub(crate) struct OwnerWriterRegistration {
    life: Arc<WriterLife>,
    head: Arc<HeadCell>,
    mount: Arc<()>,
    negative: RequestCancellation,
    admission: WriterScope,
}
impl std::fmt::Debug for OwnerWriterRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerWriterRegistration")
            .finish_non_exhaustive()
    }
}
impl OwnerWriterRegistration {
    pub(super) fn initial(
        store: &ServerStore,
        cell: &Arc<HeadCell>,
        admission: &ChatAdmissionPreparation,
    ) -> Result<Self, AppError> {
        let mut head = cell.lock()?;
        if head.state.current.is_some()
            || head.state.pending.is_some()
            || head.state.owner.as_ref().and_then(Weak::upgrade).is_some()
        {
            return Err(refused());
        }
        let (sender, _) = watch::channel(false);
        let life = Arc::new(WriterLife {
            identity: Arc::new(()),
            sender,
        });
        head.state.owner = Some(Arc::downgrade(&life));
        Ok(Self {
            life,
            head: cell.clone(),
            mount: store.registry_mount(),
            negative: store.chat_mount.observer(),
            admission: WriterScope::preparation(admission),
        })
    }
    pub(super) fn with_saved_current_head<T>(
        &self,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
        expected: &Meta,
        read: impl FnOnce() -> T,
    ) -> Result<T, AppError> {
        let head = self.head.lock()?;
        self.validate_original(&head, permit, binding)?;
        if head.state.pending.is_some() || head.state.current.as_ref() != Some(expected) {
            return Err(refused());
        }
        Ok(read())
    }

    pub(crate) fn identity(&self) -> Arc<()> {
        self.life.identity.clone()
    }
    pub(crate) fn publication_authority(&self) -> RequestCancellation {
        RequestCancellation::new(self.life.sender.subscribe(), None).with_authority(Arc::new(
            OriginalOwnerAuthority {
                mount: self.negative.clone(),
                admission: self.admission.observer.clone(),
            },
        ))
    }
    pub(crate) fn validate_original(
        &self,
        head: &OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
    ) -> Result<(), AppError> {
        self.validate_permit(head, permit)?;
        if permit.mode() != ChatAdmissionStorageMode::Current
            || binding.group() != permit.group()
            || binding.signer() != permit.signer()
            || binding.domain() != permit.domain()
            || binding.nonce() != permit.nonce()
            || !Arc::ptr_eq(&binding.owner_identity(), permit.owner_identity())
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn validate_permit(
        &self,
        head: &OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
    ) -> Result<(), AppError> {
        permit.validate_original(
            permit.store(),
            self.admission.group.as_slice(),
            &self.admission.signer,
        )?;
        if !Arc::ptr_eq(&self.mount, &permit.store().registry_mount())
            || !Arc::ptr_eq(&self.head.identity, &head.cell.identity)
            || head.cell.key != key(self.admission.group.as_slice(), &self.admission.signer)
            || head
                .state
                .owner
                .as_ref()
                .and_then(Weak::upgrade)
                .is_none_or(|owner| !Arc::ptr_eq(&owner.identity, &self.life.identity))
            || permit.domain() != self.admission.domain
            || permit.nonce() != self.admission.nonce
            || !Arc::ptr_eq(permit.owner_identity(), &self.admission.identity)
            || self.publication_authority().is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }
}
impl Drop for OwnerWriterRegistration {
    fn drop(&mut self) {
        self.life.sender.send_replace(true);
    }
}
#[derive(Debug)]
struct OriginalOwnerAuthority {
    mount: RequestCancellation,
    admission: RequestCancellation,
}
#[async_trait::async_trait]
impl RequestAuthority for OriginalOwnerAuthority {
    fn authorized(&self) -> bool {
        !self.mount.is_cancelled() && !self.admission.is_cancelled()
    }
    async fn revoked(&self) {
        let mut mount = self.mount.clone();
        let mut admission = self.admission.clone();
        if self.authorized() {
            tokio::select! { _ = mount.cancelled() => {}, _ = admission.cancelled() => {} }
        }
    }
}

/// Exact covering result, never reconstructed from a visible file or a success boolean.
/// Restore uses a separate closed source validator: no process-local fresh attempt is invented.
pub(crate) struct SavedOwnerInitialization {
    mount: Arc<()>,
    writer: Arc<()>,
    meta: Meta,
}
impl std::fmt::Debug for SavedOwnerInitialization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedOwnerInitialization")
            .finish_non_exhaustive()
    }
}
impl SavedOwnerInitialization {
    pub(super) fn metadata(&self) -> Meta {
        self.meta.clone()
    }
    pub(crate) fn validate_private_candidate(
        &self,
        head: &OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
        attempt: [u8; 32],
        frozen_digest: [u8; 32],
    ) -> Result<(), AppError> {
        permit.validate_original(permit.store(), binding.group(), binding.signer())?;
        if permit.mode() != ChatAdmissionStorageMode::Current
            || !Arc::ptr_eq(&self.mount, &permit.store().registry_mount())
            || binding
                .owner_writer_identity()
                .is_none_or(|id| !Arc::ptr_eq(id, &self.writer))
            || head
                .state
                .owner
                .as_ref()
                .and_then(Weak::upgrade)
                .is_none_or(|id| !Arc::ptr_eq(&id.identity, &self.writer))
            || head.state.pending.is_some()
            || head.state.current.as_ref() != Some(&self.meta)
            || self.meta.attempt != attempt
            || self.meta.snapshot != frozen_digest
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl OwnerHeadWriteGuard<'_> {
    /// Save the SAME frozen candidate through a real file+parent barrier. Every failure retains
    /// the whole pending terminal/head identity; exact retry never resamples its generation or
    /// signs different facts. It may reseal unchanged plaintext with a fresh AEAD nonce.
    pub(super) fn save_initialization(
        &mut self,
        permit: &ChatAdmissionStoragePermit<'_>,
        reservation: &OwnerInitializationReservation,
        frozen: &crate::store::chat_admission::fresh::FrozenPrivateServerSnapshot,
        profile: OwnerFactId,
        rng: &mut impl CryptoRngCore,
    ) -> Result<SavedOwnerInitialization, AppError> {
        let writer = reservation.writer.as_ref().ok_or_else(refused)?;
        writer.validate_permit(self, permit)?;
        if !Arc::ptr_eq(&reservation.cell.identity, &self.cell.identity)
            || frozen.bytes().len() > reservation.snapshot_limits().max_snapshot_bytes
        {
            return Err(refused());
        }
        {
            let roots = permit
                .store()
                .owner_evidence
                .pins
                .lock()
                .map_err(|_| refused())?;
            if !roots.managed
                || roots.dirty
                || roots
                    .table
                    .as_ref()
                    .ok_or_else(refused)?
                    .get(permit.group())
                    .is_none_or(|pin| pin.profile != profile)
            {
                return Err(refused());
            }
        }
        let capacity = frozen
            .bytes()
            .len()
            .checked_add(WRAPPER_FEE)
            .ok_or_else(refused)?;
        let mut frame = Encoder::with_capacity(capacity);
        frame
            .put_str("catcoms/owner-canonical-snapshot/v1")
            .map_err(|_| refused())?;
        frame.put_bytes(permit.group()).map_err(|_| refused())?;
        frame.put_bytes(permit.signer()).map_err(|_| refused())?;
        frame.put_bytes(profile.as_bytes()).map_err(|_| refused())?;
        frame.put_bytes(&permit.domain()).map_err(|_| refused())?;
        frame.put_bytes(&permit.nonce()).map_err(|_| refused())?;
        frame.put_u64(permit.revision());
        frame.put_u64(1); // This method initializes only; existing heads require exact restore/CAS.
        frame.put_bytes(&[]).map_err(|_| refused())?; // no parent on this first local head
        frame
            .put_bytes(&reservation.attempt())
            .map_err(|_| refused())?;
        frame.put_bytes(frozen.bytes()).map_err(|_| refused())?;
        let plain = Zeroizing::new(frame.finish());
        if plain.len() > capacity {
            return Err(refused());
        }
        let mut hash = blake3::Hasher::new();
        hash.update(b"catcoms/owner-head-id/v1\0");
        hash.update(&plain);
        let meta = Meta {
            generation: 1,
            head: *hash.finalize().as_bytes(),
            snapshot: frozen.digest(),
            attempt: reservation.attempt(),
            profile,
        };
        if let Some(current) = &self.state.current {
            if current != &meta || self.state.pending.is_some() {
                return Err(refused());
            }
            // In-process exact result lookup only. A restored visible file never enters this
            // state until its own mandatory canonical durability barrier has succeeded.
            return Ok(SavedOwnerInitialization {
                mount: permit.store().registry_mount(),
                writer: writer.identity(),
                meta,
            });
        }
        if self
            .state
            .pending
            .as_ref()
            .is_some_and(|pending| pending != &meta)
        {
            return Err(refused());
        }
        self.state.pending = Some(meta.clone());
        let sealed = seal(&permit.store().keys.db_key()?, &plain, rng)?;
        writer.validate_permit(self, permit)?;
        permit
            .store()
            .write_owner_canonical_record(permit, self, &crate::store::frame(&sealed))?;
        self.state.current = Some(meta.clone());
        self.state.pending = None;
        writer.validate_permit(self, permit)?;
        Ok(SavedOwnerInitialization {
            mount: permit.store().registry_mount(),
            writer: writer.identity(),
            meta,
        })
    }
}

impl SavedOwnerInitialization {
    /// Called inside the private factory AFTER its exact frozen-byte validation and scope bind,
    /// BEFORE consuming the original fresh source or moving Server. A negative race/error keeps
    /// the complete private object and paid frozen buffer, so retry cannot strand the candidate.
    pub(crate) fn bind_private_runtime<T: catcoms_rt::MeshTransport, R: CryptoRngCore>(
        &self,
        head: &OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
        sync: &mut catcoms_sync::ChannelSync<T, R>,
    ) -> Result<(), AppError> {
        self.validate_private_candidate(
            head,
            permit,
            binding,
            self.meta.attempt,
            self.meta.snapshot,
        )?;
        let founder = {
            let roots = permit
                .store()
                .owner_evidence
                .pins
                .lock()
                .map_err(|_| refused())?;
            if !roots.managed || roots.dirty {
                return Err(refused());
            }
            let pin = roots
                .table
                .as_ref()
                .ok_or_else(refused)?
                .get(permit.group())
                .ok_or_else(refused)?;
            if pin.profile != self.meta.profile {
                return Err(refused());
            }
            pin.founder
        };
        super::initialize::bind_saved_runtime(sync, binding, self.meta.profile, founder)
    }
}

/// Actual restart barrier, distinct from any fresh completion. No constructor accepts an old
/// Signed profile, raw snapshot, a saved success bit or fabricated fresh attempt provenance.
pub(crate) struct SavedOwnerRestore {
    mount: Arc<()>,
    head: Arc<()>,
    admission_owner: Arc<()>,
    meta: Meta,
}
impl std::fmt::Debug for SavedOwnerRestore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedOwnerRestore").finish_non_exhaustive()
    }
}
impl OwnerHeadWriteGuard<'_> {
    /// Re-authenticate current bytes and flush THAT opened file and its exact child parent.
    /// Failure remains unbound even if the file is visible/authentic after an earlier rename.
    /// Initializing is permitted only for this negative private recovery barrier; it cannot
    /// register a publisher until the SAME owner has crossed the actual Current inventory save.
    pub(crate) fn establish_restore_barrier(
        &mut self,
        permit: &ChatAdmissionStoragePermit<'_>,
        source: &ValidatedOwnerSnapshot,
    ) -> Result<SavedOwnerRestore, AppError> {
        source.validate_original(permit.store())?;
        permit.validate_original(permit.store(), source.group(), source.signer())?;
        if !Arc::ptr_eq(&source.cell.identity, &self.cell.identity)
            || permit.domain() != source.domain()
            || permit.nonce() != source.nonce()
            || permit.revision() != source.revision()
            || self.state.pending.is_some()
            || self
                .state
                .current
                .as_ref()
                .is_some_and(|known| known != &source.meta)
        {
            return Err(refused());
        }
        let (file, raw) = permit
            .store()
            .read_owner_canonical_record(&self.cell.key, source.file_cap)?;
        let plain = Zeroizing::new(unseal(
            &permit.store().keys.db_key()?,
            &crate::store::unframe(&raw)?,
        )?);
        if plain.as_slice() != source.plain.as_slice() {
            return Err(refused());
        }
        // Same actual opened handle, not a second path lookup or visible-file success boolean.
        permit
            .store()
            .sync_opened_owner_canonical_record(permit, self, &file)?;
        source.validate_original(permit.store())?;
        self.state.current = Some(source.meta.clone());
        Ok(SavedOwnerRestore {
            mount: permit.store().registry_mount(),
            head: self.cell.identity.clone(),
            admission_owner: permit.owner_identity().clone(),
            meta: source.meta.clone(),
        })
    }
}
impl SavedOwnerRestore {
    pub(super) fn metadata(&self) -> Meta {
        self.meta.clone()
    }
    pub(crate) fn validate_source(
        &self,
        head: &OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
        source: &ValidatedOwnerSnapshot,
    ) -> Result<(), AppError> {
        source.validate_original(permit.store())?;
        permit.validate_original(permit.store(), source.group(), source.signer())?;
        if !Arc::ptr_eq(&self.mount, &permit.store().registry_mount())
            || !Arc::ptr_eq(&self.head, &head.cell.identity)
            || !Arc::ptr_eq(&self.admission_owner, permit.owner_identity())
            || permit.nonce() != source.nonce()
            || permit.domain() != source.domain()
            || permit.revision() != source.revision()
            || head.state.pending.is_some()
            || head.state.current.as_ref() != Some(&self.meta)
            || self.meta != source.meta
            || self.meta.snapshot != source.snapshot_digest()
        {
            return Err(refused());
        }
        Ok(())
    }
    /// The original current scope and completed restore barrier may register ONE writer. An
    /// existing live writer, including pre-registration, refuses; no first-open/CID selection.
    pub(crate) fn register_writer(
        &self,
        head: &mut OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
        source: &ValidatedOwnerSnapshot,
        binding: &ChatAdmissionBinding,
    ) -> Result<OwnerWriterRegistration, AppError> {
        self.validate_source(head, permit, source)?;
        if permit.mode() != ChatAdmissionStorageMode::Current
            || binding.group() != source.group()
            || binding.signer() != source.signer()
            || !Arc::ptr_eq(&binding.owner_identity(), permit.owner_identity())
            || head.state.owner.as_ref().and_then(Weak::upgrade).is_some()
        {
            return Err(refused());
        }
        let (sender, _) = watch::channel(false);
        let life = Arc::new(WriterLife {
            identity: Arc::new(()),
            sender,
        });
        head.state.owner = Some(Arc::downgrade(&life));
        Ok(OwnerWriterRegistration {
            life,
            head: source.cell.clone(),
            mount: self.mount.clone(),
            negative: permit.store().chat_mount.observer(),
            admission: WriterScope::current(binding),
        })
    }
    /// Final exact-decode transfer check. The restore factory exposes no mutable borrow between
    /// constructing that decode and supplying its original source here. Re-encoding an SDK
    /// HashMap is deliberately not used as a byte-identity proof.
    pub(crate) fn validate_private_candidate(
        &self,
        head: &OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
        source: &ValidatedOwnerSnapshot,
    ) -> Result<(), AppError> {
        self.validate_source(head, permit, source)?;
        if permit.mode() != ChatAdmissionStorageMode::Current
            || binding.group() != source.group()
            || binding.signer() != source.signer()
            || binding.domain() != source.domain()
            || binding.nonce() != source.nonce()
            || binding.publication_authority().is_cancelled()
            || head
                .state
                .owner
                .as_ref()
                .and_then(Weak::upgrade)
                .is_none_or(|owner| {
                    binding
                        .owner_writer_identity()
                        .is_none_or(|id| !Arc::ptr_eq(id, &owner.identity))
                })
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Trusted restore-factory transfer only: after scope bind, BEFORE moving its exact private
    /// decode, independently verify pinned facts and attach original writer cancellation.
    pub(crate) fn bind_private_runtime<T: catcoms_rt::MeshTransport, R: CryptoRngCore>(
        &self,
        head: &OwnerHeadWriteGuard<'_>,
        permit: &ChatAdmissionStoragePermit<'_>,
        binding: &ChatAdmissionBinding,
        source: &ValidatedOwnerSnapshot,
        sync: &mut catcoms_sync::ChannelSync<T, R>,
    ) -> Result<(), AppError> {
        self.validate_private_candidate(head, permit, binding, source)?;
        super::initialize::bind_saved_runtime(sync, binding, source.profile(), *source.founder())
    }
}
