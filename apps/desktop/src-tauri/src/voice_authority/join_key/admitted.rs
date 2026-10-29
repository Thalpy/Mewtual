//! Native-only admitted key custody. The Existing generation/pilot grammar stays
//! in the parent module; these typed entries require the original admitted call.
use super::*;
use catcoms_sync::voice_v2::admitted_presence::AdmittedPresenceReceipt;
use catcoms_sync::voice_v2::call_activation::RootResidenceReturned;

/// Public metadata for ONE actual admitted key publication. This is deliberately
/// a different type from SelectedHandoffOwner: it cannot enter an Existing pilot
/// Registry, adopt an old Session, select a role, or supply Ready/RTC authority.
/// The private body and SAME last payer also keep its final weak-grant allocation
/// sponsored when the canonical generation has already retired.
#[derive(Clone)]
pub(crate) struct AdmittedPublishedKey {
    selected: Arc<AdmittedKeySelection>,
    _funding: Arc<SelectedFunding>,
}
struct SelectedFunding {
    _credit: Credit,
}
#[allow(dead_code)] // Original admitted bootstrap metadata; op31 consumer is a later gate.
struct AdmittedKeySelection {
    id: String,
    binding: Binding,
    join: Nonce,
    lease: CallLease,
    grant: std::sync::Weak<CallGrant>,
    executor: [u8; 32],
    point: [u8; 65],
    resources: Arc<Resources>,
}
#[allow(dead_code)] // This type cannot enter the Existing bootstrap; admitted op31 is not wired.
impl AdmittedPublishedKey {
    pub(crate) fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.selected, &other.selected)
    }
    pub(crate) fn id(&self) -> &str {
        &self.selected.id
    }
    pub(crate) fn binding(&self) -> &Binding {
        &self.selected.binding
    }
    pub(crate) fn join(&self) -> Nonce {
        self.selected.join
    }
    pub(crate) fn lease(&self) -> &CallLease {
        &self.selected.lease
    }
    pub(crate) fn executor(&self) -> [u8; 32] {
        self.selected.executor
    }
    pub(crate) fn point(&self) -> [u8; 65] {
        self.selected.point
    }
    pub(crate) fn resources(&self) -> Arc<Resources> {
        self.selected.resources.clone()
    }
    /// Upgrade only outside Coordinator. This is still historical metadata, not
    /// proof of current selection; the admitted caller repeats its original Root
    /// activation/profile/generation checks at every positive effect.
    pub(crate) fn grant(&self) -> Result<Arc<CallGrant>, catcoms_sync::voice_v2::VoiceError> {
        self.selected
            .grant
            .upgrade()
            .filter(|grant| grant.lease().is_live() && grant.lease().is_admitted())
            .ok_or(catcoms_sync::voice_v2::VoiceError::Unauthorized)
    }
}

/// Original key-publication custody, retained by the separately paid native
/// canonical publication task through exact Actor outcome AND argument return.
/// No invoke waiter owns it, and an uncertain reply cannot finish or remint it.
pub(crate) struct AdmittedPublicationWork {
    owner: Owner,
    original: ActivationObserver,
    executor: [u8; 32],
    permit: String,
    point: [u8; 65],
    finished: bool,
    // LAST: aliases and the actual inline Work precede the SAME generation fee.
    _funding: Arc<GenerationFunding>,
}
impl AdmittedPublicationWork {
    pub(crate) fn owner_id(&self) -> &str {
        &self.owner.id
    }
    pub(crate) fn executor(&self) -> [u8; 32] {
        self.executor
    }
    pub(crate) fn permit(&self) -> &str {
        &self.permit
    }
    pub(crate) fn grant(&self) -> Arc<CallGrant> {
        self.owner._grant.clone()
    }
    pub(crate) fn original(&self) -> &ActivationObserver {
        &self.original
    }
    pub(crate) fn join(&self) -> Nonce {
        self.owner.join
    }
    pub(crate) fn point(&self) -> [u8; 65] {
        self.point
    }
}
// Keeping Work inline avoids an extra allocation/return obligation. Its native
// publication task has its own measured fee; generation metadata pays these aliases.
#[allow(clippy::large_enum_variant)]
pub(crate) enum AdmittedPublicationPlan {
    Accepted(JoinKeyPublication),
    Start(AdmittedPublicationWork),
}

/// A receiver borrowed from the ORIGINAL key-generation watch, paired with its
/// SAME metadata payer. It may outlive both Coordinator and the extracted row.
/// No naked Receiver escapes and silently leaves its shared allocation unpaid.
#[must_use = "keep the paired original watch owner until settlement returns"]
pub(crate) struct AdmittedKeyDrain {
    receiver: tokio::sync::watch::Receiver<bool>,
    _funding: Arc<GenerationFunding>,
}
impl AdmittedKeyDrain {
    /// Await only on the sanctioned native/Tauri Tokio task. This closed private
    /// integration never accepts a peer/JS-supplied Rust executor or RawWaker.
    /// Tokio watch's fanout is not promised resilient to an arbitrary Rust waker
    /// panicking; the reentry fixture below proves outside-lock notification only.
    pub(crate) async fn wait(mut self) -> Result<(), String> {
        while !*self.receiver.borrow_and_update() {
            self.receiver
                .changed()
                .await
                .map_err(|_| "voice key settlement owner stopped".to_string())?;
        }
        Ok(())
    }
}

