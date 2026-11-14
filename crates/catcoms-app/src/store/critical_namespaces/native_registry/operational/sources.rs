//! Original generic native source collection. The facade retains only its
//! permanent negative gate; this owner retains actual affine SourceOwners.
//!
//! Command preparations and workers must not keep this issuer/owner alive
//! across Root writers. Only independently funded runtime custody and prepared
//! key metadata travel to a worker. This collection is not a ready-group DTO.
use super::facade::{IndexOwner, NativeTextVault};
use super::native_ui::UiCapture;
use super::source_admission::Scope;
use super::upgrade_source::owning::SourceWork;
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use catcoms_rt::{CryptoRngCore, MeshTransport, NativeChatSourceOwner};
use std::marker::PhantomData;
use std::sync::Mutex;
use zeroize::Zeroizing;
mod network;
mod retirement;
pub use retirement::{
    NativeTextSourceRetirement, NativeTextSourceRetirementRefusal, NativeTextSourceRetirementStatus,
};

const MAX_SOURCES: usize = 64;
const CONTROL_BYTES: usize = 64 * 1024;
const ENTRY_BYTES: usize = 512 * 1024;

/// Trusted local resource configuration. These fees cover named collection,
/// runtime metadata and network-frame overlap, not opaque SDK or process RSS.
#[derive(Clone)]
pub struct NativeTextSourcePolicy {
    pool: CriticalCatalogReadPool,
    maximum: usize,
    founder: crate::store::FreshV2TextFounderPolicy,
}
impl NativeTextSourcePolicy {
    pub fn new(
        pool: CriticalCatalogReadPool,
        maximum: usize,
        founder: crate::store::FreshV2TextFounderPolicy,
    ) -> Result<Self, AppError> {
        if !(1..=MAX_SOURCES).contains(&maximum)
            || pool.capacity() < Self::required_capacity(maximum)?
        {
            return Err(refused());
        }
        Ok(Self {
            pool,
            maximum,
            founder,
        })
    }
    pub fn required_capacity(maximum: usize) -> Result<usize, AppError> {
        maximum
            .checked_add(2)
            // Runtime custody and each transport attempt have distinct real
            // debits. Allow one completed attempt's metadata to overlap a new
            // attempt; further retained metadata honestly causes Busy.
            .and_then(|n| n.checked_mul(ENTRY_BYTES + 2 * catcoms_net::NATIVE_TCP_LIFETIME_BYTES))
            .and_then(|n| n.checked_add(CONTROL_BYTES))
            .ok_or_else(refused)
    }
    /// Pay before borrowing the native Store or original facade. This contains
    /// no source authority and a losing capture retains its own payment.
    pub fn prepare<T: MeshTransport, R: CryptoRngCore>(
        &self,
    ) -> Result<NativeTextSourceBudget<T, R>, AppError> {
        if runtime_invoice::<T, R>().is_none_or(|n| n > ENTRY_BYTES)
            || std::mem::size_of::<NativeTextSources<T, R>>()
                .checked_add(
                    self.maximum
                        .checked_mul(std::mem::size_of::<Entry<T, R>>())
                        .ok_or_else(refused)?,
                )
                .and_then(|n| n.checked_add(retirement::invoice::<T, R>()))
                .is_none_or(|n| n > CONTROL_BYTES)
        {
            return Err(refused());
        }
        Ok(NativeTextSourceBudget {
            policy: self.clone(),
            metadata: self.pool.try_reserve(CONTROL_BYTES)?,
            _types: PhantomData,
        })
    }
}

pub struct NativeTextSourceBudget<T, R> {
    policy: NativeTextSourcePolicy,
    metadata: CatalogReadReservation,
    _types: PhantomData<fn() -> (T, R)>,
}

/// Infallible source capture, including unsupported/closed originals. Keep the
/// whole value outside every native/Root guard through finish or destruction.
pub struct NativeTextSourcePreparation<T, R> {
    scope: Arc<Scope>,
    work: Option<SourceWork>,
    budget: NativeTextSourceBudget<T, R>,
}

