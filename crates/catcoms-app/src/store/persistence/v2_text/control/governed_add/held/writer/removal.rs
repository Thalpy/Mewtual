//! Physical writer for one Remove on the exact selected founder archive.
use super::*;
use crate::store::persistence::v2_text::control::governed_add::archive::removal;
use crate::store::persistence::v2_text::io::archive_child;
use catcoms_replication::chat::owner_header::{OwnerBodyId, OwnerHeaderId, SignedOwnerHeader};
use catcoms_sync::owner_v2::{V2FounderRemoveOutcomeData, V2InstalledFounderAddData};

const ROLE: &[u8] = b"catcoms/private-chat-text-governed-remove/v1\0";
const STEPS: [&[u8]; 4] = [b"intent", b"sdk-entry", b"sdk-outcome", b"header-attempt"];
const RECORD_DOMAIN: &str = "catcoms/private-chat-text-governed-remove-record/v1";
const MAX_RECORD: usize = archive::MAX_BYTES;
const SMALL: usize = 1024;

struct CertifiedOutcome {
    identity: [u8; 32],
    data: PreparedSmallControl,
    _fee: CatalogReadReservation,
}

struct PendingSelection {
    expected: V2InstalledFounderAddData,
    path: PathBuf,
    archive: PreparedSmallControl,
    head: PreparedSmallControl,
    entered: bool,
    _fee: CatalogReadReservation,
}

pub(in crate::store) struct PreparedGovernedRemoveFence {
    paths: [PathBuf; 5],
    prior: HeldColdFounderAdd,
    admission: OwnerBodyId,
    verifier: [u8; 32],
    intent: PreparedSmallControl,
    entry: PreparedSmallControl,
    outcome: Option<CertifiedOutcome>,
    attempt: Option<PreparedSmallControl>,
    selection: Option<PendingSelection>,
    _fee: CatalogReadReservation,
}

impl PreparedGovernedRemoveFence {
    pub(in crate::store) fn require_warm_review_parent_anchor(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        anchor: (OwnerHeaderId, u64),
    ) -> Result<(), AppError> {
        self.prior.require_warm_review_parent_anchor(guard, anchor)
    }

    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        seed: &SelectedV2TextHead,
        installed: (TextHeadIdData, V2InstalledFounderAddData),
        admission: OwnerBodyId,
        verifier: [u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        selected_head::next_supported_epoch(installed.1.epoch)?;
        source.require_original_negative(store)?;
        let prior = HeldColdFounderAdd::capture(
            store,
            source.source(),
            *seed.head_data().context.spec_data().pin.as_bytes(),
        )?;
        prior.require_installed_parent(source.source(), installed.0, installed.1)?;
        let (context, parent, archive, _, _) = prior.writer_parent_data();
        let target = seed.physical_query_data().target;
        let fee = reserve(
            store,
            15 * MAX_TEXT_PATH + 10 * SMALL + 4 * std::mem::size_of::<Self>(),
        )?;
        let child = |name| governed_add_successor_child(target, name, parent.as_bytes());
        let [entry, outcome, attempt, head] = REMOVE_STAGES;
        let paths = [
            child(STAGE)?,
            child(entry)?,
            child(outcome)?,
            child(attempt)?,
            child(head)?,
        ];
        let key = Zeroizing::new(store.keys.db_key()?);
        let intent = seal_record(
            &key,
            0,
            parent.as_bytes(),
            &[
                context.as_bytes(),
                archive.as_bytes(),
                admission.as_bytes(),
                &verifier,
            ],
            rng,
        )?;
        let entry = seal_record(&key, 1, blake3::hash(&intent).as_bytes(), &[], rng)?;
        source.require_original_negative(store)?;
        Ok(Self {
            paths,
            prior,
            admission,
            verifier,
            intent: PreparedSmallControl::new(intent),
            entry: PreparedSmallControl::new(entry),
            outcome: None,
            attempt: None,
            selection: None,
            _fee: fee,
        })
    }

    pub(in crate::store) fn matches_admission(&self, admission: OwnerBodyId) -> bool {
        self.admission == admission
    }

    /// The host only enters the native private candidate after these two
    /// original files have both been covered under Current.
    pub(in crate::store) fn cover_sdk_entry(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.prior.require_writer_parent_current(guard)?;
        self.intent.cover(&self.paths[0], SMALL, guard)?;
        self.entry.cover(&self.paths[1], SMALL, guard)?;
        self.prior.require_writer_parent_current(guard)
    }

