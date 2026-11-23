//! Read only the named immutable predecessors of one selected later Add.
use super::super::removal::{self, RemovalRecord};
use super::*;
use crate::store::persistence::v2_text::io::{
    archive_child, read_control_into, require_named, OpenedTextControl,
};
use catcoms_replication::chat::owner_bodies::RemovalBody;
use catcoms_sync::{
    V2AdmissionStepData, V2FirstAddProofData, V2FirstJoinDeliveryData, V2GoverningJoinDeliveryData,
    V2GoverningJoinPrefixData, V2GoverningJoinStepData, MAX_V2_GOVERNING_JOIN_PREFIX_BYTES,
    MAX_V2_GOVERNING_JOIN_STEPS, MAX_V2_JOIN_REQUEST_PACKET_BYTES,
};
use selected_head::GovernedHeadKind;
use std::path::Path;

/// Small held identity per immutable predecessor. Ciphertext and SDK buffers
/// are released after each read. The selected head and its own archive have
/// their independent covering check.
struct Predecessor {
    head: [u8; selected_head::FRAME_LEN],
    id: ArchiveHeadId,
    kind: GovernedHeadKind,
    file: OpenedTextControl,
}

struct PriorRequest {
    digest: Option<[u8; 32]>,
    fetch_verifier: Option<[u8; 32]>,
}

pub(super) struct PublicStep {
    pub(super) header: SignedOwnerHeader,
    body: MembershipBody,
    pub(super) image: Vec<u8>,
}

enum MembershipBody {
    Add(AdmissionBody),
    Remove(RemovalBody),
}

pub(super) enum RecoveredMembershipJournal {
    Add(RecoveredAddJournal),
    Remove(Zeroizing<Vec<u8>>),
}

pub(super) struct RecoveredArchiveChain {
    predecessors: Vec<Predecessor>,
    pub(super) journal: RecoveredMembershipJournal,
    first: PublicStep,
    successors: Vec<PublicStep>,
    prior_requests: Vec<PriorRequest>,
    pub(super) prefix: Vec<u8>,
}

impl RecoveredArchiveChain {
    pub(super) const fn invoice_bytes() -> usize {
        use catcoms_replication::chat::owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE;
        4 * (MAX_BYTES + 1)
            + 4 * MAX_V2_GOVERNING_JOIN_PREFIX_BYTES
            + 2 * REPLAY_MATH_PRIVATE_INVOICE
            + 4 * std::mem::size_of::<Self>()
            + MAX_V2_GOVERNING_JOIN_STEPS
                * (4 * std::mem::size_of::<Predecessor>()
                    + 4 * std::mem::size_of::<PublicStep>()
                    + 4 * std::mem::size_of::<PriorRequest>()
                    + 128)
    }

