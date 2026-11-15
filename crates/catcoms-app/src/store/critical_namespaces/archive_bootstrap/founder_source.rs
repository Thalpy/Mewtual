//! Saved original FI -> same private scope/S0/genesis/bootstrap source. No row, actor or
//! source I/O is admitted here. Incomplete private stages retain this owner before mutation;
//! they can NEVER fall back into the old untouched Planning/Founding constructor.
use super::*;
use crate::store::persistence::{
    PreparedBootstrapSourceTerminal, RegisteredBootstrapSource, SavedBootstrapSourceFiles,
    SavedBootstrapSourcePrepared, SavedBootstrapSourceTerminal,
};
use catcoms_sync::owner_v2::{
    BootstrapSourceContextData, BootstrapSourceId, FounderArchivePreparation,
    FounderBootstrapPreparation, FrozenFounder, PreparedBootstrapSource, RetainedFounderSigning,
    RETAINED_FOUNDER_SIGNING_INVOICE,
};

const PHASE_METADATA_FEE: usize = 64 * 1024;
#[path = "founder_source_handoff.rs"]
mod handoff;
pub(in crate::store) use handoff::{
    CompletedWarmFounderOrigin, WarmFounderCurrentReservation, WarmSavedSourceSdkReopenEvidence,
};
#[path = "founder_source_prepared.rs"]
mod prepared;
#[path = "founder_source_terminal.rs"]
mod terminal;
#[path = "founder_warm.rs"]
mod warm;
#[path = "founder_source_write.rs"]
mod write;
pub use warm::{
    FreshV2TextFounderBegin, FreshV2TextFounderBinding, FreshV2TextFounderPolicy,
    PreparedFreshV2TextFounder,
};

enum SourcePhase {
    PendingPrivateStage,
    // Install BEFORE either primitive. This phase survives error/unwind from preparation
    // or consuming freeze, retaining raw signatures while temporary signed DATA drains.
    Signing {
        signing: Box<RetainedFounderSigning>,
        _fee: CatalogReadReservation,
    },
    Frozen(Box<FrozenFounder>),
    Archive(Box<FounderArchivePreparation>),
    Bootstrap(Box<FounderBootstrapPreparation>),
    Transferred {
        source: Box<FounderBootstrapPreparation>,
        context: Box<BootstrapSourceContextData>,
        id: BootstrapSourceId,
    },
    // This is also a temporary exclusive move-out sentinel. No borrow/I/O path accepts it;
    // returned actual originals restore their exact phase before an error returns.
    HeldConsumingRefusal,
    // The complete original runtime moved to the closed warm driver. The creator
    // and its fee stay owned here; none of the earlier source APIs accepts this phase.
    RuntimeTransferred,
}

