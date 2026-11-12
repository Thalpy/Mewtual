//! Owning SAME-resident -> next unsigned SDK preparation after actual next SavedA.
//!
//! The flat resident never becomes a request cache. A separate paid phase owns
//! partial/completed SDK output and the SAME model Box across errors, then moves
//! whole into the existing I custody. No genesis call or mutable model/SDK loan.
use super::*;
use crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard;
use crate::store::persistence::v2_text::{
    PreparedV2TextAcceptedQuery, V2TextAcceptedQueryResultData,
};

/// Distinct owning phase, paid by actual next-A metadata plus a NEW SDK2MiB. The
/// model/domain and original model1MiB move intact from the flat resident. Both
/// Option payloads are structurally exclusive; failure never reconstructs either.
pub(in crate::store) struct PreparedV2NextUnsignedAppend {
    original: OriginalV2TextResidentObservation,
    data: Option<FirstUnsignedAppendData>,
    complete: Option<PreparedV2UnsignedOriginalCustody>,
}

// Name every actual wrapper/owning move slot; do not assume the first Root64KiB
// phase also covers this larger pending sum. The next transaction's complete
// compiled metadata invoice must include this term before the owner take.
const NEXT_UNSIGNED_METADATA_LAYOUT_BYTES: usize =
    4 * std::mem::size_of::<PreparedV2NextUnsignedAppend>()
        + 4 * std::mem::size_of::<Option<PreparedV2NextUnsignedAppend>>()
        + 4 * std::mem::size_of::<FirstUnsignedAppendData>()
        + 4 * std::mem::size_of::<PreparedV2UnsignedOriginalCustody>()
        + 8 * std::mem::size_of::<OriginalV2TextResidentObservation>()
        + 4 * 64;

#[cfg(all(test, windows))]
thread_local! {
    static NEXT_SDK_FEE_NOTICE: std::cell::RefCell<Option<Arc<std::sync::atomic::AtomicBool>>> = const { std::cell::RefCell::new(None) };
    static NEXT_OWNER_CACHE_STOP: std::cell::RefCell<Option<Arc<V2TextProjectionDomain>>> = const { std::cell::RefCell::new(None) };
}
impl PreparedV2NextUnsignedAppend {
    pub(in crate::store) const fn metadata_layout_bytes() -> usize {
        NEXT_UNSIGNED_METADATA_LAYOUT_BYTES
    }