    /// Both scans share one caller-paid immutable archive buffer. The first
    /// follows ciphertext references back to the anchored first Add. The
    /// second verifies signed journals in forward order on those same files.
    pub(super) fn capture(
        key: &[u8; 32],
        directory: &Path,
        raw_head: &[u8],
        latest_archive: &[u8],
        (context, original_head, original_archive): (
            TextContextIdData,
            TextHeadIdData,
            ArchiveHeadId,
        ),
        trusted: &GenesisPublicData<'_>,
        mut require_original: impl FnMut() -> Result<(), AppError>,
    ) -> Result<Self, AppError> {
        require_original()?;
        let selected = selected_head::GovernedHeadData::open_governed_data(key, raw_head)?;
        let count = usize::try_from(selected.transition.epoch).map_err(|_| refused())?;
        if !(2..=MAX_V2_GOVERNING_JOIN_STEPS).contains(&count)
            || selected.transition.context != context
        {
            return Err(refused());
        }
        let mut buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
        let predecessors = capture_predecessors(
            key,
            directory,
            &selected,
            latest_archive,
            (original_head, original_archive),
            &mut buffer,
            &mut require_original,
        )?;

        let mut originals = predecessors.iter();
        let first = originals.next().ok_or_else(refused)?;
        require_original()?;
        let mut archive = first.reread(directory, &mut buffer)?;
        let (first_add, mut replay) =
            authenticate_first_archive(key, &first.head, &archive.frame, context, trusted)?;
        buffer = Some(std::mem::take(&mut archive.frame));
        let first = PublicStep {
            image: first_add.image_data()?.to_vec(),
            header: first_add.header.clone(),
            body: MembershipBody::Add(first_add.admission.clone()),
        };
        let first_proof = first.first_proof(trusted)?;
        let mut prior_requests = Vec::with_capacity(count - 1);
        prior_requests.push(PriorRequest {
            digest: Some(*blake3::hash(&first_add.request).as_bytes()),
            fetch_verifier: None,
        });
        drop(first_add);
        let mut successors = Vec::with_capacity(count - 1);
        let mut public_bytes = first_proof.len();
        for original in originals {
            require_original()?;
            let mut archive = original.reread(directory, &mut buffer)?;
            let journal = replay.verify_membership(key, &original.head, &archive.frame, trusted)?;
            prior_requests.push(journal.prior_request()?);
            buffer = Some(std::mem::take(&mut archive.frame));
            successors.push(journal.public_step(&mut public_bytes)?);
        }
        require_original()?;
        let journal = replay.verify_membership(key, raw_head, latest_archive, trusted)?;
        successors.push(journal.public_step(&mut public_bytes)?);
        let steps: Vec<_> = successors.iter().map(PublicStep::proof_data).collect();
        let prefix = V2GoverningJoinPrefixData::encode(&first_proof, &steps)?;
        // In-place changes during the forward replay keep the same file ID.
        // Recheck each authenticated archive digest before publishing the chain.
        for original in &predecessors {
            require_original()?;
            let mut archive = original.reread(directory, &mut buffer)?;
            buffer = Some(std::mem::take(&mut archive.frame));
        }
        require_original()?;
        let chain = Self {
            predecessors,
            journal,
            first,
            successors,
            prior_requests,
            prefix,
        };
        chain.require_original_predecessors(directory)?;
        Ok(chain)
    }

    pub(super) fn require_original_predecessors(&self, directory: &Path) -> Result<(), AppError> {
        for original in &self.predecessors {
            require_named(
                &archive_child(directory, original.id.as_bytes())?,
                &original.file,
            )?;
        }
        Ok(())
    }

    pub(super) fn public_steps(&self) -> impl Iterator<Item = &PublicStep> {
        std::iter::once(&self.first).chain(&self.successors)
    }

    pub(super) fn encode_successor_proof(
        &self,
        trusted: &GenesisPublicData<'_>,
        next: V2GoverningJoinStepData<'_>,
    ) -> Result<Vec<u8>, AppError> {
        let proof = self.first.first_proof(trusted)?;
        let steps: Vec<_> = self
            .successors
            .iter()
            .map(PublicStep::proof_data)
            .chain(std::iter::once(next))
            .collect();
        Ok(V2GoverningJoinPrefixData::encode(&proof, &steps)?)
    }