/// Last Sender/Receiver and selected-owner destructors can wake arbitrary Rust
/// tasks. This ticket MUST leave Coordinator before either notification or drop.
#[must_use = "move this ticket outside Coordinator/UI guards before finishing or dropping it"]
pub(crate) struct AdmittedKeySettlement {
    drained: Option<AdmittedKeyDrain>,
    notify: Option<tokio::sync::watch::Sender<bool>>,
    _retired: Option<AdmittedKeyRetirement>,
    _engine: Option<Credit>,
    _funding: Option<Arc<GenerationFunding>>,
}
impl AdmittedKeySettlement {
    pub(crate) fn finish_outside_guard(mut self) -> Option<AdmittedKeyDrain> {
        if let Some(notify) = self.notify.take() {
            notify.send_replace(true);
            drop(notify);
        }
        let drained = self.drained.take();
        drop(self); // actual retired row/engine destruction is outside Coordinator
        drained
    }
}
impl Drop for AdmittedKeySettlement {
    fn drop(&mut self) {
        // An abandoned trusted caller still owes the original notification. The
        // ticket's ownership contract requires Drop outside Coordinator/UI too;
        // last Sender/Receiver destruction would otherwise be a callout anyway.
        if let Some(notify) = self.notify.take() {
            notify.send_replace(true);
        }
    }
}

/// A KNOWN terminal Root result, staged only after the canonical native publisher
/// has joined its original Actor Work. Outer method errors are pre-mutation
/// mismatches; this owned completion always retains the terminal reply plus any
/// deferred original-watch notification until the caller has released guards.
#[must_use = "move this terminal ticket outside Coordinator/UI guards before finishing or dropping it"]
pub(crate) struct AdmittedKeyCompletion {
    result: Option<Result<JoinKeyPublication, String>>,
    notify: Option<tokio::sync::watch::Sender<bool>>,
    _retired: Option<AdmittedKeyRetirement>,
    _engine: Option<Credit>,
    _funding: Arc<GenerationFunding>,
}
impl AdmittedKeyCompletion {
    pub(crate) fn finish_outside_guard(mut self) -> Result<JoinKeyPublication, String> {
        if let Some(notify) = self.notify.take() {
            notify.send_replace(true);
            drop(notify);
        }
        self.result.take().expect("one known admitted completion")
    }
}
impl Drop for AdmittedKeyCompletion {
    fn drop(&mut self) {
        // Lost local replies are not permission to strand a settled watch. All
        // actual owned fields and the SAME payer still survive this callback.
        if let Some(notify) = self.notify.take() {
            notify.send_replace(true);
        }
    }
}

/// Exact closed row returned for destruction AFTER Coordinator unlock. The
/// process index owner independently retains the shared HashMap backing, even
/// when it becomes empty; this row fee pays only the original row/watch graph.
#[must_use = "drop the actual retired row outside Coordinator/UI guards"]
pub(crate) struct AdmittedKeyRetirement {
    _row: Generation,
    _funding: Arc<GenerationFunding>,
}

/// Exact negative proof for an original closed SDK generation which NEVER
/// froze a point or admitted publication work. Only Coordinator constructs this
/// observation; absence of a generation/task alone is not such a certificate.
/// It has no positive factory or conversion to key admission, and carries the
/// SAME original metadata payment if its native reply outlives the row.
pub(crate) struct AdmittedPublicationNotStarted {
    owner: String,
    executor: [u8; 32],
    permit: String,
    join: Nonce,
    _funding: Arc<GenerationFunding>,
}
impl AdmittedPublicationNotStarted {
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }
    pub(crate) fn executor(&self) -> [u8; 32] {
        self.executor
    }
    pub(crate) fn permit(&self) -> &str {
        &self.permit
    }
    pub(crate) fn join(&self) -> Nonce {
        self.join
    }
}

enum AdmittedKeyTerminal {
    Published,
    Refused(catcoms_sync::voice_v2::VoiceError),
    // Actual original Root residence returned after closure, but no exact
    // publication outcome was recovered. Never relabel this as RootRejected.
    ClosedUnknown,
}

fn admitted_join_index_layout_bytes() -> usize {
    #[repr(C, align(2))]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    // The bounded32 entries use at most64 buckets at the pinned std/hashbrown
    // 7/8 load factor. Growth from32 to64 holds BOTH backing allocations until
    // rehash returns. Account every inline key/value slot, all control bytes,
    // mirrored SIMD groups and alignment/tail padding in that actual peak.
    // Key/permit heap buffers remain with their individual row metadata fee.
    fn table_bytes(buckets: usize) -> Option<usize> {
        let entry = std::alloc::Layout::new::<(String, Generation)>();
        let alignment = entry.align().max(16);
        let data = entry.size().checked_mul(buckets)?;
        let control = data
            .checked_add(alignment - 1)
            .map(|n| n & !(alignment - 1))?;
        let bytes = control.checked_add(buckets + 16)?;
        let layout = std::alloc::Layout::from_size_align(bytes, alignment).ok()?;
        Some(layout.pad_to_align().size())
    }
    let Some(backing) = table_bytes(64)
        .zip(table_bytes(32))
        .and_then(|(new, old)| new.checked_add(old))
    else {
        return usize::MAX;
    };
    backing
        + std::mem::size_of::<HashMap<String, Generation>>()
        + std::mem::size_of::<ArcAllocation<JoinIndexFunding>>()
}