/// Actual checked UI/facade association, not a native window credential itself.
/// Only the real main-window factory may supply this in production. It cannot
/// expose a UI command, source observation, raw Current or positive SDK grant.
pub struct NativeTextSourceCommand {
    scope: Arc<Scope>,
    ui: UiCapture,
}

/// Affine collection owner. A UI lock keeps it alive; Leave/whole retirement
/// explicitly closes source counters BEFORE waiting for the runtime bodies.
pub struct NativeTextSources<T: MeshTransport, R: CryptoRngCore> {
    scope: Arc<Scope>,
    entries: Mutex<Vec<Arc<Entry<T, R>>>>,
    policy: NativeTextSourcePolicy,
    // Original physical custody and control payment remain until entries leave.
    work: SourceWork,
    retirement: Arc<retirement::Control>,
}

struct Entry<T: MeshTransport, R: CryptoRngCore> {
    invocation: [u8; 32],
    request: [u8; 32],
    // Outside the operation mutex. Revocation must never wait for SDK/file IO.
    owner: NativeChatSourceOwner,
    runtime: Arc<RuntimeCell<T, R>>,
}

/// Fee-only custody. Never put a runtime/issuer, drain or registry in a source
/// counter: each of those can own a path back to that very counter.
pub(in crate::store::critical_namespaces::native_registry) struct RuntimeMetadata {
    pool: CriticalCatalogReadPool,
    _debit: CatalogReadReservation,
}

/// All genuine source resources precede their OWN original physical custody
/// and payment. An escaping worker never relies on the collection remaining
/// alive to keep its Current or invoice alive.
struct Runtime<T: MeshTransport, R: CryptoRngCore> {
    founder_policy: crate::store::FreshV2TextFounderPolicy,
    request: Option<NativeTextCreationRequest>,
    device_started: bool,
    driver: Option<crate::store::PreparedFreshV2TextFounder<T, R>>,
    device: Option<catcoms_mls::MlsDevice>,
    transport: Option<T>,
    rng: Option<R>,
    group_rng: Option<R>,
    clock: Option<Box<dyn catcoms_rt::Clock + Send>>,
    network: Option<network::Reservation>,
    work: SourceWork,
    _metadata: Arc<RuntimeMetadata>,
}

/// Bounded request DATA. It does not select a protocol or grant an issuer.
pub struct NativeTextCreationSettings {
    pub display_name: String,
    pub server_name: String,
    pub advertise: String,
    pub relay: String,
    pub rendezvous: String,
    pub is_dm: bool,
}
pub struct NativeTextCreationRequest {
    invocation: [u8; 32],
    digest: [u8; 32],
    settings: NativeTextCreationSettings,
}
impl NativeTextCreationRequest {
    pub fn new(
        invocation: [u8; 32],
        mut settings: NativeTextCreationSettings,
    ) -> Result<Self, AppError> {
        // Freeze the same fallback and byte limits used by BOTH downstream
        // native rows and archive founder intents, before any retained entry
        // or key work. Retries hash the resulting operation, not UI spelling.
        const NAME_LIMIT: usize = super::super::data::MAX_NAME;
        if invocation == [0; 32]
            || settings.display_name.is_empty()
            || settings.display_name.len() > NAME_LIMIT
            || settings.server_name.len() > NAME_LIMIT
            || [&settings.advertise, &settings.relay, &settings.rendezvous]
                .iter()
                .any(|v| v.len() > 4096)
        {
            return Err(refused());
        }
        // Length checks are O(1) and precede both scanning and copying input.
        if settings.server_name.trim().is_empty() {
            settings.server_name = settings.display_name.clone();
        }
        let mut hash = blake3::Hasher::new();
        hash.update(b"Mewtual native founder request v1\0");
        for field in [
            &settings.display_name,
            &settings.server_name,
            &settings.advertise,
            &settings.relay,
            &settings.rendezvous,
        ] {
            hash.update(&(field.len() as u64).to_be_bytes());
            hash.update(field.as_bytes());
        }
        hash.update(&[u8::from(settings.is_dm)]);
        Ok(Self {
            invocation,
            digest: *hash.finalize().as_bytes(),
            settings,
        })
    }
}

