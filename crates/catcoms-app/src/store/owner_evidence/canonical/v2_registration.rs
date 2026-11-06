//! Unique V2 writer resource/custody consumer paired with the actual namespace
//! owner guards, genuine scope issuer and unexposed caller. No v1 HeadCell, SDK
//! budget, decoded label or parsed head may create a registration here.
use super::*;
use crate::store::chat_admission::v2_current::{V2CurrentPreparation, V2InitializingAdmission};
use crate::store::critical_namespaces::{
    current_layout::{OriginalV2GenesisGroupObservation, PreparedV2GenesisGroupNamespace},
    read_pool::{CatalogReadReservation, CriticalCatalogReadPool},
};
use crate::store::persistence::{
    v2_genesis_activation::SavedV2GenesisActivationSeed, GenesisSourceDataRef,
};
use catcoms_rt::RequestAuthority;

const MAX_WRITERS: usize = crate::store::MAX_CHAT_ADMISSION_IDENTITIES;
const WRITER_METADATA_FEE: usize = 8 * 1024;
const MAX_GROUP: usize = 256;
#[cfg(test)]
thread_local! { static BUFFER_ALLOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
thread_local! {
    static BEFORE_FOREIGN_REFUSAL: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static BEFORE_INSTALLED_FINAL_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// Only the canonical family can inspect this map. It has a separately prepaid fixed-capacity
/// Vec, never implicit BTree growth or an owner-checkpoint budget used as V2 authority.
#[derive(Default)]
pub(super) struct Runtime {
    buffer: Option<SlotBuffer>,
}
impl Runtime {
    /// Exact existing-generation proof under canonical. This creates no strong writer and
    /// cannot renew, select a replacement, or install. The surrounding typed role separately
    /// proves the source, head, task and scope, then repeats negatives after physical waits.
    fn require_installed_observation(
        &self,
        key: &[u8; 32],
        observation: &V2WriterObservation,
    ) -> Result<(), AppError> {
        let slots = &self.buffer.as_ref().ok_or_else(refused)?.slots;
        let index = slots
            .binary_search_by_key(key, |slot| slot.key)
            .map_err(|_| refused())?;
        let slot = &slots[index];
        if !slot.owner.ptr_eq(&observation.original_life) || slot.owner.strong_count() == 0 {
            return Err(refused());
        }
        #[cfg(test)]
        if let Some(hook) = BEFORE_INSTALLED_FINAL_CHECK.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
        // The original may drain after the count snapshot. Never upgrade it to close that
        // race: the native receiver/closed authority must expose its original cancellation.
        if observation.is_cancelled() {
            return Err(refused());
        }
        Ok(())
    }
    /// NEGATIVE exclusion under the same already-held canonical runtime. Cancellation does
    /// not mean drain: any remaining original strong owner still excludes a second writer.
    /// Do not upgrade this Weak for a mere absence test. A concurrent final strong Drop
    /// could otherwise make the temporary upgrade run Sender/owner cleanup under canonical.
    pub(super) fn require_no_live_writer(&self, key: &[u8; 32]) -> Result<(), AppError> {
        let Some(buffer) = &self.buffer else {
            return Ok(());
        };
        if let Ok(index) = buffer.slots.binary_search_by_key(key, |slot| slot.key) {
            if buffer.slots[index].owner.strong_count() != 0 {
                return Err(refused());
            }
        }
        // A zero strong count cannot be resurrected by an observer. This helper neither
        // replaces nor drops the inert slot/its fee, and is not an installation capability.
        Ok(())
    }
}
struct SlotBuffer {
    slots: Vec<Slot>,
    // LAST: all slot Weak allocations/fees and the actual Vec drain before this refund.
    _fee: CatalogReadReservation,
}
struct Slot {
    key: [u8; 32],
    owner: Weak<WriterLife>,
    // LAST: a dead owner's Weak still pins its Arc body. This ORIGINAL generation's real
    // metadata fee also follows old observations after a replacement selects another fee.
    _fee: Arc<CatalogReadReservation>,
}
struct WriterLife {
    identity: Arc<()>,
    sender: watch::Sender<bool>,
    #[cfg(test)]
    drop_notice: Option<std::sync::mpsc::Sender<std::thread::ThreadId>>,
}
#[cfg(test)]
impl Drop for WriterLife {
    fn drop(&mut self) {
        if let Some(notice) = self.drop_notice.take() {
            let _ = notice.send(std::thread::current().id());
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
enum ExistingOwner {
    SameOriginal,
    Absent,
}
/// Negative slot classification used by BOTH real installation and final installed proof.
/// Cancellation alone never establishes absence: the original may still own its drain.
fn classify_original_owner(
    slot: &Slot,
    proposed: &Arc<WriterLife>,
) -> Result<ExistingOwner, AppError> {
    // The borrowed proposed Arc keeps the same-owner case alive. For a foreign original,
    // never create a temporary strong owner: concurrent original Drop could make it the
    // last Arc and run watch/Sender cleanup under this actual canonical writer.
    if slot.owner.ptr_eq(&Arc::downgrade(proposed)) {
        Ok(ExistingOwner::SameOriginal)
    } else if slot.owner.strong_count() != 0 {
        #[cfg(test)]
        {
            let hook = BEFORE_FOREIGN_REFUSAL.with(|hook| hook.borrow_mut().take());
            if let Some(hook) = hook {
                hook();
            }
        }
        // Concurrent draining can only make this a conservative refusal. Count0 cannot
        // resurrect; an explicit later attempt can observe the now-dead original safely.
        Err(refused())
    } else {
        Ok(ExistingOwner::Absent)
    }
}
struct ScopeStamp {
    group: Vec<u8>,
    signer: [u8; 32],
    domain: [u8; 32],
    nonce: [u8; 32],
    owner_identity: Arc<()>,
}

/// Negative observation only. Keep identity FIRST and cancellation/accounting LAST, so a
/// clone cannot leave an unpaid old identity/watch generation behind after replacement.
/// No Sender, STRONG WriterLife, graph, key or Store is retained by this type or its authority.
#[derive(Clone)]
pub(in crate::store) struct V2WriterObservation {
    identity: Arc<()>,
    // Same generation's paid Weak pins only the tiny original allocation, never its sender.
    // Keep it before cancellation/accounting so it drains before the actual charge can drop.
    original_life: Weak<WriterLife>,
    cancellation: RequestCancellation,
}
impl V2WriterObservation {
    pub(in crate::store) fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    /// Returns the cached original negative/accounting chain; it allocates no new authority.
    pub(in crate::store) fn cancellation(&self) -> RequestCancellation {
        self.cancellation.clone()
    }
}

/// Preparation alone grants no Current or publication. The concrete pre-Root owner guard
/// must install this SAME object, and the complete physical head/Current2 closure must follow.
pub(in crate::store) struct PreparedV2WriterRegistration {
    // Retired slot/unused buffers are deliberately kept across the guarded operation. Drain
    // them only outside ALL host writers, never refund the last account lease under a lock.
    retired: Option<Slot>,
    expansion: Option<SlotBuffer>,
    stamp: ScopeStamp,
    task: OriginalV2GenesisGroupObservation,
    activation: Arc<SavedV2GenesisActivationSeed>,
    mount: Arc<()>,
    observation: V2WriterObservation,
    // Keep the sole strong life until the other original metadata drains. Drop first sends
    // cancellation; ANY remaining strong registration excludes replacement even when false.
    life: Arc<WriterLife>,
    // LAST, and independently shared by Runtime slot plus every escaping negative authority.
    _fee: Arc<CatalogReadReservation>,
}
impl std::fmt::Debug for PreparedV2WriterRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2WriterRegistration")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2WriterObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2WriterObservation")
            .finish_non_exhaustive()
    }
}
impl Drop for PreparedV2WriterRegistration {
    fn drop(&mut self) {
        self.cancel_original();
    }
}

/// The actual ORIGINAL instance's accounting follows its original cancellation observation,
/// including after Store Drop or slot replacement. A latest-slot fee alone cannot pay an
/// arbitrary number of old watch generations. Arbitrary awaiting task/waker allocations are
/// separate governor/native obligations; this closed genesis slice starts no publisher.
struct AccountedAuthority {
    mount: RequestCancellation,
    scope: RequestCancellation,
    // LAST after both weak original cancellation chains, never a positive owner.
    _fee: Arc<CatalogReadReservation>,
}
/// A lower driver may replace its effect authority while preserving completion keepalive.
/// Keep the SAME fee in BOTH chains: authority Drop order covers its original receivers,
/// and this metadata-only owner covers a deliberately replaced effect authority.
struct ObservationAccounting {
    _fee: Arc<CatalogReadReservation>,
}
impl std::fmt::Debug for ObservationAccounting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2WriterObservationAccounting")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for AccountedAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountedV2WriterAuthority")
            .finish_non_exhaustive()
    }
}
#[async_trait::async_trait]
impl RequestAuthority for AccountedAuthority {
    fn authorized(&self) -> bool {
        !self.mount.is_cancelled() && !self.scope.is_cancelled()
    }
    async fn revoked(&self) {
        let mut mount = self.mount.clone();
        let mut scope = self.scope.clone();
        if self.authorized() {
            tokio::select! { _ = mount.cancelled() => {}, _ = scope.cancelled() => {} }
        }
    }
}