pub(super) fn admitted_generation_layout_bytes() -> usize {
    #[repr(C, align(2))]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    std::mem::size_of::<Generation>()
        + std::mem::size_of::<ArcAllocation<GenerationFunding>>()
        + generation_watch_layout_bytes()
        + 64 + 64 // maximum native permit and table-key String buffers
        + std::mem::size_of::<AdmittedKeyDrain>()
        + 2 * std::mem::size_of::<tokio::sync::watch::Sender<bool>>()
        + 4 * std::mem::size_of::<Arc<GenerationFunding>>()
        // A completed public reply must remain recoverable without reserving a
        // new native publication task. Its five bounded64-byte String buffers
        // and actual DTO header overlap the retained row, including under full
        // ordinary-pool pressure. Serialized IPC copies remain caller-owned.
        + (std::mem::size_of::<JoinKeyPublication>() + 5 * 64).max(
            // The private proof remains alive while its public identity is
            // constructed. Count BOTH original owner/permit heaps and the
            // four output heaps, not the larger side of that overlap alone.
            std::mem::size_of::<AdmittedPublicationNotStarted>() + 2 * 64
                + std::mem::size_of::<crate::voice_admitted_key::PublicationReply>() + 4 * 64,
        )
    // The native canonical publication fee separately measures its actual Work,
    // completion, return frames and reply backings; their SAME funding aliases
    // are counted here. Shared table capacity has the distinct process payment.
}

impl Coordinator {
    pub(super) fn ensure_join_index(&mut self) -> Result<(), String> {
        if self.join_index_funding.is_some() {
            return Ok(());
        }
        if admitted_join_index_layout_bytes() > JOIN_INDEX_BYTES {
            return Err("voice key index capacity reached".into());
        }
        let credit = self
            .resources
            .charge_process_retained(JOIN_INDEX_BYTES)
            .map_err(|error| error.to_string())?;
        // Payment precedes the original plain owner allocation and first table
        // insertion of EITHER profile. No synthetic group or fresh independent
        // Resources is used. The process fee remains through actual map return.
        self.join_index_funding = Some(Arc::new(JoinIndexFunding { _credit: credit }));
        Ok(())
    }
}

impl Generation {
    fn admitted_matches_work(&self, work: &AdmittedPublicationWork) -> bool {
        self.profile == GenerationProfile::Admitted
            && self
                .admitted_original
                .as_ref()
                .is_some_and(|original| original.is_same(&work.original))
            && self.executor == work.executor
            && self.permit == work.permit
            && self.join == work.owner.join
            && self.grant.ptr_eq(&Arc::downgrade(&work.owner._grant))
            && self.point == Some(work.point)
    }

    pub(in crate::voice_authority) fn blocks_admitted_activation_forget(
        &self,
        original: &ActivationObserver,
    ) -> bool {
        self.profile == GenerationProfile::Admitted
            && self
                .admitted_original
                .as_ref()
                .is_some_and(|held| held.is_same(original))
            && (self.engine.is_some() || self.publishing || self.admitted_task.is_some())
    }
}

impl Coordinator {
    /// Pure final native-bookkeeping check. The adapter still owns its exact UI
    /// commit guard and repeats the atomic UI-generation check before this call.
    /// This helper neither creates an activation nor upgrades an Existing owner.
    fn admitted_key_owner_ref(
        &self,
        handle: &crate::voice_call_activation::Handle,
    ) -> Result<&Owner, String> {
        // Native bookkeeping must pay the SAME original Root pool, even when a
        // trusted integration accidentally retains a genuine foreign Handle.
        // Scope labels alone cannot authorize another process/group ledger.
        if !handle.original().uses_resources(&self.resources)
            || !self.activations.iter().any(|held| held.same(handle))
            || !handle.active()
        {
            return Err("admitted voice owner changed".into());
        }
        self.controller_parent(handle.controller(), handle.binding().generation)?;
        self.owner
            .as_ref()
            .filter(|owner| {
                owner.id == handle.id()
                    && owner.binding == *handle.binding()
                    && owner.lease.is_live()
                    && owner.lease.is_admitted()
                    && owner.lease.is_same(&owner._grant.lease())
                    && handle.original().matches_grant(&owner._grant)
            })
            .ok_or_else(|| "admitted voice owner changed".into())
    }

    /// Work producers need an owned snapshot through their actual Root turn.
    /// Pure completed-history readers borrow the same checked original instead,
    /// avoiding duplicate owner/binding String buffers beside their reply peak.
    fn admitted_key_owner(
        &self,
        handle: &crate::voice_call_activation::Handle,
    ) -> Result<Owner, String> {
        self.admitted_key_owner_ref(handle).cloned()
    }

    /// Native-authorized browser computation for one exact admitted activation.
    /// No private key crosses IPC. Lost replies recover this SAME permit/join;
    /// an unrelated executor receives no original public generation identity.
    pub(crate) fn admit_admitted_join_key(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
        executor: [u8; 32],
    ) -> Result<JoinKeyAdmission, String> {
        self.admitted_key_owner_ref(handle)?;
        if handle.require_key_only_purpose().is_err() {
            return Ok(JoinKeyAdmission {
                owner: handle.id().into(),
                executor: hex::encode(executor),
                permit: String::new(),
                join: String::new(),
                kind: "refused",
            });
        }
        self.admit_admitted_join_key_for_purpose(handle, executor, false)
    }