/// Caller-owned original RNG/clock. Existing requests ignore these unused
/// candidate inputs without drawing them or replacing the retained originals.
pub struct NativeTextCreationInputs<R> {
    rng: R,
    group_rng: R,
    clock: Box<dyn catcoms_rt::Clock + Send>,
}
impl<R> std::fmt::Debug for NativeTextCreationInputs<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextCreationInputs")
            .finish_non_exhaustive()
    }
}
impl<R> NativeTextCreationInputs<R> {
    pub fn new(rng: R, group_rng: R, clock: Box<dyn catcoms_rt::Clock + Send>) -> Self {
        Self {
            rng,
            group_rng,
            clock,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextCreationRefusal {
    Unavailable,
    Busy,
    Conflict,
    ExistingRequiresRestore,
    Storage,
    DeviceOutcomeUnknown,
    NetworkOutcomeUnknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextCreationProgress {
    /// The device is cached in its original operation. No Birth, runtime row,
    /// ready group, invite or source installation is established by this DATA.
    DevicePrepared,
    /// Original seed, complete public identity and sealed reservation retained.
    /// No socket, Birth, ready group or installation follows from this DATA.
    NetworkPrepared,
    /// Covered Pending row and original immutable network file. Still no SDK
    /// creation, Active row or native backend installation is established.
    PendingCovered,
    /// Actual matching Mesh owner retained; listener/dial acceptance and a
    /// native backend installation are separate, unestablished facts.
    TransportPrepared,
    /// Whole NativeRequired founder retained through its actual warm handoff.
    /// This does not install a native server or expose an invite.
    FounderPrepared,
    /// Covered Active storage DATA from that same retained whole founder.
    /// Actual native command admission/publication remains separate.
    FounderStored,
}

/// Whole finite work, outside every native/registry/operation writer. The
/// runtime loan records/restores first, then admitted key Ends are released.
/// This job carries NO Entry/Sources/SourceOwner or transient issuer pin.
pub struct NativeTextCreationJob<T: MeshTransport, R: CryptoRngCore> {
    loan: RuntimeLoan<T, R>,
    keys: NativeFounderKeys,
    command: NativeTextSourceCommand,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for NativeTextCreationJob<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextCreationJob")
            .finish_non_exhaustive()
    }
}
impl<T: MeshTransport, R: CryptoRngCore> NativeTextCreationJob<T, R> {
    /// Actual native Device primitive. All outputs are cached before a late
    /// command check; an entered unknown outcome never generates another key.
    pub fn prepare_device(
        mut self,
    ) -> Result<NativeTextCreationProgress, NativeTextCreationRefusal> {
        self.keys
            .require_device()
            .map_err(|_| NativeTextCreationRefusal::Unavailable)?;
        if !self.command.scope.is_open() || !self.command.ui.is_current_cached() {
            return Err(NativeTextCreationRefusal::Unavailable);
        }
        let body = self
            .loan
            .body
            .as_mut()
            .ok_or(NativeTextCreationRefusal::Unavailable)?;
        if !body.work.is_live_cached() {
            return Err(NativeTextCreationRefusal::Unavailable);
        }
        // The concrete whole driver owns the SAME device after the input
        // handoff. Its presence cannot authorize another generation, nor may
        // an exact retry mistake that owned handoff for a lost Device result.
        if body.device.is_none() && body.driver.is_none() {
            if body.device_started {
                return Err(NativeTextCreationRefusal::DeviceOutcomeUnknown);
            }
            self.keys
                .enter_device()
                .map_err(|_| NativeTextCreationRefusal::Unavailable)?;
            body.device_started = true;
            #[cfg(test)]
            tests::device_generation();
            body.device = Some(
                catcoms_mls::MlsDevice::generate()
                    .map_err(|_| NativeTextCreationRefusal::DeviceOutcomeUnknown)?,
            );
            #[cfg(test)]
            tests::after_device_cache();
        }
        if !self.command.scope.is_open()
            || !self.command.ui.is_current_cached()
            || !body.work.is_live_cached()
        {
            return Err(NativeTextCreationRefusal::Unavailable);
        }
        self.keys
            .require_device()
            .map_err(|_| NativeTextCreationRefusal::Unavailable)?;
        self.loan.recorded = true;
        Ok(NativeTextCreationProgress::DevicePrepared)
    }
}

struct RuntimeSlot<T: MeshTransport, R: CryptoRngCore> {
    body: Option<Runtime<T, R>>,
    running: bool,
    interrupted: bool,
    poisoned: bool,
    // Monotone and serialized with borrowing the whole body. A preparation
    // captured before close cannot borrow a body restored by cleanup's Busy.
    retiring: bool,
    retirement: retirement::BodyState,
}
struct RuntimeCell<T: MeshTransport, R: CryptoRngCore> {
    state: Mutex<RuntimeSlot<T, R>>,
    returned: tokio::sync::Notify,
}

/// A finite worker moves its body OUT of the control mutex. SDK/file effects
/// and their destructors never run under that writer. Unpolled drop or unwind
/// restores the exact mutated body before the outer prepared Keys retire.
struct RuntimeLoan<T: MeshTransport, R: CryptoRngCore> {
    body: Option<Runtime<T, R>>,
    cell: Arc<RuntimeCell<T, R>>,
    recorded: bool,
}
impl<T: MeshTransport, R: CryptoRngCore> Drop for RuntimeLoan<T, R> {
    fn drop(&mut self) {
        let mut body = self.body.take();
        {
            let mut slot = self.cell.state.lock().unwrap_or_else(|e| e.into_inner());
            if slot.body.is_none() {
                slot.body = body.take();
            } else {
                // This is unreachable with the unique running bit. Preserve the
                // existing body and refuse all future work; any losing whole
                // runtime is destroyed only AFTER the control writer leaves.
                slot.poisoned = true;
            }
            slot.running = false;
            slot.interrupted = !self.recorded;
        }
        self.cell.returned.notify_waiters();
        drop(body);
    }
}

/// Named metadata and bounded network-buffer overlap only. The actual generic
/// body/inputs and worker envelopes are included, not just the Entry Arc. SDK
/// backing/serialization has the existing separate founder/host invoices.
fn runtime_invoice<T: MeshTransport, R: CryptoRngCore>() -> Option<usize> {
    [
        2 * std::mem::size_of::<Runtime<T, R>>(),
        std::mem::size_of::<RuntimeCell<T, R>>(),
        std::mem::size_of::<RuntimeLoan<T, R>>(),
        std::mem::size_of::<NativeFounderKeys>(),
        std::mem::size_of::<NativeTextSourceCommand>(),
        2 * std::mem::size_of::<Entry<T, R>>(),
        // Original frame + Pending copy + retained activation dependency,
        // with bounded ciphertext/plaintext validation and reread overlap.
        6 * 64 * 1024,
        // Actual TCP wrappers use their OWN per-attempt debit. These inline
        // preparation/return envelopes remain part of the runtime account.
        2 * std::mem::size_of::<catcoms_net::NativeTcpPreparation>(),
        48 * 1024, // bounded request/control/return envelopes and Arc headers
    ]
    .into_iter()
    .try_fold(0usize, |sum, size| sum.checked_add(size))
}

macro_rules! opaque_source_debug {
    ($($ty:ty),+) => {$(impl std::fmt::Debug for $ty {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
opaque_source_debug!(
    NativeTextSourcePolicy,
    NativeTextSourceCommand,
    NativeTextCreationSettings,
    NativeTextCreationRequest
);
impl<T, R> std::fmt::Debug for NativeTextSourceBudget<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextSourceBudget")
            .finish_non_exhaustive()
    }
}
impl<T, R> std::fmt::Debug for NativeTextSourcePreparation<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextSourcePreparation")
            .finish_non_exhaustive()
    }
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for NativeTextSources<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextSources").finish_non_exhaustive()
    }
}

impl NativeTextVault {
    /// Pure capture only; policy/control were prepaid outside the Store guard.
    pub fn capture_native_sources<T: MeshTransport, R: CryptoRngCore>(
        &self,
        budget: NativeTextSourceBudget<T, R>,
    ) -> NativeTextSourcePreparation<T, R> {
        NativeTextSourcePreparation {
            scope: self.sources.observe(),
            work: match &self.index {
                IndexOwner::Version3(index) => index.capture_source_work(),
                IndexOwner::Version2(_) => None,
            },
            budget,
        }
    }

    /// Caller keeps its capture in an OUTER slot on refusal. The move after
    /// validation is infallible, so no final paid capture is dropped under the
    /// caller's native Store/commit guard.
    pub fn bind_native_source_command(
        &self,
        capture: &mut Option<UiCapture>,
    ) -> Result<NativeTextSourceCommand, AppError> {
        let ui = capture.as_ref().ok_or_else(refused)?;
        self.require_ui(ui)?;
        if !self.sources.observe().is_open() || !matches!(self.index, IndexOwner::Version3(_)) {
            return Err(refused());
        }
        Ok(NativeTextSourceCommand {
            scope: self.sources.observe(),
            ui: capture.take().expect("validated outer UI capture"),
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSourcePreparation<T, R> {
    /// Finish outside guards. Allocations precede the once-only claim; no
    /// failed allocator/policy preparation can issue another source collection.
    pub fn finish(self) -> Result<NativeTextSources<T, R>, Box<Self>> {
        if self.work.as_ref().is_none_or(|work| !work.is_live_cached()) || !self.scope.is_open() {
            return Err(Box::new(self));
        }
        let entries = Vec::with_capacity(self.budget.policy.maximum);
        if self.scope.claim_once().is_err() {
            return Err(Box::new(self));
        }
        if !self.scope.is_open() || self.work.as_ref().is_none_or(|work| !work.is_live_cached()) {
            return Err(Box::new(self));
        }
        let Self {
            scope,
            work,
            budget,
        } = self;
        Ok(NativeTextSources {
            scope,
            entries: Mutex::new(entries),
            policy: budget.policy,
            work: work.expect("checked original source work"),
            retirement: Arc::new(retirement::Control::new(budget.metadata)),
        })
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextSources<T, R> {
    /// Preparation may read the original native index and allocate; call only
    /// outside EVERY native/Root guard. Lookup precedes device/key generation.
    /// The native checked factory must already have captured the original UI
    /// and actual mount before its first scheduling/authorization wait.
    pub fn prepare_creation(
        &self,
        command: NativeTextSourceCommand,
        request: NativeTextCreationRequest,
        inputs: NativeTextCreationInputs<R>,
    ) -> Result<NativeTextCreationJob<T, R>, NativeTextCreationRefusal> {
        use NativeTextCreationRefusal as Error;
        self.require_command(&command)
            .map_err(|_| Error::Unavailable)?;
        let existing = {
            let entries = self.entries.try_lock().map_err(|_| Error::Busy)?;
            entries
                .iter()
                .find(|entry| entry.invocation == request.invocation)
                .cloned()
        };
        let mut candidate = None;
        let selected = if let Some(entry) = existing {
            if entry.request != request.digest {
                return Err(Error::Conflict);
            }
            entry
        } else {
            self.work
                .require_native_founder_configuration()
                .map_err(|_| Error::Unavailable)?;
            if self
                .work
                .lookup_creation(request.invocation, request.digest)
                .map_err(|_| Error::Storage)?
                .is_some()
            {
                return Err(Error::ExistingRequiresRestore);
            }
            // Fee-only source counter custody, never a runtime/cell/issuer.
            let metadata = Arc::new(RuntimeMetadata {
                pool: self.policy.pool.clone(),
                _debit: self
                    .policy
                    .pool
                    .try_reserve(ENTRY_BYTES)
                    .map_err(|_| Error::Busy)?,
            });
            let work = self
                .work
                .for_paid_runtime(&metadata)
                .ok_or(Error::Unavailable)?;
            let owner =
                NativeChatSourceOwner::for_trusted_source(&command.ui.command, metadata.clone())
                    .map_err(|_| Error::Unavailable)?;
            let entry = Arc::new(Entry {
                invocation: request.invocation,
                request: request.digest,
                owner,
                runtime: Arc::new(RuntimeCell {
                    state: Mutex::new(RuntimeSlot {
                        body: Some(Runtime {
                            founder_policy: self.policy.founder,
                            request: Some(request),
                            device_started: false,
                            driver: None,
                            device: None,
                            transport: None,
                            rng: Some(inputs.rng),
                            group_rng: Some(inputs.group_rng),
                            clock: Some(inputs.clock),
                            network: None,
                            work,
                            _metadata: metadata,
                        }),
                        running: false,
                        interrupted: false,
                        poisoned: false,
                        retiring: false,
                        retirement: retirement::BodyState::new(),
                    }),
                    returned: tokio::sync::Notify::new(),
                }),
            });
            candidate = Some(entry);
            let result = {
                let mut entries = self.entries.try_lock().map_err(|_| Error::Busy)?;
                let current = candidate.as_ref().unwrap();
                if !self.scope.is_open() {
                    Err(Error::Unavailable)
                } else if let Some(old) = entries
                    .iter()
                    .find(|old| old.invocation == current.invocation)
                {
                    if old.request != current.request {
                        Err(Error::Conflict)
                    } else {
                        Ok(old.clone())
                    }
                } else if entries.len() >= self.policy.maximum {
                    Err(Error::Busy)
                } else {
                    entries.push(current.clone());
                    Ok(current.clone())
                }
            };
            result?
        };
        // Source-owner/issuer pins are transient and leave BEFORE runtime/Root
        // writers. Preparing cuts here does not enter a primitive or spend it.
        let prepared = NativeFounderKeys::prepare(&selected.owner, &command.ui.command);
        let cell = selected.runtime.clone();
        drop(selected);
        drop(candidate); // a racing unused candidate may own real files/payment
        let keys = prepared.map_err(|_| Error::Unavailable)?;
        #[cfg(test)]
        retirement::tests::after_key_preparation();
        let body = {
            let mut state = cell.state.try_lock().map_err(|_| Error::Busy)?;
            if state.retiring || !self.scope.is_open() {
                return Err(Error::Unavailable);
            }
            if state.running || state.poisoned {
                return Err(Error::Busy);
            }
            let body = state.body.take().ok_or(Error::Unavailable)?;
            state.running = true;
            body
        };
        Ok(NativeTextCreationJob {
            loan: RuntimeLoan {
                body: Some(body),
                cell,
                recorded: false,
            },
            keys,
            command,
        })
    }
    /// SC gate first. The short entries guard only visits affine source owners;
    /// it never takes an operation writer or drops/awaits any runtime here.
    pub fn close_admission(&self) {
        self.scope.close_first();
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        for entry in entries.iter() {
            entry.owner.close();
        }
    }
    fn require_command(&self, command: &NativeTextSourceCommand) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.scope, &command.scope)
            || !self.scope.is_open()
            || !self.work.is_live_cached()
            || !command.ui.is_current_cached()
        {
            return Err(refused());
        }
        Ok(())
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Drop for NativeTextSources<T, R> {
    fn drop(&mut self) {
        self.close_admission();
    }
}

#[cfg(test)]
mod tests;
