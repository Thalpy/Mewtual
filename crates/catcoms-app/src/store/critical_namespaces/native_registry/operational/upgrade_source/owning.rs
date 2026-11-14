//! Whole-owner index2 -> index3 producer. Admission, physical IO and policy
//! installation share one original mount. No raw Current/Store escapes this
//! controller, and an ambiguous attempt can only be retried or closed.
use super::*;
use crate::store::critical_namespaces::{
    native_registry::preparation::NativeRegistryHandler, pending_join_codec as grammar,
};
use crate::store::persistence::native_registry::lifecycle::upgrade::{UpgradeInventory, UpgradeIo};
use crate::store::{ServerStore, V2CurrentLayoutStore};
use catcoms_crypto::unseal;
mod physical;
pub(in crate::store::critical_namespaces::native_registry) use physical::{
    AuthenticationNegative, MetadataWork, UiWork,
};

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces) enum Edge {
    BeforeAdmission,
    AfterInstall,
    AfterDestination,
    AfterInspection,
    BeforeColdExposure,
    BeforeMetadataAdmission,
    AfterMetadataCommit,
    BeforePhysicalInstall,
}
#[cfg(test)]
thread_local! {
    static EDGE: std::cell::RefCell<Option<(Edge, Box<dyn FnOnce()>)>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(in crate::store::critical_namespaces) fn at_edge(edge: Edge, hook: Box<dyn FnOnce()>) {
    EDGE.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some((edge, hook));
    });
}
#[cfg(test)]
pub(in crate::store::critical_namespaces::native_registry) fn observe_edge(edge: Edge) {
    let hook = EDGE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|(wanted, _)| *wanted == edge) {
            slot.take()
        } else {
            None
        }
    });
    if let Some((_, hook)) = hook {
        hook();
    }
}

struct Claims {
    old: interlock::IndexClaim,
    new: lifecycle_admission::Claim,
}

/// FIRST is always an armed dual closer. Even a failed admission cannot leave
/// extracted physical resources awaiting a later wrapper's Drop implementation.
pub(in crate::store::critical_namespaces::native_registry) struct Upgrade {
    close: Option<CloseOwners>,
    io: Option<UpgradeIo>,
    claims: Option<Claims>,
    complete: bool,
    // LAST: original files, buffers, whole mount and both invoices remain owned.
    prepared: PreparedUpgrade,
}
impl std::fmt::Debug for Upgrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index3Upgrade").finish_non_exhaustive()
    }
}

