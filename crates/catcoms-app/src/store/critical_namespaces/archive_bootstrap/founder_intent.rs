//! Actual original plan -> cached marker/intent/seed -> concrete Root persistence. All three
//! small candidates are cached independently before post-work checks; retry never signs/seals
//! them again. There is no path to an Initializing row or a mutable source/Server here.
use super::*;

/// Caller-owned exact retry copy in addition to Root's inert complete transaction debt. This
/// is SMALL (at most1064+1064+4136B); both copies and decoding/AEAD scratch are in the separate
///256KiB original control allowance. The later LARGE source is transferred, never cloned here.
#[derive(Default)]
pub(super) struct OriginalFoundingFrames {
    marker: Option<Zeroizing<Vec<u8>>>,
    intent: Option<Zeroizing<Vec<u8>>>,
    seed: Option<Zeroizing<Vec<u8>>>,
}
impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderPlanning<T, R> {
    #[cfg(test)]
    pub(super) fn retained_founding_frames_for_test(&self) -> (&[u8], &[u8], &[u8]) {
        (
            self.founding.marker.as_ref().unwrap(),
            self.founding.intent.as_ref().unwrap(),
            self.founding.seed.as_ref().unwrap(),
        )
    }
    fn cache_founding_frames(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        let source = ActualFounderSource::founding(&self.creator);
        source.validate_original(store)?;
        source.untouched_scope()?;
        let plan = self.plan.as_ref().ok_or_else(refused)?;
        plan.validate_captured(&self.creator.original)?;
        let facts = plan.data()?;
        let debit = self.creator.original.data();
        let id = plan.debit_id();
        let key = Zeroizing::new(store.keys.db_key()?);
        if self.founding.marker.is_none() {
            let data = codec::TargetMarkerData {
                invocation: debit.invocation,
                debit: id,
                founding: facts.clone(),
            };
            self.founding.marker =
                Some(codec::seal_target_marker_data(&key, &data, debit, id, rng)?);
            source.validate_original(store)?;
        }
        if self.founding.intent.is_none() {
            let marker =
                codec::TargetMarkerId::of(self.founding.marker.as_ref().ok_or_else(refused)?);
            let data = codec::IntentData {
                invocation: debit.invocation,
                debit: id,
                sequence: 1,
                parent: None,
                io_spent: 0,
                kind: codec::IntentKind::Founding {
                    facts: facts.clone(),
                    marker,
                },
            };
            data.successor(None, debit, id)?;
            self.founding.intent = Some(codec::seal_intent_data(&key, &data, debit, id, rng)?);
            source.validate_original(store)?;
        }
        if self.founding.seed.is_none() {
            let plain = codec::encode_transition_seed(
                &key,
                debit,
                id,
                None,
                Some(self.founding.marker.as_ref().ok_or_else(refused)?),
                self.founding.intent.as_ref().ok_or_else(refused)?,
            )?;
            self.founding.seed = Some(codec::seal_transition_seed(&key, &plain, rng)?);
            source.validate_original(store)?;
        }
        source.validate_original(store)
    }

    /// Complete the first real post-SDK durable transition while keeping the original creator,
    /// plan and exact encrypted transaction on every error. Root registers its entire inert
    /// debt BEFORE mkdir/write; its private Saved result, rather than a decoded label, proves
    /// the actual target/marker/intent barriers. It does not attest a private source or Current.
    pub fn save_founding_intent(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ArchiveFounderPlanningProgress, AppError> {
        let source = ActualFounderSource::founding(&self.creator);
        source.validate_original(store)?;
        source.untouched_scope()?;
        if self.plan.as_ref().is_none_or(|plan| plan.data().is_err()) {
            self.prepare_ram_plan(store, rng)?;
        }
        {
            // Refuse already-known old-source conflicts BEFORE even small AEAD work. An
            // exact prepared Root debt may already own a target; a new candidate needs actual
            // target absence. No foreign/partial directory is adopted from matching labels.
            let source = ActualFounderSource::founding(&self.creator);
            let plan = self.plan.as_ref().ok_or_else(refused)?;
            store.with_original_archive_founder(&source, plan, |admission| {
                let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
                vacancy.validate_original()?;
                if self.prepared_founding.is_none() {
                    store.require_no_archive_bootstrap_target(
                        admission.group(),
                        admission.signer(),
                    )?;
                }
                Ok(())
            })?;
        }
        self.cache_founding_frames(store, rng)?;
        let source = ActualFounderSource::founding(&self.creator);
        let plan = self.plan.as_ref().ok_or_else(refused)?;
        let captured = &self.creator.original;
        let frames = &self.founding;
        let prepared = &mut self.prepared_founding;
        let saved = &mut self.saved_founding;
        store.with_original_archive_founder(&source, plan, |admission| {
            let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
            vacancy.validate_original()?;
            if prepared.is_none() {
                // Small exact owned copies only. A preparation refusal keeps the original
                // cached bytes/nonces, while successful registration owns complete debt.
                *prepared = Some(store.prepare_original_founding_intent(
                    captured,
                    &vacancy,
                    Zeroizing::new(frames.marker.as_ref().ok_or_else(refused)?.to_vec()),
                    Zeroizing::new(frames.intent.as_ref().ok_or_else(refused)?.to_vec()),
                    Zeroizing::new(frames.seed.as_ref().ok_or_else(refused)?.to_vec()),
                )?);
            }
            if saved.is_none() {
                *saved = Some(std::sync::Arc::new(store.save_original_founding_intent(
                    prepared.as_mut().ok_or_else(refused)?,
                    captured,
                    &vacancy,
                )?));
            }
            saved
                .as_ref()
                .ok_or_else(refused)?
                .validate_original(store, captured, &vacancy)
        })?;
        // Actual immutable saved result replaces ONLY this exact transient weak slot. The
        // original life/identity is untouched; attaching the scope later uses that SAME owner.
        store.release_saved_archive_founder_predecessor(
            &source,
            self.plan.as_mut().ok_or_else(refused)?,
            self.saved_founding.as_ref().ok_or_else(refused)?.clone(),
        )?;
        source.validate_original(store)?;
        Ok(ArchiveFounderPlanningProgress::FoundingIntentSavedUnexposed)
    }
}
