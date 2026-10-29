//! Exact native admission for one browser-held key per accepted join. These fixed records own
//! accounting, not private keys. A lost reply cannot create another operation or let another
//! WebView adopt a generation whose original engine may still be running.
use super::*;
use catcoms_sync::voice_v2::call_activation::ActivationObserver;
use serde::Serialize;

const MAX_GENERATIONS: usize = 32;
// The complete Existing heap/reply graph measured 1,312 bytes. Its old
// provisional 1 KiB fee was insufficient even with separately paid table
// backing. Keep the full guard and charge 1.5 KiB within unchanged ceilings.
pub(super) const GENERATION_BYTES: usize = 1536;
// The admitted row has additional immutable issuer/task aliases plus a prepaid
// completed-history reply; it has its own distinct measured fee.
const ADMITTED_GENERATION_BYTES: usize = 2 * 1024;
const GENERATE_BYTES: usize = 128;
const SELECTED_BYTES: usize = 1024;

mod admitted;
use admitted::admitted_generation_layout_bytes;
#[allow(unused_imports)] // Fixed native return types; admitted bootstrap itself awaits op31.
pub(crate) use admitted::{
    AdmittedKeyCompletion, AdmittedKeyDrain, AdmittedKeyRetirement, AdmittedKeySettlement,
    AdmittedPublicationPlan, AdmittedPublicationWork, AdmittedPublishedKey,
};
pub(super) const JOIN_INDEX_BYTES: usize = 64 * 1024;

/// Plain process backing payment for BOTH profiles, with no Coordinator/Runtime/
/// task backedge. The first actual generation prepays the SAME typed table; the
/// Coordinator field follows it and never refunds on empty retained capacity.
pub(super) struct JoinIndexFunding {
    _credit: catcoms_sync::voice_v2::resources::ProcessRetainedCredit,
}

/// Issuer provenance is immutable for the whole browser-key generation. A
/// renderer token, recovered public point, or later selected channel cannot
/// reinterpret an Existing generation as an admitted one (or the reverse).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationProfile {
    Existing,
    Admitted,
}
impl GenerationProfile {
    fn of(owner: &Owner) -> Self {
        if owner.lease.is_admitted() {
            Self::Admitted
        } else {
            Self::Existing
        }
    }
}

/// Both profiles share their SAME plain fee with exact drain owners, so a late
/// SDK settlement receiver cannot outlive metadata payment after row extraction.
/// The immutable variant still prevents one profile from adopting the other.
enum GenerationMetadata {
    Existing(Arc<GenerationFunding>),
    Admitted(Arc<GenerationFunding>),
}
struct GenerationFunding {
    _credit: Credit,
}

