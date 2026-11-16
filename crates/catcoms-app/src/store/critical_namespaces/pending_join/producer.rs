//! Actual finite-checkpoint source producer. Only the explicit network consumer adds saved
//! arms and signed INPUT DATA; SDK/old initializer/ScopeIntent/Frozen/ready remain next gates.
use super::*;
use crate::store::chat_admission::journal_basis::JournalBasis;
use crate::store::owner_evidence::canonical::OwnerPendingCreation;
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_sync::{DirectJoinLimits, PreparedDirectJoin, RecoveredDirectJoinSource};

pub(super) mod activation;
pub(super) use activation::network::Shared as NetworkShared;
mod execution_write;
pub use activation::{
    ActivatedCheckpointJoin, CheckpointJoinNetwork, CheckpointJoinSubmissionStatus,
    ReturnedCheckpointJoinActivation,
};

const MASTER: usize = 512 * 1024;
const REQUEST: usize = 9 + 64 * 1024 + 32 * 1024;
const SOURCE_PLAIN: usize = 640 * 1024;
// Named source buffers: master, request, original encrypted cache, sealing/decrypt plaintext,
// bounded invite object/verification encodings and two bounded group/capture copies. This
// logical inventory excludes separately paid raw providers and still-unadmitted typed SDK work.
const SOURCE_FEE: usize = MASTER
    + REQUEST
    + (SOURCE_PLAIN + 16)
    + (SOURCE_PLAIN + 16)
    + 24
    + 3 * 64 * 1024
    + 2 * 64 * std::mem::size_of::<String>()
    + 2048
    + 2 * 256
    + std::mem::size_of::<JournalledCheckpointJoinPreparation>();
// In addition to the named byte owners, the retained candidate holds TWO complete fixed
// summaries. Version2 input/arm metadata enlarges them; charge their real Rust layout rather
// than assuming the old fixed8KiB metadata allowance still covers that container. READ_FEE
// pays the original Parsed; the candidate Parsed coexists through summary/seal and is paid
// separately here. The remaining8KiB covers fixed encoder/hash/AEAD/Arc/file scratch only.
const WRITE_FEE: usize = codec::MAX_CATALOG_PLAIN
    + U
    + codec::MAX_CATALOG_FRAMED
    + std::mem::size_of::<PendingCandidate>()
    + std::mem::size_of::<codec::Parsed<'static>>()
    + 8192;

enum Source {
    Local(PreparedDirectJoin),
    Recovered(RecoveredDirectJoinSource),
    // Only an actual registered network cell owns the moved immutable secret source. The
    // remaining preparation is control DATA; it cannot return an old initializer or master.
    Taken,
}

/// Secret source DATA retained under actual fees and original mounted custody. Public effects
/// save/schedule its exact Pending record or move it into one original negative transport/task
/// owner. The explicit configured network consumer can save arms and signed INPUT DATA.
/// No `OwnerJoinedInitialization`, generic source extraction, SDK receipt/intent or ready-owner
/// conversion is exposed by this source.
pub struct JournalledCheckpointJoinPreparation {
    source: Source,
    original: Arc<Original>,
    metadata: codec::Metadata,
    transition: Option<codec::Metadata>,
    immutable: Vec<u8>,
    expected: codec::Summary,
    creation: OwnerPendingCreation,
    // Secret/ciphertext data and every original capture drop before their real quota owner.
    _source_fee: CatalogReadReservation,
}
impl std::fmt::Debug for JournalledCheckpointJoinPreparation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JournalledCheckpointJoinPreparation")
            .finish_non_exhaustive()
    }
}