impl PreparedUpgrade {
    /// No IO effects precede BOTH claims. Refusal returns this same prepared
    /// owner; it never reconstructs a new index2 owner from remembered metadata.
    pub(in crate::store::critical_namespaces::native_registry) fn admit(
        mut self,
    ) -> Result<Upgrade, Box<Self>> {
        // Transfer the armed closer BEFORE building/cacheing admission-local IO.
        // A panic at any later edge drops this wrapper's closer before its IO.
        let mut upgrade = Upgrade {
            close: self.close.take(),
            io: None,
            claims: None,
            complete: false,
            prepared: self,
        };
        let admission = (|| -> Result<(), AppError> {
            let prepared = &mut upgrade.prepared;
            prepared.handler.require()?;
            let original = &mut prepared.original.as_mut().ok_or_else(refused)?.original;
            let physical = Arc::get_mut(&mut original.physical).ok_or_else(refused)?;
            physical
                .cell
                .get_mut()
                .map_err(|_| refused())?
                .require_idle()?;
            physical.handler.require()?;
            let store = physical.current.checked_store()?;
            let inventory = UpgradeInventory::capture(
                store,
                prepared.handler.debit(),
                prepared.handler.pool(),
            )?;
            let io = UpgradeIo::prepare(store, inventory, &prepared.output.bytes())?;
            upgrade.io = Some(io);
            #[cfg(test)]
            if let Some(hook) = prepared.io_retirement.take() {
                upgrade.io.as_mut().unwrap().observe_retirement(hook);
            }
            let guard = store
                .pending_join_catalog
                .try_lock()
                .map_err(|_| refused())?;
            let catalog = guard.as_ref().ok_or_else(refused)?;
            if catalog.pending.is_some()
                || catalog.current.framed != *blake3::hash(prepared.snapshot.catalog()).as_bytes()
                || !matches!(store.current_layout_policy.as_ref().and_then(|p| p.native_registry.as_ref()),
                    Some(NativeRegistryHandler::Operational(actual)) if actual.same(&physical.handler))
            {
                return Err(refused());
            }
            physical.current.cached_store()?;
            prepared.expected_current.require(store)?;
            prepared.snapshot.require(store)?;
            physical.handler.require_index_idle_cached()?;
            #[cfg(test)]
            observe_edge(Edge::BeforeAdmission);
            let new = prepared.handler.claim()?;
            let old = match physical.handler.gate.claim() {
                Ok(old) => old,
                Err(error) => {
                    // Proven no-effect local refusal. The original old owner
                    // remains alive, and no file or policy was changed.
                    let finished = new.finish();
                    drop(guard);
                    drop(finished);
                    return Err(error);
                }
            };
            upgrade.claims = Some(Claims { old, new });
            drop(guard);
            Ok(())
        })();
        match admission {
            Ok(()) => Ok(upgrade),
            Err(_) => {
                // All refusals here are proven pre-effect and before any retained
                // old claim. Destroy the unused IO outside the catalog guard,
                // then restore its affine closer by plain moves. Boxing the
                // refused whole owner allocates only outside the writer.
                drop(upgrade.io.take());
                let close = upgrade.close.take();
                let mut prepared = upgrade.prepared;
                prepared.close = close;
                Err(Box::new(prepared))
            }
        }
    }
}

/// Only the owning controller can construct this concrete edge. Current merely
/// forwards its private Store; there is no public Store loan or callback seam.
pub(in crate::store::critical_namespaces) struct CatalogHandoff<'a> {
    expected: &'a ExpectedCurrentInputs,
    output: &'a Output,
    io: &'a UpgradeIo,
    old: &'a Handler,
    new: &'a lifecycle_admission::Handler,
    claims: &'a mut Option<Claims>,
    complete: &'a mut bool,
}
impl CatalogHandoff<'_> {
    pub(in crate::store::critical_namespaces) fn install(
        &mut self,
        store: &mut ServerStore,
    ) -> Result<(), AppError> {
        self.old.require()?;
        self.new.require()?;
        store.validate_mounted_current_layout()?;
        self.expected.require(store)?;
        self.io.require_complete(store, &self.output.bytes())?;
        // Parse and fund candidates BEFORE the writer. In particular, do not
        // call the older self-locking installer or its count-zero restriction.
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = Zeroizing::new(unseal(
            &key,
            &crate::store::unframe(self.output.bytes().catalog)?,
        )?);
        let parsed = grammar::parse_lifecycle_native_registry_layout(
            &plain,
            &store.chat_vault_domain()?,
            true,
        )?;
        let next = parsed.summary(*blake3::hash(self.output.bytes().catalog).as_bytes());
        let replacement = NativeRegistryHandler::Lifecycle(self.new.clone());
        let mut retirement = None;
        let mut finished = None;
        let result = (|| {
            let mut guard = store
                .pending_join_catalog
                .try_lock()
                .map_err(|_| refused())?;
            let catalog = guard.as_mut().ok_or_else(refused)?;
            if catalog.pending.is_some() || store.chat_mount.observer().is_cancelled() {
                return Err(refused());
            }
            self.expected.require(store)?;
            self.io.require_complete(store, &self.output.bytes())?;
            let policy = store.current_layout_policy.as_mut().ok_or_else(refused)?;
            match policy.native_registry.as_ref() {
                Some(NativeRegistryHandler::Operational(actual))
                    if actual.same(self.old)
                        && catalog.current.framed
                            == *blake3::hash(self.output.bytes().original).as_bytes() =>
                {
                    let previous = policy
                        .native_registry
                        .replace(replacement)
                        .ok_or_else(refused)?;
                    let previous_summary = std::mem::replace(&mut catalog.current, next);
                    // Cache installation by its ACTUAL handler+summary before
                    // any later fallible proof; retry accepts exactly this pair.
                    retirement = Some((previous, previous_summary));
                }
                Some(NativeRegistryHandler::Lifecycle(actual))
                    if actual.same(self.new) && catalog.current.framed == next.framed => {}
                _ => return Err(refused()),
            }
            #[cfg(test)]
            observe_edge(Edge::AfterInstall);
            self.expected.require(store)?;
            self.io.require_complete(store, &self.output.bytes())?;
            if let Some(claims) = self.claims.take() {
                let old = claims.old.finish();
                let new = claims.new.finish();
                let idle = matches!(old, Ok(interlock::ClaimRelease::Idle))
                    && new.state == lifecycle_admission::Release::Idle;
                // Retain completion metadata through the catalog guard. The
                // whole original old handler likewise keeps its payer alive.
                finished = Some(new);
                if !idle {
                    self.old.gate.close();
                    self.new.close(); // close BEFORE letting a JOIN writer in.
                    return Err(refused());
                }
                *self.complete = true;
            } else if !*self.complete {
                self.new.close();
                return Err(refused());
            }
            if !self.old.gate.idle() || self.new.require_idle_cached().is_err() {
                self.old.gate.close();
                self.new.close();
                return Err(refused());
            }
            Ok(())
        })();
        // A final account refund/summary destruction must never run under the
        // catalog mutex, including errors after installation or claim finish.
        drop(finished);
        drop(retirement);
        result
    }
}