    /// Fixed physical-purpose producer, called only by its separate native
    /// command. Purpose and exact history are bound before SDK permission; the
    /// same generation cannot subsequently enter the key-only factory.
    pub(crate) fn admit_admitted_physical_join_key(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
        executor: [u8; 32],
    ) -> Result<JoinKeyAdmission, String> {
        self.admitted_key_owner_ref(handle)?;
        if handle.require_physical_purpose().is_err() {
            return Ok(JoinKeyAdmission {
                owner: handle.id().into(),
                executor: hex::encode(executor),
                permit: String::new(),
                join: String::new(),
                kind: "refused",
            });
        }
        self.admit_admitted_join_key_for_purpose(handle, executor, true)
    }

    // `physical` is a closed native choice from the two fixed entry points,
    // never a renderer flag. All actual authority remains the issuer/Handle.
    fn admit_admitted_join_key_for_purpose(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
        executor: [u8; 32],
        physical: bool,
    ) -> Result<JoinKeyAdmission, String> {
        let owner = self.admitted_key_owner(handle)?;
        if !self.join_generations.contains_key(&owner.id) {
            let prepaid = if physical {
                Some(
                    crate::voice_call_activation::physical::PhysicalHistory::prepay(handle)
                        .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };
            let mut generation = self.generation_row(&owner, executor, true)?;
            generation.admitted_original = Some(handle.original().clone());
            if let Some(prepaid) = prepaid {
                let history = prepaid.bind(handle, executor, generation.join, &generation.permit);
                handle
                    .install_physical_history(&history)
                    .map_err(|e| e.to_string())?;
            } else {
                handle
                    .select_key_only_purpose()
                    .map_err(|e| e.to_string())?;
            }
            self.join_generations.insert(owner.id.clone(), generation);
        }
        let generation = &self.join_generations[&owner.id];
        if generation.profile != GenerationProfile::Admitted
            || generation
                .admitted_original
                .as_ref()
                .is_none_or(|original| !original.is_same(handle.original()))
            || !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
        {
            return Err("voice key belongs to another protocol or generation".into());
        }
        if generation.executor != executor {
            return Ok(JoinKeyAdmission {
                owner: owner.id,
                executor: hex::encode(executor),
                permit: String::new(),
                join: String::new(),
                kind: "refused",
            });
        }
        if physical
            && handle.physical_history(executor).is_none_or(|history| {
                !history.matches(handle, executor, generation.join, &generation.permit)
            })
        {
            return Err("physical voice intent changed".into());
        }
        Ok(JoinKeyAdmission {
            owner: owner.id,
            executor: hex::encode(executor),
            permit: generation.permit.clone(),
            join: hex::encode(generation.join),
            kind: if generation.settlement_requested {
                "closed"
            } else {
                "admitted"
            },
        })
    }

    /// Read exact completed history BEFORE reserving a new publication task/RPC.
    /// A lost accepted reply needs only this bounded public projection, even when
    /// unrelated work fills the ordinary pool. This does not renew the call or
    /// grant publication/SDK authority: the original admitted activation must
    /// still be current, and every frozen generation field must match.
    ///
    /// No row, permit, task, source work or credit is created. The caller pays its
    /// bounded command/reply frame and must repeat this check under its original
    /// UI guard and final Coordinator lock before choosing a new Start path.
    pub(crate) fn completed_admitted_join_key_publication(
        &self,
        handle: &crate::voice_call_activation::Handle,
        executor: [u8; 32],
        permit: &str,
        point: [u8; 65],
    ) -> Result<Option<JoinKeyPublication>, String> {
        let owner = self.admitted_key_owner_ref(handle)?;
        let generation = self
            .join_generations
            .get(&owner.id)
            .ok_or("voice key is unavailable")?;
        if generation.profile != GenerationProfile::Admitted
            || generation
                .admitted_original
                .as_ref()
                .is_none_or(|original| !original.is_same(handle.original()))
            || generation.executor != executor
            || generation.permit != permit
            || !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
            || point[0] != 4
            || generation.point.is_some_and(|frozen| frozen != point)
        {
            return Err("admitted voice key publication changed".into());
        }
        if generation.published {
            if generation.point != Some(point) {
                return Err("admitted voice key publication changed".into());
            }
            return Ok(Some(generation.publication(&owner.id)));
        }
        if generation.settlement_requested {
            return Err("admitted voice key publication is unavailable".into());
        }
        Ok(None)
    }

    /// Cleanup observation for publication never started under this exact native
    /// SDK generation. End can precede a first publish's capacity-refusal reply;
    /// only this retained, irreversible tuple permits cleanup without a terminal
    /// publication ACK. No row is created, no current positive grant is required
    /// or upgraded, and unknown/missing rows remain NoProof.
    pub(crate) fn never_started_closed_admitted_publication(
        &self,
        handle: &crate::voice_call_activation::Handle,
        executor: [u8; 32],
        permit: &str,
    ) -> Option<AdmittedPublicationNotStarted> {
        if !handle.original().is_closed()
            || !handle.original().uses_resources(&self.resources)
            || !self.activations.iter().any(|held| held.same(handle))
        {
            return None;
        }
        let generation = self.join_generations.get(handle.id())?;
        if generation.profile != GenerationProfile::Admitted
            || generation.executor != executor
            || generation.permit != permit
            || generation.point.is_some()
            || generation.publishing
            || generation.published
            || generation.admitted_task.is_some()
            || generation
                .admitted_original
                .as_ref()
                .is_none_or(|original| !original.is_same(handle.original()))
        {
            return None;
        }
        // The immutable original anchors the actual Grant identity even after
        // its strong owner has returned. Upgrading a Grant here would wrongly
        // require live positive ownership for an entirely negative certificate.
        let GenerationMetadata::Admitted(funding) = &generation._metadata else {
            return None;
        };
        Some(AdmittedPublicationNotStarted {
            owner: handle.id().into(),
            executor,
            permit: generation.permit.clone(),
            join: generation.join,
            _funding: funding.clone(),
        })
    }

    /// Freeze the original generated point before the separately paid canonical
    /// publication task can queue Root. Duplicate accepted history is recoverable;
    /// a running/uncertain publication is never replaced by another worker.
    pub(crate) fn prepare_admitted_join_key_publication(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
        executor: [u8; 32],
        permit: &str,
        point: [u8; 65],
    ) -> Result<AdmittedPublicationPlan, String> {
        let owner = self.admitted_key_owner(handle)?;
        let generation = self
            .join_generations
            .get_mut(&owner.id)
            .ok_or("voice key is unavailable")?;
        if generation.profile != GenerationProfile::Admitted
            || generation
                .admitted_original
                .as_ref()
                .is_none_or(|original| !original.is_same(handle.original()))
            || generation.executor != executor
            || generation.permit != permit
            || !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
            || point[0] != 4
            || generation.point.is_some_and(|frozen| frozen != point)
        {
            return Err("admitted voice key publication changed".into());
        }
        if generation.published {
            return Ok(AdmittedPublicationPlan::Accepted(
                generation.publication(&owner.id),
            ));
        }
        if generation.settlement_requested
            || generation.publishing
            || generation.admitted_task.is_some()
        {
            return Err("admitted voice key publication is unavailable".into());
        }
        let GenerationMetadata::Admitted(funding) = &generation._metadata else {
            return Err("voice key belongs to another protocol".into());
        };
        let funding = funding.clone();
        generation.point = Some(point);
        generation.publishing = true;
        Ok(AdmittedPublicationPlan::Start(AdmittedPublicationWork {
            owner,
            original: handle.original().clone(),
            executor,
            permit: generation.permit.clone(),
            point,
            finished: false,
            _funding: funding,
        }))
    }

    /// A distinct typed snapshot for future admitted bootstrap only. It cannot be
    /// converted to the Existing SelectedHandoffOwner accepted by the old Registry.
    /// No round/role/capacity promise follows from publication or this metadata.
    #[allow(dead_code)] // Typed future bootstrap entry, no Existing/pilot conversion.
    pub(crate) fn admitted_published_key(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
        executor: [u8; 32],
    ) -> Result<AdmittedPublishedKey, String> {
        let owner = self.admitted_key_owner(handle)?;
        let generation = self
            .join_generations
            .get_mut(&owner.id)
            .ok_or("voice key is unavailable")?;
        if generation.profile != GenerationProfile::Admitted
            || generation
                .admitted_original
                .as_ref()
                .is_none_or(|original| !original.is_same(handle.original()))
            || generation.executor != executor
            || !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
            || !generation.published
            || generation.publishing
        {
            return Err("admitted voice key is unavailable".into());
        }
        if let Some(selected) = &generation.admitted_selected {
            return Ok(selected.clone());
        }
        #[repr(C, align(2))]
        struct ArcAllocation<T> {
            strong: usize,
            weak: usize,
            value: T,
        }
        let bytes = std::mem::size_of::<ArcAllocation<AdmittedKeySelection>>()
            + std::mem::size_of::<ArcAllocation<SelectedFunding>>()
            + std::mem::size_of::<ArcAllocation<CallGrant>>()
            + 2 * std::mem::size_of::<AdmittedPublishedKey>()
            + owner.id.len()
            + owner.binding.group.len();
        if bytes > SELECTED_BYTES {
            return Err("voice selection capacity reached".into());
        }
        // The paired wrapper pays through actual body/weak allocation return,
        // independently of the native Cell and generation-table lifetime.
        let funding = Arc::new(SelectedFunding {
            _credit: self
                .resources
                .charge(
                    owner.binding.scope.group,
                    SELECTED_BYTES,
                    ResourceKind::Retained,
                )
                .map_err(|error| error.to_string())?,
        });
        let selected = AdmittedPublishedKey {
            selected: Arc::new(AdmittedKeySelection {
                id: owner.id.clone(),
                binding: owner.binding.clone(),
                join: owner.join,
                lease: owner.lease.clone(),
                grant: Arc::downgrade(&owner._grant),
                executor,
                point: generation
                    .point
                    .expect("published admitted generation has its point"),
                resources: self.resources.clone(),
            }),
            _funding: funding,
        };
        generation.admitted_selected = Some(selected.clone());
        Ok(selected)
    }

    /// Validate the supplied ORIGINAL typed snapshot without materializing a new
    /// cache, consuming key-generation authority, or consulting a historical token.
    #[allow(dead_code)] // Pure final check for the future admitted bootstrap consumer.
    pub(crate) fn is_current_admitted_published_key(
        &self,
        handle: &crate::voice_call_activation::Handle,
        selected: &AdmittedPublishedKey,
    ) -> bool {
        let Ok(owner) = self.admitted_key_owner(handle) else {
            return false;
        };
        let key = &selected.selected;
        if key.id != owner.id
            || key.binding != owner.binding
            || key.join != owner.join
            || !key.lease.is_admitted()
            || !key.lease.is_same(&owner.lease)
            || !key.grant.ptr_eq(&Arc::downgrade(&owner._grant))
        {
            return false;
        }
        self.join_generations
            .get(&owner.id)
            .is_some_and(|generation| {
                generation.profile == GenerationProfile::Admitted
                    && generation.executor == key.executor
                    && generation.point == Some(key.point)
                    && generation.published
                    && !generation.publishing
                    && generation
                        .admitted_selected
                        .as_ref()
                        .is_some_and(|cached| cached.is_same(selected))
            })
    }

    /// Negative-only collection. This never requires a live UI or replacement
    /// call, and never removes a still-running SDK/publication owner. The caller
    /// must take/drop the returned actual row AFTER releasing Coordinator.
    pub(crate) fn take_retired_admitted_join_key(
        &mut self,
        owner_id: &str,
    ) -> Option<AdmittedKeyRetirement> {
        let generation = self.join_generations.get(owner_id)?;
        if generation.profile != GenerationProfile::Admitted
            || generation.engine.is_some()
            || generation.publishing
            || generation.admitted_task.is_some()
            || self.owner.as_ref().is_some_and(|owner| {
                owner.id == owner_id
                    && owner.lease.is_live()
                    && generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
            })
        {
            return None;
        }
        let GenerationMetadata::Admitted(funding) = &generation._metadata else {
            return None;
        };
        let funding = funding.clone();
        let row = self.join_generations.remove(owner_id)?;
        Some(AdmittedKeyRetirement {
            _row: row,
            _funding: funding,
        })
    }

    /// Sweep one ended row even when its acknowledged publication task was
    /// already collected while the call remained active. Such rows deliberately
    /// survive generic pruning; scanning only task-bearing rows would eventually
    /// exhaust the finite generation table after normal calls.
    ///
    /// Extract one existing entry directly: no allocation/key clone or renewed
    /// cleanup payment is needed under pressure. The plain key drops while the
    /// actual row still owns its fee; row/watch/selection destructors remain in
    /// the returned ticket and MUST run after Coordinator is released.
    pub(crate) fn take_one_retired_admitted_join_key(&mut self) -> Option<AdmittedKeyRetirement> {
        let owner = self.owner.as_ref();
        let resources = &self.resources;
        let (key, row) = self
            .join_generations
            .extract_if(|id, generation| {
                generation.profile == GenerationProfile::Admitted
                    && generation.engine.is_none()
                    && !generation.publishing
                    && generation.admitted_task.is_none()
                    && generation
                        .admitted_original
                        .as_ref()
                        .is_some_and(|original| original.uses_resources(resources))
                    && !owner.is_some_and(|owner| {
                        owner.id == *id
                            && owner.lease.is_live()
                            && generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
                    })
            })
            .next()?;
        let GenerationMetadata::Admitted(funding) = &row._metadata else {
            unreachable!("admitted rows have admitted metadata");
        };
        let funding = funding.clone();
        drop(key);
        Some(AdmittedKeyRetirement {
            _row: row,
            _funding: funding,
        })
    }

    /// SDK settlement for the exact admitted generation. A closed row created
    /// before delayed admission is permanent for that original call/executor.
    /// Unknown ended calls need no tombstone: their actual NativeActive identity
    /// cannot be resurrected. Deferred notifications/destructors leave the guard.
    pub(crate) fn settle_admitted_join_key(
        &mut self,
        owner_id: &str,
        executor: [u8; 32],
    ) -> Result<AdmittedKeySettlement, String> {
        if !self.join_generations.contains_key(owner_id) {
            let owner = self
                .activations
                .iter()
                .find(|handle| handle.id() == owner_id)
                .and_then(|handle| {
                    self.admitted_key_owner(handle)
                        .ok()
                        .map(|owner| (owner, handle.original().clone()))
                });
            let Some((owner, original)) = owner else {
                return Ok(AdmittedKeySettlement {
                    drained: None,
                    notify: None,
                    _retired: None,
                    _engine: None,
                    _funding: None,
                });
            };
            let mut generation = self.generation_row(&owner, executor, false)?;
            generation.admitted_original = Some(original);
            self.join_generations.insert(owner_id.into(), generation);
        }
        let (drained, notify, engine, funding) = {
            let generation = self
                .join_generations
                .get_mut(owner_id)
                .expect("original admitted generation exists");
            if generation.profile != GenerationProfile::Admitted || generation.executor != executor
            {
                return Err("voice key belongs to another protocol or browser generation".into());
            }
            let GenerationMetadata::Admitted(funding) = &generation._metadata else {
                return Err("voice key belongs to another protocol".into());
            };
            let funding = funding.clone();
            generation.settlement_requested = true;
            if generation.publishing {
                (
                    Some(AdmittedKeyDrain {
                        receiver: generation.drained.subscribe(),
                        _funding: funding.clone(),
                    }),
                    None,
                    None,
                    funding,
                )
            } else {
                (
                    None,
                    Some(generation.drained.clone()),
                    generation.engine.take(),
                    funding,
                )
            }
        };
        let retired = self.take_retired_admitted_join_key(owner_id);
        Ok(AdmittedKeySettlement {
            drained,
            notify,
            _retired: retired,
            _engine: engine,
            _funding: Some(funding),
        })
    }

    /// Consume only an actual KNOWN Root terminal outcome, after original Actor
    /// Work retirement. Never call this for an unknown publisher/queue unwind;
    /// that native invocation retains its original Work and remains quarantined.
    /// A foreign receipt/mismatched Work refuses BEFORE mutating any generation.
    /// The owned completion must leave Coordinator/UI guards before it is finished.
    pub(crate) fn finish_admitted_join_key_publication(
        &mut self,
        work: &mut AdmittedPublicationWork,
        outcome: &Result<AdmittedPresenceReceipt, catcoms_sync::voice_v2::VoiceError>,
    ) -> Result<AdmittedKeyCompletion, String> {
        if work.finished {
            return Err("voice key publication already finished".into());
        }
        if let Ok(receipt) = outcome {
            if !receipt.matches(
                &work.original,
                &work.owner._grant,
                work.owner.join,
                work.point,
            ) {
                return Err("admitted voice key receipt changed".into());
            }
        }
        let terminal = match outcome {
            Ok(_) => AdmittedKeyTerminal::Published,
            Err(error) => AdmittedKeyTerminal::Refused(*error),
        };
        self.complete_admitted_join_key_publication(work, terminal)
    }

    /// UNKNOWN publication can close only after the actual Actor invocation has
    /// retired and the SAME Root residence has actually returned. This is exact
    /// negative cleanup, never a rejected-result certificate, key regeneration,
    /// Source/lower settlement, or authority to reopen the ended call.
    pub(crate) fn close_admitted_publication_after_root_return(
        &mut self,
        work: &mut AdmittedPublicationWork,
        returned: &RootResidenceReturned,
    ) -> Result<AdmittedKeyCompletion, String> {
        if !work.original.is_closed() || !returned.matches(&work.original) {
            return Err("admitted voice publication has not returned".into());
        }
        self.complete_admitted_join_key_publication(work, AdmittedKeyTerminal::ClosedUnknown)
    }

    fn complete_admitted_join_key_publication(
        &mut self,
        work: &mut AdmittedPublicationWork,
        terminal: AdmittedKeyTerminal,
    ) -> Result<AdmittedKeyCompletion, String> {
        if work.finished {
            return Err("voice key publication already finished".into());
        }
        let current = self.activations.iter().any(|handle| {
            handle.original().is_same(&work.original)
                && self.admitted_key_owner(handle).is_ok_and(|owner| {
                    owner.id == work.owner.id
                        && owner.binding == work.owner.binding
                        && owner.join == work.owner.join
                        && Arc::ptr_eq(&owner._grant, &work.owner._grant)
                })
        });
        let (reply, notify, engine, funding) = {
            let generation = self
                .join_generations
                .get_mut(&work.owner.id)
                .ok_or("voice key is unavailable")?;
            if !generation.admitted_matches_work(work) || !generation.publishing {
                return Err("admitted voice key publication changed".into());
            }
            let GenerationMetadata::Admitted(funding) = &generation._metadata else {
                return Err("voice key belongs to another protocol".into());
            };
            let funding = funding.clone();
            work.finished = true;
            generation.publishing = false;
            generation.published |= matches!(terminal, AdmittedKeyTerminal::Published);
            let engine = if generation.settlement_requested {
                generation.engine.take()
            } else {
                None
            };
            let notify = generation
                .settlement_requested
                .then(|| generation.drained.clone());
            let reply = match terminal {
                AdmittedKeyTerminal::Published if current => {
                    Ok(generation.publication(&work.owner.id))
                }
                AdmittedKeyTerminal::Published => {
                    Err("voice endpoint ended before publication completed".into())
                }
                AdmittedKeyTerminal::Refused(error) => Err(error.to_string()),
                AdmittedKeyTerminal::ClosedUnknown => {
                    Err("the call ended before key publication could be confirmed".into())
                }
            };
            (reply, notify, engine, funding)
        };
        let retired = self.take_retired_admitted_join_key(&work.owner.id);
        Ok(AdmittedKeyCompletion {
            result: Some(reply),
            notify,
            _retired: retired,
            _engine: engine,
            _funding: funding,
        })
    }

    /// Exact original task recovery is negative/history only. It does not need
    /// a live owner/grant/UI and cannot create a task when no original is stored.
    pub(crate) fn admitted_publication_task(
        &self,
        handle: &crate::voice_call_activation::Handle,
        executor: [u8; 32],
        permit: &str,
        point: [u8; 65],
    ) -> Option<crate::voice_admitted_key::PendingPublication> {
        let generation = self.join_generations.get(handle.id())?;
        if generation.profile != GenerationProfile::Admitted
            || generation
                .admitted_original
                .as_ref()
                .is_none_or(|original| !original.is_same(handle.original()))
            || generation.permit != permit
            || generation.point != Some(point)
        {
            return None;
        }
        self.admitted_publication_task_for_cleanup(handle.id(), executor)
    }

    /// Recover only an already-installed publication's negative/history owner.
    /// End, lock or replacement can remove the live activation and browser-side
    /// permit/point before SDK settlement arrives, so none is an input authority
    /// here. The immutable native row still binds the executor and every frozen
    /// task field to the SAME original activation. Unknown IDs and other
    /// executors cannot mint work, adopt a successor or discover its key permit.
    ///
    /// The returned handle owns its original payment and must leave Coordinator
    /// before its final destructor runs. Comparisons never take the Cell lock:
    /// the actual publication worker uses the opposite Cell -> Coordinator order.
    pub(crate) fn admitted_publication_task_for_cleanup(
        &self,
        owner_id: &str,
        executor: [u8; 32],
    ) -> Option<crate::voice_admitted_key::PendingPublication> {
        let generation = self.join_generations.get(owner_id)?;
        if generation.profile != GenerationProfile::Admitted || generation.executor != executor {
            return None;
        }
        let original = generation.admitted_original.as_ref()?;
        if !original.uses_resources(&self.resources) {
            return None;
        }
        let pending = generation.admitted_task.as_ref()?;
        pending
            .matches_generation(
                original,
                owner_id,
                executor,
                &generation.permit,
                generation.join,
                generation.point?,
            )
            .then(|| pending.clone())
    }

    /// Canonical custody only: install an already-paid task for the same frozen
    /// generation before Actor enqueue. Closing in the meantime can still retain
    /// this negative owner, but cannot grant its worker a positive first effect.
    pub(crate) fn install_admitted_publication_task(
        &mut self,
        work: &AdmittedPublicationWork,
        pending: &crate::voice_admitted_key::PendingPublication,
    ) -> Result<(), String> {
        if work.finished || !pending.matches(work) {
            return Err("admitted voice publication task changed".into());
        }
        let generation = self
            .join_generations
            .get_mut(work.owner_id())
            .ok_or("voice key is unavailable")?;
        if !generation.publishing || !generation.admitted_matches_work(work) {
            return Err("admitted voice publication task changed".into());
        }
        if let Some(original) = &generation.admitted_task {
            return if original.is_same(pending) {
                Ok(())
            } else {
                Err("admitted voice publication task changed".into())
            };
        }
        generation.admitted_task = Some(pending.clone());
        Ok(())
    }

    /// Only after a known completion (or exact unknown-closed proof) AND actual
    /// Actor Work retirement may the native task remove its original cell. The
    /// returned handle must leave Coordinator before its last destructor runs.
    pub(crate) fn retire_admitted_publication_task(
        &mut self,
        work: &AdmittedPublicationWork,
        pending: &crate::voice_admitted_key::PendingPublication,
    ) -> Option<crate::voice_admitted_key::PendingPublication> {
        if !work.finished || !pending.matches(work) || !pending.is_collectible() {
            return None;
        }
        let generation = self.join_generations.get_mut(work.owner_id())?;
        if generation.publishing || !generation.admitted_matches_work(work) {
            return None;
        }
        if !generation
            .admitted_task
            .as_ref()
            .is_some_and(|original| original.is_same(pending))
        {
            return None;
        }
        generation.admitted_task.take()
    }

    /// Explicit-result collection, not terminal-result creation. Known success
    /// AND refusal stay canonically recoverable through End/lock until the exact
    /// receiver acknowledges AND the original execution Box actually returns.
    /// SDK settlement is independent: it must also have returned the engine fee
    /// before this maintenance path removes the remaining publication owner.
    /// All predicates here are immutable metadata or pure atomics: never take the
    /// publication Cell mutex while Coordinator is held.
    pub(crate) fn collect_completed_admitted_publication(
        &mut self,
        owner_id: &str,
    ) -> Option<crate::voice_admitted_key::PendingPublication> {
        let generation = self.join_generations.get(owner_id)?;
        if generation.profile != GenerationProfile::Admitted
            || generation.publishing
            || generation.engine.is_some()
        {
            return None;
        }
        let original = generation.admitted_original.as_ref()?;
        let pending = generation.admitted_task.as_ref()?;
        if !original.uses_resources(&self.resources)
            || !pending.is_collectible()
            || !pending.matches_generation(
                original,
                owner_id,
                generation.executor,
                &generation.permit,
                generation.join,
                generation.point?,
            )
        {
            return None;
        }
        self.join_generations
            .get_mut(owner_id)?
            .admitted_task
            .take()
    }

    /// Bounded one-row variant for ordinary prepare/admit/ACK/End maintenance.
    /// Borrow the existing map key only during selection; move the original task
    /// before optionally extracting its ended row, without allocating a Vec/key.
    /// The returned pair's actual destructors belong outside Coordinator.
    pub(crate) fn collect_one_completed_admitted_publication(
        &mut self,
    ) -> Option<(
        crate::voice_admitted_key::PendingPublication,
        Option<AdmittedKeyRetirement>,
    )> {
        let pending = self
            .join_generations
            .iter()
            .find_map(|(owner, generation)| {
                let original = generation.admitted_original.as_ref()?;
                let pending = generation.admitted_task.as_ref()?;
                (generation.profile == GenerationProfile::Admitted
                    && !generation.publishing
                    && generation.engine.is_none()
                    && original.uses_resources(&self.resources)
                    && pending.is_collectible()
                    && generation.point.is_some_and(|point| {
                        pending.matches_generation(
                            original,
                            owner,
                            generation.executor,
                            &generation.permit,
                            generation.join,
                            point,
                        )
                    }))
                .then(|| pending.clone())
            })?;
        // The immutable scan is over before the mutable borrow. This SAME paid
        // task owns the bounded ID; no heap key clone is needed. The returned
        // original keeps the payer alive when this temporary alias drops here.
        let original = self.collect_completed_admitted_publication(pending.owner_id())?;
        let retired = self.take_retired_admitted_join_key(original.owner_id());
        Some((original, retired))
    }
}

#[cfg(test)]
mod tests;