fn buffer_bytes() -> Result<usize, AppError> {
    MAX_WRITERS
        .checked_mul(std::mem::size_of::<Slot>())
        .and_then(|n| n.checked_add(std::mem::size_of::<SlotBuffer>() + 64))
        .ok_or_else(refused)
}
fn check_writer_invoice() -> Result<(), AppError> {
    let named = std::mem::size_of::<PreparedV2WriterRegistration>()
        + std::mem::size_of::<ScopeStamp>() + MAX_GROUP
        + std::mem::size_of::<WriterLife>() + std::mem::size_of::<AccountedAuthority>()
        + std::mem::size_of::<ObservationAccounting>() + 64
        + std::mem::size_of::<V2WriterObservation>() + std::mem::size_of::<Slot>()
        + std::mem::size_of::<OriginalV2GenesisGroupObservation>()
        + 8 * 64 // tiny identity, Arc/accounting/control blocks and owning-return scratch
        + 1024; // closed watch/cancellation metadata; not arbitrary task/RSS admission
    if named > WRITER_METADATA_FEE {
        return Err(refused());
    }
    Ok(())
}
impl SlotBuffer {
    fn prepare(pool: &CriticalCatalogReadPool) -> Result<Self, AppError> {
        let fee = pool.try_reserve(buffer_bytes()?)?;
        #[cfg(test)]
        BUFFER_ALLOCATIONS.with(|count| count.set(count.get() + 1));
        let slots = Vec::with_capacity(MAX_WRITERS);
        if slots.capacity() != MAX_WRITERS {
            return Err(refused());
        }
        Ok(Self { slots, _fee: fee })
    }
}