impl Upgrade {
    pub(in crate::store::critical_namespaces::native_registry) fn retry(
        &mut self,
    ) -> Result<(), AppError> {
        self.prepared.handler.require()?;
        let original = &mut self
            .prepared
            .original
            .as_mut()
            .ok_or_else(refused)?
            .original;
        let physical = Arc::get_mut(&mut original.physical).ok_or_else(refused)?;
        physical
            .cell
            .get_mut()
            .map_err(|_| refused())?
            .require_idle()?;
        let store = physical.current.checked_store()?;
        self.prepared.expected_current.require(store)?;
        self.io
            .as_mut()
            .ok_or_else(refused)?
            .advance_upgrade(store, &self.prepared.output.bytes())?;
        let mut handoff = CatalogHandoff {
            expected: &self.prepared.expected_current,
            output: &self.prepared.output,
            io: self.io.as_ref().ok_or_else(refused)?,
            old: &physical.handler,
            new: &self.prepared.handler,
            claims: &mut self.claims,
            complete: &mut self.complete,
        };
        physical.current.finish_native_index3(&mut handoff)
    }
}

/// Opaque selected3 DATA owner. No raw physical loan, server factory, native
/// command, Source or SDK authority is exposed by successful storage upgrade.
pub(in crate::store::critical_namespaces) struct Store {
    _close: lifecycle_admission::AdmissionOwner,
    // Only finite custody is shared. No work capsule can clone this closer.
    physical: Arc<physical::Physical>,
}
impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index3Store").finish_non_exhaustive()
    }
}
impl Store {
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn handler_for_test(
        &self,
    ) -> lifecycle_admission::Handler {
        self.physical.handler_for_test()
    }
    #[cfg(test)]
    pub(in crate::store::critical_namespaces) fn actual_store_for_test(
        &self,
    ) -> Result<&ServerStore, AppError> {
        self.physical.actual_store_for_test()
    }
    /// Explicit strict3 cold open. Handler payment and original fixed-marker
    /// inspection precede Current/JOIN cleaners. First-torn cleanup that leaves
    /// index2 refuses this open; it cannot manufacture a version3 owner.
    pub(in crate::store::critical_namespaces::native_registry) fn open(
        dir: impl AsRef<std::path::Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        mut current: crate::store::V2CurrentLayoutPolicy,
        policy: Policy,
    ) -> Result<Self, AppError> {
        if policy.pool.same_account(&current.archive.pending.pool)
            || current
                .member_outbox
                .as_ref()
                .is_none_or(|member| policy.pool.same_account(&member.pool))
        {
            return Err(refused());
        }
        let handler = lifecycle_admission::Handler::from_reserved(&policy, policy.reserve()?)?;
        // Allocate all physical control metadata before common opening moves
        // a real Current. The unexposed empty physical rejects every operation.
        let physical = physical::Physical::allocate(handler.clone());
        current.native_registry = Some(NativeRegistryHandler::Lifecycle(handler.clone()));
        let mut prepared =
            crate::store::PreparedV2CurrentLayoutStore::open(dir, passphrase, rng, current)?;
        prepared.retry(rng)?;
        let current = prepared
            .into_current_layout_store()
            .map_err(|_| refused())?;
        let mut value = Self {
            _close: lifecycle_admission::AdmissionOwner::new(&handler),
            physical,
        };
        Arc::get_mut(&mut value.physical)
            .expect("unexposed original physical3")
            .install(current);
        value.row_count_data()?;
        #[cfg(test)]
        observe_edge(Edge::BeforeColdExposure);
        value.physical.finish_cold()?;
        Ok(value)
    }
    pub(in crate::store::critical_namespaces) fn row_count_data(
        &mut self,
    ) -> Result<usize, AppError> {
        self.physical.row_count_data()
    }