    pub(in crate::store) fn prepare_outcome(
        &mut self,
        store: &ServerStore,
        outcome: &V2FounderRemoveOutcomeData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        let parent = self.entry.covered_frame()?;
        let identity = outcome_identity(outcome, self.admission)?;
        if let Some(prepared) = &self.outcome {
            return (prepared.identity == identity)
                .then_some(())
                .ok_or_else(refused);
        }
        let body = outcome.unsigned.body_data().encode();
        let (epoch, auth) = outcome.result.selected_epoch_data();
        let fields = [
            outcome.result.commit_data(),
            outcome.result.secret_snapshot_data(),
            body.as_slice(),
            outcome.unsigned.image_data(),
            outcome.unsigned.signing_bytes_data(),
            &epoch.to_be_bytes(),
            &auth,
        ];
        let size = outcome_size(&fields)?;
        let fee = reserve(store, 5 * (size + 1))?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let raw = seal_record(&key, 2, blake3::hash(parent).as_bytes(), &fields, rng)?;
        let attempt = seal_record(
            &key,
            3,
            blake3::hash(&raw).as_bytes(),
            &[outcome.unsigned.signing_bytes_data()],
            rng,
        )?;
        self.outcome = Some(CertifiedOutcome {
            identity,
            data: PreparedSmallControl::new(raw),
            _fee: fee,
        });
        self.attempt = Some(PreparedSmallControl::new(attempt));
        Ok(())
    }

    pub(in crate::store) fn cover_signing_attempt(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        self.prior.require_writer_parent_current(guard)?;
        self.intent.require_covered(&self.paths[0])?;
        self.entry.require_covered(&self.paths[1])?;
        self.outcome
            .as_mut()
            .ok_or_else(refused)?
            .data
            .cover(&self.paths[2], MAX_RECORD, guard)?;
        self.attempt
            .as_mut()
            .ok_or_else(refused)?
            .cover(&self.paths[3], SMALL, guard)?;
        self.prior.require_writer_parent_current(guard)
    }

    pub(in crate::store) fn prepare_archive(
        &mut self,
        store: &ServerStore,
        header: &SignedOwnerHeader,
        outcome: &V2FounderRemoveOutcomeData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.attempt.as_ref().ok_or_else(refused)?.covered_frame()?;
        if self.outcome.as_ref().ok_or_else(refused)?.identity
            != outcome_identity(outcome, self.admission)?
        {
            return Err(refused());
        }
        let (epoch, authenticator) = outcome.result.selected_epoch_data();
        let expected = V2InstalledFounderAddData {
            epoch,
            authenticator,
            header: header.id(),
            state: header.spec().state,
        };
        if let Some(selection) = &self.selection {
            return (expected == selection.expected)
                .then_some(())
                .ok_or_else(refused);
        }
        let size = outcome_size(&[
            outcome.result.secret_snapshot_data(),
            outcome.result.commit_data(),
            outcome.unsigned.image_data(),
        ])?;
        let fee = reserve(store, 6 * (size + 1))?;
        let original = self.prior.writer_parent_data().4;
        let key = Zeroizing::new(store.keys.db_key()?);
        let prepared = removal::seal_data(&key, original, self.verifier, header, outcome, rng)?;
        let target = self.paths[4].parent().ok_or_else(refused)?;
        let path = archive_child(target, removal::archive_id(&prepared.archive).as_bytes())?;
        self.selection = Some(PendingSelection {
            expected,
            path,
            archive: PreparedSmallControl::new(prepared.archive),
            head: PreparedSmallControl::new(prepared.head),
            entered: false,
            _fee: fee,
        });
        Ok(())
    }

