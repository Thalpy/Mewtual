//! Original actual archive founder's pre-row negative custody. This child never interprets a
//! decoded intent or matching scalar group as Fresh. It accepts only the private source borrow
//! created by the consuming actual `Server::found` factory, and installs no admission row.
use super::*;
use crate::store::chat_admission::group_lineage::{
    ChatAdmissionGroupBounds, GROUP_BOUNDS_ALLOCATION_BYTES,
};
use crate::store::critical_namespaces::archive_bootstrap::founder::plan::ActualFounderSource;
use crate::store::critical_namespaces::archive_bootstrap::founder_codec as codec;
use crate::store::persistence::{
    CapturedFounderInvocation, FounderCreationRevoker, SavedFoundingIntent,
};
use catcoms_rt::{CryptoRngCore, MeshTransport};
mod reservation;

/// RAM-only planned lineage. Private construction requires the actual original source, actual
/// mounted-v6 VACANT table, and the same creator identity/control custody. No sender, Store,
/// SDK, decoded proof or generic adopter is stored here. Only the original creator owns life.
pub(in crate::store) struct ArchiveFounderPlan {
    capture: ChatAdmissionCapture,
    facts: codec::FoundingData,
    nonce_sampled: bool,
    debit: codec::DebitId,
    invocation: codec::InvocationData,
    identity: Arc<()>,
    observer: RequestCancellation,
    revoker: FounderCreationRevoker,
    control: Arc<CatalogReadReservation>,
    registered: bool,
    // ONLY the actual physical saved result can replace transient registry exclusion. It
    // contains inert exact ciphertext/debt/original weak metadata, never the creator sender.
    saved_foundation: Option<Arc<SavedFoundingIntent>>,
    group_bounds: ChatAdmissionGroupBounds,
    // LAST: independent paid range-key/cursor metadata, not inferred from a serialized budget.
    _group_bounds_fee: CatalogReadReservation,
}
impl std::fmt::Debug for ArchiveFounderPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveFounderPlan").finish_non_exhaustive()
    }
}
impl ArchiveFounderPlan {
    #[cfg(test)]
    pub(in crate::store) fn sampled_nonce_data_for_test(&self) -> Option<[u8; 32]> {
        self.nonce_sampled.then_some(self.facts.nonce)
    }
    pub(in crate::store) fn data(&self) -> Result<&codec::FoundingData, AppError> {
        if !self.nonce_sampled
            || (!self.registered && self.saved_foundation.is_none())
            || self.observer.is_cancelled()
        {
            return Err(refused());
        }
        Ok(&self.facts)
    }
    pub(in crate::store) fn debit_id(&self) -> codec::DebitId {
        self.debit
    }
    pub(in crate::store) fn validate_captured(
        &self,
        captured: &CapturedFounderInvocation,
    ) -> Result<(), AppError> {
        if self.observer.is_cancelled()
            || captured.observer().is_cancelled()
            || !Arc::ptr_eq(&self.identity, &captured.owner_identity())
            || self.invocation != captured.data().invocation
            || self.facts.signer != captured.data().signer
        {
            return Err(refused());
        }
        self.facts.validate(captured.data())?;
        if let Some(saved) = &self.saved_foundation {
            saved.retains_original(captured, &self.facts, self.debit)?;
        }
        Ok(())
    }
    fn prepared_entry(&self) -> Entry {
        Entry {
            key: self.capture.key.clone(),
            original: None,
            mount: self.capture.mount_identity.clone(),
            source: *self.debit.as_bytes(),
            revision: 1,
            kind: Kind::ArchiveBootstrap,
            identity: self.identity.clone(),
            revoker: PredecessorRevoker::Archive(self.revoker.clone()),
            negative: self.observer.clone(),
            continuation: None,
            _metadata_fee: MetadataCustody::Archive {
                _fee: self.control.clone(),
            },
        }
    }
}

