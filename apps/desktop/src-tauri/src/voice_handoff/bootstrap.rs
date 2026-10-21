//! Immutable process-owned setup for the disabled native/browser channel. This registry has
//! no network listener or renderer command. A lost caller cannot reset its selected key,
//! ephemeral agreement or physical counters. Pending, Ready and closed fences share64 slots;
//! uncertain rows are never evicted. This finite pilot requires lifecycle collection before
//! live activation; saturation refuses setup rather than reconstructing old session history.
use super::channel::bootstrap::{Prepared, PublicSetup};
use super::channel::{Refused, Session};
use crate::voice_authority::SelectedHandoffOwner;
use crate::{
    check_voice_activation_generation, check_voice_binding, require_unlocked_session, AppState,
};
use catcoms_rt::CryptoRngCore;
use catcoms_sync::voice_v2::resources::{Credit, ProcessRetainedCredit, ResourceKind, Resources};
use std::sync::{Arc, Mutex};

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
    _credit: Arc<Credit>,
}
struct State {
    pending: bool,
    closed: bool,
    session: Option<Arc<Session>>,
    public: Option<Arc<SetupOutput>>,
    // Original native ECDH credit continues through browser setup/outgoing SDK promises.
    // Lease revocation and caller loss are NOT evidence that this work has drained.
    browser_setup: Option<Credit>,
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
/// The dormant wrapper belongs to AppState. Allocate/pay its fixed slot index lazily, from
/// the actual shared native pool, before any row or bootstrap becomes externally observable.
#[derive(Default)]
pub(crate) struct Registry {
    index: Mutex<Option<Box<Index>>>,
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
    /// Bind the fixed service infrastructure to the SAME native process pool before opening
    /// a listener. This may allocate the separately prepaid empty index, never a group/row.
    pub(super) fn bind_pool(&self, resources: &Arc<Resources>) -> Result<(), Refused> {
        let mut held = self.index.lock().unwrap_or_else(|error| error.into_inner());
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
        let row = held
            .as_ref()
            .and_then(|index| index.rows[usize::from(slot)].as_ref())
            .ok_or(Refused::Binding)?;
        let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
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
                closed: false,
                session: None,
                public: None,
                browser_setup: None,
            }),
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
            return Ok(row.status());
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
                closed: true,
                session: None,
                public: None,
                browser_setup: None,
            }),
            _credit: Arc::new(credit),
        });
        index.serial = serial;
        index.rows[slot] = Some(row);
        Ok(Status::Closed {
            native_setup_drained: true,
        })
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
}
/// Native-only immutable control routing. Retain the actual row, Session and paid public
/// output across IO; copied session IDs alone cannot create this owner or authorize actions.
pub(super) struct Route {
    row: Arc<Row>,
    session: Arc<Session>,
    public: Arc<SetupOutput>,
}
impl Route {
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
    /// A snapshot of NATIVE setup state only. It neither proves browser drainage nor lets a
    /// late Ready response revive a retired call, room, join or action.
    pub(super) fn snapshot(&self) -> [u8; 3] {
        let state = self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let closed = state.closed || !self.row.selected.lease().is_live();
        [1, if closed { 2 } else { 1 }, u8::from(!state.pending)]
    }
}
impl Row {
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
        if state.closed || !self.selected.lease().is_live() {
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
        if self
            .row
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed
        {
            return Err(Refused::Binding);
        }
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
        if state.closed || !state.pending || !self.row.selected.lease().is_live() {
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

/// Internal bridge consumer only. No command exposes it until authenticated delivery and
/// browser ownership are integrated. Fixed public metadata alone grants no Root/RTC authority.
pub(crate) async fn ensure(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
    rng: &mut dyn CryptoRngCore,
) -> Result<Status, Refused> {
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
/// This native-only consumer is not exposed as a command until its browser adapter is wired.
pub(crate) async fn settle_setup(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<Status, Refused> {
    let _commit = state.ui_session_commit.clone().lock_owned().await;
    let status = {
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
            .close(owner, executor, selected.clone())
        {
            Ok(status) => status,
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
    state.voice_channels.retire_setup(owner, executor)?;
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
