//! One mounted fresh-founder chain. Historical DATA and owning retries, not native authority.
use super::handoff::{bridge_layout_bytes, CompletedWarmFounderOrigin};
use super::*;
use crate::store::{
    GenesisSeedBudget, MemberSaveJob, MemberSaveNativeAdmission, PreparedV2GenesisCurrent,
    PreparedV2TextHost, PreparedV2TextRead, RetainedBootstrapGenesisSeed,
    RetainedBootstrapSourceData, V2CurrentLayoutStore, V2MemberSaveOutcome, V2MemberSaveRefusal,
    V2MemberSignRequest, V2TextHostPolicy,
};
use catcoms_rt::RequestCancellation;
use catcoms_sync::owner_v2::V2GenesisRuntimeParts;

/// Bounded historical binding from the genuine producer. DATA only: no decoder
/// or owner constructor exists. Its eventual native row codec is separate work.
pub struct FreshV2TextFounderBinding {
    pub(in crate::store) vault: [u8; 32],
    pub(in crate::store) layout: [u8; 32],
    pub(in crate::store) group: Vec<u8>,
    pub(in crate::store) signer: [u8; 32],
    pub(in crate::store) source: [u8; 32],
    pub(in crate::store) profile: [u8; 32],
    pub(in crate::store) pin: [u8; 32],
}
impl FreshV2TextFounderBinding {
    pub fn binding_version_data(&self) -> u16 {
        1
    }
    pub fn vault_layout_data(&self) -> (&[u8; 32], &[u8; 32]) {
        (&self.vault, &self.layout)
    }
    pub fn group_data(&self) -> &[u8] {
        &self.group
    }
    pub fn signer_data(&self) -> &[u8; 32] {
        &self.signer
    }
    pub fn source_data(&self) -> &[u8; 32] {
        &self.source
    }
    pub fn profile_data(&self) -> &[u8; 32] {
        &self.profile
    }
    pub fn pin_data(&self) -> &[u8; 32] {
        &self.pin
    }
}
impl std::fmt::Debug for FreshV2TextFounderBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FreshV2TextFounderBinding { .. }")
    }
}

/// Trusted local limits, fixed before original creation. No caller-supplied
/// receipt, source, node, selection or authorization flag enters this driver.
#[derive(Clone, Copy, Debug)]
pub struct FreshV2TextFounderPolicy {
    founder: ArchiveFounderPolicy,
    seed: GenesisSeedBudget,
    host: V2TextHostPolicy,
}
impl FreshV2TextFounderPolicy {
    /// Reject incompatible source/receiver grammars before a creation grant
    /// can be spent. This performs no IO, allocation, signing or SDK work.
    pub fn new(
        founder: ArchiveFounderPolicy,
        seed: GenesisSeedBudget,
        host: V2TextHostPolicy,
    ) -> Result<Self, AppError> {
        let policy = Self {
            founder,
            seed,
            host,
        };
        policy.validate()?;
        Ok(policy)
    }
    fn validate(&self) -> Result<(), AppError> {
        if !self.seed.supports_founder_limits(self.founder.limits) {
            return Err(refused());
        }
        Ok(())
    }
}

/// An existing invocation returns the caller's untouched inputs. It is never
/// adopted as a fresh runtime, even if its stored labels look identical.
// Existing owns arbitrary caller T/R/device allocations. Returning those inline
// must not allocate another Box after an admission refusal.
#[allow(clippy::large_enum_variant)]
pub enum FreshV2TextFounderBegin<T: MeshTransport, R: CryptoRngCore> {
    Original(PreparedFreshV2TextFounder<T, R>),
    Existing {
        status: ArchiveFounderExisting,
        inputs: ArchiveFounderInputs<T, R>,
    },
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for FreshV2TextFounderBegin<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FreshV2TextFounderBegin { .. }")
    }
}