    pub(super) fn encode_join_delivery(
        &self,
        key: &[u8; 32],
        directory: &Path,
        trusted: &GenesisPublicData<'_>,
        request: &[u8],
    ) -> Result<Vec<u8>, AppError> {
        if !(1..=MAX_V2_JOIN_REQUEST_PACKET_BYTES).contains(&request.len()) {
            return Err(refused());
        }
        if let RecoveredMembershipJournal::Add(journal) = &self.journal {
            if journal.request.as_slice() == request {
                return Ok(V2GoverningJoinDeliveryData::encode(
                    &journal.response,
                    &self.prefix,
                )?);
            }
        }
        let requested = blake3::hash(request);
        let index = self
            .prior_requests
            .iter()
            .position(|prior| prior.digest.as_ref() == Some(requested.as_bytes()))
            .ok_or_else(refused)?;
        let first_proof = self.first.first_proof(trusted)?;
        if index == 0 {
            let original = self.predecessors.first().ok_or_else(refused)?;
            let selected =
                selected_head::GovernedHeadData::open_governed_data(key, &original.head)?;
            let mut buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
            let archive = original.reread(directory, &mut buffer)?;
            let (first, _) = authenticate_first_archive(
                key,
                &original.head,
                &archive.frame,
                selected.transition.context,
                trusted,
            )?;
            if first.request.as_slice() != request {
                return Err(refused());
            }
            return Ok(V2FirstJoinDeliveryData::encode(
                &first.response,
                &first_proof,
            )?);
        }
        let RecoveredMembershipJournal::Add(journal) =
            self.read_later_journal(key, directory, trusted, index)?
        else {
            return Err(refused());
        };
        if journal.request.as_slice() != request {
            return Err(refused());
        }
        let steps: Vec<_> = self
            .successors
            .iter()
            .take(index)
            .map(PublicStep::proof_data)
            .collect();
        let proof = V2GoverningJoinPrefixData::encode(&first_proof, &steps)?;
        Ok(V2GoverningJoinDeliveryData::encode(
            &journal.response,
            &proof,
        )?)
    }

    pub(super) fn encode_member_add(
        &self,
        key: &[u8; 32],
        directory: &Path,
        trusted: &GenesisPublicData<'_>,
        request: &V2MemberAddRequestData,
    ) -> Result<Vec<u8>, AppError> {
        let (parent, epoch) = request.parent_data();
        let index = usize::try_from(epoch).map_err(|_| refused())?;
        if index == self.predecessors.len() {
            return self.journal.encode_member_add(request);
        }
        let prior = self.prior_requests.get(index).ok_or_else(refused)?;
        let step = index
            .checked_sub(1)
            .and_then(|i| self.successors.get(i))
            .ok_or_else(refused)?;
        if parent != step.header.spec().parent
            || !prior
                .fetch_verifier
                .is_some_and(|expected| request.matches_verifier_data(expected))
        {
            return Err(refused());
        }
        self.read_later_journal(key, directory, trusted, index)?
            .encode_member_add(request)
    }

    fn read_later_journal(
        &self,
        key: &[u8; 32],
        directory: &Path,
        trusted: &GenesisPublicData<'_>,
        index: usize,
    ) -> Result<RecoveredMembershipJournal, AppError> {
        let original = self
            .predecessors
            .get(index)
            .filter(|record| record.kind != GovernedHeadKind::FirstAdd)
            .ok_or_else(refused)?;
        let selected = selected_head::GovernedHeadData::open_governed_data(key, &original.head)?;
        let mut buffer = Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1)));
        let archive = original.reread(directory, &mut buffer)?;
        let mut replay = AddReplay::from_genesis(trusted)?;
        for step in self.public_steps().take(index) {
            replay.core = replay
                .core
                .derive_successor(&step.header, step.body_data(), &step.image)
                .map_err(|_| refused())?
                .next;
        }
        replay.selected = Some((
            selected.transition.before_head,
            selected.transition.before_archive,
        ));
        replay.verify_membership(key, &original.head, &archive.frame, trusted)
    }
}

