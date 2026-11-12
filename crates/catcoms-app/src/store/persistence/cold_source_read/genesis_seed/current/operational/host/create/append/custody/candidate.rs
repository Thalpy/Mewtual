//! SAME original SDK Header and one owning Core candidate after genuine SavedS.
//!
//! This child is declared only with its owning physical S delegate. It supplies DATA
//! for later B preparation, never a B receipt, selected successor or projection token.
use super::*;
use crate::store::persistence::v2_text::SavedV2RawAppendSignature;
use catcoms_replication::chat::V2VerifiedAppendCandidate;

// Count the wrapper and temporary borrowed views separately from transition.rs's
// already-invoiced source/next ReplayMath and append scratch. The owning payload is
// structurally limited to ONE candidate in FirstUnsignedAppendData; no clone exists.
pub(super) const CANDIDATE_BRIDGE_METADATA_BYTES: usize =
    2 * std::mem::size_of::<Option<V2VerifiedAppendCandidate>>()
        + 2 * std::mem::size_of::<catcoms_sync::owner_v2::V2AppendDataView<'static>>();

#[cfg(all(test, windows))]
thread_local! {
    static CANDIDATE_OBSERVATION: std::cell::RefCell<Option<(
        Arc<[std::sync::atomic::AtomicUsize; 2]>, Option<Box<dyn FnOnce() + Send>>
    )>> = const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, windows))]