/// Existing-profile settlement keeps the ORIGINAL watch allocation paid even
/// when End/pruning removes its row before the native waiter is polled again.
/// No raw receiver escapes without the same plain metadata payment.
#[must_use = "retain the original paid watch owner until settlement returns"]
pub(crate) struct JoinKeyDrain {
    receiver: tokio::sync::watch::Receiver<bool>,
    _funding: Arc<GenerationFunding>,
}
impl JoinKeyDrain {
    /// Only the closed native/Tauri Tokio path awaits this private drain. Peers
    /// and JS cannot supply a Rust executor/waker. Existing send_replace/pruning
    /// notifications still run under Coordinator as before; this wrapper does
    /// not claim generic arbitrary-Rust-waker reentry/panic resilience.
    pub(crate) async fn wait(mut self) -> Result<(), String> {
        while !*self.receiver.borrow_and_update() {
            self.receiver
                .changed()
                .await
                .map_err(|_| "voice key settlement owner stopped".to_string())?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn borrow(&self) -> tokio::sync::watch::Ref<'_, bool> {
        self.receiver.borrow()
    }
}

/// Pinned Tokio1.53.3 watch::Shared<bool>/BigNotify heap, including the Arc
/// header and conservative optional fallback counter. Shared table backing pays
/// inline Generation slots separately; each row pays its own actual watch heap.
fn generation_watch_layout_bytes() -> usize {
    #[allow(dead_code)]
    struct WatchShared {
        value: std::sync::RwLock<bool>,
        state: std::sync::atomic::AtomicUsize,
        receivers: std::sync::atomic::AtomicUsize,
        senders: std::sync::atomic::AtomicUsize,
        next: std::sync::atomic::AtomicUsize,
        notify_rx: [tokio::sync::Notify; 8],
        notify_tx: tokio::sync::Notify,
    }
    #[repr(C)]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    std::mem::size_of::<ArcAllocation<WatchShared>>()
}

/// Complete Existing retained row heap and one bounded public-reply peak. The
/// SAME process-level index payment now covers ALL inline table slots and both
/// allocations during rehash, so excluding that backing here is explicit.
/// The original Grant's raw Weak can retain its allocation after affine custody
/// returns; include that body/header rather than charging only a pointer.
pub(super) fn existing_generation_layout_bytes() -> usize {
    #[repr(C)]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    generation_watch_layout_bytes()
        + std::mem::size_of::<ArcAllocation<GenerationFunding>>()
        + std::mem::size_of::<ArcAllocation<CallGrant>>()
        + 2 * 64 // actual table-key and permit String heaps
        + std::mem::size_of::<JoinKeyDrain>()
        + 2 * std::mem::size_of::<Arc<GenerationFunding>>()
        + std::mem::size_of::<JoinKeyPublication>()
        + 5 * 64 // owner/executor/permit/join/key_ref bounded reply String heaps
}

pub(super) struct Generation {
    profile: GenerationProfile,
    admitted_original: Option<ActivationObserver>,
    executor: [u8; 32],
    permit: String,
    join: Nonce,
    grant: std::sync::Weak<CallGrant>,
    engine: Option<Credit>,
    settlement_requested: bool,
    point: Option<[u8; 65]>,
    publishing: bool,
    published: bool,
    drained: tokio::sync::watch::Sender<bool>,
    selected: Option<Arc<SelectedHandoffOwner>>,
    admitted_selected: Option<AdmittedPublishedKey>,
    admitted_task: Option<crate::voice_admitted_key::PendingPublication>,
    // LAST: actual watch and cached selection owners precede their own metadata.
    _metadata: GenerationMetadata,
}

/// Native-only identity for the byte-bearing pilot. Constructed once from the actual published
/// generation, never from a renderer projection. Keeping this snapshot cannot keep an ended
/// call live: End revokes the original shared lease synchronously. Root must still check the
/// actual grant, actor incarnation, membership and epoch at every first protocol effect.
pub(crate) struct SelectedHandoffOwner {
    id: String,
    binding: Binding,
    join: Nonce,
    lease: CallLease,
    // Generation pruning runs under Coordinator. Its cached snapshot must never become the
    // last affine CallGrant owner there; only native workers upgrade outside native locks.
    grant: std::sync::Weak<CallGrant>,
    executor: [u8; 32],
    point: [u8; 65],
    resources: Arc<Resources>,
    _metadata: Credit,
}
impl SelectedHandoffOwner {
    // Native actor consumer remains disabled with the byte-bearing pilot.
    #[allow(dead_code)]
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn binding(&self) -> &Binding {
        &self.binding
    }
    pub(crate) fn join(&self) -> Nonce {
        self.join
    }
    pub(crate) fn lease(&self) -> &CallLease {
        &self.lease
    }
    pub(crate) fn executor(&self) -> [u8; 32] {
        self.executor
    }
    pub(crate) fn point(&self) -> [u8; 65] {
        self.point
    }
    pub(crate) fn resources(&self) -> Arc<Resources> {
        self.resources.clone()
    }
    /// Upgrade outside Coordinator/ledger locks and retain through the actual actor turn.
    /// This historical snapshot cannot resurrect an affine grant that has already gone away.
    #[allow(dead_code)] // consumed by the next native actor integration, not a renderer command
    pub(crate) fn grant(&self) -> Result<Arc<CallGrant>, catcoms_sync::voice_v2::VoiceError> {
        self.grant
            .upgrade()
            .filter(|grant| grant.lease().is_live() && !grant.lease().is_admitted())
            .ok_or(catcoms_sync::voice_v2::VoiceError::Unauthorized)
    }
    pub(crate) fn grant_identity(&self) -> std::sync::Weak<CallGrant> {
        self.grant.clone()
    }
}

/// All fields are public metadata. `closed` is history only and never permission to generate
/// again, even if a remounted browser no longer has the private key.
#[derive(Clone, Serialize)]
pub(crate) struct JoinKeyAdmission {
    pub owner: String,
    pub executor: String,
    pub permit: String,
    pub join: String,
    pub kind: &'static str,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JoinKeyPublication {
    pub owner: String,
    pub executor: String,
    pub permit: String,
    pub join: String,
    pub key_ref: String,
}

/// Native-owned publication carries the exact affine grant outside Coordinator. Its actual
/// actor turn and the UI commit guard must outlive an abandoned command/reply waiter.
pub(crate) struct JoinKeyWork {
    owner: Owner,
    executor: [u8; 32],
    point: [u8; 65],
    finished: bool,
}
impl JoinKeyWork {
    pub fn grant(&self) -> Arc<CallGrant> {
        self.owner._grant.clone()
    }
    pub fn join(&self) -> Nonce {
        self.owner.join
    }
    pub fn point(&self) -> [u8; 65] {
        self.point
    }
}
// Keep the affine native work inline: its already paid generation metadata covers this wrapper.
// Boxing Start would add an otherwise unnecessary heap allocation and a new custody obligation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum PublicationPlan {
    Accepted(JoinKeyPublication),
    Start(JoinKeyWork),
}