impl ServerStore {
    pub(in crate::store) fn prepare_pending_checkpoint_join(
        &self,
        device: MlsDevice,
        invite: InviteToken,
        pin: CapturedOwnerProfilePin,
        creation: OwnerPendingCreation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<JournalledCheckpointJoinPreparation, AppError> {
        let policy = self.pending_join_policy.as_ref().ok_or_else(refused)?;
        self.retry_pending_join_catalog()?;
        let source_fee = policy.pool.try_reserve(SOURCE_FEE)?;
        pin.validate_original(self, &invite.group_id)?;
        let invite_digest = PreparedDirectJoin::invite_data_digest(&invite)?;
        let expected = {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_ref().ok_or_else(refused)?;
            if mounted.pending.is_some() {
                return Err(refused());
            }
            mounted.current.clone()
        };
        if let Some(row) = expected.rows.iter().flatten().find(|row| {
            row.group.as_slice() == invite.group_id.as_slice()
                && row.invite_nonce == invite.invite_nonce
        }) {
            if row.invite != invite_digest
                || row.profile != *pin.profile().as_bytes()
                || row.founder != *pin.founder()
            {
                return Err(refused());
            }
            // Coalesce the ORIGINAL stored source even if the caller generated an unused new
            // device. It never gets a replacement KP or gets to consume this one-use invite.
            drop(device);
            let capture = self.capture_chat_admission(crate::store::ChatAdmissionKey::new(
                row.group.as_slice(),
                row.signer,
            )?)?;
            if JournalBasis::from_capture(&capture) != row.basis {
                return Err(refused());
            }
            let original = Original::new(self, capture, pin)?;
            let (recovered, immutable) =
                self.read_original_pending_source(row, &expected, &creation)?;
            return Ok(JournalledCheckpointJoinPreparation {
                source: Source::Recovered(recovered),
                original,
                metadata: row.clone(),
                transition: None,
                immutable,
                expected,
                creation,
                _source_fee: source_fee,
            });
        }
        if expected.count == codec::MAX_ROWS
            || expected
                .rows
                .iter()
                .flatten()
                .any(|row| row.scheduling == codec::Scheduling::Active)
        {
            return Err(refused());
        }
        let signer = device
            .public_key_bytes()
            .try_into()
            .map_err(|_| refused())?;
        let capture = self.capture_chat_admission(crate::store::ChatAdmissionKey::new(
            &invite.group_id,
            signer,
        )?)?;
        if expected
            .rows
            .iter()
            .flatten()
            .any(|row| row.group.as_slice() == invite.group_id.as_slice() && row.signer == signer)
        {
            return Err(refused());
        }
        let limits = direct_limits(&creation, self)?;
        let mut source = PreparedDirectJoin::prepare(device, invite, limits)?;
        let request = source.request_identity()?;
        let mut nonce = [0; 32];
        rng.fill_bytes(&mut nonce);
        // Repeated deterministic RNG output must not alias an existing source nonce. Domain
        // and checked generation bind this local identity; no timestamp chooses a winner.
        let mut hash = blake3::Hasher::new();
        hash.update(b"catcoms/destination-join-source-nonce/v1\0");
        hash.update(&self.chat_vault_domain()?);
        hash.update(
            &expected
                .generation
                .checked_add(1)
                .ok_or_else(refused)?
                .to_be_bytes(),
        );
        hash.update(&nonce);
        let nonce = *hash.finalize().as_bytes();
        if expected
            .rows
            .iter()
            .flatten()
            .any(|row| row.source == nonce)
        {
            return Err(refused());
        }
        let basis = JournalBasis::from_capture(&capture);
        let immutable = encode_immutable(
            &self.chat_vault_domain()?,
            nonce,
            source.group(),
            signer,
            *pin.profile().as_bytes(),
            *pin.founder(),
            &request,
            &basis,
        )?;
        let binding = codec::binding_digest(&immutable);
        let sealed = source.seal_source(&self.keys.db_key()?, binding, rng)?;
        let (seal_nonce, encrypted) = sealed.encrypted_parts();
        let mut hash = blake3::Hasher::new();
        hash.update(seal_nonce);
        hash.update(encrypted);
        let metadata = codec::Metadata {
            source: nonce,
            revision: 1,
            scheduling: codec::Scheduling::Active,
            group: codec::Group::from_slice(source.group())?,
            signer,
            profile: *pin.profile().as_bytes(),
            founder: *pin.founder(),
            invite_nonce: *request.invite_nonce(),
            invite: *request.invite_digest(),
            request: *request.request_digest(),
            package: *request.package_reference(),
            basis,
            binding,
            ciphertext: *hash.finalize().as_bytes(),
            execution: if expected.join_version == Some(super::execution_payload::VERSION) {
                Some(super::execution_state::State::not_granted())
            } else {
                None
            },
        };
        Ok(JournalledCheckpointJoinPreparation {
            source: Source::Local(source),
            original: Original::new(self, capture, pin)?,
            metadata,
            transition: None,
            immutable,
            expected,
            creation,
            _source_fee: source_fee,
        })
    }

    fn read_original_pending_source(
        &self,
        row: &codec::Metadata,
        expected: &codec::Summary,
        creation: &OwnerPendingCreation,
    ) -> Result<(RecoveredDirectJoinSource, Vec<u8>), AppError> {
        let policy = self.pending_join_policy.as_ref().ok_or_else(refused)?;
        let read = policy.pool.try_reserve(READ_FEE)?;
        let (_file, raw) = self
            .open_pending_join_catalog_slot(Slot::Current, &read)?
            .ok_or_else(refused)?;
        if *blake3::hash(&raw).as_bytes() != expected.framed {
            return Err(refused());
        }
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &unframe(&raw)?)?);
        let parsed = self.parse_configured_join_catalog(&plain)?;
        if parsed.digest != expected.digest {
            return Err(refused());
        }
        let source = parsed
            .rows
            .iter()
            .flatten()
            .find(|source| source.metadata == *row)
            .ok_or_else(refused)?;
        let sealed = catcoms_sync::SealedDirectJoinSource::from_encrypted_parts(
            source.encrypted[..24].try_into().map_err(|_| refused())?,
            &source.encrypted[24..],
        )?;
        let recovered = RecoveredDirectJoinSource::open(
            &sealed,
            &self.keys.db_key()?,
            row.binding,
            direct_limits(creation, self)?,
        )?;
        let identity = recovered.request_identity()?;
        if recovered.group() != row.group.as_slice()
            || recovered.signer() != &row.signer
            || *identity.invite_nonce() != row.invite_nonce
            || *identity.invite_digest() != row.invite
            || *identity.request_digest() != row.request
            || *identity.package_reference() != row.package
        {
            return Err(refused());
        }
        // The caller holds SOURCE_FEE before this dependent copy and the recovered data. Read
        // scratch/parsed borrows are dropped here; no dormant whole-catalog plaintext survives.
        Ok((recovered, source.immutable.to_vec()))
    }
}