fn observe_dispatch(which: usize) {
    CANDIDATE_OBSERVATION.with(|slot| {
        if let Some((counts, _)) = slot.borrow().as_ref() {
            counts[which].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
}

impl PreparedV2UnsignedOriginalCustody {
    /// Reprove THIS actual selected S, then finish its original Header and derive at
    /// most one successor outside Current writers. A historical S receipt, matching
    /// Header bytes, or another original plan can never select this mathematical work.
    /// Errors retain all existing Header/candidate DATA in the SAME whole S/I owner.
    pub(in crate::store) fn materialize_candidate_after_saved_s<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        retained: &PreparedV2TextRetainTransaction,
        saved: &SavedV2RawAppendSignature,
    ) -> Result<(), AppError> {
        // Once B owns a preallocated advance phase, candidateNone may mean an
        // already installed model. Never touch old S or materialize/derive here again.
        if self.advance.is_some() {
            return Err(refused());
        }
        // Account waits and original-plan validation precede the final Current proof.
        // SavedS retains no SDK/model: the whole S's private delegate supplies both
        // this custody and its own receipt without exposing either mutable owner.
        self.validate_accounts(store)?;
        self.require_original_plan_data(retained)?;
        saved
            .original_unsigned()
            .require_same_original(&self.original)?;
        self.data
            .resources
            .require_same_original(saved.resources())?;
        self.original.validate_negative()?;
        let source = owner.borrow_current()?;
        self.data
            .resources
            .require_bound_original_negative(store, source.source())?;
        let resources = self.data.resources.clone();
        store.with_v2_text_current(&source, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            saved.validate_selected_under_current_guard(&guard)
        })?;
        drop(source);

        // A lost return or later failure may already own the exact candidate. Its
        // Some slot is never replaced or rederived, including after cancellation.
        if self.data.candidate.is_none() {
            let prepared = self.data.prepared.as_mut().ok_or_else(refused)?;
            let materialized = match prepared
                .borrow_existing_materialized_append_data(&self.original.body.original)
            {
                Ok(_) => Ok(()),
                // This negative-only getter checks opaque identity first. Terminal
                // means an actual missing raw/Header, never a reason to sign again.
                Err(catcoms_sync::owner_v2::V2CreateRefusal::Terminal) => {
                    #[cfg(all(test, windows))]
                    observe_dispatch(0);
                    owner.materialize_signed_append_data(
                        store,
                        self.data
                            .resident_owner
                            .as_ref()
                            .ok_or_else(refused)?
                            .model
                            .replay_data(),
                        prepared,
                    )
                }
                Err(_) => return Err(refused()),
            };
            // Sync caches a complete Header BEFORE its own late postcheck. Even an
            // error cannot discard that original output or fall back to a new child.
            // Core still performs its own original-domain liveness checks before and
            // after derivation; a failed derivation grants no selected-root authority.
            let data = prepared
                .borrow_existing_materialized_append_data(&self.original.body.original)
                .map_err(|_| refused())?;
            #[cfg(all(test, windows))]
            observe_dispatch(1);
            let candidate = self
                .data
                .resident_owner
                .as_ref()
                .ok_or_else(refused)?
                .model
                .derive_verified_append(data.header, data.body, data.image)
                .map_err(|_| refused())?;
            // Infallible owning cache comes BEFORE every late negative or returned
            // materialization error. Do not add a callback/fallible step above it.
            self.data.candidate = Some(candidate);
            #[cfg(all(test, windows))]
            {
                let after = CANDIDATE_OBSERVATION.with(|slot| {
                    slot.borrow_mut()
                        .as_mut()
                        .and_then(|(_, after)| after.take())
                });
                if let Some(after) = after {
                    after();
                }
            }
            materialized.map_err(|_| refused())?;
        }
        self.original.validate_negative()?;
        let source = owner.borrow_current()?;
        self.data
            .resources
            .require_bound_original_negative(store, source.source())
    }

    /// Existing immutable candidate DATA only. This does not prove present selected
    /// S/B, renew a cancelled origin, issue a projection, or lend the live verifier.
    pub(in crate::store) fn borrow_verified_candidate_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<&V2VerifiedAppendCandidate, AppError> {
        // B has a separate exact-original candidate DATA loan after preallocation.
        // Generic S-era DATA must not bypass that identity or re-adopt candidateNone.
        if self.advance.is_some() {
            return Err(refused());
        }
        self.require_original_plan_data(retained)?;
        self.data.candidate.as_ref().ok_or_else(refused)
    }

    /// Observe actual Header/derive dispatches and optionally cancel the SAME domain
    /// only after its actual owning candidate cache. The non-Send reset clears this
    /// thread's hook on unwind as well; no mutable SDK/model escapes to the fixture.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn observe_candidate_work_for_test(
        &self,
        counts: Arc<[std::sync::atomic::AtomicUsize; 2]>,
        cancel_after_cache: bool,
    ) -> impl Drop + 'static {
        struct Reset(std::marker::PhantomData<std::rc::Rc<()>>);
        impl Drop for Reset {
            fn drop(&mut self) {
                CANDIDATE_OBSERVATION.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        let after = cancel_after_cache.then(|| {
            let domain = self
                .data
                .resident_owner_data()
                .expect("original candidate observation owner")
                .domain
                .clone();
            Box::new(move || domain.cancel()) as Box<dyn FnOnce() + Send>
        });
        CANDIDATE_OBSERVATION.with(|slot| {
            assert!(slot.borrow().is_none(), "nested candidate-work observation");
            *slot.borrow_mut() = Some((counts, after));
        });
        Reset(std::marker::PhantomData)
    }

    /// Read only already-owned output, including after a post-cache cancellation.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn verified_candidate_digest_for_test(
        &self,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<[u8; 32], AppError> {
        let candidate = self.borrow_verified_candidate_data(retained)?;
        let mut hash = blake3::Hasher::new();
        hash.update(&candidate.header_data().encode());
        hash.update(candidate.image_data());
        Ok(*hash.finalize().as_bytes())
    }
}

// The actual selected-B consumer lives in advance.rs. Its preallocated closed
// phase distinguishes a never-derived/ready candidate from one already consumed.
// Both this materializer and the generic S-era DATA loan refuse once B owns that
// phase; only its exact-B-bound DATA loan may borrow CandidateReady. Actual retired
// math and AdvancedPendingFinal cache together, and destruction stays outside writers.