/// Borrowed ORIGINAL writer+actual source+RAM plan. It cannot be constructed by Root, from
/// records, or from a callback. Root must keep it and its subordinate VACANT guard borrowed
/// through every target/debt wait; negative validation repeats the same physical source.
pub(in crate::store) struct ArchiveFounderAdmission<'writer, T: MeshTransport, R: CryptoRngCore> {
    store: &'writer ServerStore,
    source: &'writer ActualFounderSource<'writer, T, R>,
    plan: &'writer ArchiveFounderPlan,
    runtime: &'writer Runtime,
}
impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderAdmission<'_, T, R> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.store
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.plan.capture.key.group()
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        self.plan.capture.key.signer()
    }
    pub(in crate::store) fn plan_data(&self) -> &codec::FoundingData {
        &self.plan.facts
    }
    pub(in crate::store) fn debit_id(&self) -> codec::DebitId {
        self.plan.debit
    }
    pub(in crate::store) fn require_founding_phase(&self) -> Result<(), AppError> {
        self.source.require_founding_phase()
    }
    pub(in crate::store) fn validate_transferred_source(
        &self,
        captured: &CapturedFounderInvocation,
        context: &catcoms_sync::owner_v2::BootstrapSourceContextData,
        id: catcoms_sync::owner_v2::BootstrapSourceId,
    ) -> Result<(), AppError> {
        self.validate_source_only(captured)?;
        self.source
            .validate_transferred_source(captured, context, id, &self.plan.facts)
    }
    /// NO Root or subordinate writer acquisition here. Physical persistence already holds
    /// those ORIGINAL locks and must repeat its own Root/session validation separately.
    pub(in crate::store) fn validate_source_only(
        &self,
        captured: &CapturedFounderInvocation,
    ) -> Result<(), AppError> {
        self.source.validate_captured(captured)?;
        self.source.validate_source_only()?;
        self.source.validate_admission_phase(&self.plan.facts)?;
        self.plan.validate_captured(captured)?;
        if (!self.plan.registered && self.plan.saved_foundation.is_none())
            || !self.plan.nonce_sampled
            || self.plan.capture.mount.is_cancelled()
            || !Arc::ptr_eq(
                &self.plan.capture.mount_identity,
                &self.store.chat_mount.identity,
            )
            || self.source.group()? != self.plan.facts.group
            || self.source.fresh_attempt()? != self.plan.facts.fresh_attempt
            || self.source.debit_id()? != self.plan.debit
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn validate_original(&self) -> Result<(), AppError> {
        validate_plan_source(self.store, self.source, self.plan, self.runtime)
    }
}

fn validate_plan_source<T: MeshTransport, R: CryptoRngCore>(
    store: &ServerStore,
    source: &ActualFounderSource<'_, T, R>,
    plan: &ArchiveFounderPlan,
    runtime: &Runtime,
) -> Result<(), AppError> {
    if plan.group_bounds.group() != plan.capture.key.group() {
        return Err(refused());
    }
    plan.group_bounds.require_absent(runtime)?;
    // This deliberately avoids chat_vault_format/domain/capture getters: admission is already
    // held. Root identity/source checks acquire only the subsequent Root lock in the order.
    source.validate_original(store)?;
    source.validate_admission_phase(&plan.facts)?;
    plan.validate_captured(source.captured())?;
    if !runtime.managed
        || runtime.dirty
        || runtime.format != Some(catcoms_storage::VaultFormat::CriticalNamespaces)
        || !Arc::ptr_eq(&plan.capture.mount_identity, &store.chat_mount.identity)
        || plan.capture.mount.is_cancelled()
        || plan.capture.original.is_some()
        || plan.capture.admission.is_some()
        || plan.capture.issued.lock().map_err(|_| refused())?.is_some()
        || runtime
            .table
            .as_ref()
            .ok_or_else(refused)?
            .contains_key(&plan.capture.key)
        || runtime.owners.contains_key(&plan.capture.key)
        || source.group()? != plan.facts.group
        || source.fresh_attempt()? != plan.facts.fresh_attempt
        || source.debit_id()? != plan.debit
    {
        return Err(refused());
    }
    let existing = runtime
        .predecessors
        .slots
        .iter()
        .flatten()
        .find(|entry| entry.key == plan.capture.key);
    if plan.registered {
        let entry = existing.ok_or_else(refused)?;
        if !plan.nonce_sampled
            || entry.kind != Kind::ArchiveBootstrap
            || entry.original.is_some()
            || entry.source != *plan.debit.as_bytes()
            || entry.revision != 1
            || !Arc::ptr_eq(&entry.identity, &plan.identity)
            || !Arc::ptr_eq(&entry.mount, &plan.capture.mount_identity)
            || entry.negative.is_cancelled()
        {
            return Err(refused());
        }
    } else if existing.is_some() {
        return Err(refused());
    }
    Ok(())
}