fn direct_limits(
    creation: &OwnerPendingCreation,
    store: &ServerStore,
) -> Result<DirectJoinLimits, AppError> {
    let limits = creation.snapshot_limits(store)?;
    Ok(DirectJoinLimits::new(
        limits.max_mls_bytes.min(MASTER),
        limits.max_mls_backing_bytes,
        limits.max_snapshot_bytes.min(16 * 1024 * 1024),
    )?)
}

#[allow(clippy::too_many_arguments)]
fn encode_immutable(
    domain: &[u8; 32],
    nonce: [u8; 32],
    group: &[u8],
    signer: [u8; 32],
    profile: [u8; 32],
    founder: [u8; 32],
    request: &catcoms_sync::DirectJoinRequestIdentity,
    basis: &JournalBasis,
) -> Result<Vec<u8>, AppError> {
    if group.is_empty() || group.len() > 256 {
        return Err(refused());
    }
    let mut encoder = Encoder::with_capacity(2048);
    encoder
        .put_str(codec::IMMUTABLE_TAG)
        .map_err(|_| refused())?;
    encoder.put_bytes(domain).map_err(|_| refused())?;
    encoder.put_bytes(&nonce).map_err(|_| refused())?;
    encoder.put_u8(1); // direct checkpoint protocol
    encoder.put_u8(1); // finite OwnerFact v1 root, never public-owner v2
    for field in [
        group,
        &signer,
        &profile,
        &founder,
        request.invite_nonce(),
        request.invite_digest(),
        request.request_digest(),
        request.package_reference(),
    ] {
        encoder.put_bytes(field).map_err(|_| refused())?;
    }
    encoder.put_u32(basis.encoded_len() as u32);
    basis.encode_into(&mut encoder)?;
    if encoder.len() > 2048 {
        return Err(refused());
    }
    Ok(encoder.finish())
}

impl JournalledCheckpointJoinPreparation {
    /// Pause scheduling DATA while preserving the SAME original encrypted keys/request. This
    /// object has no network worker; a previously activated original task is closed before the
    /// dependent scheduling write, including when that write returns an uncertain result.
    pub fn pause_pending(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.set_scheduling(store, codec::Scheduling::Paused, rng)
    }

    /// Resume scheduling DATA only; success cannot turn this object into a runnable attempt.
    /// Another Active source refuses without dropping, replacing or evicting either source.
    pub fn resume_pending(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.set_scheduling(store, codec::Scheduling::Active, rng)
    }

