//! Immutable process-owned native/browser channel setup. A lost caller cannot reset its
//! selected key, ephemeral agreement or physical counters. Pending, Ready and closed fences
//! share64 slots; uncertain rows are never evicted. Collection requires actual independent
//! owners to drain and the original lease to die permanently; serials never reset on reuse.
use super::channel::bootstrap::{Prepared, PublicSetup};
use super::channel::{Refused, Session, SessionIdentity};
use crate::voice_authority::SelectedHandoffOwner;
use crate::{
    check_voice_activation_generation, check_voice_binding, require_unlocked_session, AppState,
};
use catcoms_rt::CryptoRngCore;
use catcoms_sync::voice_v2::resources::{Credit, ProcessRetainedCredit, ResourceKind, Resources};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

mod admission_mode;
#[cfg(test)]
mod diagnostics;
pub(super) use admission_mode::observation_layout_bytes as admission_mode_layout_bytes;
use admission_mode::AdmissionPin;

const ROWS: usize = 64;
const INDEX_BYTES: usize = 4096;
const ROW_BYTES: usize = 8192;

struct Index {
    serial: u128,
    resources: Arc<Resources>,
    rows: [Option<Arc<Row>>; ROWS],
    _credit: ProcessRetainedCredit,
}
struct Row {
    selected: Arc<SelectedHandoffOwner>,
    device: [u8; 32],
    session_id: [u8; 16],
    slot: u8,
    state: Mutex<State>,
    projection_busy: AtomicBool,
    _credit: Arc<Credit>,
}
struct State {
    pending: bool,
    // Close-first cleanup must fence producers before checking for outstanding actions.
    // Keep Control routing independently available until those actual owners drain.
    effects_closed: bool,
    closed: bool,
    session: Option<Arc<Session>>,
    public: Option<Arc<SetupOutput>>,
    // Original native ECDH credit continues through browser setup/outgoing SDK promises.
    // Lease revocation and caller loss are NOT evidence that this work has drained.
    browser_setup: Option<Credit>,
    retirement: Option<Retirement>,
    // Mode selection and setup ACK MUST share this mutex. Selection is sticky;
    // cancellation or a lost reply cannot restore an old automatic producer.
    admission: Option<AdmissionPin>,
}
/// Direct Session/header/reply and public-output owners can outlive Row references. Paid
/// weak witnesses keep their control blocks attributable until actual strong owners drain.
/// Closed rows cannot reinstall a strong owner from these witnesses.
struct Retirement {
    session: Weak<Session>,
    public: Weak<SetupOutput>,
    // Fixed original identity only; no Session/key/credit is kept alive by final history.
    identity: Option<Arc<SessionIdentity>>,
}
/// Native-only evidence that the ORIGINAL closed channel has no remaining Session, header,
/// reply, setup ticket or public-output owner and its lease is permanently dead. Only the
/// registry's locked last-owner collection can construct this value. Renderer identifiers,
/// FinalACK, a dead lease alone, and a returned close reply cannot mint this evidence.
/// Retain the paid Row until the ledger has observed its original provenance; drop it only
/// outside both registries' guards. This proves channel collection, not action/SDK drainage.
pub(super) struct CollectedChannel {
    // Declaration order is custody: this independent token must drop BEFORE the paid Row.
    // Otherwise its last allocation can outlive bootstrap's metadata credit when no action
    // Phase or Session remains. The Row's own retirement token is paid until _credit drops.
    identity: Option<Arc<SessionIdentity>>,
    row: Arc<Row>,
}
impl CollectedChannel {
    pub(super) fn identity(&self) -> Option<&Arc<SessionIdentity>> {
        // This Row is already detached from the registry and cannot be mutated. A cloned
        // token in CollectedChannel avoids returning a reference through the State guard.
        self.identity.as_ref()
    }
    pub(super) fn selected(&self) -> &SelectedHandoffOwner {
        &self.row.selected
    }
}
/// Exact retries share ONE immutable projection/allocation. Every returned owner retains the
/// original row payment even after the registry is destroyed; a caller cannot clone raw public
/// buffers without separately paying its copy. Future IPC serialization needs its own flight.
pub(crate) struct SetupOutput {
    public: PublicSetup,
    _credit: Arc<Credit>,
}
impl SetupOutput {
    pub(crate) fn public(&self) -> &PublicSetup {
        &self.public
    }
}
pub(crate) struct PublicProjection {
    output: Arc<SetupOutput>,
}
impl serde::Serialize for PublicProjection {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // No intermediate native heap String. The fixed reply is public metadata only; the
        // pinned Tauri serializer owns the subsequent JSON String/HTTP response allocation.
        let mut hex = [0u8; 2048];
        let transcript = self.output.public().transcript();
        let used = transcript.len() * 2;
        hex::encode_to_slice(transcript, &mut hex[..used]).map_err(serde::ser::Error::custom)?;
        let text = std::str::from_utf8(&hex[..used]).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(text)
    }
}
/// One original payment/serial position for EVERY successful setup variant. Closed and
/// Pending replies allocate the same bounded JSON as Ready, so they cannot be bare Strings.
pub(super) struct MetadataProjection {
    row: Arc<Row>,
}
impl MetadataProjection {
    /// Original registry allocation, including after End. This is historical provenance,
    /// not a live grant upgrade or permission to construct replacement work.
    pub(super) fn selected(&self) -> &SelectedHandoffOwner {
        &self.row.selected
    }

