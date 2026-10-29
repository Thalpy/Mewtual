//! Exact worker-local test evidence. The harness sponsors this separately bounded
//! capsule; production builds contain neither it nor a new producer API. Only the
//! optional Arc handles enter the measured native8KiB task/completion graph.
use super::*;
use catcoms_sync::voice_v2::crypto_actions::HandoffLoan;
use std::sync::Mutex;

const HARNESS_BYTES: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) struct ActionIdentity {
    pub address: usize,
    pub serial: u64,
    pub version: u64,
    pub input: [u8; 32],
    pub keys: [[u8; 32]; 2],
    pub handshake: [u8; 32],
    pub generation: u64,
    pub deadline: u64,
}
impl ActionIdentity {
    pub(in crate::voice_handoff) fn of(action: &Arc<CryptoAction>) -> Self {
        let binding = action
            .protected_binding()
            .expect("actual protected derivation");
        Self {
            address: Arc::as_ptr(action) as usize,
            serial: action.serial(),
            version: action.version(),
            input: action.input_tag(),
            keys: [binding.local_key, binding.peer_key],
            handshake: binding.handshake,
            generation: binding.generation,
            deadline: binding.deadline,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::voice_handoff) struct Observation {
    pub selector: Option<PairSelector>,
    pub parent: Option<(RowId, usize)>,
    pub action: Option<ActionIdentity>,
    // Pair1, Derive2, attachment3, retained-publication attachment4.
    pub stage: u8,
    pub pressured: bool,
}
struct State {
    observation: Observation,
    loan: Option<HandoffLoan>,
}
pub(in crate::voice_handoff) struct Witness {
    pressure: bool,
    state: Mutex<State>,
}
impl Witness {
    fn new(pressure: bool) -> Arc<Self> {
        // Fixed harness graph includes allocation/control block, original real
        // loan wrapper and all observation/return copies. No Row/Action/Grant Arc
        // is retained here; the loan's original Root payment remains independent.
        assert!(Self::layout_bytes() <= HARNESS_BYTES);
        Arc::new(Self {
            pressure,
            state: Mutex::new(State {
                observation: Observation::default(),
                loan: None,
            }),
        })
    }
    pub(in crate::voice_handoff) fn layout_bytes() -> usize {
        std::mem::size_of::<Self>()
            + 2 * std::mem::size_of::<usize>()
            + 3 * std::mem::size_of::<Arc<Self>>()
            + 3 * std::mem::size_of::<Observation>()
            + std::mem::size_of::<Option<HandoffLoan>>()
            + std::mem::size_of::<catcoms_sync::voice_v2::crypto_actions::ProtectedCryptoBinding>()
            + std::mem::size_of::<ActionIdentity>()
            + std::mem::size_of::<std::sync::MutexGuard<'_, State>>()
            + 64
    }
    pub(in crate::voice_handoff) fn selected(&self, selector: PairSelector) {
        self.state.lock().unwrap().observation.selector = Some(selector);
    }
    pub(in crate::voice_handoff) fn parent(&self, id: RowId, address: usize) {
        self.state.lock().unwrap().observation.parent = Some((id, address));
    }
    pub(in crate::voice_handoff) fn stage(&self, stage: u8) {
        self.state.lock().unwrap().observation.stage = stage;
    }
    pub(in crate::voice_handoff) fn attachment(&self, action: &Arc<CryptoAction>, bytes: usize) {
        let identity = ActionIdentity::of(action);
        let loan = if self.pressure {
            Some(
                action
                    .borrow_handoff(bytes)
                    .expect("harness pressure setup must acquire the actual original handoff loan"),
            )
        } else {
            None
        };
        let mut state = self.state.lock().unwrap();
        state.observation.stage = 3;
        state.observation.action = Some(identity);
        state.observation.pressured = loan.is_some();
        let previous = std::mem::replace(&mut state.loan, loan);
        drop(state);
        drop(previous);
    }
    pub(in crate::voice_handoff) fn observation(&self) -> Observation {
        self.state.lock().unwrap().observation
    }
    pub(in crate::voice_handoff) fn release_pressure(&self) {
        let loan = self.state.lock().unwrap().loan.take();
        drop(loan); // original Workflow/callback destruction outside the capsule guard
    }
}

fn observed(
    context: &Context,
    route: &Route,
    selector: Selection,
    pressure: bool,
) -> Result<NativeWaiter, Refused> {
    let (kind, cursor) = diagnostic_selection(&selector);
    let result = spawn_inner(context, route, selector, Some(Witness::new(pressure)));
    route
        .session()
        .fixture_native_admission(kind, cursor, result.as_ref().err().copied());
    result
}
pub(in crate::voice_handoff) fn derive(
    context: &Context,
    route: &Route,
    selector: PairSelector,
    pressure: bool,
) -> Result<NativeWaiter, Refused> {
    observed(context, route, Selection::Exact(selector), pressure)
}
pub(in crate::voice_handoff) fn discover(
    context: &Context,
    route: &Route,
    cursor: u8,
    pressure: bool,
) -> Result<NativeWaiter, Refused> {
    let selected = route.selected();
    let permit = PeerSnapshotPermit::new(&selected.resources(), selected.binding().scope)?;
    observed(
        context,
        route,
        Selection::Discover { cursor, permit },
        pressure,
    )
}

#[test]
fn exact_worker_capsule_is_a_separate_bounded_harness_graph() {
    eprintln!(
        "exact worker harness capsule graph {}/{}",
        Witness::layout_bytes(),
        HARNESS_BYTES
    );
    assert!(Witness::layout_bytes() <= HARNESS_BYTES);
}