    fn set_scheduling(
        &mut self,
        store: &ServerStore,
        desired: codec::Scheduling,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original
            .pin
            .validate_original(store, self.metadata.group.as_slice())?;
        self.creation.snapshot_limits(store)?;
        store.retry_pending_join_catalog()?;
        let mut current = store.pending_join_summary()?;
        if !current
            .rows
            .iter()
            .flatten()
            .any(|row| row.source == self.metadata.source)
        {
            self.save_pending(store, rng)?;
            current = store.pending_join_summary()?;
        }
        let row = current
            .rows
            .iter()
            .flatten()
            .find(|row| row.source == self.metadata.source)
            .ok_or_else(refused)?;
        if let Some(target) = &self.transition {
            if target.scheduling != desired {
                return Err(refused());
            }
            if row == target {
                self.metadata = target.clone();
                self.transition = None;
                self.expected = current;
                return self.confirm_existing(store);
            }
        }
        if row != &self.metadata {
            return Err(refused());
        }
        if row.scheduling == desired {
            return self.confirm_existing(store);
        }
        // Original negative closure precedes the dependent scheduling write. An uncertain
        // failure cannot keep an old worker running or renew it from the unchanged revision.
        store.close_original_join_predecessor(
            &self.original.capture,
            &self.metadata.source,
            self.metadata.revision,
        )?;
        let mut target = row.clone();
        target.revision = target.revision.checked_add(1).ok_or_else(refused)?;
        target.scheduling = desired;
        let policy = store.pending_join_policy.as_ref().ok_or_else(refused)?;
        let write_fee = policy.pool.try_reserve(WRITE_FEE)?;
        let read_fee = policy.pool.try_reserve(READ_FEE)?;
        let (file, raw) = store
            .open_pending_join_catalog_slot(Slot::Current, &read_fee)?
            .ok_or_else(refused)?;
        if *blake3::hash(&raw).as_bytes() != current.framed {
            return Err(refused());
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
        let parsed = store.parse_configured_join_catalog(&plain)?;
        if parsed.digest != current.digest {
            return Err(refused());
        }
        let candidate_plain = encode_schedule(&parsed, &target, &store.chat_vault_domain()?)?;
        let candidate_parse = store.parse_configured_join_catalog(&candidate_plain)?;
        let mut next = candidate_parse.summary([0; 32]);
        codec::successor(&current, &next)?;
        let encoded = Zeroizing::new(frame(&seal(&key, &candidate_plain, rng)?));
        next.framed = *blake3::hash(&encoded).as_bytes();
        let candidate = PendingCandidate {
            identity: Arc::new(()),
            bytes: encoded,
            next: next.clone(),
            expected: current,
            original: self.original.clone(),
            created: None,
            execution_task: None,
            received_input: None,
            continuation_outcome: None,
            _fee: write_fee,
        };
        // Parsed views own no buffers; their borrows end at the last use above. Drop only the
        // actual byte/file/quota owners, avoiding a meaningless drop of those fixed views.
        drop(candidate_plain);
        drop(plain);
        drop(raw);
        drop(file);
        drop(read_fee);
        drop(key);
        // Remember only THIS intended transition, before it can reach I/O. A later revision
        // from another preparation cannot be adopted as a retry of this old object.
        self.transition = Some(target.clone());
        store.install_pending_join_candidate(candidate)?;
        self.metadata = target;
        self.transition = None;
        self.expected = next;
        Ok(())
    }

    /// Save ONLY exact Pending DATA under its original admission/catalog CAS. On ambiguous I/O
    /// the per-mount serializer retains the SAME complete framed candidate independently of
    /// this caller. No ready owner, network method or original-key getter follows from success.
    pub fn save_pending(
        &mut self,
        store: &ServerStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.original
            .pin
            .validate_original(store, self.metadata.group.as_slice())?;
        self.creation.snapshot_limits(store)?;
        // Existing exact source coalescing is DATA only. Crossing its actual original file/root
        // barrier again does not mint a Fresh/Current or recovered network continuation.
        if matches!(self.source, Source::Recovered(_)) {
            return self.confirm_existing(store);
        }
        store.retry_or_install_pending(self, rng)
    }

    fn confirm_existing(&self, store: &ServerStore) -> Result<(), AppError> {
        if let Source::Recovered(source) = &self.source {
            if source.group() != self.metadata.group.as_slice()
                || source.signer() != &self.metadata.signer
                || source.source_binding() != &self.metadata.binding
            {
                return Err(refused());
            }
        }
        let opened = store.inspect_pending_join_catalog()?;
        if !opened
            .selected
            .rows
            .iter()
            .flatten()
            .any(|row| row == &self.metadata)
        {
            return Err(refused());
        }
        store.with_original_join_catalog_capture(&self.original.capture, |permit| {
            let mounted = store.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_ref().ok_or_else(refused)?;
            permit.validate_original()?;
            self.original
                .pin
                .validate_original(store, self.metadata.group.as_slice())?;
            if mounted.pending.is_some() || mounted.current.digest != opened.selected.digest {
                return Err(refused());
            }
            permit.validate_original()?;
            // No source changes after this original admission point. The physical barrier
            // confirms already-known DATA only; mutation is limited to an inspected duplicate.
            store.settle_pending_join_catalog(&opened)
        })
    }
}

impl ServerStore {
    fn pending_join_summary(&self) -> Result<codec::Summary, AppError> {
        let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
        let mounted = mounted.as_ref().ok_or_else(refused)?;
        if mounted.pending.is_some() {
            return Err(refused());
        }
        Ok(mounted.current.clone())
    }