    /// Conservative complete original Row/preparation/output inventory plus ONE serial
    /// metadata reply. General allocator, framework copies after handoff and OS overhead
    /// are outside this application-owner bound. Refuse rather than raise its8KiB payment.
    pub(super) fn require_reply_memory(&self, reply_bytes: usize) -> Result<(), Refused> {
        let arc_words = 2 * std::mem::size_of::<usize>();
        let retained = std::mem::size_of::<Row>()
            + std::mem::size_of::<SessionIdentity>()
            + arc_words * 2
            + std::mem::size_of::<Ticket>()
            + std::mem::size_of::<SetupOutput>()
            + std::mem::size_of::<Credit>()
            + arc_words * 2
            + std::mem::size_of::<Arc<Session>>()
            + std::mem::size_of::<Begin>()
            + std::mem::size_of::<Status>()
            + std::mem::size_of::<PublicProjection>()
            + std::mem::size_of::<MetadataProjection>();
        let bytes = retained.checked_add(reply_bytes).ok_or(Refused::Capacity)?;
        #[cfg(test)]
        eprintln!("native work metadata row/reply inventory {bytes}/{ROW_BYTES}");
        if bytes > ROW_BYTES {
            return Err(Refused::Capacity);
        }
        Ok(())
    }

    pub(super) fn transcript(&self, output: Arc<SetupOutput>) -> Result<PublicProjection, Refused> {
        if !Arc::ptr_eq(&self.row._credit, &output._credit) {
            return Err(Refused::Binding);
        }
        Ok(PublicProjection { output })
    }
}
impl Drop for MetadataProjection {
    fn drop(&mut self) {
        self.row.projection_busy.store(false, Ordering::Release);
    }
}
/// The dormant wrapper belongs to AppState. Allocate/pay its fixed slot index lazily, from
/// the actual shared native pool, before any row or bootstrap becomes externally observable.
#[derive(Default)]
pub(crate) struct Registry {
    index: Mutex<Option<Box<Index>>>,
    stopping: AtomicBool,
}
pub(crate) enum Status {
    Pending,
    Closed { native_setup_drained: bool },
    Ready(Arc<SetupOutput>),
}
#[allow(clippy::large_enum_variant)]
enum Begin {
    Existing(Status),
    Work(Ticket),
}
struct Ticket {
    row: Arc<Row>,
    prepared: Option<Prepared>,
    installed: bool,
}

