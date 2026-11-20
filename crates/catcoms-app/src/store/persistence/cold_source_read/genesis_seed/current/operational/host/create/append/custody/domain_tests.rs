//! Test-only assertions inside the closed owning family. These exercise the real
//! domain/anchor/verifier constructors without lending an Origin, View, SDK or signer.
use super::*;
use crate::store::persistence::PreparedV2TextSourceObserver;
use std::sync::atomic::{AtomicBool, Ordering};

type ActualInputs<'a, T, R> = (
    &'a ServerStore,
    &'a V2TextOwnedCurrent<T, R>,
    &'a SelectedV2TextHead,
    &'a PreparedV2TextRetainTransaction,
);

impl PreparedV2UnsignedOriginalCustody {
    /// Two genuinely live domains on the SAME actual source have equal public
    /// genesis, but must never exchange the opaque domain-bound founder anchor.
    pub(in crate::store) fn assert_other_live_domain_rejected_for_test<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        selected: &SelectedV2TextHead,
        retained: &PreparedV2TextRetainTransaction,
    ) {
        let first = Self::actual_domain_for_test(store, owner).unwrap();
        let other = Self::actual_domain_for_test(store, owner).unwrap();
        first.validate_negative().unwrap();
        other.validate_negative().unwrap();
        let source = owner.borrow_current().unwrap();
        let mut anchor = None;
        let mut bound = None;
        store
            .with_v2_text_current(&source, |admission| {
                let guard = store.lock_v2_text_current_group(admission, retained.resources())?;
                anchor = Some(selected.founder_anchor_from_retained_data(&guard, retained)?);
                first.bind_anchor_into(&guard, &mut anchor, &mut bound)
            })
            .unwrap();
        assert!(anchor.is_none() && bound.is_some());
        assert!(store
            .with_v2_text_current(&source, |admission| {
                let guard = store.lock_v2_text_current_group(admission, retained.resources())?;
                other.bind_anchor_into(&guard, &mut anchor, &mut bound)
            })
            .is_err());
        // The rejected re-observation cannot consume or relabel the original slot.
        assert!(anchor.is_none() && bound.is_some());
        store
            .with_v2_text_current(&source, |admission| {
                let guard = store.lock_v2_text_current_group(admission, retained.resources())?;
                first.bind_anchor_into(&guard, &mut anchor, &mut bound)
            })
            .unwrap();
        let genesis = source.source().data();
        // Rejection occurs at exact domain identity, before any Core model exists.
        assert!(other
            .verifier_from_genesis(
                bound.as_ref().unwrap(),
                genesis.profile,
                genesis.admission,
                genesis.header,
                genesis.state,
            )
            .is_err());
        first.validate_negative().unwrap();
        other.validate_negative().unwrap();
    }

    /// A second real owner is a source mismatch test, not a claim that the fixture
    /// represents a remount of the first group. Every refused input stays usable.
    pub(in crate::store) fn assert_other_real_source_rejected_for_test<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        first: ActualInputs<'_, T, R>,
        other: ActualInputs<'_, T, R>,
    ) {
        let (store_a, owner_a, _, retained_a) = first;
        let (store_b, owner_b, selected_b, retained_b) = other;
        let domain_a = Self::actual_domain_for_test(store_a, owner_a).unwrap();
        let domain_b = Self::actual_domain_for_test(store_b, owner_b).unwrap();
        let source_a = owner_a.borrow_current().unwrap();
        let source_b = owner_b.borrow_current().unwrap();
        let observer_prepared = PreparedV2TextSourceObserver::prepare(store_b, &source_b).unwrap();
        let mut observer_b = None;
        store_b
            .with_v2_text_current(&source_b, |admission| {
                observer_b = Some(observer_prepared.capture(admission)?);
                Ok(())
            })
            .unwrap();
        let rejected_fee = Arc::new(AtomicBool::new(false));
        let pool_a = &store_a
            .archive_bootstrap_policy
            .as_ref()
            .unwrap()
            .pending
            .pool;
        let charged = pool_a.charged_for_test();
        V2TextProjectionDomain::with_fee_drop_notice_for_test(rejected_fee.clone(), || {
            assert!(V2TextProjectionDomain::prepare(
                store_a,
                &source_a,
                observer_b.as_ref().unwrap(),
            )
            .is_err());
        });
        assert!(rejected_fee.load(Ordering::SeqCst));
        assert_eq!(pool_a.charged_for_test(), charged);
        assert!(observer_b.as_ref().unwrap().is_live());
        assert!(store_b
            .with_v2_text_current(&source_b, |admission| domain_a
                .require_current_source(admission))
            .is_err());

        let mut anchor_b = None;
        let mut bound_b = None;
        store_b
            .with_v2_text_current(&source_b, |admission| {
                let guard =
                    store_b.lock_v2_text_current_group(admission, retained_b.resources())?;
                anchor_b = Some(selected_b.founder_anchor_from_retained_data(&guard, retained_b)?);
                Ok(())
            })
            .unwrap();
        assert!(store_a
            .with_v2_text_current(&source_a, |admission| {
                let guard =
                    store_a.lock_v2_text_current_group(admission, retained_a.resources())?;
                domain_a.bind_anchor_into(&guard, &mut anchor_b, &mut bound_b)
            })
            .is_err());
        assert!(anchor_b.is_some() && bound_b.is_none());
        // Use the SAME preserved founder anchor with its genuine source/domain.
        store_b
            .with_v2_text_current(&source_b, |admission| {
                let guard =
                    store_b.lock_v2_text_current_group(admission, retained_b.resources())?;
                domain_b.bind_anchor_into(&guard, &mut anchor_b, &mut bound_b)
            })
            .unwrap();
        assert!(anchor_b.is_none() && bound_b.is_some());
        domain_a.validate_negative().unwrap();
        domain_b.validate_negative().unwrap();
    }

    /// Observe the exact domain debit independently of the actual resident/SDK debits.
    /// The result is the existing whole custody, never a domain or model extractor.
    pub(in crate::store) fn prepare_with_domain_fee_notice_for_test<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        selected: &SelectedV2TextHead,
        retained: &PreparedV2TextRetainTransaction,
        notices: (Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicBool>),
    ) -> Result<Self, AppError> {
        let (domain, model, sdk) = notices;
        V2TextProjectionDomain::with_fee_drop_notice_for_test(domain, || {
            Self::prepare_for_test(store, owner, selected, retained, model, sdk)
        })
    }

    /// Close this actual original domain without revoking the independent Current.
    /// This fixture has no active publication callback: it tests a real completion,
    /// not the separate busy-callback drain boundary.
    pub(in crate::store) fn assert_explicit_domain_close_for_test<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        &self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        retained: &PreparedV2TextRetainTransaction,
    ) {
        assert!(!self.data.resident_owner_data().unwrap().domain.try_closed());
        self.data
            .resident_owner_data()
            .unwrap()
            .domain
            .validate_negative()
            .unwrap();
        self.data.resident_owner_data().unwrap().domain.cancel();
        assert!(self.data.resident_owner_data().unwrap().domain.try_closed());
        self.data.resident_owner_data().unwrap().domain.cancel();
        assert!(self.data.resident_owner_data().unwrap().domain.try_closed());
        assert!(self.original.validate_negative().is_err());
        let current = owner.borrow_current().unwrap();
        assert!(self
            .validate_original(store, current.source(), retained)
            .is_err());
        // Domain Stop is independent: the actual owner/source remains Current.
        store.with_v2_text_current(&current, |_| Ok(())).unwrap();
        self.validate_accounts_only().unwrap();
        self.validate_accounts(store).unwrap();
        self.require_original_plan_data(retained).unwrap();
        self.unsigned_data(retained).unwrap();
    }

    /// The surviving object is the genuine verifier produced by the real pinned
    /// genesis path. Its Core Origin must retain the exact small domain payment even
    /// after every Root domain/SDK alias is gone. Keep its actual model debit until
    /// AFTER model destruction; no synthetic replay model or fee substitute is used.
    pub(in crate::store) fn assert_actual_core_tail_keeps_domain_fee_for_test<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        selected: &SelectedV2TextHead,
        retained: &PreparedV2TextRetainTransaction,
    ) {
        let fee_dropped = Arc::new(AtomicBool::new(false));
        let domain =
            V2TextProjectionDomain::with_fee_drop_notice_for_test(fee_dropped.clone(), || {
                Self::actual_domain_for_test(store, owner).unwrap()
            });
        let mut first = None;
        FirstUnsignedAppendData::prepare_into(
            store, owner, selected, retained, &domain, &mut first,
        )
        .unwrap();
        let FirstUnsignedAppendData {
            candidate,
            prepared,
            original_plan,
            retain,
            resources,
            sdk_fee,
            resident_owner,
            resident_basis,
        } = first.take().unwrap();
        let ResidentVerifierOwner {
            model,
            identity,
            domain: owned_domain,
            model_fee,
        } = *resident_owner.unwrap();
        drop(identity);
        drop(resident_basis);
        // These are the actual returned owning fields. No observer/SDK/domain tail
        // is left to explain a false notice once the genuine model stands alone.
        drop(candidate);
        drop(prepared);
        drop(original_plan);
        drop(retain);
        drop(resources);
        drop(sdk_fee);
        domain.cancel();
        assert!(domain.try_closed());
        drop(owned_domain);
        drop(domain);
        assert!(!fee_dropped.load(Ordering::SeqCst));
        drop(model);
        assert!(fee_dropped.load(Ordering::SeqCst));
        drop(model_fee);
    }
}