    /// Retry only the SAME original per-mount output. No input labels, replacement source,
    /// keys or RNG choose a different candidate. This does not start or authorize a join.
    pub fn retry_pending_join_catalog(&self) -> Result<(), AppError> {
        use crate::store::chat_admission::journal_basis::JoinCatalogWritePermit;
        let original = {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            let mounted = mounted.as_ref().ok_or_else(refused)?;
            mounted
                .pending
                .as_ref()
                .map(|pending| (pending.original.clone(), pending.retained_identity()))
        };
        let Some((original, identity)) = original else {
            return Ok(());
        };
        // This opaque identity comes from mounted.pending, never decoded status/labels. Final
        // matching under BOTH owners proves the same previously admitted immutable object.
        let finished =
            self.with_retained_join_catalog_repair(&original.capture, &identity, |permit| {
                self.settle_pending_join_candidate(
                    JoinCatalogWritePermit::Retained(permit),
                    &original,
                )
            })?;
        drop(finished); // Last byte/source/fee owners drain outside BOTH writers.
        Ok(())
    }

    fn settle_pending_join_candidate(
        &self,
        permit: crate::store::chat_admission::journal_basis::JoinCatalogWritePermit<'_, '_>,
        original: &Arc<Original>,
    ) -> Result<Option<PendingCandidate>, AppError> {
        let mut mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
        let mounted = mounted.as_mut().ok_or_else(refused)?;
        original.pin.validate_original(self, original.pin.group())?;
        let Some(pending) = mounted.pending.as_mut() else {
            return Ok(None);
        };
        permit.validate_candidate(pending)?;
        if !Arc::ptr_eq(&pending.original, original)
            || mounted.current.digest != pending.expected.digest
            || mounted.current.framed != pending.expected.framed
        {
            return Err(refused());
        }
        // No negative reclassification after this point. SAME already-admitted DATA repairs
        // byte-for-byte, never creates a ticket, and cannot turn cancellation into source loss.
        self.write_pending_join_catalog_candidate(permit, pending)?;
        if let Some(outcome) = &pending.continuation_outcome {
            // Actual file/rename/root barriers above mint this original SavedData result.
            // It contains no sender and cannot transition its reserved task Active.
            outcome.mark_saved_data(pending.next.framed)?;
        }
        mounted.current = pending.next.clone();
        Ok(mounted.pending.take())
    }

