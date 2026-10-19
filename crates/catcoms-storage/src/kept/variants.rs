//! V4's immutable candidate ownership. The journal retains a working mirror for the old
//! exact-run API, but only `committed` supplies ordinary blob service. Switching is never a
//! migration, a retry grant, or permission to combine chunks from different encryptions.
use super::*;

pub(in crate::kept) const MAX_VARIANTS: usize = 4;
pub(in crate::kept) const VARIANT_CHUNK_LIMIT: usize = MAX_VARIANTS * CHUNK_LIMIT;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::kept) struct Candidate {
    pub plan: KeepPlan,
    pub verified: Vec<bool>,
    pub bad: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::kept) struct Variants {
    pub candidates: Vec<Candidate>,
    pub selected: usize,
    pub committed: Option<usize>,
}
impl Variants {
    pub(super) fn initial_fields_valid(&self) -> bool {
        self.committed.is_none()
            && self
                .candidates
                .iter()
                .all(|c| !c.bad && c.verified.iter().all(|v| !*v))
    }
    pub(super) fn encode(&self, e: &mut Encoder) {
        e.put_u8(self.candidates.len() as u8);
        e.put_u8(self.selected as u8);
        e.put_u8(self.committed.map_or(0, |index| index as u8 + 1));
        for candidate in &self.candidates {
            e.put_bytes(&candidate.plan.encode())
                .expect("bounded exact plan");
            e.put_bytes(
                &candidate
                    .verified
                    .iter()
                    .map(|bit| u8::from(*bit))
                    .collect::<Vec<_>>(),
            )
            .expect("bounded verified bits");
            e.put_u8(u8::from(candidate.bad));
        }
    }
    pub(super) fn decode(d: &mut Decoder<'_>, minimum: usize) -> Result<Self, StorageError> {
        let malformed = |_| StorageError::Malformed;
        let count = d.get_u8().map_err(malformed)? as usize;
        if !(minimum..=MAX_VARIANTS).contains(&count) {
            return Err(StorageError::Malformed);
        }
        let selected = d.get_u8().map_err(malformed)? as usize;
        let winner = d.get_u8().map_err(malformed)?;
        let committed = (winner != 0).then(|| winner as usize - 1);
        let mut candidates = Vec::with_capacity(count);
        for _ in 0..count {
            let plan = KeepPlan::decode(d.get_bytes().map_err(malformed)?)?;
            let bitmap = d.get_bytes().map_err(malformed)?;
            if bitmap.len() != plan.chunks.len() || bitmap.iter().any(|bit| *bit > 1) {
                return Err(StorageError::Malformed);
            }
            let verified = bitmap.iter().map(|bit| *bit == 1).collect();
            let bad = match d.get_u8().map_err(malformed)? {
                0 => false,
                1 => true,
                _ => return Err(StorageError::Malformed),
            };
            candidates.push(Candidate {
                plan,
                verified,
                bad,
            });
        }
        Ok(Self {
            candidates,
            selected,
            committed,
        })
    }
}
impl Record {
    /// Called only after the app has validated equivalence and the original index-set identity.
    /// Storage additionally closes ordering, size, whole-CID and duplicate-version boundaries.
    pub(in crate::kept) fn initial_variants(
        scope: KeepScope,
        plans: Vec<KeepPlan>,
        selected: usize,
        listing: [u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, StorageError> {
        Self::initial_variant_body(scope, plans, selected, listing, rng, RecordEncoding::V4)
    }

    pub(in crate::kept) fn initial_download(
        scope: KeepScope,
        plans: Vec<KeepPlan>,
        selected: usize,
        listing: [u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, StorageError> {
        Self::initial_variant_body(
            scope,
            plans,
            selected,
            listing,
            rng,
            RecordEncoding::DownloadV1,
        )
    }

    fn initial_variant_body(
        scope: KeepScope,
        plans: Vec<KeepPlan>,
        selected: usize,
        listing: [u8; 32],
        rng: &mut dyn CryptoRngCore,
        encoding: RecordEncoding,
    ) -> Result<Self, StorageError> {
        let minimum = if encoding == RecordEncoding::DownloadV1 {
            1
        } else {
            2
        };
        if !(minimum..=MAX_VARIANTS).contains(&plans.len()) || selected >= plans.len() {
            return Err(StorageError::Malformed);
        }
        let mut record = Self::initial(scope, plans[selected].clone(), listing, rng)?;
        record.encoding = encoding;
        record.variants = Some(Variants {
            candidates: plans
                .into_iter()
                .map(|plan| Candidate {
                    verified: vec![false; plan.chunks.len()],
                    plan,
                    bad: false,
                })
                .collect(),
            selected,
            committed: None,
        });
        record.remaining = record.initial_allowance();
        record.validate_initial()?;
        Ok(record)
    }
    pub(in crate::kept) fn slot_limit(&self) -> usize {
        if self.encoding.has_variants() {
            VARIANT_RECORD_LIMIT
        } else {
            RECORD_LIMIT
        }
    }
    pub(in crate::kept) fn plans(&self) -> impl Iterator<Item = &KeepPlan> {
        self.variants
            .iter()
            .flat_map(|v| v.candidates.iter().map(|c| &c.plan))
            .chain(self.variants.is_none().then_some(&self.plan))
    }
    pub(in crate::kept) fn committed_plan(&self) -> Option<&KeepPlan> {
        if self.phase != KeepPhase::Completed {
            return None;
        }
        match &self.variants {
            Some(v) => v.committed.map(|index| &v.candidates[index].plan),
            None => Some(&self.plan),
        }
    }
    pub(in crate::kept) fn plan_is_bad(&self, version: &[u8; 32]) -> bool {
        self.variants.as_ref().is_some_and(|v| {
            v.candidates
                .iter()
                .any(|c| &c.plan.version == version && c.bad)
        })
    }
    pub(in crate::kept) fn selected_is_winner(&self) -> bool {
        self.variants
            .as_ref()
            .is_none_or(|v| v.committed == Some(v.selected))
    }
    pub(in crate::kept) fn initial_allowance(&self) -> KeepAllowance {
        // Equivalence need not imply equal ciphertext padding. Reserve the largest candidate's
        // finite byte allowance once; neither selecting a larger plan nor Retry changes this ceiling.
        self.plans()
            .map(KeepAllowance::initial)
            .max_by_key(|a| a.bytes)
            .expect("at least one plan")
    }
    pub(in crate::kept) fn note_verified(&mut self, index: usize) {
        self.verified[index] = true;
        if let Some(variants) = &mut self.variants {
            variants.candidates[variants.selected].verified[index] = true;
        }
    }
    pub(in crate::kept) fn commit_selected(&mut self) {
        self.phase = KeepPhase::Completed;
        if let Some(variants) = &mut self.variants {
            variants.committed = Some(variants.selected);
        }
    }
    pub(in crate::kept) fn validate_variants(&self) -> Result<(), StorageError> {
        let Some(v) = &self.variants else {
            return if self.encoding.has_variants() {
                Err(StorageError::Malformed)
            } else {
                Ok(())
            };
        };
        let minimum = if self.encoding == RecordEncoding::DownloadV1 {
            1
        } else {
            2
        };
        if !self.encoding.has_variants()
            || !(minimum..=MAX_VARIANTS).contains(&v.candidates.len())
            || v.selected >= v.candidates.len()
            || v.committed.is_some_and(|i| i >= v.candidates.len())
        {
            return Err(StorageError::Malformed);
        }
        for (index, c) in v.candidates.iter().enumerate() {
            c.plan.charge()?;
            if c.plan.cid != self.plan.cid
                || c.verified.len() != c.plan.chunks.len()
                || (index != 0 && v.candidates[index - 1].plan.version >= c.plan.version)
            {
                return Err(StorageError::Malformed);
            }
        }
        if self.plan != v.candidates[v.selected].plan
            || self.verified != v.candidates[v.selected].verified
            || (self.phase == KeepPhase::Resumable && v.committed.is_some())
            || (self.phase == KeepPhase::Completed && v.committed.is_none())
            || v.committed
                .is_some_and(|i| v.candidates[i].verified.iter().any(|bit| !*bit))
        {
            return Err(StorageError::Malformed);
        }
        Ok(())
    }
    pub(in crate::kept) fn validate_variant_successor(
        &self,
        next: &Self,
    ) -> Result<(), StorageError> {
        match (&self.variants, &next.variants) {
            (None, None) => Ok(()),
            (Some(old), Some(new)) => {
                if old.candidates.len() != new.candidates.len()
                    || old.candidates.iter().zip(&new.candidates).any(|(old, new)| {
                        old.plan != new.plan || (old.bad && !new.bad)
                            || old.verified.iter().zip(&new.verified).any(|(old, new)| *old && !*new)
                    })
                    // A new winner is possible only for the selected fully verified candidate.
                    // Selection and commitment cannot be combined in one publication: the fresh
                    // opaque run must actually check the whole new plaintext before committing.
                    || (old.committed != new.committed
                        && (old.selected != new.selected || new.committed != Some(new.selected)
                            || new.candidates[new.selected].verified.iter().any(|bit| !*bit)))
                    || (old.selected != new.selected && self.phase != next.phase)
                {
                    return Err(StorageError::Malformed);
                }
                Ok(())
            }
            _ => Err(StorageError::Malformed), // No implicit V2/V3 enrichment or V4 downgrade.
        }
    }
    /// Select only a saved candidate. The actor's bounded tried list is per-run work, not a
    /// persistent budget. A deterministic failure excludes just this exact authenticated pair;
    /// network absence cannot exclude a manifest. A final failure still saves its verdict and a
    /// viable selection for a later run without issuing authority to cycle inside this run.
    pub(in crate::kept) fn fallback(
        &self,
        expected: &[u8; 32],
        tried: &[[u8; 32]],
        bad_current: bool,
    ) -> Result<(Option<Self>, bool), StorageError> {
        if &self.plan.version != expected {
            return Err(StorageError::Malformed);
        }
        let Some(v) = &self.variants else {
            return Ok((None, false));
        };
        if tried.len() > MAX_VARIANTS
            || !tried.contains(expected)
            || tried
                .iter()
                .any(|version| !v.candidates.iter().any(|c| &c.plan.version == version))
        {
            return Err(StorageError::Malformed);
        }
        let eligible =
            |index: usize, c: &Candidate| !(c.bad || (bad_current && index == v.selected));
        let target = v
            .candidates
            .iter()
            .enumerate()
            .find(|(i, c)| eligible(*i, c) && !tried.contains(&c.plan.version))
            .map(|(i, _)| i);
        if target.is_none() && !bad_current {
            return Ok((None, false));
        }
        let mut next = self.successor()?;
        let saved = next.variants.as_mut().expect("V4");
        if bad_current {
            saved.candidates[saved.selected].bad = true;
        }
        let selection = target.or_else(|| saved.candidates.iter().position(|c| !c.bad));
        if let Some(index) = selection {
            saved.selected = index;
            next.plan = saved.candidates[index].plan.clone();
            next.verified = saved.candidates[index].verified.clone();
        } else {
            next.blocked = true;
        }
        self.validate_successor(&next)?;
        Ok((Some(next), target.is_some()))
    }
}