impl Registry {
    /// Cleanup-only routing under the caller's ORIGINAL UI commit. End/lock may revoke the
    /// call lease while its authenticated Control session still has final delivery work.
    /// Do not reuse first-effect require_open here, and do not weaken it for factories.
    /// Release both registry guards before touching Ledger or retiring any Root owner.
    /// Native producer preflight also uses this pure provenance check before creating its
    /// task. That use grants no effect authority: final live-Route/selection/UI/grant checks
    /// remain mandatory after its asynchronous preparation.
    pub(super) fn require_cleanup_route(&self, route: &Route) -> Result<(), Refused> {
        let held = self.index.lock().unwrap_or_else(|error| error.into_inner());
        if self.stopping.load(Ordering::Acquire)
            || !held.as_ref().is_some_and(|index| {
                index
                    .rows
                    .iter()
                    .flatten()
                    .any(|row| Arc::ptr_eq(row, &route.row))
            })
        {
            return Err(Refused::Binding);
        }
        let state = route
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed
            || state.pending
            || !state
                .session
                .as_ref()
                .is_some_and(|session| Arc::ptr_eq(session, &route.session))
            || !state
                .public
                .as_ref()
                .is_some_and(|public| Arc::ptr_eq(public, &route.public))
        {
            return Err(Refused::Binding);
        }
        Ok(())
    }
    /// Irreversibly stop first effects on the exact original channel BEFORE checking its
    /// outstanding ledger work. A Capacity refusal must not let maintenance keep creating
    /// unseen children while cleanup scans. Control routing stays open for actual cleanup.
    /// Caller holds the SAME UI commit as final publication; release these registry guards
    /// before consulting the ledger or destroying any Root owner.
    fn close_effects(&self, owner: &str, executor: [u8; 32]) -> Option<Arc<SelectedHandoffOwner>> {
        self.index
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()?
            .rows
            .iter()
            .flatten()
            .find(|row| row.selected.id() == owner && row.selected.executor() == executor)
            .map(|row| {
                row.state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .effects_closed = true;
                row.selected.clone()
            })
    }

    #[cfg(test)]
    fn original_selected(
        &self,
        owner: &str,
        executor: [u8; 32],
    ) -> Option<Arc<SelectedHandoffOwner>> {
        self.index
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()?
            .rows
            .iter()
            .flatten()
            .find(|row| row.selected.id() == owner && row.selected.executor() == executor)
            .map(|row| row.selected.clone())
    }

