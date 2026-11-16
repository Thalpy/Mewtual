//! Closed SAME-node loans for activation-seed settlement, connected to the concrete scope
//! barrier, namespace, physical worker and settled reader. No code may unlink a seed merely by constructing one of these
//! loans, and no public trait or caller-supplied snapshot can construct the loan.
use super::*;
use crate::store::chat_admission::v2_current::{
    ColdV2CurrentPreparation, ColdV2InitializingPreparation,
};
use crate::store::owner_evidence::canonical::v2_registration::{
    cold_current::PreparedColdV2WriterRegistration,
    cold_initializing::PreparedColdV2InitializingWriterRegistration, V2WriterObservation,
};
use crate::store::persistence::v2_genesis_activation::initializing_repair::SavedColdV2GenesisRepairedHead;
use crate::store::persistence::v2_genesis_activation::settlement::PreparedGenesisSeedSettlement;
use crate::store::persistence::v2_genesis_activation::{
    cold_writer::PreparedColdV2CurrentWriterReceipt, ColdV2GenesisCurrentHead, SavedV2GenesisHead,
};

/// Actual concrete ownership families, never a decoded role or optional head. This core
/// stays PRIVATE to the Current implementation family: Scope receives only existing
/// concrete borrows and cannot manufacture a core or construct an owner from references.
#[derive(Clone, Copy)]
pub(super) enum GenesisCurrentOwnerRole<'owner> {
    Warm {
        scope: &'owner V2CurrentPreparation,
        head: &'owner Arc<SavedV2GenesisHead>,
        writer: &'owner PreparedV2WriterRegistration,
    },
    Cold {
        scope: &'owner ColdV2CurrentPreparation,
        head: &'owner Arc<ColdV2GenesisCurrentHead>,
        writer: &'owner PreparedColdV2WriterRegistration,
        receipt: &'owner PreparedColdV2CurrentWriterReceipt,
    },
    // This is the actual row1-repaired output with its original distinct scope and
    // registration. Neither an unowned Current2 capture nor a warm Saved is reconstructed.
    Repaired {
        scope: &'owner ColdV2InitializingPreparation,
        head: &'owner Arc<SavedColdV2GenesisRepairedHead>,
        writer: &'owner PreparedColdV2InitializingWriterRegistration,
    },
}

/// Borrow the original node-backed source AND actual scope/task/writer/resource owners.
/// Only the three real unexposed output wrappers construct this non-generic view.
/// There is no SDK, transport, RNG or Clock getter, no owned graph extraction and no raw
/// constructor. The original caller cannot drop/move any of these owners during the loan.
pub(in crate::store) struct GenesisCurrentOwnerRef<'owner> {
    pub(super) source: GenesisSourceDataRef<'owner>,
    pub(super) task: &'owner PreparedV2GenesisGroupNamespace,
    pub(super) role: GenesisCurrentOwnerRole<'owner>,
    // This is borrowed from the actual output, not a replacement metadata payment. The
    // independent settlement worker must acquire its OWN invoice before allocating.
    pub(super) wrapper_fee: &'owner CatalogReadReservation,
}

impl std::fmt::Debug for GenesisCurrentOwnerRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenesisCurrentOwnerRef")
            .finish_non_exhaustive()
    }
}