    pub(in crate::store) fn selection_entered(&self) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|selection| selection.entered)
    }

    pub(in crate::store) fn require_selected(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        (head, expected): (TextHeadIdData, V2InstalledFounderAddData),
    ) -> Result<(), AppError> {
        guard.require_governed_native_invite_parent()?;
        let selection = self
            .selection
            .as_mut()
            .filter(|selection| selection.entered && selection.expected == expected)
            .ok_or_else(refused)?;
        let (target, directory, id) = guard.target_directory_data();
        if selection.path.parent() != Some(target)
            || self.paths[4].parent() != Some(target)
            || TextHeadIdData::of_data(selection.head.covered_frame()?) != head
        {
            return Err(refused());
        }
        require_absent(&self.paths[4])?;
        selection.archive.require_covered(&selection.path)?;
        selection
            .head
            .require_covered(&fixed_child(target, "head.bin")?)?;
        self.prior.require_writer_parent_archive(guard)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }

    pub(in crate::store) fn encode_selected_member_transition(
        &mut self,
        store: &ServerStore,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        installed: (TextHeadIdData, V2InstalledFounderAddData),
        request: &catcoms_sync::V2MemberAddRequestData,
    ) -> Result<Vec<u8>, AppError> {
        self.require_selected(guard, installed)?;
        let (_, epoch) = request.parent_data();
        let wire = if epoch.checked_add(1) == Some(installed.1.epoch) {
            let selection = self.selection.as_ref().ok_or_else(refused)?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let head = selected_head::GovernedHeadData::open_governed_data(
                &key,
                selection.head.covered_frame()?,
            )?;
            removal::encode_selected_member_transition(
                &key,
                &head,
                selection.archive.covered_frame()?,
                request,
            )?
        } else {
            self.prior
                .encode_retained_member_add(store, guard.source().source(), request)?
        };
        self.require_selected(guard, installed)?;
        Ok(wire)
    }

    /// Select the exact covered archive with the already prepared matching
    /// Core and MLS pair. A lost rename reply reopens the same selected file.
    pub(in crate::store) fn select(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        expected: V2InstalledFounderAddData,
    ) -> Result<TextHeadIdData, AppError> {
        guard.require_governed_native_invite_parent()?;
        guard.require_text_control_io_quiescent()?;
        let (target, directory, id) = guard.target_directory_data();
        let head_path = fixed_child(target, "head.bin")?;
        let selection = self.selection.as_mut().ok_or_else(refused)?;
        if selection.expected != expected
            || selection.path.parent() != Some(target)
            || self.paths[4].parent() != Some(target)
        {
            return Err(refused());
        }
        self.intent.require_covered(&self.paths[0])?;
        self.entry.require_covered(&self.paths[1])?;
        self.outcome
            .as_mut()
            .ok_or_else(refused)?
            .data
            .require_covered(&self.paths[2])?;
        self.attempt
            .as_mut()
            .ok_or_else(refused)?
            .require_covered(&self.paths[3])?;
        let mut current = read_control(
            &head_path,
            crate::store::persistence::v2_text::head::MAX_TEXT_HEAD_FRAME,
        )?;
        if current.frame.as_slice() == self.prior.writer_parent_data().4 {
            self.prior.require_writer_parent_current(guard)?;
            selection
                .archive
                .cover(&selection.path, MAX_RECORD, guard)?;
            selection
                .head
                .cover(&self.paths[4], selected_head::FRAME_LEN, guard)?;
            select_governed_head(
                guard,
                &self.paths[4],
                &head_path,
                &mut current,
                &mut selection.entered,
            )?;
        } else if !selection.entered
            || current.frame.as_slice() != selection.head.covered_frame()?
        {
            return Err(refused());
        }
        require_absent(&self.paths[4])?;
        selection.archive.require_covered(&selection.path)?;
        selection.head.require_covered(&head_path)?;
        self.prior.require_writer_parent_archive(guard)?;
        crate::store::persistence::sync_directory(target).map_err(io)?;
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()?;
        Ok(TextHeadIdData::of_data(selection.head.covered_frame()?))
    }
}

fn reserve(store: &ServerStore, bytes: usize) -> Result<CatalogReadReservation, AppError> {
    let pool = &store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?
        .pending
        .pool;
    let fee = pool.try_reserve(bytes)?;
    fee.validate(pool, bytes)?;
    Ok(fee)
}

fn outcome_size(fields: &[&[u8]]) -> Result<usize, AppError> {
    fields.iter().try_fold(2 * SMALL, |total, value| {
        total
            .checked_add(value.len())
            .filter(|size| *size <= MAX_RECORD)
            .ok_or_else(refused)
    })
}

fn outcome_identity(
    outcome: &V2FounderRemoveOutcomeData<'_>,
    admission: OwnerBodyId,
) -> Result<[u8; 32], AppError> {
    let spec = outcome.unsigned.body_data().spec();
    let (epoch, auth) = outcome.result.selected_epoch_data();
    if spec.frozen_tip.context.admission.as_bytes() != admission.as_bytes()
        || spec.after_epoch != epoch
    {
        return Err(refused());
    }
    let mut hash = blake3::Hasher::new_derive_key(RECORD_DOMAIN);
    for bytes in [
        outcome.result.commit_data(),
        outcome.result.secret_snapshot_data(),
        outcome.unsigned.signing_bytes_data(),
        outcome.unsigned.image_data(),
        &auth,
    ] {
        hash.update(&(bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    Ok(*hash.finalize().as_bytes())
}

fn seal_record(
    key: &[u8; 32],
    step: usize,
    parent: &[u8; 32],
    fields: &[&[u8]],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut encoded = Encoder::with_capacity(outcome_size(fields)?);
    let (tag, domain, _) = TextControlKindData::Wal.domains();
    put(&mut encoded, tag)?;
    encoded.put_u16(3);
    for value in [ROLE, STEPS[step], parent] {
        put(&mut encoded, value)?;
    }
    for value in fields {
        put(&mut encoded, value)?;
    }
    let plain = Zeroizing::new(encoded.finish());
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}