    /// Exact public projection after ensure, outside Coordinator. If collection won this
    /// interval, refuse rather than create an unpaid historical reply. Holding the actual Row
    /// keeps its payment through delayed serialization after End/collection. The single busy
    /// gate is shared across Ready/Pending/Closed; variants cannot overlap their allocations.
    /// Stock Tauri/WebView copies after body handoff remain outside the application copy bound.
    pub(super) fn project(
        &self,
        owner: &str,
        executor: [u8; 32],
    ) -> Result<MetadataProjection, Refused> {
        let held = self.index.lock().unwrap_or_else(|error| error.into_inner());
        let row =
            held.as_ref()
                .and_then(|index| {
                    index.rows.iter().flatten().find(|row| {
                        row.selected.id() == owner && row.selected.executor() == executor
                    })
                })
                .ok_or(Refused::Binding)?;
        row.projection_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Refused::Capacity)?;
        Ok(MetadataProjection { row: row.clone() })
    }
    /// Bind the fixed service infrastructure to the SAME native process pool before opening
    /// a listener. This may allocate the separately prepaid empty index, never a group/row.
    pub(super) fn bind_pool(&self, resources: &Arc<Resources>) -> Result<(), Refused> {
        let mut held = self.index.lock().unwrap_or_else(|error| error.into_inner());
        if self.stopping.load(Ordering::Acquire) {
            return Err(Refused::Binding);
        }
        Self::index(&mut held, resources)?;
        Ok(())
    }

    /// An unauthenticated slot is only a bounded index hint. Resolve the original immutable
    /// Session/public allocation; lookup cannot create or restart setup, even after End.
    /// Closed history can answer status only, without granting any current effect authority.
    pub(super) fn route(&self, slot: u8) -> Result<Route, Refused> {
        let slot = slot
            .checked_sub(1)
            .filter(|slot| *slot < 64)
            .ok_or(Refused::Binding)?;
        let held = self.index.lock().unwrap_or_else(|error| error.into_inner());
        if self.stopping.load(Ordering::Acquire) {
            return Err(Refused::Binding);
        }
        let row = held
            .as_ref()
            .and_then(|index| index.rows[usize::from(slot)].as_ref())
            .ok_or(Refused::Binding)?;
        let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(Refused::Binding);
        }
        Ok(Route {
            row: row.clone(),
            session: state.session.clone().ok_or(Refused::Binding)?,
            public: state.public.clone().ok_or(Refused::Binding)?,
        })
    }

    fn index<'a>(
        held: &'a mut Option<Box<Index>>,
        resources: &Arc<Resources>,
    ) -> Result<&'a mut Index, Refused> {
        if held.is_none() {
            let credit = resources
                .charge_process_retained(INDEX_BYTES)
                .map_err(|_| Refused::Capacity)?;
            // Charge before constructing the fixed index/Box. No synthetic group consumes a
            // real group record, and this process payment never lends crypto/data capacity.
            *held = Some(Box::new(Index {
                serial: 0,
                resources: resources.clone(),
                rows: std::array::from_fn(|_| None),
                _credit: credit,
            }));
        }
        let index = held.as_deref_mut().expect("installed paid index");
        if !Arc::ptr_eq(&index.resources, resources) {
            return Err(Refused::Binding);
        }
        Ok(index)
    }

    fn begin(
        &self,
        selected: Arc<SelectedHandoffOwner>,
        device: [u8; 32],
    ) -> Result<Begin, Refused> {
        let resources = selected.resources();
        let mut held = self.index.lock().unwrap_or_else(|error| error.into_inner());
        if self.stopping.load(Ordering::Acquire) {
            return Err(Refused::Binding);
        }
        let index = Self::index(&mut held, &resources)?;
        if let Some(row) = index.rows.iter().flatten().find(|row| {
            row.selected.id() == selected.id() && row.selected.executor() == selected.executor()
        }) {
            if !Arc::ptr_eq(&row.selected, &selected) {
                return Err(Refused::Binding);
            }
            let status = row.status();
            if !matches!(status, Status::Closed { .. }) && row.device != device {
                return Err(Refused::Binding);
            }
            // Exact recovery is before every new Session/row/Crypto payment and before RNG.
            return Ok(Begin::Existing(status));
        }
        if !selected.lease().is_live() {
            return Err(Refused::Binding);
        }
        let slot = index
            .rows
            .iter()
            .position(Option::is_none)
            .ok_or(Refused::Capacity)?;
        let serial = index.serial.checked_add(1).ok_or(Refused::Exhausted)?;
        let credit = resources
            .charge(
                selected.binding().scope.group,
                ROW_BYTES,
                ResourceKind::Retained,
            )
            .map_err(|_| Refused::Capacity)?;
        let prepared = Prepared::new(
            selected.clone(),
            serial.to_be_bytes(),
            (slot + 1) as u8,
            device,
        )?;
        // Proven-unstarted refusal above has no row/serial effect. Once this Pending becomes
        // visible, its ticket always finishes or latches a closed fence, including unwind.
        let row = Arc::new(Row {
            selected,
            device,
            session_id: serial.to_be_bytes(),
            slot: (slot + 1) as u8,
            state: Mutex::new(State {
                pending: true,
                effects_closed: false,
                closed: false,
                session: None,
                public: None,
                browser_setup: None,
                retirement: None,
                admission: None,
            }),
            projection_busy: AtomicBool::new(false),
            _credit: Arc::new(credit),
        });
        index.serial = serial;
        index.rows[slot] = Some(row.clone());
        Ok(Begin::Work(Ticket {
            row,
            prepared: Some(prepared),
            installed: false,
        }))
    }

    /// Existing exact cleanup is available while locked. A close-first fence may only be
    /// created from a currently accepted selected owner; an unknown retired ID allocates none.
    /// This local setup settlement is separate from selected join-key generation settlement.
    fn close(
        &self,
        owner: &str,
        executor: [u8; 32],
        selected: Option<Arc<SelectedHandoffOwner>>,
    ) -> Result<Status, Refused> {
        self.close_row(owner, executor, selected)
            .map(|row| row.status())
    }
    /// Hold the actual Row across post-guard retirement; a collector cannot turn an admitted
    /// exact close into an ambiguous missing-row error between the two operations.
    fn close_row(
        &self,
        owner: &str,
        executor: [u8; 32],
        selected: Option<Arc<SelectedHandoffOwner>>,
    ) -> Result<Arc<Row>, Refused> {
        let mut held = self.index.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(row) = held.as_ref().and_then(|index| {
            index
                .rows
                .iter()
                .flatten()
                .find(|row| row.selected.id() == owner && row.selected.executor() == executor)
        }) {
            row.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .closed = true;
            return Ok(row.clone());
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(Refused::Binding);
        }
        let selected = selected
            .filter(|selected| {
                selected.id() == owner
                    && selected.executor() == executor
                    && selected.lease().is_live()
            })
            .ok_or(Refused::Binding)?;
        let resources = selected.resources();
        let index = Self::index(&mut held, &resources)?;
        let slot = index
            .rows
            .iter()
            .position(Option::is_none)
            .ok_or(Refused::Capacity)?;
        let serial = index.serial.checked_add(1).ok_or(Refused::Exhausted)?;
        let credit = resources
            .charge(
                selected.binding().scope.group,
                ROW_BYTES,
                ResourceKind::Retained,
            )
            .map_err(|_| Refused::Capacity)?;
        let row = Arc::new(Row {
            selected,
            device: [0; 32],
            session_id: serial.to_be_bytes(),
            slot: (slot + 1) as u8,
            state: Mutex::new(State {
                pending: false,
                effects_closed: true,
                closed: true,
                session: None,
                public: None,
                browser_setup: None,
                retirement: None,
                admission: None,
            }),
            projection_busy: AtomicBool::new(false),
            _credit: Arc::new(credit),
        });
        index.serial = serial;
        index.rows[slot] = Some(row.clone());
        Ok(row)
    }
    /// Only the trusted setup-settlement adapter may call this after SDK drainage, or the
    /// Status route after complete AEAD/session/transcript validation. Drop outside all
    /// Index/Row/Coordinator guards; unknown history supplies no drainage proof.
    fn retire_setup(&self, owner: &str, executor: [u8; 32]) -> Result<(), Refused> {
        let row = {
            let held = self.index.lock().unwrap_or_else(|error| error.into_inner());
            held.as_ref()
                .and_then(|index| {
                    index.rows.iter().flatten().find(|row| {
                        row.selected.id() == owner && row.selected.executor() == executor
                    })
                })
                .cloned()
                .ok_or(Refused::Binding)?
        };
        row.retire_setup()
    }

    /// Bounded collection requires an irreversibly dead original lease, no pending setup,
    /// and actual drainage of ALL detached owners. Keep the monotonic serial across reuse.
    pub(crate) fn collect_retired(&self, ledger: &super::ledger::Ledger) {
        let mut removed: [Option<Arc<Row>>; ROWS] = std::array::from_fn(|_| None);
        {
            let mut held = self.index.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(index) = held.as_mut() {
                for (slot, retired) in index.rows.iter_mut().zip(&mut removed) {
                    let reclaim = slot.as_ref().is_some_and(|row| {
                        if row.selected.lease().is_live() || Arc::strong_count(row) != 1 {
                            return false;
                        }
                        let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
                        state.closed
                            && !state.pending
                            && state.retirement.as_ref().is_some_and(|owners| {
                                owners.session.strong_count() == 0
                                    && owners.public.strong_count() == 0
                            })
                    });
                    if reclaim {
                        *retired = slot.take();
                    }
                }
            }
        }
        // Includes original selected snapshot and paid weak control blocks. No resource/
        // output destructor may execute under Index/State (or a caller's Coordinator guard).
        // Registry and Ledger locks NEVER overlap. Persist the evidence even when a delayed
        // ledger handle still retains history; a later sweep must not need a second witness.
        for row in removed.into_iter().flatten() {
            let identity = row
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retirement
                .as_ref()
                .and_then(|owners| owners.identity.clone());
            ledger.channel_collected(CollectedChannel { row, identity });
        }
        // Always sweep, including an empty/uninitialized registry. The original one-shot
        // witness may have arrived before the last cleanup/metadata handle actually dropped.
        ledger.collect_retired();
    }

    /// Process shutdown only: close every existing first-effect fence before collecting any
    /// physical/native position. No channel may be created after this Index-ordered transition.
    /// The service is the sole caller/collector, after its connection future has dropped.
    ///
    /// Unlike ordinary channel collection, this deliberately terminates Control routing:
    /// the renderer/listener is stopping. It does NOT acknowledge browser drainage, retire an
    /// uncertain purpose, compact an action row, or claim that all Workflow credits returned.
    pub(super) async fn stop_and_wait_for_physical(&self) {
        {
            let held = self.index.lock().unwrap_or_else(|error| error.into_inner());
            self.stopping.store(true, Ordering::Release);
            if let Some(index) = held.as_ref() {
                for row in index.rows.iter().flatten() {
                    row.state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .closed = true;
                }
            }
        }
        // One paid Session/waiter at a time; no heap registry, detached waiter or 64-owner
        // snapshot. Native producer tasks retain the canonical Route as well as their
        // independently paid native Session hold; wait_idle collects both ownership kinds.
        // Retired Sessions can have no Row references while a reply/worker still
        // owns them, so consult the original weak witness as well as live State.session.
        for slot in 0..ROWS {
            let session = {
                let held = self.index.lock().unwrap_or_else(|error| error.into_inner());
                held.as_ref()
                    .and_then(|index| index.rows[slot].as_ref())
                    .and_then(|row| {
                        let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
                        state.session.clone().or_else(|| {
                            state
                                .retirement
                                .as_ref()
                                .and_then(|retired| retired.session.upgrade())
                        })
                    })
            };
            // No Index/Row/UI commit/Coordinator guard may cross this await. Admitted
            // actor work still needs those owners to finish while shutdown is waiting.
            if let Some(session) = session {
                session.close_admission();
                session.wait_idle().await;
            }
        }
    }
}
/// Native-only immutable control routing. Retain the actual row, Session and paid public
/// output across IO; copied session IDs alone cannot create this owner or authorize actions.
#[derive(Clone)]
pub(super) struct Route {
    row: Arc<Row>,
    session: Arc<Session>,
    public: Arc<SetupOutput>,
}
impl Route {
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_slot(&self) -> u8 {
        self.row.slot
    }