impl<'owner> GenesisCurrentOwnerRef<'owner> {
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'owner> {
        &self.source
    }
    pub(in crate::store) fn task(&self) -> &'owner PreparedV2GenesisGroupNamespace {
        self.task
    }
    pub(in crate::store) fn warm_scope(&self) -> Option<&'owner V2CurrentPreparation> {
        match self.role {
            GenesisCurrentOwnerRole::Warm { scope, .. } => Some(scope),
            GenesisCurrentOwnerRole::Cold { .. } | GenesisCurrentOwnerRole::Repaired { .. } => None,
        }
    }
    pub(in crate::store) fn cold_scope(&self) -> Option<&'owner ColdV2CurrentPreparation> {
        match self.role {
            GenesisCurrentOwnerRole::Cold { scope, .. } => Some(scope),
            GenesisCurrentOwnerRole::Warm { .. } | GenesisCurrentOwnerRole::Repaired { .. } => None,
        }
    }
    pub(in crate::store) fn warm_head(&self) -> Option<&'owner Arc<SavedV2GenesisHead>> {
        match self.role {
            GenesisCurrentOwnerRole::Warm { head, .. } => Some(head),
            GenesisCurrentOwnerRole::Cold { .. } | GenesisCurrentOwnerRole::Repaired { .. } => None,
        }
    }
    pub(in crate::store) fn cold_head(&self) -> Option<&'owner Arc<ColdV2GenesisCurrentHead>> {
        match self.role {
            GenesisCurrentOwnerRole::Cold { head, .. } => Some(head),
            GenesisCurrentOwnerRole::Warm { .. } | GenesisCurrentOwnerRole::Repaired { .. } => None,
        }
    }

    pub(in crate::store) fn repaired_scope(&self) -> Option<&'owner ColdV2InitializingPreparation> {
        match self.role {
            GenesisCurrentOwnerRole::Repaired { scope, .. } => Some(scope),
            GenesisCurrentOwnerRole::Warm { .. } | GenesisCurrentOwnerRole::Cold { .. } => None,
        }
    }

    pub(in crate::store) fn repaired_head(
        &self,
    ) -> Option<&'owner Arc<SavedColdV2GenesisRepairedHead>> {
        match self.role {
            GenesisCurrentOwnerRole::Repaired { head, .. } => Some(head),
            GenesisCurrentOwnerRole::Warm { .. } | GenesisCurrentOwnerRole::Cold { .. } => None,
        }
    }

    /// Clone only the existing independently paid writer observation, outside writers.
    /// No sender is retained or created. The original registration remains borrowed by
    /// this owner and is still required at every installed-generation proof.
    pub(in crate::store) fn observe_writer(&self) -> V2WriterObservation {
        match self.role {
            GenesisCurrentOwnerRole::Warm { writer, .. } => writer.observe_original(),
            GenesisCurrentOwnerRole::Cold { writer, .. } => writer.observe_original(),
            GenesisCurrentOwnerRole::Repaired { writer, .. } => writer.observe_original(),
        }
    }

    /// Cached original comparisons ONLY: safe before/subsequent to a held Runtime borrow.
    /// The observation must be the actual one retained before cleanup; matching source
    /// labels, a replacement scope owner or a new registration cannot substitute for it.
    /// Full source/ID3/filesystem and canonical installation checks remain mandatory.
    pub(in crate::store) fn require_original_negative(
        &self,
        store: &ServerStore,
        original_writer: &V2WriterObservation,
    ) -> Result<(), AppError> {
        self.source.require_original_negative(store)?;
        self.task.retains_original(store, &self.source)?;
        match self.role {
            GenesisCurrentOwnerRole::Warm {
                scope,
                head,
                writer,
            } => {
                scope.require_original_source_negative(store, &self.source, self.task)?;
                scope.current_binding()?;
                let actual = scope
                    .saved_head()
                    .ok_or_else(critical_namespaces::refused)?;
                if !Arc::ptr_eq(actual, head) {
                    return Err(critical_namespaces::refused());
                }
                head.retains_original(store, &self.source, self.task)?;
                writer.require_same_observation(head.writer_observation())?;
                writer.require_same_observation(original_writer)
            }
            GenesisCurrentOwnerRole::Cold {
                scope,
                head,
                writer,
                receipt,
                ..
            } => {
                scope.require_original_source_negative(store, &self.source, self.task)?;
                scope.current_binding()?;
                if !Arc::ptr_eq(scope.head(), head) {
                    return Err(critical_namespaces::refused());
                }
                head.retains_original(store, &self.source)?;
                let current = scope.current_writer_receipt()?;
                receipt.require_same_saved_current(current)?;
                writer.require_same_observation(current.writer_observation())?;
                writer.require_same_observation(original_writer)
            }
            GenesisCurrentOwnerRole::Repaired {
                scope,
                head,
                writer,
            } => {
                scope.require_original_source_negative(store, &self.source, self.task)?;
                scope.current_binding()?;
                let actual = scope
                    .saved_head()
                    .ok_or_else(critical_namespaces::refused)?;
                if !Arc::ptr_eq(actual, head) {
                    return Err(critical_namespaces::refused());
                }
                // Deliberately cached identities/negatives only. After unlink admission,
                // independent settlement proof replaces mandatory seed-name validation.
                head.retains_original(store, &self.source, self.task)?;
                writer.require_same_observation(head.writer_observation())?;
                writer.require_same_observation(original_writer)
            }
        }
    }

    /// Account validation can wait. Run it before the full source/ID3/named-object proof,
    /// never after a final positive physical edge. This acquires no replacement payment.
    pub(in crate::store) fn validate_resources(
        &self,
        store: &ServerStore,
        original_writer: &V2WriterObservation,
    ) -> Result<(), AppError> {
        self.require_original_negative(store, original_writer)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?
            .pending
            .pool;
        self.wrapper_fee.validate(pool, WRAPPER_FEE)?;
        self.require_original_negative(store, original_writer)
    }
}