    fn retry_or_install_pending(
        &self,
        preparation: &mut JournalledCheckpointJoinPreparation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.retry_pending_join_catalog()?;
        let already = {
            let mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
            mounted
                .as_ref()
                .ok_or_else(refused)?
                .current
                .rows
                .iter()
                .flatten()
                .any(|row| row == &preparation.metadata)
        };
        if already {
            return preparation.confirm_existing(self);
        }
        let policy = self.pending_join_policy.as_ref().ok_or_else(refused)?;
        let write_fee = policy.pool.try_reserve(WRITE_FEE)?;
        let read_fee = policy.pool.try_reserve(READ_FEE)?;
        let (file, raw) = self
            .open_pending_join_catalog_slot(Slot::Current, &read_fee)?
            .ok_or_else(refused)?;
        if *blake3::hash(&raw).as_bytes() != preparation.expected.framed {
            return Err(refused());
        }
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?)?);
        let current = self.parse_configured_join_catalog(&plain)?;
        if current.digest != preparation.expected.digest {
            return Err(refused());
        }
        let Source::Local(source) = &mut preparation.source else {
            return Err(refused());
        };
        let sealed = source.seal_source(&key, preparation.metadata.binding, rng)?;
        let (nonce, ciphertext) = sealed.encrypted_parts();
        let mut encrypted = Zeroizing::new(Vec::with_capacity(24 + ciphertext.len()));
        encrypted.extend_from_slice(nonce);
        encrypted.extend_from_slice(ciphertext);
        let candidate_plain = encode_added(
            &current,
            &preparation.metadata,
            &preparation.immutable,
            &encrypted,
            &self.chat_vault_domain()?,
        )?;
        let candidate_parse = self.parse_configured_join_catalog(&candidate_plain)?;
        let mut next = candidate_parse.summary([0; 32]);
        codec::successor(&preparation.expected, &next)?;
        if !next
            .rows
            .iter()
            .flatten()
            .any(|row| row == &preparation.metadata)
        {
            return Err(refused());
        }
        let encoded = Zeroizing::new(frame(&seal(&key, &candidate_plain, rng)?));
        next.framed = *blake3::hash(&encoded).as_bytes();
        let candidate = PendingCandidate {
            identity: Arc::new(()),
            bytes: encoded,
            next,
            expected: preparation.expected.clone(),
            original: preparation.original.clone(),
            created: None,
            execution_task: None,
            received_input: None,
            continuation_outcome: None,
            _fee: write_fee,
        };
        // Release ALL large parsing/encoding scratch before taking original writers. The
        // retained framed candidate keeps its own actual fee through every ambiguous outcome.
        // The fixed parsed views own no data and are no longer borrowed here. Actual buffers
        // and their fee are released below before entering original synchronous writers.
        drop(candidate_plain);
        drop(encrypted);
        drop(plain);
        drop(raw);
        drop(file);
        drop(read_fee);
        drop(key);
        self.install_pending_join_candidate(candidate)
    }

    fn install_pending_join_candidate(&self, candidate: PendingCandidate) -> Result<(), AppError> {
        use crate::store::chat_admission::journal_basis::JoinCatalogWritePermit;
        let original = candidate.original.clone();
        let received = candidate.received_input.is_some();
        let mut candidate = Some(candidate);
        let mut retired_observation = None;
        if received {
            self.with_received_join_input_capture(&original.capture, |permit| {
                self.admit_pending_join_candidate(
                    JoinCatalogWritePermit::Received(permit),
                    &original,
                    &mut candidate,
                    &mut retired_observation,
                )
            })?;
        } else {
            self.with_original_join_catalog_capture(&original.capture, |permit| {
                self.admit_pending_join_candidate(
                    JoinCatalogWritePermit::Original(permit),
                    &original,
                    &mut candidate,
                    &mut retired_observation,
                )
            })?;
        }
        drop(retired_observation); // Never refund metadata/drop callbacks inside either writer.
                                   // Store already owns the exact immutable DATA candidate before fallible I/O/return.
        self.retry_pending_join_catalog()
    }

    fn admit_pending_join_candidate(
        &self,
        permit: crate::store::chat_admission::journal_basis::JoinCatalogWritePermit<'_, '_>,
        original: &Arc<Original>,
        candidate: &mut Option<PendingCandidate>,
        retired_observation: &mut Option<
            crate::store::chat_admission::predecessor::OriginalJoinExecutionObservation,
        >,
    ) -> Result<(), AppError> {
        #[cfg(test)]
        {
            let hook = self
                .pending_join_before_catalog_writer
                .lock()
                .map_err(|_| refused())?
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let mut mounted = self.pending_join_catalog.lock().map_err(|_| refused())?;
        let mounted = mounted.as_mut().ok_or_else(refused)?;
        let pending = candidate.as_mut().ok_or_else(refused)?;
        if pending.continuation_outcome.is_some() {
            return Err(refused());
        }
        permit.validate_candidate(pending)?; // Final same-held Runtime check AFTER catalog wait.
        original.pin.validate_original(self, original.pin.group())?;
        if mounted.pending.is_some()
            || mounted.current.digest != pending.expected.digest
            || mounted.current.framed != pending.expected.framed
        {
            return Err(refused());
        }
        if let Some(observation) = pending.execution_task.as_ref() {
            permit.require_original_join_execution(observation)?;
        }
        // Consuming the original observation and Store admission are one no-await transition.
        // Already-admitted repair needs no task liveness; first admission always checked it.
        *retired_observation = pending.execution_task.take();
        mounted.pending = candidate.take();
        Ok(())
    }
}

