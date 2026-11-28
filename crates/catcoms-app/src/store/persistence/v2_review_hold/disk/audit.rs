//! Read the confirmed hold inventory without entering the writer or
//! repairing any of its files. Invalid cases remain visible as gaps.
use super::*;
use catcoms_replication::chat::SignedAction;
use std::{collections::BTreeMap, path::PathBuf};

pub(in crate::store) struct HeldReviewQuery {
    pub(in crate::store) after: Option<[u8; 32]>,
    pub(in crate::store) limit: usize,
}

pub struct SelectedHeldCase {
    pub case: ReviewCaseId,
    pub original: OriginalActionDigest,
    pub removal: (OwnerHeaderId, u64),
    pub admission: catcoms_replication::chat::owner_facts::OwnerFactId,
    pub signed: SignedAction,
}

catcoms_rt::redacted_debug! { SelectedHeldCase; }

pub(in crate::store) struct SelectedHeldWindow {
    pub(in crate::store) records: Vec<SelectedHeldCase>,
    pub(in crate::store) next: Option<[u8; 32]>,
    pub(in crate::store) unavailable: usize,
    _fee: CatalogReadReservation,
}

struct SelectedCaseRead {
    path: PathBuf,
    file: (File, SessionId),
    bytes: Zeroizing<Vec<u8>>,
    witness: receipt::WitnessRead,
}

impl SelectedCaseRead {
    fn require(&mut self) -> Result<(), AppError> {
        require_exact_held(&self.path, &mut self.file, &self.bytes)?;
        self.witness.require()
    }
}

impl SelectedHeldWindow {
    pub(in crate::store) fn empty(fee: CatalogReadReservation) -> Self {
        Self {
            records: Vec::new(),
            next: None,
            unavailable: 0,
            _fee: fee,
        }
    }
}

impl HoldDisk {
    pub(in crate::store::persistence) fn selected_review_window(
        &self,
        key: &[u8; 32],
        query: HeldReviewQuery,
        session: &VaultSessionGuard,
        mut current: impl FnMut() -> Result<(), AppError>,
        mut verify: impl FnMut(&V2RemovedCreateRecord<'_>) -> Result<SignedAction, AppError>,
    ) -> Result<SelectedHeldWindow, AppError> {
        if !(1..=8).contains(&query.limit) {
            return Err(refused());
        }
        let _lease = session.retain_for_physical_work();
        let work = self.pool.try_reserve(WORK)?;
        self.require(key, &work)?;
        current()?;
        let names = bounded_entries(&self.directory, MAX_NAMES)?;
        if names.len() > MAX_NAMES {
            return Err(refused());
        }
        let mut cases = BTreeMap::new();
        let mut unavailable = 0;
        for name in &names {
            let Some((label, selected)) = name.to_str().and_then(parse_leaf) else {
                unavailable += 1;
                continue;
            };
            cases
                .entry(label)
                .and_modify(|held| *held |= selected)
                .or_insert(selected);
        }
        if cases.len() > MAX_CASES {
            return Err(refused());
        }
        let invoice = self
            .pool
            .try_reserve(16 * 1024 + query.limit * 3 * MAX_FRAME)?;
        let mut checked = Vec::with_capacity(query.limit);
        let mut candidates = cases
            .into_iter()
            .filter(|(label, _)| query.after.is_none_or(|after| *label > after));
        let mut last = None;
        for (label, selected) in candidates.by_ref().take(query.limit) {
            current()?;
            last = Some(label);
            if selected {
                match self.open_selected_case(key, label, &mut verify) {
                    Ok(record) => checked.push(record),
                    Err(_) => unavailable += 1,
                }
            } else {
                unavailable += 1;
            }
        }
        let next = last.filter(|_| candidates.next().is_some());
        current()?;
        let mut records = Vec::with_capacity(checked.len());
        // Later rows can take time to verify. Validate the original body
        // and witness again for every candidate before releasing the window.
        for (record, mut proof) in checked {
            if proof.require().is_ok() {
                records.push(record);
            } else {
                unavailable += 1;
            }
        }
        self.directory.require()?;
        current()?;
        Ok(SelectedHeldWindow {
            records,
            next,
            unavailable,
            _fee: invoice,
        })
    }

    fn open_selected_case(
        &self,
        key: &[u8; 32],
        label: [u8; 32],
        verify: &mut impl FnMut(&V2RemovedCreateRecord<'_>) -> Result<SignedAction, AppError>,
    ) -> Result<(SelectedHeldCase, SelectedCaseRead), AppError> {
        let witnesses = self.witnesses.as_ref().ok_or_else(refused)?;
        let path = self
            .directory
            .child(&format!("{}{HOLD_SUFFIX}", hex::encode(label)));
        let selected = self.read_frame(&path)?.ok_or_else(refused)?;
        let witness = witnesses.require_selected(&self.scope, key, label, &selected.bytes)?;
        let opened = self.scope.open(key, &selected.bytes)?;
        if self.scope.file_label(key, opened.identity.id) != label {
            return Err(refused());
        }
        let data = V2RemovedCreateRecord::decode(&opened.plain).map_err(|_| refused())?;
        let signed = verify(&data)?;
        let mut proof = SelectedCaseRead {
            path,
            file: (selected.file, selected.native),
            bytes: selected.bytes,
            witness,
        };
        proof.require()?;
        Ok((
            SelectedHeldCase {
                case: opened.identity.id,
                removal: opened.identity.removal,
                original: opened.identity.original,
                admission: data.target_data().spec().context.admission,
                signed,
            },
            proof,
        ))
    }
}

pub(super) fn parse_leaf(name: &str) -> Option<([u8; 32], bool)> {
    let (label, held) = name
        .strip_suffix(HOLD_SUFFIX)
        .map(|label| (label, true))
        .or_else(|| name.strip_suffix(STAGE_SUFFIX).map(|label| (label, false)))?;
    if label.len() != 64
        || !label
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    let mut decoded = [0; 32];
    hex::decode_to_slice(label, &mut decoded).ok()?;
    Some((decoded, held))
}