    /// Start automatic native work only after the original authenticated setup ACK has
    /// retired browser setup custody. A bound socket/Ready projection alone is insufficient;
    /// starting the15s pair budget during a stalled WebView bootstrap would strand it.
    pub(super) fn native_ready(&self) -> bool {
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        !state.closed
            && !state.effects_closed
            && !state.pending
            && state.browser_setup.is_none()
            && state.admission.is_none()
            && self.row.selected.lease().is_live()
    }
    /// Only this immutable native row supplies selection. Request labels are lookup hints,
    /// never a substitute for the allocation which owns this Session and its payments.
    pub(super) fn selected(&self) -> &Arc<SelectedHandoffOwner> {
        &self.row.selected
    }
    /// Pure first-effect fence. Repeat under the SAME original UI commit as close_row;
    /// otherwise a retained Route could start actor work after trusted channel collection.
    /// Coordinator -> Row is the existing lock order. This grants no Root authorization.
    pub(super) fn require_open(&self) -> Result<(), Refused> {
        self.with_open(|| ())
    }
    /// Keep the original route fence held through a synchronous native first effect. The
    /// callback must not await, sample Clock, reacquire Coordinator/Row, or destroy owners
    /// whose Drop can call Root. Existing lock order is UI commit -> Coordinator -> Row.
    pub(super) fn with_open<T>(&self, commit: impl FnOnce() -> T) -> Result<T, Refused> {
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed
            || state.effects_closed
            || state.pending
            || !self.row.selected.lease().is_live()
        {
            return Err(Refused::Binding);
        }
        let result = commit();
        drop(state);
        Ok(result)
    }
    /// Final engine admission has sampled Root's original Clock. Preserve UI -> Coordinator
    /// -> Row ordering but refuse contention here; waiting would reuse a stale deadline.
    pub(super) fn try_with_open<T>(
        &self,
        commit: impl FnOnce() -> Result<T, catcoms_sync::voice_v2::VoiceError>,
    ) -> Result<T, catcoms_sync::voice_v2::VoiceError> {
        use catcoms_sync::voice_v2::VoiceError;
        let state = match self.row.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        if state.closed
            || state.effects_closed
            || state.pending
            || !self.row.selected.lease().is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        commit()
    }
    #[cfg(test)]
    pub(super) fn fixture_lock_route(&self) -> impl Drop + '_ {
        self.row.state.lock().unwrap()
    }
    pub(super) fn session(&self) -> &Arc<Session> {
        &self.session
    }
    pub(super) fn session_id(&self) -> [u8; 16] {
        self.row.session_id
    }
    pub(super) fn hash(&self) -> [u8; 32] {
        self.public.public.hash
    }
    /// Complete authenticated body validation is the prerequisite. Header MAC, bad AEAD,
    /// timeout and End cannot acknowledge the browser's original setup work.
    pub(super) fn acknowledge_setup(&self) -> Result<(), Refused> {
        self.row.retire_setup()
    }
    #[cfg(test)]
    pub(super) fn fixture_browser_setup_held(&self) -> bool {
        self.row.state.lock().unwrap().browser_setup.is_some()
    }
    /// A snapshot of NATIVE setup state only. It neither proves browser drainage nor lets a
    /// late Ready response revive a retired call, room, join or action.
    pub(super) fn snapshot(&self) -> [u8; 3] {
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let closed = state.closed || state.effects_closed || !self.row.selected.lease().is_live();
        [1, if closed { 2 } else { 1 }, u8::from(!state.pending)]
    }
}
impl Row {
    /// Trusted executor closure after all real browser SDK/IO/cancel work and application
    /// buffers drained. Existing streams/replies retain their actual Session's bytes/slot
    /// until their own last drop; detached public outputs remain independently paid too.
    fn retire_channel(&self) -> Result<(), Refused> {
        let retired = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if !state.closed {
                return Err(Refused::Binding);
            }
            if state.pending {
                return Err(Refused::Capacity);
            }
            if state.retirement.is_none() {
                state.retirement = Some(Retirement {
                    identity: state
                        .session
                        .as_ref()
                        .map(|session| session.identity().clone()),
                    session: state
                        .session
                        .as_ref()
                        .map(Arc::downgrade)
                        .unwrap_or_default(),
                    public: state
                        .public
                        .as_ref()
                        .map(Arc::downgrade)
                        .unwrap_or_default(),
                });
            }
            (
                state.session.take(),
                state.public.take(),
                state.browser_setup.take(),
            )
        };
        drop(retired);
        Ok(())
    }
    fn retire_setup(&self) -> Result<(), Refused> {
        let credit = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.pending {
                return Err(Refused::Capacity);
            }
            state.browser_setup.take()
        };
        drop(credit);
        Ok(())
    }
    fn status(&self) -> Status {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed || state.effects_closed || !self.selected.lease().is_live() {
            Status::Closed {
                native_setup_drained: !state.pending,
            }
        } else if let Some(public) = &state.public {
            Status::Ready(public.clone())
        } else {
            Status::Pending
        }
    }
}
impl Ticket {
    fn compute(
        &mut self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(Arc<Session>, PublicSetup, Credit), Refused> {
        // The registry owns Pending before this first possible callback. Taking the sole
        // preparation is irreversible: an error/panic cannot create another scalar on retry.
        let prepared = self.prepared.take().ok_or(Refused::Binding)?;
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed || state.effects_closed {
            return Err(Refused::Binding);
        }
        drop(state);
        prepared.compute(rng)
    }
    /// Caller holds Coordinator from its final original-generation/selected-Arc check through
    /// this installation. Failed output stays in caller's Option and drops AFTER guards.
    fn install(
        &mut self,
        output: &mut Option<(Arc<Session>, Arc<SetupOutput>, Credit)>,
    ) -> Result<(), Refused> {
        let mut state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed
            || state.effects_closed
            || !state.pending
            || !self.row.selected.lease().is_live()
        {
            return Err(Refused::Binding);
        }
        let (session, public, setup) = output.take().ok_or(Refused::Binding)?;
        if public.public.session != self.row.session_id || public.public.slot != self.row.slot {
            // This is an internal invariant, but do not drop keys under the caller's fence.
            *output = Some((session, public, setup));
            return Err(Refused::Binding);
        }
        state.session = Some(session);
        state.public = Some(public);
        state.browser_setup = Some(setup);
        state.pending = false;
        self.installed = true;
        Ok(())
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        // Release any proven local unstarted preparation before reporting actual setup
        // drainage. A concurrent closer must not observe drained while its Crypto credit lives.
        drop(self.prepared.take());
        if !self.installed {
            let mut state = self
                .row
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            state.pending = false;
        }
        // Prepared/local crypto credit and original selected owners drop after the row guard.
        // There is no ambiguous-attempt refund, new key allocation or resettable Pending row.
    }
}

/// The trusted main-window bootstrap resolves actual selected ownership. Fixed public
/// metadata alone grants no Root/RTC authority or a new local-channel counter allocator.
pub(crate) async fn ensure(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
    rng: &mut impl CryptoRngCore,
) -> Result<Status, Refused> {
    // Collection stays outside Coordinator: even paid historical snapshot/control-block
    // owners must be destroyed after final native authority/index gates are released.
    state.voice_channels.collect_retired(&state.voice_handoff);
    let selected = state
        .voice_authority
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .selected_handoff_owner(owner, executor)
        .map_err(|_| Refused::Binding)?;
    check_voice_binding(state, selected.binding())
        .await
        .map_err(|_| Refused::Binding)?;
    let _commit = state.ui_session_commit.clone().lock_owned().await;
    require_unlocked_session(state)
        .await
        .map_err(|_| Refused::Binding)?;
    let device = {
        let servers = state.servers.lock().await;
        let binding = selected.binding();
        *servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .ok_or(Refused::Binding)?
            .device_id
            .as_bytes()
    };
    let begin = {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        require_selected(state, &mut authority, owner, executor, &selected)?;
        state.voice_channels.begin(selected.clone(), device)?
    };
    let Begin::Work(mut ticket) = begin else {
        let Begin::Existing(status) = begin else {
            unreachable!()
        };
        return Ok(status);
    };
    // No await after Pending admission. Injected RNG/arithmetic can run no final native lock;
    // the original UI commit still orders admission/completion against lock's report.
    let (session, public, setup) = ticket.compute(rng)?;
    let reply = Arc::new(SetupOutput {
        public,
        _credit: ticket.row._credit.clone(),
    });
    let mut output = Some((session, reply.clone(), setup));
    {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        require_selected(state, &mut authority, owner, executor, &selected)?;
        ticket.install(&mut output)?;
    }
    Ok(Status::Ready(reply))
}

/// Exact trusted-main-window failure cleanup, including when no channel keys could be made.
/// The retained broker must first await every real setup SDK promise/intermediate. No boolean
/// assertion or arbitrary row descriptor is accepted. A delayed ensure stays permanently closed.
/// Its main-window command accepts only the exact original owner/executor identity.
pub(crate) async fn settle_setup(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<Status, Refused> {
    finish(state, owner, executor, Closure::Setup).await
}

/// Final trusted original-broker closure, AFTER actual setup/SDK/IO/buffer drainage. This
/// accepts no boolean assertion or renderer row descriptor and remains exact after reply loss.
pub(crate) async fn collect_channel(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<Status, Refused> {
    finish(state, owner, executor, Closure::Channel).await
}
enum Closure {
    Setup,
    Channel,
}
async fn finish(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
    closure: Closure,
) -> Result<Status, Refused> {
    let _commit = state.ui_session_commit.clone().lock_owned().await;
    if let Some(original) = state.voice_channels.close_effects(owner, executor) {
        // Both setup-failure cleanup and final channel collection stop first effects now,
        // then close Control routing only when every original action is compacted; otherwise
        // ACK/settlement/phase closure could become unreachable. All factory admissions use
        // this SAME UI commit, so a new Pending row cannot race the pure preflight below.
        // No Registry/Coordinator guard overlaps the ledger check or original-owner drop.
        if !state.voice_handoff.channel_may_close(&original) {
            return Err(Refused::Capacity);
        }
    }
    let row = {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let selected = authority
            .selected_handoff_owner(owner, executor)
            .ok()
            .filter(|selected| {
                check_voice_activation_generation(state, selected.binding().generation).is_ok()
            });
        let may_admit = authority.owner(owner).is_some_and(|current| {
            check_voice_activation_generation(state, current.binding.generation).is_ok()
        });
        // Existing exact cleanup works after lock/End. Unknown retired owners allocate none;
        // only a current selection can install the close-first fence under Coordinator.
        match state
            .voice_channels
            .close_row(owner, executor, selected.clone())
        {
            Ok(row) => row,
            Err(Refused::Binding) if selected.is_none() && !may_admit => {
                // UI commit excludes a pending/new ensure. No exact historical row exists,
                // and this accepted purpose cannot admit in its original generation anymore.
                // This is authoritative no-work metadata, not a browser inference from a
                // lost/error reply. Allocate no tombstone for unknown retired identities.
                return Ok(Status::Closed {
                    native_setup_drained: true,
                });
            }
            Err(error) => return Err(error),
        }
    };
    match closure {
        Closure::Setup => row.retire_setup()?,
        Closure::Channel => row.retire_channel()?,
    }
    let status = row.status();
    drop(row);
    state.voice_channels.collect_retired(&state.voice_handoff);
    Ok(status)
}

fn require_selected(
    state: &AppState,
    authority: &mut crate::voice_authority::Coordinator,
    owner: &str,
    executor: [u8; 32],
    selected: &Arc<SelectedHandoffOwner>,
) -> Result<(), Refused> {
    check_voice_activation_generation(state, selected.binding().generation)
        .map_err(|_| Refused::Binding)?;
    let current = authority
        .selected_handoff_owner(owner, executor)
        .map_err(|_| Refused::Binding)?;
    if !Arc::ptr_eq(&current, selected) || !selected.lease().is_live() {
        return Err(Refused::Binding);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(super) use tests::{
    installed as fixture_installed, installed_when_available as fixture_installed_when_available,
    retire as fixture_retire,
};