#[cfg(test)]
mod tests;

fn encode_schedule(
    current: &codec::Parsed<'_>,
    target: &codec::Metadata,
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut payload = 4 + join_payload_tag(current)?.len() + 4usize;
    for row in current.rows.iter().flatten() {
        payload = payload
            .checked_add(
                4 + encoded_row_len(
                    &row.metadata,
                    row.immutable,
                    row.encrypted,
                    row.execution_input.as_ref(),
                )?,
            )
            .ok_or_else(refused)?;
    }
    let total = (4 + codec::TAG_V2.len() + 36 + 8 + 36 + 4 + 12usize)
        .checked_add(payload)
        .and_then(|total| total.checked_add(archive_suffix_length(current).ok()?))
        .ok_or_else(refused)?;
    if payload > codec::MAX_JOIN_PAYLOAD || total > codec::MAX_CATALOG_PLAIN {
        return Err(refused());
    }
    let mut output = Encoder::with_capacity(total);
    output.put_str(codec::TAG_V2).map_err(|_| refused())?;
    output.put_bytes(domain).map_err(|_| refused())?;
    output.put_u64(current.generation.checked_add(1).ok_or_else(refused)?);
    output.put_bytes(&current.digest).map_err(|_| refused())?;
    output.put_u32(
        1 + u32::from(current.archive.is_some()) + u32::from(current.current_layout.is_some()),
    );
    encode_join_entry_header(&mut output, current, payload)?;
    output
        .put_str(join_payload_tag(current)?)
        .map_err(|_| refused())?;
    output.put_u32(current.count as u32);
    let mut changed = false;
    for row in current.rows.iter().flatten() {
        if row.metadata.source == target.source {
            encode_row(
                &mut output,
                target,
                row.immutable,
                row.encrypted,
                row.execution_input.as_ref(),
            )?;
            changed = true;
        } else {
            encode_row(
                &mut output,
                &row.metadata,
                row.immutable,
                row.encrypted,
                row.execution_input.as_ref(),
            )?;
        }
    }
    append_original_archive(&mut output, current)?;
    if !changed || output.len() != total {
        return Err(refused());
    }
    Ok(Zeroizing::new(output.finish()))
}