impl Coordinator {
    pub(super) fn prune_join_generations(&mut self) {
        let current = self.owner.as_ref().map(|owner| owner.id.as_str());
        self.join_generations.retain(|id, generation| {
            // Admitted rows leave through an owned retirement ticket instead.
            // Last watch Sender/selected-owner destruction may invoke a waker;
            // never add those new callbacks under Coordinator's mutex.
            generation.profile == GenerationProfile::Admitted
                || current == Some(id.as_str())
                || generation.engine.is_some()
                || generation.publishing
        });
    }

    fn generation_row(
        &mut self,
        owner: &Owner,
        executor: [u8; 32],
        generate: bool,
    ) -> Result<Generation, String> {
        let profile = GenerationProfile::of(owner);
        if profile == GenerationProfile::Admitted
            && admitted_generation_layout_bytes() > ADMITTED_GENERATION_BYTES
        {
            return Err("voice key generation capacity reached".into());
        }
        if profile == GenerationProfile::Existing
            && existing_generation_layout_bytes() > GENERATION_BYTES
        {
            return Err("voice key generation capacity reached".into());
        }
        self.prune_join_generations();
        if self.join_generations.len() >= MAX_GENERATIONS {
            return Err("voice key generation capacity reached".into());
        }
        let group = owner.binding.scope.group;
        // Pay before inserting any record. Capacity refusal leaves this join untouched; credit
        // cleanup has no actor/policy callbacks and creates no new retry or engine authority.
        let metadata = self
            .resources
            .charge(
                group,
                match profile {
                    GenerationProfile::Existing => GENERATION_BYTES,
                    GenerationProfile::Admitted => ADMITTED_GENERATION_BYTES,
                },
                ResourceKind::Retained,
            )
            .map_err(|e| e.to_string())?;
        let engine = if generate {
            Some(
                self.resources
                    .charge(group, GENERATE_BYTES, ResourceKind::Crypto)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        // The table contains both profiles and its inline entry size includes
        // their common closed representation. Reserve the fixed shared backing
        // before ANY row/watch/ID allocation, including Existing-only use. Plain
        // row/engine reservations above can refund on local infrastructure
        // refusal; no generation/permit was allocated or consumed yet.
        self.ensure_join_index()?;
        let funding = Arc::new(GenerationFunding { _credit: metadata });
        let metadata = match profile {
            GenerationProfile::Existing => GenerationMetadata::Existing(funding),
            GenerationProfile::Admitted => GenerationMetadata::Admitted(funding),
        };
        Ok(Generation {
            profile,
            admitted_original: None,
            executor,
            permit: self.id()?,
            join: owner.join,
            grant: Arc::downgrade(&owner._grant),
            engine,
            settlement_requested: !generate,
            point: None,
            publishing: false,
            published: false,
            drained: tokio::sync::watch::channel(!generate).0,
            selected: None,
            admitted_selected: None,
            admitted_task: None,
            _metadata: metadata,
        })
    }

    /// Cache the same actual selection before a native factory begins. Lost callers obtain
    /// the original Arc without another fixed charge, key generation or publication. A closed
    /// generation with no published key, another executor or an equal-scope replacement fails.
    /// Ordinary key-generation settlement is allowed after successful publication: it retires
    /// the generation operation, not the selected browser's still-held nonextractable key.
    // The byte-bearing pilot is deliberately disabled until complete physical-flight custody.
    #[allow(dead_code)]
    pub(crate) fn selected_handoff_owner(
        &mut self,
        owner_id: &str,
        executor: [u8; 32],
    ) -> Result<Arc<SelectedHandoffOwner>, String> {
        let owner = self
            .owner
            .as_ref()
            .filter(|owner| {
                owner.id == owner_id && owner.lease.is_live() && !owner.lease.is_admitted()
            })
            .ok_or("voice owner changed")?;
        let generation = self
            .join_generations
            .get_mut(owner_id)
            .ok_or("voice key is unavailable")?;
        if generation.profile != GenerationProfile::Existing
            || generation.executor != executor
            || !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
            || !generation.published
            || generation.publishing
        {
            return Err("voice key is unavailable".into());
        }
        if let Some(selected) = &generation.selected {
            return Ok(selected.clone());
        }
        let layout = std::mem::size_of::<SelectedHandoffOwner>()
            .checked_add(2 * std::mem::size_of::<usize>())
            // Weak keeps the original Arc allocation alive after its affine value drains.
            .and_then(|bytes| {
                bytes.checked_add(
                    std::mem::size_of::<CallGrant>()
                        + 2 * std::mem::size_of::<usize>()
                        + std::mem::size_of::<Arc<SelectedHandoffOwner>>(),
                )
            })
            .and_then(|bytes| bytes.checked_add(owner.id.len()))
            .and_then(|bytes| bytes.checked_add(owner.binding.group.len()));
        if layout.is_none_or(|bytes| bytes > SELECTED_BYTES) {
            return Err("voice selection capacity reached".into());
        }
        // This metadata is independent of Workflow/data capacity. Prepay before the Arc or
        // Owner strings are allocated, and retain that credit if history outlives Generation.
        let metadata = self
            .resources
            .charge(
                owner.binding.scope.group,
                SELECTED_BYTES,
                ResourceKind::Retained,
            )
            .map_err(|error| error.to_string())?;
        let selected = Arc::new(SelectedHandoffOwner {
            id: owner.id.clone(),
            binding: owner.binding.clone(),
            join: owner.join,
            lease: owner.lease.clone(),
            grant: Arc::downgrade(&owner._grant),
            executor,
            point: generation
                .point
                .expect("published generation has its frozen point"),
            resources: self.resources.clone(),
            _metadata: metadata,
        });
        generation.selected = Some(selected.clone());
        Ok(selected)
    }

    /// Validate a supplied ORIGINAL allocation without creating a selection or charging
    /// metadata. The lazy selected_handoff_owner factory is for bootstrap, not this final
    /// admission check: foreign equal labels must not materialize a local cache on refusal.
    /// Pure pointer/lease/field checks are safe under Coordinator, with no Clock or callback.
    pub(crate) fn is_current_handoff_owner(&self, selected: &Arc<SelectedHandoffOwner>) -> bool {
        let Some(owner) = self.owner.as_ref() else {
            return false;
        };
        if owner.lease.is_admitted()
            || selected.lease.is_admitted()
            || owner.id != selected.id
            || !owner.lease.is_live()
            || !selected.lease.is_live()
            || !owner.lease.is_same(&selected.lease)
            || owner.binding != selected.binding
            || owner.join != selected.join
            || !selected.grant.ptr_eq(&Arc::downgrade(&owner._grant))
        {
            return false;
        }
        self.join_generations
            .get(&owner.id)
            .is_some_and(|generation| {
                generation.profile == GenerationProfile::Existing
                    && generation.executor == selected.executor
                    && generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
                    && generation.published
                    && !generation.publishing
                    && generation
                        .selected
                        .as_ref()
                        .is_some_and(|cached| Arc::ptr_eq(cached, selected))
            })
    }

    pub fn admit_join_key(
        &mut self,
        owner_id: &str,
        executor: [u8; 32],
    ) -> Result<JoinKeyAdmission, String> {
        let owner = self.owner(owner_id).ok_or("voice owner changed")?;
        if !self.join_generations.contains_key(owner_id) {
            let generation = self.generation_row(&owner, executor, true)?;
            self.join_generations.insert(owner_id.into(), generation);
        }
        let generation = &self.join_generations[owner_id];
        if generation.profile != GenerationProfile::Existing
            || !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
        {
            return Err("voice key belongs to another browser generation".into());
        }
        if generation.executor != executor {
            // A definite no-admission result is distinct from an uncertain transport reply.
            // It discloses none of the original executor/permit/join and cannot settle them.
            return Ok(JoinKeyAdmission {
                owner: owner_id.into(),
                executor: hex::encode(executor),
                permit: String::new(),
                join: String::new(),
                kind: "refused",
            });
        }
        Ok(JoinKeyAdmission {
            owner: owner_id.into(),
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

    /// Exact settlement remains available after lock/End. Settlement before a delayed admission
    /// installs a closed current-owner row; a later admission cannot rearm it. Unknown retired
    /// owners need no tombstone: their native identity can never be activated again.
    pub fn settle_join_key(
        &mut self,
        owner_id: &str,
        executor: [u8; 32],
    ) -> Result<Option<JoinKeyDrain>, String> {
        if !self.join_generations.contains_key(owner_id) {
            let Some(owner) = self.owner(owner_id) else {
                return Ok(None);
            };
            let generation = self.generation_row(&owner, executor, false)?;
            self.join_generations.insert(owner_id.into(), generation);
        }
        let generation = self
            .join_generations
            .get_mut(owner_id)
            .expect("generation inserted");
        if generation.profile != GenerationProfile::Existing || generation.executor != executor {
            return Err("voice key belongs to another browser generation".into());
        }
        generation.settlement_requested = true;
        // An already queued actor effect is real outstanding work too. Its native-owned worker
        // retires publishing before this exact execution charge can return.
        if generation.publishing {
            let GenerationMetadata::Existing(funding) = &generation._metadata else {
                unreachable!("Existing profile checked above");
            };
            return Ok(Some(JoinKeyDrain {
                receiver: generation.drained.subscribe(),
                _funding: funding.clone(),
            }));
        }
        generation.engine.take();
        generation.drained.send_replace(true);
        self.prune_join_generations();
        Ok(None)
    }

    pub fn prepare_join_key_publication(
        &mut self,
        owner_id: &str,
        executor: [u8; 32],
        permit: &str,
        point: [u8; 65],
    ) -> Result<PublicationPlan, String> {
        let owner = self.owner(owner_id).ok_or("voice owner changed")?;
        let generation = self
            .join_generations
            .get_mut(owner_id)
            .ok_or("voice key is unavailable")?;
        if generation.profile != GenerationProfile::Existing
            || generation.executor != executor
            || generation.permit != permit
            || !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
            || point[0] != 4
            || generation.point.is_some_and(|original| original != point)
        {
            return Err("voice key publication changed".into());
        }
        if generation.published {
            return Ok(PublicationPlan::Accepted(generation.publication(owner_id)));
        }
        if generation.settlement_requested || generation.publishing {
            return Err("voice key publication is unavailable".into());
        }
        // Freeze the point BEFORE actor handoff: even an uncertain actor reply cannot substitute
        // another generated key under this join. Concurrent retries cannot start another worker.
        generation.point = Some(point);
        generation.publishing = true;
        Ok(PublicationPlan::Start(JoinKeyWork {
            owner,
            executor,
            point,
            finished: false,
        }))
    }

    /// Called by the native worker for both success and refusal. No IPC waiter owns this cleanup.
    pub fn finish_join_key_publication(
        &mut self,
        work: &mut JoinKeyWork,
        accepted: bool,
    ) -> Result<JoinKeyPublication, String> {
        // The old byte-bearing pilot must never complete an admitted generation,
        // even if a future internal caller somehow supplies equal public labels.
        if work.owner.lease.is_admitted() {
            return Err("voice key belongs to another protocol".into());
        }
        // Consume this opaque work's one completion, but leave affine grant destruction with
        // its native worker after Coordinator unlock. Replaying an old completion cannot clear
        // a later retry's publishing flag or return that later worker's execution charge.
        if std::mem::replace(&mut work.finished, true) {
            return Err("voice key publication already finished".into());
        }
        let current = self
            .owner(&work.owner.id)
            .is_some_and(|owner| Arc::ptr_eq(&owner._grant, &work.owner._grant));
        let generation = self
            .join_generations
            .get_mut(&work.owner.id)
            .ok_or("voice key is unavailable")?;
        if generation.profile != GenerationProfile::Existing
            || generation.executor != work.executor
            || generation.point != Some(work.point)
        {
            return Err("voice key publication changed".into());
        }
        generation.publishing = false;
        generation.published |= accepted;
        if generation.settlement_requested {
            generation.engine.take();
            generation.drained.send_replace(true);
        }
        let reply = generation.publication(&work.owner.id);
        self.prune_join_generations();
        if accepted && current {
            Ok(reply)
        } else {
            Err("voice endpoint was not activated".into())
        }
    }
}
impl Generation {
    fn publication(&self, owner: &str) -> JoinKeyPublication {
        JoinKeyPublication {
            owner: owner.into(),
            executor: hex::encode(self.executor),
            permit: self.permit.clone(),
            join: hex::encode(self.join),
            key_ref: hex::encode(catcoms_sync::voice_v2::key_reference(
                &self.point.expect("published point"),
            )),
        }
    }
}