fn capture_predecessors(
    key: &[u8; 32],
    directory: &Path,
    selected: &selected_head::GovernedHeadData,
    latest_archive: &[u8],
    (original_head, original_archive): (TextHeadIdData, ArchiveHeadId),
    buffer: &mut Option<Zeroizing<Vec<u8>>>,
    require_original: &mut impl FnMut() -> Result<(), AppError>,
) -> Result<Vec<Predecessor>, AppError> {
    let count = usize::try_from(selected.transition.epoch).map_err(|_| refused())?;
    if !(2..=MAX_V2_GOVERNING_JOIN_STEPS).contains(&count) {
        return Err(refused());
    }
    let mut predecessors = Vec::with_capacity(count - 1);
    let mut head = predecessor_head(key, selected, latest_archive)?;
    loop {
        require_original()?;
        let prior = selected_head::GovernedHeadData::open_governed_data(key, &head)?;
        let path = archive_child(directory, prior.archive.as_bytes())?;
        let mut file = read_control_into(&path, MAX_BYTES, buffer)?;
        let previous = if prior.transition.epoch > 1 {
            Some(predecessor_head(key, &prior, &file.frame)?)
        } else {
            if prior.transition.before_head != original_head
                || prior.transition.before_archive != original_archive
                || id_data(&file.frame) != prior.archive
            {
                return Err(refused());
            }
            None
        };
        *buffer = Some(std::mem::take(&mut file.frame));
        predecessors.push(Predecessor {
            head,
            id: prior.archive,
            kind: prior.kind,
            file,
        });
        if predecessors.len() >= count {
            return Err(refused());
        }
        let Some(previous) = previous else { break };
        head = previous;
    }
    if predecessors.len() + 1 != count {
        return Err(refused());
    }
    predecessors.reverse();
    Ok(predecessors)
}

fn predecessor_head(
    key: &[u8; 32],
    head: &selected_head::GovernedHeadData,
    archive: &[u8],
) -> Result<[u8; selected_head::FRAME_LEN], AppError> {
    if head.kind == GovernedHeadKind::Remove {
        let plain = removal::open_data(key, head, archive)?;
        return RemovalRecord::decode(&plain)?
            .parent_head
            .try_into()
            .map_err(|_| refused());
    }
    let plain = open_later_data(key, head, archive)?;
    let (_, frames) = decode_later_data(&plain)?;
    frames.parent_head.try_into().map_err(|_| refused())
}

impl Predecessor {
    fn reread(
        &self,
        directory: &Path,
        buffer: &mut Option<Zeroizing<Vec<u8>>>,
    ) -> Result<OpenedTextControl, AppError> {
        let path = archive_child(directory, self.id.as_bytes())?;
        require_named(&path, &self.file)?;
        let archive = read_control_into(&path, MAX_BYTES, buffer)?;
        let expected = match self.kind {
            GovernedHeadKind::FirstAdd => id_data(&archive.frame),
            GovernedHeadKind::Add => later_id(&archive.frame),
            GovernedHeadKind::Remove => removal::archive_id(&archive.frame),
        };
        if archive.id != self.file.id || expected != self.id {
            return Err(refused());
        }
        require_named(&path, &self.file)?;
        Ok(archive)
    }
}

impl AddReplay {
    fn verify_membership(
        &mut self,
        key: &[u8; 32],
        raw_head: &[u8],
        ciphertext: &[u8],
        trusted: &GenesisPublicData<'_>,
    ) -> Result<RecoveredMembershipJournal, AppError> {
        let selected = selected_head::GovernedHeadData::open_governed_data(key, raw_head)?;
        if selected.kind == GovernedHeadKind::Add {
            return self
                .verify_later_add(key, raw_head, ciphertext, trusted)
                .map(RecoveredMembershipJournal::Add);
        }
        let transition = selected.transition;
        if self.selected != Some((transition.before_head, transition.before_archive)) {
            return Err(refused());
        }
        let plain = removal::open_data(key, &selected, ciphertext)?;
        let record = RemovalRecord::decode(&plain)?;
        let body = record.removal.spec();
        let epoch = self.core.mls_epoch_data();
        if body.before_epoch != epoch || body.after_epoch != transition.epoch {
            return Err(refused());
        }
        catcoms_mls::group::check_commit_header(
            record.commit,
            &body.group,
            body.before_epoch,
            epoch,
        )
        .map_err(|_| refused())?;
        self.core = self
            .core
            .derive_successor(
                record.row.header(),
                ReplayBody::Removal(&record.removal),
                record.row.image(),
            )
            .map_err(|_| refused())?
            .next;
        self.selected = Some((TextHeadIdData::of_data(raw_head), selected.archive));
        Ok(RecoveredMembershipJournal::Remove(plain))
    }
}