fn join_payload_tag(current: &codec::Parsed<'_>) -> Result<&'static str, AppError> {
    match current.join_version {
        None | Some(codec::JOIN_VERSION) => Ok(codec::JOIN_TAG),
        Some(super::execution_payload::VERSION) => Ok(super::execution_payload::TAG),
        _ => Err(refused()),
    }
}
fn encoded_row_len(
    metadata: &codec::Metadata,
    immutable: &[u8],
    encrypted: &[u8],
    input: Option<&super::execution_state::Input<'_>>,
) -> Result<usize, AppError> {
    if metadata.execution.is_some() {
        super::execution_payload::encoded_row_len(metadata, immutable, encrypted, input)
    } else {
        if input.is_some() {
            return Err(refused());
        }
        codec::original_row_len(immutable, encrypted)
    }
}

/// Exact borrowed descriptor bytes, not a re-created layout or an ignored second namespace.
/// All payload/framing bytes are included in the writer's existing bounded catalog invoice.
fn archive_suffix_length(current: &codec::Parsed<'_>) -> Result<usize, AppError> {
    let archive = match (&current.archive, current.archive_payload) {
        (None, None) => Ok(0),
        (Some(_), Some(payload)) => 12usize.checked_add(payload.len()).ok_or_else(refused),
        _ => Err(refused()),
    }?;
    let current_layout = match (current.current_layout, current.current_payload) {
        (None, None) => 0,
        (Some(_), Some(payload)) => 12usize.checked_add(payload.len()).ok_or_else(refused)?,
        _ => return Err(refused()),
    };
    archive.checked_add(current_layout).ok_or_else(refused)
}

/// Outer header only; payload version/tag/rows remain the actual JOIN consumer's contract.
/// No namespace preservation or archive-layout selection is delegated to payload writers.
fn encode_join_entry_header(
    output: &mut Encoder,
    current: &codec::Parsed<'_>,
    payload: usize,
) -> Result<(), AppError> {
    if payload > codec::MAX_JOIN_PAYLOAD {
        return Err(refused());
    }
    output.put_u32(codec::JOIN_ID);
    output.put_u32(current.join_version.unwrap_or(codec::JOIN_VERSION));
    output.put_u32(payload as u32);
    Ok(())
}

fn append_original_archive(
    output: &mut Encoder,
    current: &codec::Parsed<'_>,
) -> Result<(), AppError> {
    archive_suffix_length(current)?;
    if let Some(payload) = current.archive_payload {
        output.put_u32(super::super::archive_bootstrap::ARCHIVE_ID);
        output.put_u32(super::super::archive_bootstrap::ARCHIVE_VERSION);
        output.put_bytes(payload).map_err(|_| refused())?;
    }
    if let Some(payload) = current.current_payload {
        output.put_u32(super::super::archive_current_namespace_codec::NAMESPACE_ID);
        output.put_u32(super::super::archive_current_namespace_codec::NAMESPACE_VERSION);
        output.put_bytes(payload).map_err(|_| refused())?;
    }
    Ok(())
}
fn encode_row(
    output: &mut Encoder,
    metadata: &codec::Metadata,
    immutable: &[u8],
    encrypted: &[u8],
    input: Option<&super::execution_state::Input<'_>>,
) -> Result<(), AppError> {
    if metadata.execution.is_some() {
        super::execution_payload::encode_row(output, metadata, immutable, encrypted, input)
    } else {
        if input.is_some() {
            return Err(refused());
        }
        codec::encode_original_row(output, metadata, immutable, encrypted)
    }
}

fn encode_added(
    current: &codec::Parsed<'_>,
    metadata: &codec::Metadata,
    immutable: &[u8],
    encrypted: &[u8],
    domain: &[u8; 32],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if current.count >= codec::MAX_ROWS {
        return Err(refused());
    }
    let mut payload = (4 + join_payload_tag(current)?.len() + 4usize)
        .checked_add(4 + encoded_row_len(metadata, immutable, encrypted, None)?)
        .ok_or_else(refused)?;
    for row in current.rows.iter().flatten() {
        payload = payload
            .checked_add(
                4 + encoded_row_len(
                    &row.metadata,
                    row.immutable,
                    row.encrypted,
                    row.execution_input.as_ref(),
                )?,
            )
            .ok_or_else(refused)?;
    }
    let total = (4 + codec::TAG_V2.len() + 36 + 8 + 36 + 4 + 12usize)
        .checked_add(payload)
        .and_then(|total| total.checked_add(archive_suffix_length(current).ok()?))
        .ok_or_else(refused)?;
    if payload > codec::MAX_JOIN_PAYLOAD || total > codec::MAX_CATALOG_PLAIN {
        return Err(refused());
    }
    let mut output = Encoder::with_capacity(total);
    output.put_str(codec::TAG_V2).map_err(|_| refused())?;
    output.put_bytes(domain).map_err(|_| refused())?;
    output.put_u64(current.generation.checked_add(1).ok_or_else(refused)?);
    output.put_bytes(&current.digest).map_err(|_| refused())?;
    output.put_u32(
        1 + u32::from(current.archive.is_some()) + u32::from(current.current_layout.is_some()),
    );
    encode_join_entry_header(&mut output, current, payload)?;
    output
        .put_str(join_payload_tag(current)?)
        .map_err(|_| refused())?;
    output.put_u32((current.count + 1) as u32);
    let mut added = false;
    for row in current.rows.iter().flatten() {
        if !added && metadata.source < row.metadata.source {
            encode_row(&mut output, metadata, immutable, encrypted, None)?;
            added = true;
        }
        encode_row(
            &mut output,
            &row.metadata,
            row.immutable,
            row.encrypted,
            row.execution_input.as_ref(),
        )?;
    }
    if !added {
        encode_row(&mut output, metadata, immutable, encrypted, None)?;
    }
    append_original_archive(&mut output, current)?;
    if output.len() != total {
        return Err(refused());
    }
    Ok(Zeroizing::new(output.finish()))
}
