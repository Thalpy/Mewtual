//! The single narrow raw-key forwarding cut for whole original unsigned custody.
//! Declared only with the actual selected-I guard and owning Physical delegate.
use super::*;
use crate::store::critical_namespaces::current_layout::SavedUnsignedV2TextSigningGuard;
#[cfg(all(test, windows))]
thread_local! {
    static SIGN_OBSERVATION: std::cell::RefCell<Option<(Arc<std::sync::atomic::AtomicUsize>, Option<Box<dyn FnOnce() + Send>>)>> = const { std::cell::RefCell::new(None) };
}

impl PreparedV2UnsignedOriginalCustody {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn cancel_original_projection_domain_for_test(&self) {
        self.data
            .resident_owner_data()
            .expect("original signing observation owner")
            .domain
            .cancel();
    }

    /// Arm an actual SAME-domain post-dispatch cancellation without lending the
    /// domain/SDK or retaining a borrow of this mutable signing target. The non-Send
    /// reset owns only its thread's test observation and clears it on panic as well.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn observe_domain_cancel_after_sign_for_test(
        &self,
        dispatches: Arc<std::sync::atomic::AtomicUsize>,
    ) -> impl Drop + 'static {
        struct Reset(std::marker::PhantomData<std::rc::Rc<()>>);
        impl Drop for Reset {
            fn drop(&mut self) {
                SIGN_OBSERVATION.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        let domain = self
            .data
            .resident_owner_data()
            .expect("original signing observation owner")
            .domain
            .clone();
        SIGN_OBSERVATION.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "nested signer-dispatch observation"
            );
            *slot.borrow_mut() = Some((dispatches, Some(Box::new(move || domain.cancel()))));
        });
        Reset(std::marker::PhantomData)
    }
    /// Observe the real Root->SDK signer dispatch, rather than a queued job. A valid
    /// returned raw plus one dispatch composes with Sync's actual private-key count test.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn with_sign_dispatch_observation_for_test<T>(
        dispatches: Arc<std::sync::atomic::AtomicUsize>,
        after: Option<Box<dyn FnOnce() + Send>>,
        run: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                SIGN_OBSERVATION.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        SIGN_OBSERVATION.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "nested signer-dispatch observation"
            );
            *slot.borrow_mut() = Some((dispatches, after));
        });
        let _reset = Reset;
        run()
    }
    /// The actual guard borrows Current/Root/lane and genuine SavedI, never this
    /// mutable SDK slot. All account waits, codec work and physical proofs precede
    /// guard issuance. Only cached original/close checks surround the bounded signer.
    pub(in crate::store) fn sign_admitted<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        store: &ServerStore,
        owner: &V2TextOwnedCurrent<T, R>,
        retained: &PreparedV2TextRetainTransaction,
        guard: &SavedUnsignedV2TextSigningGuard<'_, '_, '_>,
    ) -> Result<(), AppError> {
        guard.require_original_unsigned(&self.original)?;
        guard.require_original_retain(&retained.original_retain())?;
        self.require_original_plan_data(retained)?;
        self.original.validate_negative()?;
        guard.validate_negative()?;
        // A lost reply can leave the original raw present. Reuse only that exact SDK
        // plan's existing bytes; a mismatch is an error, never a fresh-sign fallback.
        match self
            .data
            .prepared
            .as_ref()
            .ok_or_else(refused)?
            .borrow_existing_raw_signature_data(&self.original.body.original)
        {
            Ok(_) => return guard.validate_negative(),
            Err(catcoms_sync::owner_v2::V2CreateRefusal::Terminal) => {} // proven rawNone in this DATA API
            Err(_) => return Err(refused()),
        }
        #[cfg(all(test, windows))]
        SIGN_OBSERVATION.with(|slot| {
            if let Some((dispatches, _)) = slot.borrow().as_ref() {
                dispatches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let result = owner.sign_reserved_append_bytes_data(
            store,
            &self.original.body.original,
            self.data.prepared.as_mut().ok_or_else(refused)?,
        );
        #[cfg(all(test, windows))]
        {
            let after = SIGN_OBSERVATION.with(|slot| {
                slot.borrow_mut()
                    .as_mut()
                    .and_then(|(_, after)| after.take())
            });
            if let Some(after) = after {
                after();
            }
        }
        // Sync owns/cache-stores raw64 BEFORE its own hooks/postchecks. A late
        // negative here cannot remove that output or ask for a replacement child.
        guard.validate_negative()?;
        self.original.validate_negative()?;
        result.map_err(|_| refused())
    }
}