/// The SAME actual original creator, now retained before any private scope mutation. Private
/// producers drain before creator/provider/encoding fees; metadata credit drains last. No
/// public node/key/source setters, Clone, decoded constructor or runnable Server escape exist.
pub struct ArchiveFounderSourcePreparation<T: MeshTransport, R: CryptoRngCore> {
    phase: SourcePhase,
    detached: Option<PreparedBootstrapSource>,
    source_fee: Option<CatalogReadReservation>,
    registered: Option<RegisteredBootstrapSource>,
    prepared_journal: Option<SavedBootstrapSourcePrepared>,
    saved_files: Option<SavedBootstrapSourceFiles>,
    prepared_terminal: Option<PreparedBootstrapSourceTerminal>,
    saved_terminal: Option<SavedBootstrapSourceTerminal>,
    original: ArchiveFounderPlanning<T, R>,
    #[cfg(test)]
    before_freeze: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    before_source_return: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    before_source_io: Option<Box<dyn FnOnce() + Send>>,
    _phase_fee: CatalogReadReservation,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for ArchiveFounderSourcePreparation<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveFounderSourcePreparation")
            .finish_non_exhaustive()
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Drop for ArchiveFounderSourcePreparation<T, R> {
    fn drop(&mut self) {
        // Close weak original work BEFORE draining potentially large cached S0/ciphertext.
        // Root retains only inert complete debt; its observers cannot keep this life alive.
        self.original.close();
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFounderSourceProgress {
    /// Complete exact ciphertext was prepared from this private original (retained by it or
    /// its same inert Root debt after transfer). No SP, covering source save, Current or native
    /// success is inferred from this private preparation observation.
    BootstrapSourcePreparedUnexposed,
    /// Root owns COMPLETE ciphertext and its separate real fee, before SourcePrepared journal
    /// mutation. This is neither a saved source nor Current/native creation success.
    BootstrapCipherRetainedUnexposed,
    /// Only the actual small SourcePrepared journal passed its barriers. No source file,
    /// large-write execution arm, Current or native creation success follows from this.
    SourcePreparedJournalSavedUnexposed,
    /// Exact source seed, destination and two genesis pages passed one durably debited
    /// physical pass and final original guards. Still no scope/head/Current/native success.
    SourceFilesSavedUnexposed,
    /// The exact completed original pass also has its small durable terminal closure.
    /// Cold readers still need a NEW paid source/SDK capture; this is not Current.
    SourceSavedUnexposed,
}
pub struct ReturnedArchiveFounderPlanning<T: MeshTransport, R: CryptoRngCore> {
    original: ArchiveFounderPlanning<T, R>,
    error: AppError,
}
impl<T: MeshTransport, R: CryptoRngCore> ReturnedArchiveFounderPlanning<T, R> {
    pub fn into_parts(self) -> (ArchiveFounderPlanning<T, R>, AppError) {
        (self.original, self.error)
    }
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for ReturnedArchiveFounderPlanning<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReturnedArchiveFounderPlanning")
            .finish_non_exhaustive()
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderPlanning<T, R> {
    /// Early refusal still has an untouched node and returns the SAME complete Planning
    /// owner. Once this succeeds all private stage errors stay in SourcePreparation instead.
    // Return ownership inline: a refusal must not allocate to preserve the original owner,
    // especially when the phase's separate metadata payment itself was refused.
    #[allow(clippy::result_large_err)]
    pub fn into_source_preparation(
        self,
        store: &ServerStore,
    ) -> Result<ArchiveFounderSourcePreparation<T, R>, ReturnedArchiveFounderPlanning<T, R>> {
        self.into_source_preparation_with_access(store, FounderAccess::Library)
    }
    #[allow(clippy::result_large_err)] // preserve the same prepaid owner on refusal
    pub(in crate::store) fn into_source_preparation_with_access(
        self,
        store: &ServerStore,
        access: FounderAccess<'_>,
    ) -> Result<ArchiveFounderSourcePreparation<T, R>, ReturnedArchiveFounderPlanning<T, R>> {
        let result = (|| {
            access.require(&self.creator.origin)?;
            let source = ActualFounderSource::founding(&self.creator);
            source.validate_original(store)?;
            source.untouched_scope()?;
            let plan = self.plan.as_ref().ok_or_else(refused)?;
            let saved = self.saved_founding.as_ref().ok_or_else(refused)?;
            // Reserve before constructing the phase wrapper. This is ADDITIONAL fixed
            // metadata, never a refund/reuse of the still-live original provider/source fee.
            let configured = store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?;
            let fee = configured.pending.pool.try_reserve(PHASE_METADATA_FEE)?;
            let layout = std::mem::size_of::<ArchiveFounderSourcePreparation<T, R>>()
                // The SAME embedded Planning/node is already funded by its original creator
                // and control fee. Charge extra inline/return metadata and ALL three bounded
                // phase payload layouts conservatively, including move/conversion overlap.
                .checked_sub(std::mem::size_of::<ArchiveFounderPlanning<T, R>>())
                .and_then(|n| {
                    std::mem::size_of::<ReturnedArchiveFounderPlanning<T, R>>()
                        .checked_sub(std::mem::size_of::<ArchiveFounderPlanning<T, R>>())
                        .and_then(|returned| n.checked_add(returned))
                })
                .and_then(|n| n.checked_add(std::mem::size_of::<FrozenFounder>()))
                .and_then(|n| n.checked_add(std::mem::size_of::<FounderArchivePreparation>()))
                .and_then(|n| n.checked_add(std::mem::size_of::<FounderBootstrapPreparation>()))
                .and_then(|n| n.checked_add(std::mem::size_of::<BootstrapSourceContextData>()))
                .and_then(|n| n.checked_add(8 * 1024))
                .filter(|n| *n <= PHASE_METADATA_FEE)
                .ok_or_else(refused)?;
            let _ = layout; // named bounded context/adapter/phase storage, not SDK typed heaps
            store.with_original_archive_founder(&source, plan, |admission| {
                let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
                saved.validate_original(store, &self.creator.original, &vacancy)?;
                plan.reservation_for_saved_source(&self.creator.original, saved)?;
                Ok(())
            })?;
            source.validate_original(store)?;
            Ok(fee)
        })();
        match result {
            Ok(fee) => Ok(ArchiveFounderSourcePreparation {
                phase: SourcePhase::PendingPrivateStage,
                detached: None,
                source_fee: None,
                registered: None,
                prepared_journal: None,
                saved_files: None,
                prepared_terminal: None,
                saved_terminal: None,
                original: self,
                #[cfg(test)]
                before_freeze: None,
                #[cfg(test)]
                before_source_return: None,
                #[cfg(test)]
                before_source_io: None,
                _phase_fee: fee,
            }),
            Err(error) => Err(ReturnedArchiveFounderPlanning {
                original: self,
                error,
            }),
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderSourcePreparation<T, R> {
    pub fn close(&self) {
        self.original.close();
    }
    /// This must be the LAST observation before private progress. A full S0/provider matcher
    /// may wait long enough for the original Root or weak creator to change, so checking Root
    /// before that matcher is insufficient. No full comparison may follow this final check.
    fn finish_private_observation(
        &mut self,
        store: &ServerStore,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        #[cfg(test)]
        if let Some(hook) = self.before_source_return.take() {
            hook();
        }
        self.original.creator.original.validate_original(store)?;
        if self.original.creator.original.observer().is_cancelled() {
            return Err(refused());
        }
        Ok(ArchiveFounderSourceProgress::BootstrapSourcePreparedUnexposed)
    }
    /// Pending stage has no Root/SP effect capability. Validate exact current private facts;
    /// actual original Fresh/complete provider+scope equality is checked by the producer next.
    fn validate_pending(&self) -> Result<(), AppError> {
        let creator = &self.original.creator;
        if creator.original.observer().is_cancelled() {
            return Err(refused());
        }
        let plan = self.original.plan.as_ref().ok_or_else(refused)?;
        plan.validate_captured(&creator.original)?;
        let facts = plan.data()?;
        let server = creator.server.as_ref().ok_or_else(refused)?;
        let fresh = server.sync.fresh_chat_admission().ok_or_else(refused)?;
        if creator.group_data()? != facts.group
            || fresh.signer() != &facts.signer
            || fresh.attempt_id() != facts.fresh_attempt
            || fresh.key_package_reference().is_some()
            || fresh.authenticated_welcome_hash().is_some()
        {
            return Err(refused());
        }
        match server.sync.chat_scope_label() {
            None if server.sync.chat_scope_revision() == 0 => {}
            Some(label)
                if !label.retired()
                    && label.domain() == facts.scope_domain
                    && label.nonce() == facts.nonce
                    && matches!(
                        (label.initializing(), server.sync.chat_scope_revision()),
                        (true, 1) | (false, 2)
                    ) => {}
            _ => return Err(refused()),
        }
        let saved = self.original.saved_founding.as_ref().ok_or_else(refused)?;
        plan.reservation_for_saved_source(&creator.original, saved)?;
        Ok(())
    }

    /// Source preparation performs no disk source/SP operation. Original Root is checked
    /// before/after private producer work, and every partial actual producer stays retained.
    pub fn prepare_private_source(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        self.prepare_private_source_with_access(store, rng, FounderAccess::Library)
    }
    pub(in crate::store) fn prepare_private_source_with_access(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
        mut access: FounderAccess<'_>,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        access.require(&self.original.creator.origin)?;
        self.original.creator.original.validate_original(store)?;
        self.validate_pending()?;
        if matches!(self.phase, SourcePhase::HeldConsumingRefusal) {
            return Err(refused());
        }
        if let SourcePhase::Transferred {
            source,
            context,
            id,
        } = &self.phase
        {
            let key = Zeroizing::new(store.keys.db_key()?);
            let node = &self
                .original
                .creator
                .server
                .as_ref()
                .ok_or_else(refused)?
                .sync;
            source.validate_transferred_source(node, &key, context, *id)?;
            return self.finish_private_observation(store);
        }
        if matches!(self.phase, SourcePhase::PendingPrivateStage) {
            let creator = &mut self.original.creator;
            let plan = self.original.plan.as_ref().ok_or_else(refused)?;
            let saved = self.original.saved_founding.as_ref().ok_or_else(refused)?;
            let reservation = plan.reservation_for_saved_source(&creator.original, saved)?;
            let facts = plan.data()?;
            let server = creator.server.as_mut().ok_or_else(refused)?;
            server.sync.require_managed_chat_scope();
            server
                .sync
                .stage_managed_chat_scope(facts.scope_domain, facts.nonce)?;
            server.sync.attach_managed_chat_reservation(&reservation)?;
            let configured = store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?;
            let fee = configured
                .pending
                .pool
                .try_reserve(RETAINED_FOUNDER_SIGNING_INVOICE)?;
            let signing = server.sync.begin_owner_v2_founder_signing()?;
            self.phase = SourcePhase::Signing {
                signing: Box::new(signing),
                _fee: fee,
            };
        }
        if let SourcePhase::Signing { signing, .. } = &mut self.phase {
            let server = self.original.creator.server.as_ref().ok_or_else(refused)?;
            let prepared = access.prepare_bootstrap(&server.sync, signing)?;
            #[cfg(test)]
            {
                FOUNDERS_PREPARED.with(|count| count.set(count.get() + 1));
                if let Some(hook) = self.before_freeze.take() {
                    hook();
                }
            }
            // Borrow Signing THROUGH freeze. Losing a temporary PreparedFounder must not
            // lose its actual primitive results or let a retry enter either key again.
            let frozen = Box::new(prepared.freeze(&server.sync)?);
            self.phase = SourcePhase::Frozen(frozen);
            self.original.creator.original.validate_original(store)?;
        }
        if let SourcePhase::Frozen(frozen) = &mut self.phase {
            let node = &mut self
                .original
                .creator
                .server
                .as_mut()
                .ok_or_else(refused)?
                .sync;
            frozen.cache_private_snapshot(node, self.original.creator.policy.limits)?;
            self.original.creator.original.validate_original(store)?;
        }
        if matches!(self.phase, SourcePhase::Frozen(_)) {
            let node = &self
                .original
                .creator
                .server
                .as_ref()
                .ok_or_else(refused)?
                .sync;
            let SourcePhase::Frozen(frozen) =
                std::mem::replace(&mut self.phase, SourcePhase::HeldConsumingRefusal)
            else {
                return Err(refused());
            };
            match FounderArchivePreparation::from_cached_source(
                *frozen,
                node,
                self.original.creator.original.data().invocation.vault,
            ) {
                Ok(archive) => self.phase = SourcePhase::Archive(Box::new(archive)),
                Err(returned) => {
                    self.phase = SourcePhase::Frozen(Box::new(returned.into_original()));
                    return Err(refused());
                }
            }
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        if let SourcePhase::Archive(archive) = &mut self.phase {
            let node = &self
                .original
                .creator
                .server
                .as_ref()
                .ok_or_else(refused)?
                .sync;
            archive.prepare(node, &key, rng)?;
            self.original.creator.original.validate_original(store)?;
        }
        if matches!(self.phase, SourcePhase::Archive(_)) {
            let node = &self
                .original
                .creator
                .server
                .as_ref()
                .ok_or_else(refused)?
                .sync;
            let SourcePhase::Archive(archive) =
                std::mem::replace(&mut self.phase, SourcePhase::HeldConsumingRefusal)
            else {
                return Err(refused());
            };
            match FounderBootstrapPreparation::from_archive(*archive, node) {
                Ok(bootstrap) => self.phase = SourcePhase::Bootstrap(Box::new(bootstrap)),
                Err(returned) => {
                    self.phase = SourcePhase::Archive(Box::new(returned.into_original()));
                    return Err(refused());
                }
            }
        }
        let SourcePhase::Bootstrap(bootstrap) = &mut self.phase else {
            return Err(refused());
        };
        let node = &self
            .original
            .creator
            .server
            .as_ref()
            .ok_or_else(refused)?
            .sync;
        bootstrap.prepare(node, &key, rng)?;
        bootstrap.borrow_prepared_for_handoff(node, &key)?;
        self.finish_private_observation(store)
    }

    /// Reserve real detached/decrypt/verification custody BEFORE the actual one-time take.
    /// Root refusal preserves these same complete output+fee Options; no second output can be
    /// made from the private Transferred owner, even before successful Root registration.
    fn detach_for_root(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<(), AppError> {
        self.prepare_private_source_with_access(store, rng, access)?;
        if matches!(self.phase, SourcePhase::Transferred { .. }) {
            return Ok(());
        }
        {
            // A complete ACTUAL Bootstrap can reobserve its physical FI parent after private
            // scope attachment. Pending/decoded/merely scoped states cannot get this guard.
            let key = Zeroizing::new(store.keys.db_key()?);
            let SourcePhase::Bootstrap(bootstrap) = &self.phase else {
                return Err(refused());
            };
            let source = ActualFounderSource {
                creator: &self.original.creator,
                phase: ActualSourcePhase::Bootstrap {
                    source: bootstrap,
                    key: &key,
                },
            };
            let plan = self.original.plan.as_ref().ok_or_else(refused)?;
            let saved = self.original.saved_founding.as_ref().ok_or_else(refused)?;
            store.with_original_archive_founder(&source, plan, |admission| {
                let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
                saved.validate_original(store, &self.original.creator.original, &vacancy)
            })?;
        }
        if self.source_fee.is_none() {
            self.source_fee = Some(
                store.reserve_original_bootstrap_source_custody(&self.original.creator.original)?,
            );
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        let node = &self
            .original
            .creator
            .server
            .as_ref()
            .ok_or_else(refused)?
            .sync;
        let SourcePhase::Bootstrap(mut source) =
            std::mem::replace(&mut self.phase, SourcePhase::HeldConsumingRefusal)
        else {
            return Err(refused());
        };
        let selected = source
            .borrow_prepared_for_handoff(node, &key)
            .map(|output| (output.context_data().clone(), output.id()));
        let (context, id) = match selected {
            Ok(selected) => selected,
            Err(error) => {
                self.phase = SourcePhase::Bootstrap(source);
                return Err(error.into());
            }
        };
        let output = match source.take_prepared_for_handoff(node, &key) {
            Ok(output) => output,
            Err(error) => {
                self.phase = SourcePhase::Bootstrap(source);
                return Err(error.into());
            }
        };
        // Cache terminal ownership BEFORE any post-take check can fail. No bytes are dropped
        // because a caller lost an observed result; the same owner still has both Options.
        self.detached = Some(output);
        self.phase = SourcePhase::Transferred {
            source,
            context: Box::new(context),
            id,
        };
        self.original.creator.original.validate_original(store)?;
        Ok(())
    }

    /// Register only the SAME actual Transferred source, using concrete borrowed guards. The
    /// Root method moves complete ciphertext+fee into inert Store debt BEFORE any SP mutation.
    /// There is deliberately no public mutable output/source or raw-cipher registration API.
    pub fn register_private_source(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        self.register_private_source_with_access(store, rng, FounderAccess::Library)
    }
    pub(in crate::store) fn register_private_source_with_access(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        access.require(&self.original.creator.origin)?;
        self.detach_for_root(store, rng, access)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let SourcePhase::Transferred {
            source: bootstrap,
            context,
            id,
        } = &self.phase
        else {
            return Err(refused());
        };
        let source = ActualFounderSource {
            creator: &self.original.creator,
            phase: ActualSourcePhase::Transferred {
                source: bootstrap,
                key: &key,
                context,
                id: *id,
            },
        };
        let plan = self.original.plan.as_ref().ok_or_else(refused)?;
        let captured = &self.original.creator.original;
        let saved = self.original.saved_founding.as_ref().ok_or_else(refused)?;
        let detached = &mut self.detached;
        let fee = &mut self.source_fee;
        let registered = &mut self.registered;
        store.with_original_archive_founder(&source, plan, |admission| {
            let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
            vacancy.validate_transferred_source(captured, context, *id)?;
            if registered.is_none() {
                *registered = Some(if detached.is_some() || fee.is_some() {
                    store.register_original_bootstrap_source(
                        captured,
                        &vacancy,
                        saved.clone(),
                        detached,
                        fee,
                    )?
                } else {
                    // Actual lower handoff may have succeeded before an observed reply was
                    // lost. Lookup is exact original cell/cipher/fee, never a new source/arm.
                    store.lookup_original_bootstrap_source(captured, &vacancy, context, *id)?
                });
            }
            registered
                .as_ref()
                .ok_or_else(refused)?
                .validate_original(store, captured, &vacancy)
        })?;
        self.original.creator.original.validate_original(store)?;
        if self.original.creator.original.observer().is_cancelled() {
            return Err(refused());
        }
        Ok(ArchiveFounderSourceProgress::BootstrapCipherRetainedUnexposed)
    }
}

#[cfg(test)]
std::thread_local! {static FOUNDERS_PREPARED:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
#[cfg(test)]
#[path = "founder_source_tests.rs"]
mod tests;