impl PreparedV2WriterRegistration {
    /// Close this exact writer before its owning node starts draining payload/T/R resources.
    /// It exposes no Sender or renewal: the still-strong life continues excluding replacement
    /// until the original actually drains. Owning wrappers call this OUTSIDE host writers.
    pub(in crate::store) fn cancel_original(&self) {
        self.life.sender.send_replace(true);
    }

    /// Resource and negative-life preparation ONLY, outside all host writers. Concrete opaque
    /// Some1 scope input excludes an arbitrary public trait with an allocating authority.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        scope: &V2CurrentPreparation,
    ) -> Result<Self, AppError> {
        source.require_original_negative(store)?;
        scope.require_original_source_negative(store, source, task)?;
        check_writer_invoice()?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = Arc::new(pool.try_reserve(WRITER_METADATA_FEE)?);
        // A negative allocation hint, not permission. Once installed the map is non-evicting
        // for this original Store. Concurrent first preparations can carry unused paid maps.
        let needs_buffer = store
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?
            .v2_writers
            .buffer
            .is_none();
        let expansion = if needs_buffer {
            Some(SlotBuffer::prepare(pool)?)
        } else {
            None
        };
        let stamp = ScopeStamp {
            group: scope.group().to_vec(),
            signer: *scope.signer(),
            domain: scope.domain(),
            nonce: scope.nonce(),
            owner_identity: scope.owner_identity(),
        };
        let (sender, _) = watch::channel(false);
        let life = Arc::new(WriterLife {
            identity: Arc::new(()),
            sender,
            #[cfg(test)]
            drop_notice: None,
        });
        let cancellation = RequestCancellation::new(
            life.sender.subscribe(),
            Some(Arc::new(ObservationAccounting { _fee: fee.clone() })),
        )
        .with_authority(Arc::new(AccountedAuthority {
            mount: store.chat_mount.observer(),
            scope: scope.reservation_observer(),
            _fee: fee.clone(),
        }));
        source.require_original_negative(store)?;
        scope.require_original_source_negative(store, source, task)?;
        Ok(Self {
            retired: None,
            expansion,
            stamp,
            task: task.observe_original(),
            activation: scope.activation().clone(),
            mount: store.registry_mount(),
            observation: V2WriterObservation {
                identity: life.identity.clone(),
                original_life: Arc::downgrade(&life),
                cancellation,
            },
            life,
            _fee: fee,
        })
    }

    fn require_initializing(
        &self,
        admission: &V2InitializingAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        let permit = admission.scope_permit();
        permit.validate_original(admission.store(), admission.group(), admission.signer())?;
        self.task.require_same_original(admission.task())?;
        if permit.mode() != ChatAdmissionStorageMode::Initializing
            || permit.revision() != 1
            || !Arc::ptr_eq(&self.mount, &admission.store().registry_mount())
            || !Arc::ptr_eq(&self.activation, admission.activation())
            || !self
                .observation
                .original_life
                .ptr_eq(&Arc::downgrade(&self.life))
            || self.stamp.group != admission.group()
            || &self.stamp.signer != admission.signer()
            || self.stamp.domain != permit.domain()
            || self.stamp.nonce != permit.nonce()
            || !Arc::ptr_eq(&self.stamp.owner_identity, permit.owner_identity())
            || self.observation.is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Call ONLY after the complete namespace/admission closure has drained. This releases
    /// unused/retired metadata accounts without reentering a pool under any host writer.
    pub(in crate::store) fn drain_unused_outside_writers(&mut self) {
        drop(self.retired.take());
        drop(self.expansion.take());
    }

    pub(in crate::store) fn observe_original(&self) -> V2WriterObservation {
        self.observation.clone()
    }
    pub(in crate::store) fn require_same_observation(
        &self,
        observation: &V2WriterObservation,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.observation.identity, &observation.identity)
            || !self
                .observation
                .original_life
                .ptr_eq(&observation.original_life)
            || self.observation.is_cancelled()
            || observation.is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl super::current_vacancy::InitializingV2OwnerNamespace<'_, '_> {
    /// Insert only under the actual already-held Init1 admission -> pins -> canonical writer,
    /// BEFORE Root. A live cancelled original still owns its drain and excludes replacement.
    pub(in crate::store) fn install_writer(
        &mut self,
        prepared: &mut PreparedV2WriterRegistration,
    ) -> Result<(), AppError> {
        let (admission, runtime) = self.registration_parts();
        prepared.require_initializing(admission)?;
        let identifier = key(admission.group(), admission.signer());
        if runtime.buffer.is_none() {
            if prepared.expansion.is_none() {
                return Err(refused());
            }
            // The buffer transfer itself has no fallible/allocation gap. Later source checks
            // can still refuse; the original paid buffer then remains installed for retry.
            prepared.require_initializing(admission)?;
            runtime.buffer = prepared.expansion.take();
        }
        let slots = &mut runtime.buffer.as_mut().ok_or_else(refused)?.slots;
        match slots.binary_search_by_key(&identifier, |slot| slot.key) {
            Ok(index) => {
                if classify_original_owner(&slots[index], &prepared.life)?
                    == ExistingOwner::SameOriginal
                {
                    return Ok(());
                }
                if prepared.retired.is_some() {
                    return Err(refused());
                }
                prepared.require_initializing(admission)?;
                let next = Slot {
                    key: identifier,
                    owner: Arc::downgrade(&prepared.life),
                    _fee: prepared._fee.clone(),
                };
                let old = std::mem::replace(&mut slots[index], next);
                prepared.retired = Some(old);
            }
            Err(index) => {
                if slots.len() >= MAX_WRITERS || slots.len() == slots.capacity() {
                    return Err(refused());
                }
                prepared.require_initializing(admission)?;
                // Exact fixed capacity was paid/allocated outside writers. Insertion moves
                // at most one bounded Vec; it never allocates, evicts or refunds a slot.
                slots.insert(
                    index,
                    Slot {
                        key: identifier,
                        owner: Arc::downgrade(&prepared.life),
                        _fee: prepared._fee.clone(),
                    },
                );
            }
        }
        prepared.require_initializing(admission)
    }

    /// Original installed instance proof before taking Root. This neither acquires a lock nor
    /// selects whichever logical writer is newest. Keep `prepared` borrowed through Head I/O.
    pub(in crate::store) fn require_installed_writer(
        &mut self,
        prepared: &PreparedV2WriterRegistration,
    ) -> Result<(), AppError> {
        let (admission, runtime) = self.registration_parts();
        prepared.require_initializing(admission)?;
        let identifier = key(admission.group(), admission.signer());
        runtime.require_installed_observation(&identifier, &prepared.observation)
    }
}

#[cfg(test)]
mod tests;
