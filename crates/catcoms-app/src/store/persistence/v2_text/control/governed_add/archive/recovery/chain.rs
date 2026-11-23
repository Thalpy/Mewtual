//! Read only the named immutable predecessors of one selected later Add.
use super::*;
use crate::store::persistence::v2_text::io::{
    archive_child, read_control_into, require_named, OpenedTextControl,
};
use catcoms_sync::{
    V2AdmissionStepData, V2FirstAddProofData, V2GoverningJoinPrefixData, V2GoverningJoinStepData,
    MAX_V2_GOVERNING_JOIN_PREFIX_BYTES, MAX_V2_GOVERNING_JOIN_STEPS,
};
use std::path::Path;

/// Small held identity per immutable predecessor. Ciphertext and SDK buffers
/// are released after each read. The selected head and its own archive have
/// their independent covering check.
struct Predecessor {
    head: [u8; selected_head::FRAME_LEN],
    id: ArchiveHeadId,
    file: OpenedTextControl,
}

pub(super) struct PublicStep {
    pub(super) header: SignedOwnerHeader,
    pub(super) admission: AdmissionBody,
    pub(super) image: Vec<u8>,
}

pub(super) struct RecoveredArchiveChain {
    predecessors: Vec<Predecessor>,
    pub(super) journal: RecoveredAddJournal,
    first: PublicStep,
    successors: Vec<PublicStep>,
    pub(super) prefix: Vec<u8>,
    pub(super) nonces: Vec<[u8; 16]>,
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
        let first_proof = V2FirstAddProofData::encode(
            trusted.profile,
            V2AdmissionStepData {
                admission: trusted.admission,
                header: trusted.header,
                image: trusted.image,
            },
            V2AdmissionStepData {
                admission: &first_add.admission,
                header: &first_add.header,
                image: first_add.image_data()?,
            },
        )?;
        let first = PublicStep {
            image: first_add.image_data()?.to_vec(),
            header: first_add.header.clone(),
            admission: first_add.admission.clone(),
        };
        drop(first_add);
        let mut successors = Vec::with_capacity(count - 1);
        let mut public_bytes = first_proof.len();
        for original in originals {
            require_original()?;
            let mut archive = original.reread(directory, &mut buffer)?;
            let journal = replay.verify_later_add(key, &original.head, &archive.frame, trusted)?;
            buffer = Some(std::mem::take(&mut archive.frame));
            successors.push(journal.public_step(&mut public_bytes)?);
        }
        require_original()?;
        let journal = replay.verify_later_add(key, raw_head, latest_archive, trusted)?;
        successors.push(journal.public_step(&mut public_bytes)?);
        let steps: Vec<_> = successors.iter().map(PublicStep::proof_data).collect();
        let prefix = V2GoverningJoinPrefixData::encode(&first_proof, &steps)?;
        // In-place changes during the forward replay keep the same file ID.
        // Recheck each authenticated archive digest before publishing the chain.
        for (index, original) in predecessors.iter().enumerate() {
            require_original()?;
            let mut archive = original.reread(directory, &mut buffer)?;
            let id = if index == 0 {
                id_data(&archive.frame)
            } else {
                later_id(&archive.frame)
            };
            if id != original.id {
                return Err(refused());
            }
            buffer = Some(std::mem::take(&mut archive.frame));
        }
        require_original()?;
        let chain = Self {
            predecessors,
            journal,
            first,
            successors,
            prefix,
            nonces: replay.nonces,
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
        if archive.id != self.file.id {
            return Err(refused());
        }
        require_named(&path, &self.file)?;
        Ok(archive)
    }
}

impl RecoveredAddJournal {
    fn public_step(&self, total: &mut usize) -> Result<PublicStep, AppError> {
        let [admission, _, image] = outcome::governing_bodies(&self.entry, &self.outcome)?;
        *total = total
            .checked_add(image.len() + self.header.encode().len() + admission.len())
            .filter(|sum| *sum <= MAX_V2_GOVERNING_JOIN_PREFIX_BYTES)
            .ok_or_else(refused)?;
        Ok(PublicStep {
            header: self.header.clone(),
            admission: self.admission.clone(),
            image: image.to_vec(),
        })
    }
}

impl PublicStep {
    fn proof_data(&self) -> V2GoverningJoinStepData<'_> {
        V2GoverningJoinStepData {
            header: &self.header,
            image: &self.image,
            body: ReplayBody::Admission(&self.admission),
        }
    }
}

#[cfg(test)]
mod tests;