impl RecoveredMembershipJournal {
    fn prior_request(&self) -> Result<PriorRequest, AppError> {
        match self {
            Self::Add(journal) => Ok(PriorRequest {
                digest: Some(*blake3::hash(&journal.request).as_bytes()),
                fetch_verifier: journal.fetch_verifier,
            }),
            Self::Remove(raw) => Ok(PriorRequest {
                digest: None,
                fetch_verifier: Some(RemovalRecord::decode(raw)?.fetch_verifier),
            }),
        }
    }

    fn encode_member_add(&self, request: &V2MemberAddRequestData) -> Result<Vec<u8>, AppError> {
        match self {
            Self::Add(add) => add.encode_member_add(request),
            Self::Remove(raw) => RemovalRecord::decode(raw)?.encode_member_transition(request),
        }
    }

    fn public_step(&self, total: &mut usize) -> Result<PublicStep, AppError> {
        match self {
            Self::Add(add) => add.public_step(total),
            Self::Remove(raw) => {
                let step = RemovalRecord::decode(raw)?;
                reserve_public_step(
                    total,
                    step.row.header(),
                    step.row.image(),
                    step.removal.encode().len(),
                )?;
                Ok(PublicStep {
                    header: step.row.header().clone(),
                    body: MembershipBody::Remove(step.removal),
                    image: step.row.image().to_vec(),
                })
            }
        }
    }

    pub(super) fn selected_admission(
        &self,
    ) -> Option<catcoms_replication::chat::owner_facts::OwnerFactId> {
        match self {
            Self::Add(add) => Some(add.admission.admission_id()),
            Self::Remove(_) => None,
        }
    }

    pub(super) fn snapshot_data(&self) -> Result<&[u8], AppError> {
        match self {
            Self::Add(add) => outcome::archive_snapshot(&add.entry, &add.outcome),
            Self::Remove(raw) => Ok(RemovalRecord::decode(raw)?.snapshot),
        }
    }
}

impl RecoveredAddJournal {
    fn public_step(&self, total: &mut usize) -> Result<PublicStep, AppError> {
        let [admission, _, image] = outcome::governing_bodies(&self.entry, &self.outcome)?;
        reserve_public_step(total, &self.header, image, admission.len())?;
        Ok(PublicStep {
            header: self.header.clone(),
            body: MembershipBody::Add(self.admission.clone()),
            image: image.to_vec(),
        })
    }
}

fn reserve_public_step(
    total: &mut usize,
    header: &SignedOwnerHeader,
    image: &[u8],
    body_bytes: usize,
) -> Result<(), AppError> {
    *total = total
        .checked_add(image.len() + header.encode().len() + body_bytes)
        .filter(|sum| *sum <= MAX_V2_GOVERNING_JOIN_PREFIX_BYTES)
        .ok_or_else(refused)?;
    Ok(())
}

impl PublicStep {
    fn first_proof(&self, trusted: &GenesisPublicData<'_>) -> Result<Vec<u8>, AppError> {
        let MembershipBody::Add(admission) = &self.body else {
            return Err(refused());
        };
        Ok(V2FirstAddProofData::encode(
            trusted.profile,
            V2AdmissionStepData {
                admission: trusted.admission,
                header: trusted.header,
                image: trusted.image,
            },
            V2AdmissionStepData {
                admission,
                header: &self.header,
                image: &self.image,
            },
        )?)
    }

    fn proof_data(&self) -> V2GoverningJoinStepData<'_> {
        V2GoverningJoinStepData {
            header: &self.header,
            image: &self.image,
            body: self.body_data(),
        }
    }

    pub(super) fn body_data(&self) -> ReplayBody<'_> {
        match &self.body {
            MembershipBody::Add(body) => ReplayBody::Admission(body),
            MembershipBody::Remove(body) => ReplayBody::Removal(body),
        }
    }
}

#[cfg(test)]
mod tests;