impl<T, R> UnexposedV2GenesisCurrent<T, R> {
    /// Pure owning loan from this SAME actual node/resources. Even if a previous cleanup
    /// attempt has unlinked its seed, this cached association check performs no old
    /// mandatory-seed validation; the retained cleanup phase selects the full verifier.
    pub(in crate::store) fn settlement_owner_ref(
        &self,
    ) -> Result<GenesisCurrentOwnerRef<'_>, AppError> {
        let head = self
            .scope
            .saved_head()
            .ok_or_else(critical_namespaces::refused)?;
        self.activation.require_same_saved_head(head)?;
        Ok(GenesisCurrentOwnerRef {
            source: GenesisSourceDataRef {
                seed: self.node.borrow_genesis_seed_data(),
                resources: &self.resources,
            },
            task: &self.task,
            role: GenesisCurrentOwnerRole::Warm {
                scope: &self.scope,
                head,
                writer: &self.writer,
            },
            wrapper_fee: &self.fee,
        })
    }

    /// One explicit cleanup attempt for THIS original saved Current/node. All refusals
    /// retain the same node/T/R/resources/scope/writer and exact prepared cleanup phase.
    /// No new source execution, SDK graph, creation nonce or activation is issued.
    pub fn retry_seed_settlement(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        // Split actual owner fields from its mutable cleanup slot BEFORE the loan. There
        // is no Option hole, fallible graph move, RefCell permission adapter or copied node.
        let Self {
            node,
            resources,
            activation,
            task,
            scope,
            writer,
            fee,
            seed_settlement,
            ..
        } = self;
        let head = scope
            .saved_head()
            .ok_or_else(critical_namespaces::refused)?;
        activation.require_same_saved_head(head)?;
        let owner = GenesisCurrentOwnerRef {
            source: GenesisSourceDataRef {
                seed: node.borrow_genesis_seed_data(),
                resources,
            },
            task,
            role: GenesisCurrentOwnerRole::Warm {
                scope,
                head,
                writer,
            },
            wrapper_fee: fee,
        };
        retry_owned_settlement(&owner, seed_settlement, layout, rng)
    }
}

/// Current-family code only. The actual wrapper constructed the closed owner loan from its
/// OWN disjoint fields and keeps every actual resource owner through this finite attempt.
pub(super) fn retry_owned_settlement(
    owner: &GenesisCurrentOwnerRef<'_>,
    cleanup: &mut Option<PreparedGenesisSeedSettlement>,
    layout: &V2CurrentLayoutStore,
    rng: &mut impl CryptoRngCore,
) -> Result<(), AppError> {
    let store = layout.checked_store()?;
    if cleanup.is_none() {
        *cleanup = Some(PreparedGenesisSeedSettlement::prepare(store, owner)?);
    }
    let prepared = cleanup
        .as_mut()
        .expect("retained before any cleanup effect");
    prepared.require_same_owner(store, owner)?;
    let resources = prepared.resources();
    let buffers = store.reserve_v2_genesis_scope_buffers()?;
    let result = (|| {
        // A failed previous/unrelated scope write may have dirtied the SAME latest table.
        // This maintenance grants no cleanup; a NEW even-clean covering write follows it.
        store.resync_v2_current_scope_inventory(&buffers, rng)?;
        if let Some(saved) = prepared.saved_outcome() {
            // The actual unlink/covering barrier was already recorded by THIS owner.
            // A lost reply needs a full settled read, not another even-clean write/RNG
            // invocation. The preceding DATA maintenance still repairs unrelated debt.
            return Ok(saved.clone());
        }
        store.with_durable_genesis_current2_settlement(
            owner,
            prepared,
            &buffers,
            rng,
            |admission, original| {
                let guard = store.lock_genesis_seed_settlement_group(admission, &resources)?;
                original.settle(&guard)
            },
        )
    })();
    // A pool refund can wait/cancel. Do it BEFORE the last actual settled read/proof, on
    // both error and success; no Root/Runtime/pins/canonical guard survives this edge.
    drop(buffers);
    let saved = result?;
    store.with_settled_genesis_current2_read(owner, &saved, |admission| {
        let guard = store.lock_settled_genesis_current_group(admission, &resources)?;
        saved.validate_under_group_guard(&guard)
    })
}

/// Dispatch by actual admitted in-process state BEFORE any named-seed validator. A parent
/// barrier/final-proof error needs explicit retry; it is never a reason to invoke the old
/// mandatory-seed branch or to construct a replacement source/worker. None means no unlink
/// was admitted and the existing original full validator remains the appropriate lane.
pub(super) fn validate_after_cleanup(
    owner: &GenesisCurrentOwnerRef<'_>,
    cleanup: Option<&PreparedGenesisSeedSettlement>,
    layout: &V2CurrentLayoutStore,
) -> Option<Result<(), AppError>> {
    let prepared = cleanup.filter(|prepared| prepared.requires_independent_validation())?;
    Some((|| {
        let saved = prepared
            .saved_outcome()
            .ok_or_else(critical_namespaces::refused)?;
        let resources = saved.resources();
        let store = layout.checked_store()?;
        store.with_settled_genesis_current2_read(owner, saved, |admission| {
            let guard = store.lock_settled_genesis_current_group(admission, &resources)?;
            saved.validate_under_group_guard(&guard)
        })
    })())
}