    /// Accounts and immutable original DATA are checked before the FINAL actual
    /// next SavedA/completed accepted-miss proof. All writers are released before
    /// taking/caching the SAME owner and before entering SDK derivation. A failed
    /// call retains whichever original owning phase has already been cached.
    pub(in crate::store) fn prepare_after_saved_a_into<T: MeshTransport, R: CryptoRngCore>(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        retained: &PreparedV2TextRetainTransaction,
        accepted: &PreparedV2TextAcceptedQuery,
        resident: &mut Option<PreparedV2TextResidentProjection>,
        next: &mut Option<Self>,
    ) -> Result<(), AppError> {
        let source = owner.borrow_current()?;
        retained
            .producer_data()
            .validate_resources_original(store, source.source())?;
        accepted.query_resources().validate_prepaid_account()?;
        accepted.require_same_retained_data(retained)?;
        if !matches!(
            accepted.result_data()?,
            V2TextAcceptedQueryResultData::Absent
        ) {
            return Err(refused());
        }
        // Constructor is private to the actual owning next A, which preserved the
        // concrete completed-WC predecessor before MAIN changed to this exact W_A.
        let next_a = retained.borrow_next_resident_unsigned_source_data()?;
        let mut sdk_fee = None;
        if let Some(current) = next.as_ref() {
            if resident.is_some() {
                return Err(refused());
            }
            next_a
                .original_resident()
                .require_same_original(&current.original)?;
            current.require_original_data(retained)?;
            current.validate_accounts(store)?;
            current.original.validate_negative()?;
        } else {
            let original = resident.as_ref().ok_or_else(refused)?;
            original.validate_completed_parent_data(store)?;
            next_a
                .original_resident()
                .require_same_original(&original.observe_original())?;
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            let fee = pool.try_reserve(APPEND_DATA_BUFFER_BYTES)?;
            #[cfg(all(test, windows))]
            let fee = {
                let mut fee = fee;
                NEXT_SDK_FEE_NOTICE.with(|slot| {
                    if let Some(notice) = slot.borrow_mut().take() {
                        fee.observe_drop_for_test(notice);
                    }
                });
                fee
            };
            fee.validate(pool, APPEND_DATA_BUFFER_BYTES)?;
            // Metadata already exists on the genuine NEXT A; no old B/read/work
            // allowance can pay this new owner-bearing request/SDK preparation.
            retained
                .producer_data()
                .metadata_resources()
                .validate_prepaid_account()?;
            original.validate_completed_parent_data(store)?;
            sdk_fee = Some(fee);
        }
        retained
            .producer_data()
            .validate_resources_original(store, source.source())?;
        let resources = retained.resources().clone();
        store.with_v2_text_current(&source, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            if let Some(current) = next.as_ref() {
                current.require_current_source_under_guard(&guard)?;
            } else {
                resident
                    .as_ref()
                    .ok_or_else(refused)?
                    .require_current_source_under_guard(&guard)?;
            }
            // Fresh actual SavedA plus its SAME completed accepted query, not a
            // caller boolean, old WC, token row or inherited historical miss.
            next_a
                .saved_data()
                .validate_selected_under_current_guard(&mut guard)?;
            accepted.validate_completed_under_current_guard(&mut guard)?;
            if let Some(current) = next.as_ref() {
                current.require_current_source_under_guard(&guard)?;
            } else {
                resident
                    .as_ref()
                    .ok_or_else(refused)?
                    .require_current_source_under_guard(&guard)?;
            }
            Ok(())
        })?;
        drop(source);
        if next.is_none() {
            let original = resident.as_ref().ok_or_else(refused)?.observe_original();
            original.validate_negative()?;
            let resident = resident.take().expect("checked SAME flat resident");
            let PreparedV2TextResidentProjection { selected, owner } = resident;
            // Infallibly cache the WHOLE owner-bearing next phase BEFORE every
            // SDK call, callback, late negative or destruction of old flat DATA.
            *next = Some(Self {
                original: original.clone(),
                data: Some(FirstUnsignedAppendData {
                    candidate: None,
                    prepared: None,
                    original_plan: None,
                    retain: retained.observe_original(),
                    resources: retained.resources().clone(),
                    sdk_fee: sdk_fee.take().expect("checked NEW original SDK2MiB"),
                    resident_basis: Some(original),
                    resident_owner: Some(owner),
                }),
                complete: None,
            });
            drop(selected); // OUTSIDE writers, after the SAME owner is cached
            #[cfg(all(test, windows))]
            {
                let domain = NEXT_OWNER_CACHE_STOP.with(|slot| slot.borrow_mut().take());
                if let Some(domain) = domain {
                    domain.cancel();
                }
            }
        }
        let current = next.as_mut().expect("cached owning next phase");
        current.original.validate_negative()?;
        if current.complete.is_none() {
            let data = current.data.as_mut().ok_or_else(refused)?;
            if data.prepared.is_none() {
                owner
                    .prepare_create_append_data(
                        store,
                        data.resident_owner
                            .as_ref()
                            .ok_or_else(refused)?
                            .model
                            .replay_data(),
                        retained.producer_data().action_data()?,
                        &mut data.prepared,
                    )
                    .map_err(|_| refused())?;
            }
            if data.original_plan.is_none() {
                data.original_plan = Some(
                    owner
                        .observe_append_plan(
                            store,
                            data.resident_owner
                                .as_ref()
                                .ok_or_else(refused)?
                                .model
                                .replay_data(),
                            data.prepared.as_ref().ok_or_else(refused)?,
                        )
                        .map_err(|_| refused())?,
                );
            }
            PreparedV2UnsignedOriginalCustody::take_complete_into(
                store,
                owner,
                retained,
                &mut current.data,
                &mut current.complete,
            )?;
        }
        current.require_original_data(retained)?;
        current.original.validate_negative()
    }

    /// DATA-only owning handoff to the existing I consumer. All fallible original
    /// checks precede the take; cancellation does not discard completed bytes. This
    /// does not supply a fresh I/Save/sign/current role or renew an expired origin.
    pub(in crate::store) fn take_complete_into(
        retained: &PreparedV2TextRetainTransaction,
        input: &mut Option<Self>,
        output: &mut Option<PreparedV2UnsignedOriginalCustody>,
    ) -> Result<(), AppError> {
        if let Some(existing) = output.as_ref() {
            if input.is_some() {
                return Err(refused());
            }
            existing.require_original_plan_data(retained)?;
            return retained
                .borrow_next_resident_unsigned_source_data()?
                .original_resident()
                .require_same_original(existing.data.resident_basis.as_ref().ok_or_else(refused)?);
        }
        let current = input.as_ref().ok_or_else(refused)?;
        current.require_original_data(retained)?;
        if current.data.is_some() || current.complete.is_none() {
            return Err(refused());
        }
        let mut current = input.take().expect("same checked complete next owner");
        *output = Some(
            current
                .complete
                .take()
                .expect("same complete original custody"),
        );
        // Only empty wrapper/tiny observation destruction follows its owning cache.
        Ok(())
    }
    fn require_original_data(
        &self,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<(), AppError> {
        retained
            .borrow_next_resident_unsigned_source_data()?
            .original_resident()
            .require_same_original(&self.original)?;
        match (&self.data, &self.complete) {
            (Some(data), None) => {
                data.retain
                    .require_same_original(&retained.observe_original())?;
                data.resources.require_same_original(retained.resources())?;
                self.original
                    .require_same_original(data.resident_basis.as_ref().ok_or_else(refused)?)?;
                self.original.require_owner(data.resident_owner_data()?)
            }
            (None, Some(complete)) => {
                complete.require_original_plan_data(retained)?;
                self.original.require_same_original(
                    complete.data.resident_basis.as_ref().ok_or_else(refused)?,
                )?;
                self.original
                    .require_owner(complete.data.resident_owner_data()?)
            }
            _ => Err(refused()),
        }
    }
    fn validate_accounts(&self, store: &ServerStore) -> Result<(), AppError> {
        match (&self.data, &self.complete) {
            (Some(data), None) => data.validate_accounts(store),
            (None, Some(complete)) => complete.validate_accounts(store),
            _ => Err(refused()),
        }
    }
    fn require_current_source_under_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        let data = match (&self.data, &self.complete) {
            (Some(data), None) => data,
            (None, Some(complete)) => &complete.data,
            _ => return Err(refused()),
        };
        data.resident_owner_data()?
            .domain
            .require_current_source(guard.scope_admission())?;
        self.original.validate_negative()
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn with_sdk_fee_notice_for_test<T>(
        notice: Arc<std::sync::atomic::AtomicBool>,
        run: impl FnOnce() -> T,
    ) -> T {
        struct Reset(std::marker::PhantomData<std::rc::Rc<()>>);
        impl Drop for Reset {
            fn drop(&mut self) {
                NEXT_SDK_FEE_NOTICE.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        NEXT_SDK_FEE_NOTICE.with(|slot| {
            assert!(slot.borrow().is_none(), "nested NEXT SDK fee observation");
            *slot.borrow_mut() = Some(notice);
        });
        let _reset = Reset(std::marker::PhantomData);
        let output = run();
        NEXT_SDK_FEE_NOTICE
            .with(|slot| assert!(slot.borrow().is_none(), "missing actual NEXT SDK2MiB debit"));
        output
    }
}

#[cfg(all(test, windows))]
impl PreparedV2NextUnsignedAppend {
    /// Read actual original owner and SDK/cache presence after the owning cut.
    pub(in crate::store) fn cached_owner_state_for_test(
        &self,
        original: &OriginalV2TextResidentObservation,
    ) -> Result<([u8; 32], u64, bool, bool), AppError> {
        let data = match (&self.data, &self.complete) {
            (Some(data), None) => data,
            (None, Some(complete)) => &complete.data,
            _ => return Err(refused()),
        };
        let owner = data.resident_owner_data()?;
        original.require_owner(owner)?;
        Ok((
            *owner.model.replay_data().header().id().as_bytes(),
            owner.model.replay_data().header().spec().sequence,
            data.prepared.is_some(),
            self.complete.is_some(),
        ))
    }

    pub(in crate::store) fn complete_digest_for_test(
        &self,
        retained: &PreparedV2TextRetainTransaction,
    ) -> Result<[u8; 32], AppError> {
        self.require_original_data(retained)?;
        let view = self
            .complete
            .as_ref()
            .ok_or_else(refused)?
            .unsigned_data(retained)?;
        let mut digest = blake3::Hasher::new();
        for bytes in [
            view.signing_bytes,
            view.image,
            &view.record.encode(),
            &view.body.encode(),
        ] {
            digest.update(&(bytes.len() as u64).to_be_bytes());
            digest.update(bytes);
        }
        Ok(*digest.finalize().as_bytes())
    }
}

#[cfg(all(test, windows))]
impl PreparedV2TextResidentProjection {
    /// Hard-coded real domain Stop ONLY after the next owning value is cached,
    /// before any SDK derivation. No callback, model loan or fake permission.
    pub(in crate::store) fn observe_next_owner_cache_domain_stop_for_test(
        &self,
    ) -> impl Drop + 'static {
        struct Reset(std::marker::PhantomData<std::rc::Rc<()>>);
        impl Drop for Reset {
            fn drop(&mut self) {
                let old = NEXT_OWNER_CACHE_STOP.with(|slot| slot.borrow_mut().take());
                drop(old);
            }
        }
        NEXT_OWNER_CACHE_STOP.with(|slot| {
            assert!(slot.borrow().is_none(), "nested NEXT owner cache stop");
            *slot.borrow_mut() = Some(self.owner.domain.clone());
        });
        Reset(std::marker::PhantomData)
    }
}

#[cfg(all(test, windows))]
impl PreparedV2UnsignedOriginalCustody {
    /// Original-object fixed observations only, never a replay/model loan.
    pub(in crate::store) fn next_model_state_for_test(
        &self,
        original: &OriginalV2TextResidentObservation,
    ) -> Result<([u8; 32], u64), AppError> {
        let owner = self.data.resident_owner_data()?;
        original.require_owner(owner)?;
        Ok((
            *owner.model.replay_data().header().id().as_bytes(),
            owner.model.replay_data().header().spec().sequence,
        ))
    }
}