impl ServerStore {
    /// Capture DATA from the actual owned node, never from a supplied Server/row. All bounded
    /// metadata is prepared under the creator's existing named fee before acquiring writers.
    pub(in crate::store) fn capture_original_archive_plan<T: MeshTransport, R: CryptoRngCore>(
        &self,
        source: &ActualFounderSource<'_, T, R>,
    ) -> Result<ArchiveFounderPlan, AppError> {
        source.validate_original(self)?;
        source.untouched_scope()?;
        let captured = source.captured();
        let group_bounds_fee = self
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool
            .try_reserve(GROUP_BOUNDS_ALLOCATION_BYTES)?;
        let group_bounds = ChatAdmissionGroupBounds::new(source.group()?)?;
        let key = ChatAdmissionKey::new(source.group()?, captured.data().signer)?;
        let facts = codec::FoundingData {
            group: key.group.clone(),
            signer: *key.signer(),
            fresh_attempt: source.fresh_attempt()?,
            scope_domain: captured.data().invocation.vault,
            nonce: [0; 32],
            revision: 1,
        };
        let plan = ArchiveFounderPlan {
            capture: ChatAdmissionCapture {
                key,
                original: None,
                mount_identity: self.chat_mount.identity.clone(),
                mount: self.chat_mount.observer(),
                admission: None,
                issued: Mutex::new(None),
            },
            facts,
            nonce_sampled: false,
            debit: source.debit_id()?,
            invocation: captured.data().invocation,
            identity: captured.owner_identity(),
            observer: captured.observer(),
            revoker: captured.revoker(),
            control: captured.control_custody(),
            registered: false,
            saved_foundation: None,
            group_bounds,
            _group_bounds_fee: group_bounds_fee,
        };
        let runtime = self.chat_mount.locked()?;
        // SAME admission -> bounded catalog, before any pin/head/Root subordinate. Cold JOIN
        // sources remain exclusions even before they activate a process-local predecessor.
        self.require_no_pending_checkpoint_join_group(plan.capture.key.group())?;
        validate_plan_source(self, source, &plan, &runtime)?;
        drop(runtime);
        Ok(plan)
    }

    /// No fallible work after installation can lose the once-sampled nonce. If the original
    /// negative loses after the draw, `plan` retains it and refuses forever rather than renewing.
    pub(in crate::store) fn register_original_archive_plan<T: MeshTransport, R: CryptoRngCore>(
        &self,
        source: &ActualFounderSource<'_, T, R>,
        plan: &mut ArchiveFounderPlan,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        // A concrete saved target already replaced this transient slot. Re-registering it
        // would create a false unresolved job/lifetime cap and cannot renew the original plan.
        if plan.saved_foundation.is_some() {
            return Err(refused());
        }
        // Both cloned keys and custody are bounded/paid and prepared outside all writers.
        let entry = (!plan.registered).then(|| plan.prepared_entry());
        let mut runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(plan.capture.key.group())?;
        validate_plan_source(self, source, plan, &runtime)?;
        if plan.registered {
            let permit = ArchiveFounderAdmission {
                store: self,
                source,
                plan,
                runtime: &runtime,
            };
            let vacancy = self.lock_archive_vacant_owner_namespace(&permit)?;
            vacancy.validate_original()?;
            self.require_no_archive_bootstrap_target(permit.group(), permit.signer())?;
            return permit.validate_original();
        }
        // Reject full metadata before RNG. No dead/expired entry is evicted to invent absence.
        let index = runtime
            .predecessors
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or_else(refused)?;
        if !plan.nonce_sampled {
            {
                let permit = ArchiveFounderAdmission {
                    store: self,
                    source,
                    plan,
                    runtime: &runtime,
                };
                let vacancy = self.lock_archive_vacant_owner_namespace(&permit)?;
                vacancy.validate_original()?;
                self.require_no_archive_bootstrap_target(permit.group(), permit.signer())?;
                permit.validate_original()?;
            }
            // Preserve before any post-draw validation. The old pool guards are released only
            // to mutate this borrowed plan; admission remains held. Reacquire/recheck below,
            // because an old pool-only registration could have won during this small gap.
            rng.fill_bytes(&mut plan.facts.nonce);
            plan.nonce_sampled = true;
        }
        // Install NEGATIVE exclusion first; it conveys no old-head absence. A pool-only old
        // reservation could win while the preceding guards drain so the sampled nonce can be
        // retained. Reacquire below: that conflict refuses/preserves BOTH sources and this weak
        // exclusion, never returning successful vacancy or admitting Root I/O from stale state.
        validate_plan_source(self, source, plan, &runtime)?;
        runtime.predecessors.slots[index] = entry;
        plan.registered = true;
        {
            let permit = ArchiveFounderAdmission {
                store: self,
                source,
                plan,
                runtime: &runtime,
            };
            let vacancy = self.lock_archive_vacant_owner_namespace(&permit)?;
            vacancy.validate_original()?;
            self.require_no_archive_bootstrap_target(permit.group(), permit.signer())?;
            permit.validate_original()?;
        }
        drop(runtime);
        Ok(())
    }

