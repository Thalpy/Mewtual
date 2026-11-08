//! Deterministic test-only observations of the real selected-B Core/final cuts.
//! Registration, error strings and negative handles are prepared outside writers.
//! No injected action can run before BOTH owning caches at its actual cut.
use super::*;
use catcoms_sync::local_chat_scope::ManagedChatScopeRevocation;
use std::sync::atomic::{AtomicUsize, Ordering};

type ActualHeaderState = ([u8; 32], [u8; 32], Option<[u8; 32]>, bool, bool);

struct Observation {
    counts: Arc<[AtomicUsize; 2]>,
    // Errors are allocated at registration, never after the final physical proof.
    lost_returns: [Option<AppError>; 2],
    cancel_domain: Option<(usize, Arc<V2TextProjectionDomain>)>,
    close_scope: Option<(usize, Arc<dyn ManagedChatScopeRevocation>)>,
}
thread_local! {
    static OBSERVATION: std::cell::RefCell<Option<Observation>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Counts the actual Root -> Core dispatch, including a refused Core call. This
/// runs no hook/action, allocates nothing, and cannot mutate the original custody.
pub(super) fn note_core_dispatch() {
    OBSERVATION.with(|slot| {
        if let Some(observation) = slot.borrow().as_ref() {
            observation.counts[0].fetch_add(1, Ordering::SeqCst);
        }
    });
}

/// Called ONLY after the actual retired+PendingFinal or basis+Final owning caches.
/// Move every action out before its real negative callback; never hold RefCell's
/// borrow across cancellation, revocation, destruction, or returning the lost reply.
pub(super) fn after_cache(cut: usize) -> Result<(), AppError> {
    let (lost_return, domain, scope) = OBSERVATION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(observation) = slot.as_mut() else {
            return (None, None, None);
        };
        if cut == 1 {
            observation.counts[1].fetch_add(1, Ordering::SeqCst);
        }
        let domain = if observation
            .cancel_domain
            .as_ref()
            .is_some_and(|(at, _)| *at == cut)
        {
            observation.cancel_domain.take().map(|(_, domain)| domain)
        } else {
            None
        };
        let scope = if observation
            .close_scope
            .as_ref()
            .is_some_and(|(at, _)| *at == cut)
        {
            observation.close_scope.take().map(|(_, scope)| scope)
        } else {
            None
        };
        (observation.lost_returns[cut].take(), domain, scope)
    });
    if let Some(domain) = domain {
        domain.cancel();
    }
    if let Some(scope) = scope {
        scope.revoke();
    }
    match lost_return {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Reset(std::marker::PhantomData<std::rc::Rc<()>>);
impl Drop for Reset {
    fn drop(&mut self) {
        let old = OBSERVATION.with(|slot| slot.borrow_mut().take());
        drop(old);
    }
}
fn install(observation: Observation) -> Reset {
    OBSERVATION.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "nested accepted-advance observation"
        );
        *slot.borrow_mut() = Some(observation);
    });
    // A TLS observation must be cleared by the registering thread, including when
    // its operation panics. Rc's marker makes this RAII reset explicitly non-Send.
    Reset(std::marker::PhantomData)
}

impl PreparedV2UnsignedOriginalCustody {
    /// Observe real Core dispatch/first final-basis install without borrowing an
    /// owner. Host tests may register this before the first original is constructed.
    pub(in crate::store) fn with_accepted_advance_observation_for_test<T>(
        counts: Arc<[AtomicUsize; 2]>,
        run: impl FnOnce() -> T,
    ) -> T {
        let _reset = install(Observation {
            counts,
            lost_returns: [None, None],
            cancel_domain: None,
            close_scope: None,
        });
        run()
    }

    /// Register a one-shot test plan on THIS actual original. Index0 is after
    /// actual retired+AdvancedPendingFinal; index1 is after actual basis+Final.
    /// These test flags cannot issue any guard/proof. The optional Scope handle is
    /// the fixture's real negative-only revoker captured before acquiring Current.
    pub(in crate::store) fn observe_accepted_advance_work_for_test(
        &self,
        counts: Arc<[AtomicUsize; 2]>,
        lost_returns: [bool; 2],
        cancel_domain_at: Option<usize>,
        close_scope_at: Option<(usize, Arc<dyn ManagedChatScopeRevocation>)>,
    ) -> impl Drop + 'static {
        assert!(cancel_domain_at.is_none_or(|at| at < 2));
        assert!(close_scope_at.as_ref().is_none_or(|(at, _)| *at < 2));
        let lost_returns = std::array::from_fn(|i| {
            lost_returns[i].then(|| AppError::Invalid("injected accepted cut lost return".into()))
        });
        install(Observation {
            counts,
            lost_returns,
            cancel_domain: cancel_domain_at.map(|at| {
                (
                    at,
                    self.data
                        .resident_owner_data()
                        .expect("original advance observation owner")
                        .domain
                        .clone(),
                )
            }),
            close_scope: close_scope_at,
        })
    }

    /// Observe the actual installed model, immutable SDK Header and actual owned
    /// retired parent's Header. This reads original objects, not synthetic progress.
    /// Fixed IDs plus presence facts remain readable after the Source closes.
    pub(in crate::store) fn accepted_header_state_for_test(
        &self,
    ) -> Result<ActualHeaderState, AppError> {
        let sdk = self
            .data
            .prepared
            .as_ref()
            .ok_or_else(refused)?
            .borrow_existing_materialized_append_data(&self.original.body.original)
            .map_err(|_| refused())?;
        let cell = self.advance.as_ref().ok_or_else(refused)?;
        Ok((
            *self
                .data
                .resident_owner_data()?
                .model
                .replay_data()
                .header()
                .id()
                .as_bytes(),
            *sdk.header.id().as_bytes(),
            cell.retired
                .as_ref()
                .map(|old| *old.parent_data().header().id().as_bytes()),
            self.data.candidate.is_some(),
            cell.basis.is_some(),
        ))
    }
}

#[test]
fn accepted_observation_resets_after_unwind_and_rejects_nested_registration_without_replacing_it() {
    let counts = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let panic = std::panic::catch_unwind(|| {
        PreparedV2UnsignedOriginalCustody::with_accepted_advance_observation_for_test(
            counts.clone(),
            || panic!("exercise original observation's unwind reset"),
        );
    });
    assert!(panic.is_err());
    PreparedV2UnsignedOriginalCustody::with_accepted_advance_observation_for_test(
        counts.clone(),
        || {
            let nested = std::panic::catch_unwind(|| {
                PreparedV2UnsignedOriginalCustody::with_accepted_advance_observation_for_test(
                    Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]),
                    || (),
                );
            });
            assert!(nested.is_err());
            // Registration tests only: no synthetic Core dispatch or final proof.
            OBSERVATION.with(|slot| {
                assert!(Arc::ptr_eq(
                    &slot.borrow().as_ref().unwrap().counts,
                    &counts
                ));
            });
        },
    );
    OBSERVATION.with(|slot| assert!(slot.borrow().is_none()));
}