    /// Native facade's shared DATA read still serializes through the original
    /// physical cell. The older exclusive owner API remains unchanged.
    pub(in crate::store::critical_namespaces::native_registry) fn native_row_count_data(
        &self,
    ) -> Result<usize, AppError> {
        self.physical.row_count_data()
    }

    pub(in crate::store::critical_namespaces::native_registry) fn close_admission(&self) {
        self.physical.close_admission();
    }
    pub(in crate::store::critical_namespaces::native_registry) fn lookup_creation_data(
        &self,
        invocation: [u8; 32],
        request: [u8; 32],
    ) -> Result<Option<crate::store::NativeCreationStatusData>, AppError> {
        self.physical.lookup_creation_data(invocation, request)
    }

    /// Storage-only founder transitions on the SAME poisoned cell. Outgoing
    /// debt must retire outside guards before another physical phase can begin.
    pub(in crate::store::critical_namespaces::native_registry) fn birth_founder_data(
        &mut self,
        request: &super::super::super::lifecycle::founder::Pending,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            super::super::mount::NativeCreationStatusData,
            Option<super::super::super::lifecycle::cell::CompletedDebt>,
        ),
        AppError,
    > {
        self.physical.birth_founder_data(request, rng)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn activate_founder_data(
        &mut self,
        request: &super::super::super::lifecycle::founder::Activate,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            super::super::mount::NativeCreationStatusData,
            Option<super::super::super::lifecycle::cell::CompletedDebt>,
        ),
        AppError,
    > {
        self.physical.activate_founder_data(request, rng)
    }
    /// Private local metadata CAS. No native caller/SDK authority is implied.
    /// The returned finite debt must retire outside every enclosing guard.
    pub(in crate::store::critical_namespaces::native_registry) fn rename_metadata_data(
        &mut self,
        request: &super::super::super::lifecycle::rename::Request,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            super::super::super::lifecycle::rename::Metadata,
            Option<super::super::super::lifecycle::cell::CompletedDebt>,
        ),
        AppError,
    > {
        self.physical.rename_metadata_data(request, rng)
    }
    /// Store already-minted Legacy invite DATA under the same metadata CAS.
    /// Native minting/current membership and command authorization are separate.
    pub(in crate::store::critical_namespaces::native_registry) fn refresh_legacy_invite_data(
        &mut self,
        request: &super::super::super::lifecycle::refresh::Request,
        rng: &mut impl CryptoRngCore,
    ) -> Result<
        (
            super::super::super::lifecycle::rename::Metadata,
            Option<super::super::super::lifecycle::cell::CompletedDebt>,
        ),
        AppError,
    > {
        self.physical.refresh_legacy_invite_data(request, rng)
    }
    pub(in crate::store::critical_namespaces::native_registry) fn capture_ui_work(
        &self,
    ) -> Option<UiWork> {
        self.physical.capture_ui()
    }
    /// One prepaid original physical capsule, not admission or a native/SDK
    /// credential. Cached capture performs no IO/allocation/account lock;
    /// retire the returned WHOLE capsule outside all native/writer guards.
    pub(in crate::store::critical_namespaces::native_registry) fn capture_metadata_work(
        &self,
    ) -> Option<MetadataWork> {
        self.physical.capture()
    }
}

/// All moved upgrade debt retires together, outside the writer and BEFORE the
/// selected owner is exposed. The actual independent payer is deliberately last.
struct Retirement {
    _old_close: interlock::AdmissionOwner,
    _io: UpgradeIo,
    _prepared: PreparedUpgrade,
    _old_admission: interlock::AdmissionOwner,
    _old_cell: std::sync::Mutex<super::super::cell::IndexCell>,
    // Finite physical exclusion survives destination destruction on unwind.
    // It grants no mount or native authority and retires after every file.
    _session: catcoms_storage::VaultPhysicalWorkLease,
    _old_handler: Handler,
    _new_fee_tail: lifecycle_admission::Handler,
}
// Fixed Rust graph metadata belongs to the independently reserved control
// invoice, separately from the retained frame/AEAD/row terms. This constant is
// inert size DATA, never a reservation, source or physical loan.
pub(in crate::store::critical_namespaces::native_registry) const OWNERSHIP_METADATA: usize = 8
    * (std::mem::size_of::<Upgrade>()
        + std::mem::size_of::<Store>()
        + std::mem::size_of::<Retirement>()
        + std::mem::size_of::<Claims>()
        + std::mem::size_of::<CatalogHandoff<'static>>())
    + physical::METADATA
    + 512;
impl Upgrade {
    pub(in crate::store::critical_namespaces::native_registry) fn into_store(
        mut self,
    ) -> Result<Store, Box<Self>> {
        if self.retry().is_err() || !self.complete {
            return Err(Box::new(self));
        }
        // Allocation precedes the last uniqueness proof and every original
        // Current extraction. Both original affine closers remain armed here.
        let destination_handler = self.prepared.handler.clone();
        let destination_physical = physical::Physical::allocate(destination_handler.clone());
        let Some(source) = self.prepared.original.as_mut() else {
            return Err(Box::new(self));
        };
        let session = match source.original.physical.current.cached_store() {
            Ok(store) => store._session.retain_for_physical_work(),
            Err(_) => return Err(Box::new(self)),
        };
        if Arc::get_mut(&mut source.original.physical).is_none() {
            return Err(Box::new(self));
        }
        // No allocation, fallible call, observer or external weak loan occurs
        // between the last uniqueness proof and this consuming move tail.
        let source = self.prepared.original.take().unwrap();
        let NativeIndexStore {
            _admission,
            physical,
        } = source.original;
        let physical = match Arc::try_unwrap(physical) {
            Ok(value) => value,
            Err(_) => unreachable!("exclusive original physical mount"),
        };
        let super::super::mount::NativeIndexPhysical {
            cell,
            current,
            handler,
        } = physical;
        let CloseOwners { old, new } = self.close.take().unwrap();
        let retirement = Retirement {
            _old_close: old,
            _io: self.io.take().unwrap(),
            _prepared: self.prepared,
            _old_admission: _admission,
            _old_cell: cell,
            _session: session,
            _old_handler: handler,
            _new_fee_tail: destination_handler.clone(),
        };
        // Destination is declared AFTER the complete retirement bundle. An
        // unwind therefore closes3 before any old input/buffer is destroyed.
        // Its existing affine closer travels here without creating a new gate.
        let mut destination = Store {
            _close: new,
            physical: destination_physical,
        };
        // Destination's FIRST closer is already armed if this invariant ever
        // regresses. No observer or weak loan exists before the plain move.
        #[cfg(test)]
        observe_edge(Edge::BeforePhysicalInstall);
        Arc::get_mut(&mut destination.physical)
            .expect("unexposed original physical3")
            .install(current);
        #[cfg(test)]
        observe_edge(Edge::AfterDestination);
        drop(retirement);
        Ok(destination)
    }
}