    /// Concrete original factory's effect scope. The return type cannot retain a subordinate
    /// writer: its actual borrow of this local permit ties it to the held admission callback.
    pub(in crate::store) fn with_original_archive_founder<T: MeshTransport, R: CryptoRngCore, O>(
        &self,
        source: &ActualFounderSource<'_, T, R>,
        plan: &ArchiveFounderPlan,
        effect: impl FnOnce(&ArchiveFounderAdmission<'_, T, R>) -> Result<O, AppError>,
    ) -> Result<O, AppError> {
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(plan.capture.key.group())?;
        if !plan.registered && plan.saved_foundation.is_none() {
            return Err(refused());
        }
        let permit = ArchiveFounderAdmission {
            store: self,
            source,
            plan,
            runtime: &runtime,
        };
        permit.validate_original()?;
        let result = effect(&permit);
        drop(runtime);
        result
    }

    /// Replace one exact RAM exclusion with its ACTUAL durable target/intent fence. Missing
    /// entries never count as completion: a caller must retain this concrete saved result,
    /// and the same source/file/root is checked before removal. No generic slot eviction,
    /// weak-expiry inference, sender renewal or lifetime-five-group limit is introduced.
    pub(in crate::store) fn release_saved_archive_founder_predecessor<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        &self,
        source: &ActualFounderSource<'_, T, R>,
        plan: &mut ArchiveFounderPlan,
        saved: Arc<SavedFoundingIntent>,
    ) -> Result<(), AppError> {
        let mut runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(plan.capture.key.group())?;
        validate_plan_source(self, source, plan, &runtime)?;
        if let Some(original) = &plan.saved_foundation {
            if !Arc::ptr_eq(original, &saved) {
                return Err(refused());
            }
            let permit = ArchiveFounderAdmission {
                store: self,
                source,
                plan,
                runtime: &runtime,
            };
            let vacancy = self.lock_archive_vacant_owner_namespace(&permit)?;
            return saved.validate_original(self, source.captured(), &vacancy);
        }
        if !plan.registered {
            return Err(refused());
        }
        {
            let permit = ArchiveFounderAdmission {
                store: self,
                source,
                plan,
                runtime: &runtime,
            };
            let vacancy = self.lock_archive_vacant_owner_namespace(&permit)?;
            saved.validate_original(self, source.captured(), &vacancy)?;
            saved.retains_original(source.captured(), &plan.facts, plan.debit)?;
        }
        // The SAME admission remains held. No supported path deletes/replaces this immutable
        // saved target; old generic/pool adapters must consult its durable negative fence.
        // Drop subordinate borrows before mutating the slot, then drain its shared metadata
        // only after admission. No new quota owner/large payload is constructed here.
        let index = runtime
            .predecessors
            .slots
            .iter()
            .position(|entry| {
                entry.as_ref().is_some_and(|entry| {
                    entry.kind == Kind::ArchiveBootstrap
                        && entry.key == plan.capture.key
                        && entry.source == *plan.debit.as_bytes()
                        && entry.original.is_none()
                        && entry.revision == 1
                        && Arc::ptr_eq(&entry.identity, &plan.identity)
                        && Arc::ptr_eq(&entry.mount, &plan.capture.mount_identity)
                })
            })
            .ok_or_else(refused)?;
        let retired = runtime.predecessors.slots[index].take();
        plan.saved_foundation = Some(saved);
        plan.registered = false;
        drop(runtime);
        drop(retired);
        Ok(())
    }
}