/// Actual whole Host and creator custody, with bounded intermediate owning slots.
/// No Host/Server/SDK extractor exists. The public save methods retain this exact
/// origin for the entire operation; native still must supply checked key/save
/// admission and its actual row/session lifecycle. This is fresh-only and disabled.
pub struct PreparedFreshV2TextFounder<T: MeshTransport, R: CryptoRngCore> {
    origin: FounderOrigin,
    // Resource consumers drain before the creator and this driver's extra fee.
    host: Option<Box<PreparedV2TextHost<T, R>>>,
    current: Option<Box<PreparedV2GenesisCurrent<T, R>>>,
    seed: Option<RetainedBootstrapGenesisSeed>,
    read: Option<RetainedBootstrapSourceData>,
    parts: Option<V2GenesisRuntimeParts<T, R>>,
    warm_origin: Option<CompletedWarmFounderOrigin>,
    binding: Option<FreshV2TextFounderBinding>,
    seed_settled: bool,
    ready: bool,
    #[cfg(test)]
    pause_after_current: bool,
    policy: FreshV2TextFounderPolicy,
    creation: Option<Box<ArchiveFounderCreation<T, R>>>,
    planning: Option<Box<ArchiveFounderPlanning<T, R>>>,
    // Retained even after its old SDK drains. Dropping it revokes the original
    // creator; moving just the Host out would silently lose that lifetime.
    source: Option<Box<ArchiveFounderSourcePreparation<T, R>>>,
    fee: CatalogReadReservation,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for PreparedFreshV2TextFounder<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedFreshV2TextFounder")
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Drop for PreparedFreshV2TextFounder<T, R> {
    fn drop(&mut self) {
        // Synchronously retire original creation before any SDK/large payload
        // destructor. Whole Host's Drop separately closes its actual domains.
        if let Some(source) = &self.source {
            source.close();
        }
        if let Some(planning) = &self.planning {
            planning.close();
        }
        if let Some(creation) = &self.creation {
            creation.close();
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedFreshV2TextFounder<T, R> {
    /// Additional driver/Box/refusal storage only. Existing creator, source,
    /// source-read, restored SDK and Current fees remain independently live.
    fn invoice() -> Result<usize, AppError> {
        let layouts = [
            std::mem::size_of::<Self>(),
            std::mem::size_of::<ArchiveFounderCreation<T, R>>(),
            std::mem::size_of::<ArchiveFounderPlanning<T, R>>(),
            std::mem::size_of::<ArchiveFounderSourcePreparation<T, R>>(),
            std::mem::size_of::<PreparedV2GenesisCurrent<T, R>>(),
            std::mem::size_of::<PreparedV2TextHost<T, R>>(),
            V2GenesisRuntimeParts::<T, R>::allocation_bytes(),
            V2GenesisRuntimeParts::<T, R>::warm_refusal_allocation_bytes(),
            bridge_layout_bytes(),
            16 * 1024, // bounded metadata/return envelopes and transient group DATA
        ];
        layouts.into_iter().try_fold(0usize, |sum, bytes| {
            sum.checked_add(bytes).ok_or_else(refused)
        })
    }

    /// One finite original attempt. Every successful stage is cached before
    /// any late check. A failure retains the same input, nonce, output and debt;
    /// no loop grants another persistent source execution allowance.
    pub fn retry(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.retry_with_access(layout, rng, FounderAccess::Library)
    }
    pub(in crate::store) fn retry_with_access(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
        mut access: FounderAccess<'_>,
    ) -> Result<(), AppError> {
        access.require(&self.origin)?;
        let store = layout.checked_store()?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.fee.validate(pool, Self::invoice()?)?;
        if let Some(creation) = self.creation.as_mut() {
            let progress = match &mut access {
                FounderAccess::Library => creation.advance(store, rng)?,
                FounderAccess::Native(keys) => {
                    creation.advance_native(store, rng, &mut keys.sdk_entry())?
                }
            };
            if progress != ArchiveFounderProgress::GroupCreatedUnexposed {
                return Err(refused());
            }
            let creation = *self.creation.take().ok_or_else(refused)?;
            match creation.into_ram_plan_with_access(store, access.reborrow()) {
                Ok(planning) => self.planning = Some(Box::new(planning)),
                Err(returned) => {
                    let (creation, error) = returned.into_parts();
                    self.creation = Some(Box::new(creation));
                    return Err(error);
                }
            }
        }
        if let Some(planning) = self.planning.as_mut() {
            planning.save_founding_intent_with_access(store, rng, access.reborrow())?;
            let planning = *self.planning.take().ok_or_else(refused)?;
            match planning.into_source_preparation_with_access(store, access.reborrow()) {
                Ok(source) => self.source = Some(Box::new(source)),
                Err(returned) => {
                    let (planning, error) = returned.into_parts();
                    self.planning = Some(Box::new(planning));
                    return Err(error);
                }
            }
        }
        let source = self.source.as_mut().ok_or_else(refused)?;
        source.original.creator.original.validate_original(store)?;
        if self.host.is_none() && self.current.is_none() {
            if self.parts.is_none() {
                source.save_source_terminal_with_access(store, rng, access.reborrow())?;
                if self.read.is_none() {
                    let group = source.original.creator.group_data()?;
                    let signer = source.original.creator.original.data().signer;
                    self.read = Some(layout.read_saved_bootstrap_source_data(group, signer)?);
                }
                let (parts, origin) = source.take_completed_runtime(
                    store,
                    self.read.as_ref().ok_or_else(refused)?,
                    &self.fee,
                )?;
                self.parts = Some(parts);
                self.warm_origin = Some(origin);
                // The parts are retained before this late original check.
                source.original.creator.original.validate_original(store)?;
            }
            // The original SDK has drained in take_completed_runtime before
            // this independent paid restore. A refused restore returns the exact
            // original read, never another physical read or source-write grant.
            if self.seed.is_none() {
                let result = match &mut access {
                    FounderAccess::Library => {
                        let read = self.read.take().ok_or_else(refused)?;
                        store.prepare_genesis_seed(read, self.policy.seed)
                    }
                    FounderAccess::Native(keys) => {
                        let completed = self.warm_origin.as_ref().ok_or_else(refused)?;
                        let evidence = completed.warm_sdk_evidence(
                            &self.origin,
                            self.read.as_ref().ok_or_else(refused)?,
                        )?;
                        let read = self.read.take().ok_or_else(refused)?;
                        store.prepare_warm_native_genesis_seed(
                            read,
                            self.policy.seed,
                            &evidence,
                            keys,
                        )
                    }
                };
                match result {
                    // Cache actual graph+source+all fees BEFORE any late
                    // original/UI check. A refusal retains this same seed.
                    Ok(seed) => self.seed = Some(seed),
                    Err(returned) => {
                        self.read = Some(returned.into_parts().0);
                        return Err(refused());
                    }
                }
            }
            self.seed
                .as_ref()
                .ok_or_else(refused)?
                .validate_original(store)?;
            source.original.creator.original.validate_original(store)?;
            access.require(&self.origin)?;
            let seed = self.seed.take().ok_or_else(refused)?;
            let parts = self.parts.take().ok_or_else(refused)?;
            let origin = self.warm_origin.take().ok_or_else(refused)?;
            self.current = Some(Box::new(PreparedV2GenesisCurrent::from_original_runtime(
                seed, parts, origin,
            )));
        }
        if let Some(current) = self.current.as_mut() {
            current.retry(layout, rng)?;
            #[cfg(test)]
            if self.pause_after_current {
                return Err(refused());
            }
            let current = current.take_current(layout)?.ok_or_else(refused)?;
            self.host = Some(Box::new(PreparedV2TextHost::from_read_with_origin(
                PreparedV2TextRead::from_warm(current),
                self.policy.host,
                self.origin.clone(),
            )));
            drop(self.current.take());
        }
        let host = self.host.as_mut().ok_or_else(refused)?;
        if !self.seed_settled {
            host.retry_original_seed_settlement_with_access(layout, rng, access.reborrow())?;
            self.seed_settled = true;
        }
        // Cache DATA before the final selected-head proof; this cannot install
        // or reopen a native backend and never substitutes for that proof.
        if self.binding.is_none() {
            self.binding = Some(host.fresh_founder_binding_data(layout)?);
        }
        source.original.creator.original.validate_original(store)?;
        host.retry_empty_promotion_with_access(layout, rng, access.reborrow())?;
        if source.original.creator.original.observer().is_cancelled() {
            return Err(refused());
        }
        access.require(&self.origin)?;
        self.ready = true;
        Ok(())
    }

    /// Advisory DATA only. Every later signer/save repeats its own actual
    /// Current/source/native checks; this bit grants no authority.
    pub fn is_ready_data(&self) -> bool {
        self.ready
            && self
                .source
                .as_ref()
                .is_some_and(|source| !source.original.creator.original.observer().is_cancelled())
    }

    /// Borrow historical DATA only after successful completion. Native must
    /// still authenticate its original row/session before disclosure or use.
    pub fn binding_data(&self) -> Option<&FreshV2TextFounderBinding> {
        self.is_ready_data().then_some(())?;
        self.binding.as_ref()
    }

    /// Keep the genuine creator alive while using the original Host's existing
    /// two-phase member-save driver. This supplies no native key admission.
    pub fn prepare_member_save(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2MemberSignRequest<'_>,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        self.prepare_member_save_with_access(layout, request, negative, rng, FounderAccess::Library)
    }
    pub(in crate::store) fn prepare_member_save_with_access(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2MemberSignRequest<'_>,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        access
            .require(&self.origin)
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        if !self.is_ready_data() {
            return Err(V2MemberSaveRefusal::Unavailable);
        }
        self.host
            .as_mut()
            .ok_or(V2MemberSaveRefusal::Unavailable)?
            .prepare_member_save_with_access(layout, request, negative, rng, access)
    }

    /// No IO/wait is added at the existing finite final edge. The actual
    /// native factory must still authenticate and retain its supplied custody.
    pub fn admit_member_save(
        &mut self,
        layout: &V2CurrentLayoutStore,
        token: [u8; 32],
        native: MemberSaveNativeAdmission,
    ) -> Result<MemberSaveJob, (MemberSaveNativeAdmission, V2MemberSaveRefusal)> {
        self.admit_member_save_with_access(layout, token, native, FounderAccess::Library)
    }
    pub(in crate::store) fn admit_member_save_with_access(
        &mut self,
        layout: &V2CurrentLayoutStore,
        token: [u8; 32],
        native: MemberSaveNativeAdmission,
        access: FounderAccess<'_>,
    ) -> Result<MemberSaveJob, (MemberSaveNativeAdmission, V2MemberSaveRefusal)> {
        if access.require(&self.origin).is_err() {
            return Err((native, V2MemberSaveRefusal::Unavailable));
        }
        if !self.is_ready_data() {
            return Err((native, V2MemberSaveRefusal::Unavailable));
        }
        match self.host.as_mut() {
            Some(host) => host.admit_member_save_with_access(layout, token, native, access),
            None => Err((native, V2MemberSaveRefusal::Unavailable)),
        }
    }

    /// Close actual Host/creator admission and drain already admitted finite
    /// work. A Busy result still leaves the original closed for the next retry.
    pub fn close_and_drain(&mut self) -> Result<(), AppError> {
        self.ready = false;
        if let Some(source) = &self.source {
            source.close();
        }
        if let Some(planning) = &self.planning {
            planning.close();
        }
        if let Some(creation) = &self.creation {
            creation.close();
        }
        match self.host.as_mut() {
            Some(host) => host.cancel_and_drain_admitted_promotion(),
            None => Ok(()),
        }
    }
}

impl V2CurrentLayoutStore {
    /// Begin on this ONE already mounted vault with all required concrete
    /// handlers. Existing invocation and early refusal preserve untouched inputs.
    /// It creates no native row and cannot enable a protocol mode.
    #[allow(clippy::result_large_err)] // preserve the caller's original owned inputs inline
    pub fn prepare_fresh_v2_text_founder<T: MeshTransport, R: CryptoRngCore>(
        &self,
        invocation: [u8; 32],
        name: &str,
        policy: FreshV2TextFounderPolicy,
        inputs: ArchiveFounderInputs<T, R>,
    ) -> Result<FreshV2TextFounderBegin<T, R>, ReturnedArchiveFounderInputs<T, R>> {
        self.prepare_fresh_v2_text_founder_with_origin(
            invocation,
            name,
            policy,
            inputs,
            FounderOrigin::Library,
        )
    }
    #[allow(clippy::result_large_err)] // return the caller's owned inputs without another allocation
    pub(in crate::store) fn prepare_fresh_v2_text_founder_with_origin<
        T: MeshTransport,
        R: CryptoRngCore,
    >(
        &self,
        invocation: [u8; 32],
        name: &str,
        policy: FreshV2TextFounderPolicy,
        inputs: ArchiveFounderInputs<T, R>,
        origin: FounderOrigin,
    ) -> Result<FreshV2TextFounderBegin<T, R>, ReturnedArchiveFounderInputs<T, R>> {
        let prepared = (|| {
            policy.validate()?;
            let store = self.checked_store()?;
            let configured = store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?;
            if configured.pending.execution.is_none() || store.member_outbox_policy().is_none() {
                return Err(refused());
            }
            store.validate_mounted_member_outbox_layout()?;
            configured
                .pending
                .pool
                .try_reserve(PreparedFreshV2TextFounder::<T, R>::invoice()?)
        })();
        let fee = match prepared {
            Ok(fee) => fee,
            Err(error) => return Err(ReturnedArchiveFounderInputs { inputs, error }),
        };
        let store = match self.checked_store() {
            Ok(store) => store,
            Err(error) => return Err(ReturnedArchiveFounderInputs { inputs, error }),
        };
        match store.prepare_archive_founder_with_origin(
            invocation,
            name,
            policy.founder,
            inputs,
            origin.clone(),
        )? {
            ArchiveFounderBegin::Original(creation) => Ok(FreshV2TextFounderBegin::Original(
                PreparedFreshV2TextFounder {
                    origin,
                    host: None,
                    current: None,
                    read: None,
                    seed: None,
                    parts: None,
                    warm_origin: None,
                    binding: None,
                    seed_settled: false,
                    ready: false,
                    #[cfg(test)]
                    pause_after_current: false,
                    policy,
                    creation: Some(Box::new(creation)),
                    planning: None,
                    source: None,
                    fee,
                },
            )),
            ArchiveFounderBegin::Existing { status, inputs } => {
                Ok(FreshV2TextFounderBegin::Existing { status, inputs })
            }
        }
    }
}

#[cfg(test)]
#[path = "founder_warm_tests.rs"]
mod tests;
