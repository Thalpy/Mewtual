//! Real authenticated framing and paid native producer on the blocked-TCP call fixture.
//! The production Host still rejects this staged opcode family. No SDK/RTC is simulated.
use super::*;
use catcoms_sync::voice_v2::sdp::InitialSdpIntent;
use catcoms_sync::voice_v2::{chain::sdp_handoff::NativeRowPurpose, VoiceError};
use std::sync::atomic::Ordering;

/// A fixture-only clock with ONE scoped task trigger. It wraps production SystemClock,
/// advances monotonically for every original runtime after firing, and cannot be consumed
/// by Actor/maintenance tasks. No handler hook or replacement runtime supplies authority.
#[derive(Debug, Default)]
pub(crate) struct ExpiryClock {
    offset: std::sync::atomic::AtomicU64,
    trigger: std::sync::Mutex<Option<(tokio::task::Id, usize, u64)>>,
    fired: std::sync::atomic::AtomicBool,
}
impl ExpiryClock {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn arm(&self, delta: u64) {
        assert!(!self.fired.load(Ordering::Acquire));
        let mut trigger = self.trigger.lock().unwrap();
        assert!(trigger.is_none());
        *trigger = Some((tokio::task::id(), 2, delta));
    }
}
impl catcoms_rt::Clock for ExpiryClock {
    fn now_ms(&self) -> u64 {
        let mut trigger = self.trigger.lock().unwrap();
        if let Some((task, left, delta)) = trigger.as_mut() {
            if Some(*task) == tokio::task::try_id() {
                *left -= 1;
                if *left == 0 {
                    self.offset.fetch_add(*delta, Ordering::AcqRel);
                    self.fired.store(true, Ordering::Release);
                    *trigger = None;
                }
            }
        }
        drop(trigger);
        catcoms_rt::Clock::now_ms(&catcoms_rt::SystemClock)
            .saturating_add(self.offset.load(Ordering::Acquire))
    }
    fn monotonic_ms(&self) -> u64 {
        catcoms_rt::Clock::monotonic_ms(&catcoms_rt::SystemClock)
            .saturating_add(self.offset.load(Ordering::Acquire))
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        catcoms_rt::Clock::sleep(&catcoms_rt::SystemClock, duration)
    }
}
tokio::task_local! { static EXPIRY_CLOCK: Arc<ExpiryClock>; }
tokio::task_local! { static RESULT_COMPLETE: Arc<tokio::sync::Barrier>; }
/// Both real native endpoints finish their Result before either intentionally closes
/// its PC. This test-only rendezvous supplies no protocol state or effect authority.
pub(crate) async fn with_result_completion<F: std::future::Future>(
    barrier: Arc<tokio::sync::Barrier>,
    future: F,
) -> F::Output {
    RESULT_COMPLETE.scope(barrier, future).await
}
pub(crate) async fn with_expiry_clock<F: std::future::Future>(
    clock: Arc<ExpiryClock>,
    future: F,
) -> F::Output {
    EXPIRY_CLOCK.scope(clock, future).await
}

async fn exchange(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    logical: [u8; 16],
    operation: u8,
    body: &[u8],
) -> Vec<u8> {
    let header = (
        route.session().fixture_counts().1 + 1,
        operation,
        if operation == 15 { 2 } else { 0 },
        logical,
    );
    let incoming = if operation == 15 {
        let wire = service::tests::signed_frame(public, header, body);
        let authenticated = service::tests::authenticated_header(route, &wire);
        let permit = state
            .voice_handoff
            .sdp_begin_intake(
                route.selected(),
                route.session().identity(),
                &authenticated,
                crate::voice_runtime(state).clock(),
            )
            .unwrap();
        let mut incoming = authenticated.admit_sdp(permit).unwrap();
        let count = incoming.remaining_mut().len();
        incoming
            .remaining_mut()
            .copy_from_slice(&wire[wire.len() - count..]);
        incoming.advance(count).unwrap();
        assert_eq!(incoming.open().unwrap(), body);
        incoming
    } else {
        opened_frame(public, route, header, body, None)
    };
    let context = context::Context::new(state);
    let reply = if operation == 17 {
        super::super::sdp_input::pull(&context, route, incoming).await
    } else {
        super::super::sdp_http::apply(&context, route, incoming).await
    }
    .unwrap();
    let result = opened_reply(public, header, body, &reply);
    drop(reply);
    // Other native maintenance may briefly own this same fence. A forgotten
    // Completion publication guard would instead keep it locked indefinitely.
    drop(
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            state.ui_session_commit.lock(),
        )
        .await
        .expect("actual completed worker publication guard was not collected"),
    );
    assert!(parent.id().logical() != [0; 16]);
    result
}
fn resolve(original: &[u8], seal: bool, close: bool) -> Vec<u8> {
    let mut body = original[..225].to_vec();
    body.push(u8::from(!seal));
    body.push(u8::from(close));
    body
}
fn existing(parent: &ledger::ResolvedRow, seal: bool) -> Vec<u8> {
    let mut body = vec![8];
    body.extend_from_slice(&parent.id().logical());
    body.push(u8::from(!seal));
    body
}
pub(super) async fn exercise(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    pc_frame: &[u8],
    intent: &Arc<InitialSdpIntent>,
    effect: u8,
) -> RetiredProbe {
    let seal = intent.local_is_low();
    let operation = if seal { 15 } else { 16 };
    let mut original = pc_frame[..193].to_vec();
    original[0] = 8;
    original.extend_from_slice(&parent.pc_test_proof());
    let mut begin = original.clone();
    if seal {
        begin.extend_from_slice(&3u16.to_be_bytes());
        begin.extend_from_slice(b"v=0");
    }
    let capacity =
        super::super::sdp_replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
    if !seal {
        let mut premature = original.clone();
        premature.extend_from_slice(&3u16.to_be_bytes());
        premature.extend_from_slice(b"v=0");
        assert_eq!(
            exchange(
                state,
                public,
                route,
                parent,
                parent.id().logical(),
                15,
                &premature
            )
            .await,
            capacity
        );
        assert_eq!(
            state.voice_handoff.sdp_test_rows(parent.id()),
            0,
            "premature HIGH cannot poison its only Seal row"
        );
    }
    let clock = crate::voice_runtime(state).clock();
    let remaining = intent
        .remaining_at(clock.now_ms(), clock.monotonic_ms())
        .unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_millis(remaining), async {
        loop {
            let reply = exchange(
                state,
                public,
                route,
                parent,
                parent.id().logical(),
                operation,
                &begin,
            )
            .await;
            if reply != capacity {
                assert!(
                    seal || (effect >= 2 && reply.len() == 220 && reply[37] == 1),
                    "HIGH without an actual incoming Offer must wait"
                );
                return reply;
            }
            let found = exchange(
                state,
                public,
                route,
                parent,
                parent.id().logical(),
                27,
                &resolve(&original, seal, false),
            )
            .await;
            if !seal && found.len() == 220 && (effect < 2 || found[37] == 1) {
                return found;
            }
            clock.sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original native HTTP observer bound");
    assert_eq!(&first[..2], &[8, 0]);
    assert_eq!(first.len(), 220);
    assert_eq!(first[37], u8::from(seal || effect >= 2));
    let child: [u8; 16] = first[2..18].try_into().unwrap();
    let history = exchange(
        state,
        public,
        route,
        parent,
        child,
        25,
        &existing(parent, seal),
    )
    .await;
    assert_eq!(
        &history[2..86],
        &first[2..86],
        "exact same native row and actual Action"
    );
    if seal {
        let repeated = exchange(
            state,
            public,
            route,
            parent,
            parent.id().logical(),
            15,
            &begin,
        )
        .await;
        assert_eq!(&repeated[2..86], &first[2..86]);
        let mut changed = begin.clone();
        *changed.last_mut().unwrap() = b'1';
        assert_eq!(
            exchange(
                state,
                public,
                route,
                parent,
                parent.id().logical(),
                15,
                &changed
            )
            .await,
            super::super::sdp_replies::refused(factory::Refused::Protocol(
                VoiceError::RetryMismatch
            ))
        );
        let unchanged = exchange(
            state,
            public,
            route,
            parent,
            child,
            25,
            &existing(parent, seal),
        )
        .await;
        assert_eq!(&unchanged[2..86], &first[2..86]);
        // This staged slice exercises the actual Input queue/hook and class2 response;
        // production SDP dispatch and browser/RTC execution are still disabled.
        let action = requests::Action {
            serial: u64::from_be_bytes(first[38..46].try_into().unwrap()),
            version: u64::from_be_bytes(first[46..54].try_into().unwrap()),
            input_tag: first[54..86].try_into().unwrap(),
        };
        if effect >= 2 {
            Box::pin(super::super::sdp_result::exercise(
                state,
                public,
                route,
                parent,
                pc_frame,
                ledger::RowId::from_logical(child).unwrap(),
                action,
                effect == 3,
            ))
            .await;
        } else if effect == 1 {
            Box::pin(super::super::sdp_engine::exercise(
                state,
                public,
                route,
                parent,
                pc_frame,
                ledger::RowId::from_logical(child).unwrap(),
                action,
            ))
            .await;
        } else {
            Box::pin(super::super::sdp_input::exercise(
                state,
                public,
                route,
                parent,
                pc_frame,
                ledger::RowId::from_logical(child).unwrap(),
                action,
            ))
            .await;
        }
    }
    if !seal {
        if let Ok(clock) = EXPIRY_CLOCK.try_with(Arc::clone) {
            expiry_after_history(state, public, route, parent, &original, intent, clock).await;
        }
        if effect >= 2 {
            let action = requests::Action {
                serial: u64::from_be_bytes(first[38..46].try_into().unwrap()),
                version: u64::from_be_bytes(first[46..54].try_into().unwrap()),
                input_tag: first[54..86].try_into().unwrap(),
            };
            Box::pin(super::super::sdp_result::exercise_open(
                state,
                public,
                route,
                parent,
                ledger::RowId::from_logical(child).unwrap(),
                action,
                effect == 2,
            ))
            .await;
        }
    }
    if effect >= 2 {
        let barrier = RESULT_COMPLETE.with(Arc::clone);
        barrier.wait().await;
        let pc = requests::pc::Request::decode(parent.id().logical(), pc_frame).unwrap();
        parent.close_pc(route.session().identity(), &pc).unwrap();
    }
    let returned = exchange(
        state,
        public,
        route,
        parent,
        child,
        24,
        &existing(parent, seal),
    )
    .await;
    assert_eq!(returned[36], 1);
    assert_eq!(returned[37], u8::from(seal || effect >= 2));
    assert_eq!(
        returned[86],
        u8::from(seal || effect >= 2),
        "engine settlement follows actual fixture completion only"
    );
    if seal {
        let mut input = existing(parent, true);
        input.extend_from_slice(&first[38..86]);
        assert_eq!(
            exchange(state, public, route, parent, child, 17, &input).await,
            returned,
            "settled Input recovers canonical history with no fresh Copy after PC/row close"
        );
    }
    let recovered = exchange(
        state,
        public,
        route,
        parent,
        parent.id().logical(),
        27,
        &resolve(&original, seal, true),
    )
    .await;
    assert_eq!(&recovered[2..86], &returned[2..86]);
    let other = exchange(
        state,
        public,
        route,
        parent,
        parent.id().logical(),
        27,
        &resolve(&original, !seal, true),
    )
    .await;
    assert_eq!(
        other,
        super::super::sdp_replies::missing(
            parent.id(),
            if seal {
                NativeRowPurpose::Open
            } else {
                NativeRowPurpose::Seal
            },
            true
        )
    );
    assert_eq!(state.voice_handoff.sdp_test_rows(parent.id()), 1);
    RetiredProbe {
        original,
        seal,
        returned,
        other,
    }
}
/// Fixture-only retained request/observations, never a replacement capability. The
/// later request must still prove the same actual native parent/Session after retirement.
pub(super) struct RetiredProbe {
    original: Vec<u8>,
    seal: bool,
    returned: Vec<u8>,
    other: Vec<u8>,
}
impl RetiredProbe {
    pub(super) async fn after_plan_retirement(
        self,
        state: &AppState,
        public: &bootstrap::SetupOutput,
        route: &bootstrap::Route,
        parent: &ledger::ResolvedRow,
    ) {
        let before = state.voice_handoff.sdp_test_rows(parent.id());
        let found = exchange(
            state,
            public,
            route,
            parent,
            parent.id().logical(),
            27,
            &resolve(&self.original, self.seal, false),
        )
        .await;
        assert_eq!(
            found, self.returned,
            "retired parent preserves same collected child history"
        );
        let missing = exchange(
            state,
            public,
            route,
            parent,
            parent.id().logical(),
            27,
            &resolve(&self.original, !self.seal, true),
        )
        .await;
        assert_eq!(
            missing, self.other,
            "never-installed purpose preserves its original close fence"
        );
        assert_eq!(
            state.voice_handoff.sdp_test_rows(parent.id()),
            before,
            "historical resolution creates no new native child"
        );
    }
}

/// Pins expiry at the fresh post-history observation. This no-action Open cannot invoke
/// an accepted-Open Clock callback; that separate vertical remains for Input/Result tests.
async fn expiry_after_history(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    original: &[u8],
    intent: &Arc<InitialSdpIntent>,
    clock: Arc<ExpiryClock>,
) {
    let before = state.voice_handoff.sdp_test_rows(parent.id());
    let open_before = exchange(
        state,
        public,
        route,
        parent,
        parent.id().logical(),
        27,
        &resolve(original, false, false),
    )
    .await;
    let mut body = original.to_vec();
    body.extend_from_slice(&3u16.to_be_bytes());
    body.extend_from_slice(b"v=0");
    let header = (
        route.session().fixture_counts().1 + 1,
        15,
        2,
        parent.id().logical(),
    );
    let wire = service::tests::signed_frame(public, header, &body);
    let authenticated = service::tests::authenticated_header(route, &wire);
    let permit = state
        .voice_handoff
        .sdp_begin_intake(
            route.selected(),
            route.session().identity(),
            &authenticated,
            crate::voice_runtime(state).clock(),
        )
        .unwrap();
    let mut incoming = authenticated.admit_sdp(permit).unwrap();
    let count = incoming.remaining_mut().len();
    incoming
        .remaining_mut()
        .copy_from_slice(&wire[wire.len() - count..]);
    incoming.advance(count).unwrap();
    assert_eq!(incoming.open().unwrap(), body);
    let original_clock = crate::voice_runtime(state).clock();
    let remaining = intent
        .remaining_at(original_clock.now_ms(), original_clock.monotonic_ms())
        .unwrap();
    let context = context::Context::new(state);
    let target = route.clone();
    let witness = clock.clone();
    let reply = tokio::spawn(async move {
        clock.arm(remaining + 1);
        super::super::sdp_http::apply(&context, &target, incoming).await
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        opened_reply(public, header, &body, &reply),
        super::super::sdp_replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized))
    );
    assert!(
        witness.fired.load(Ordering::Acquire),
        "fresh post-history sample consumed original Clock trigger"
    );
    drop(reply);
    assert_eq!(
        state.voice_handoff.sdp_test_rows(parent.id()),
        before,
        "expiry installed no Seal row/worker/outcome"
    );
    assert_eq!(
        exchange(
            state,
            public,
            route,
            parent,
            parent.id().logical(),
            27,
            &resolve(original, false, false)
        )
        .await,
        open_before,
        "expiry preserves the same original no-action Open history"
    );
    let closed = exchange(
        state,
        public,
        route,
        parent,
        parent.id().logical(),
        27,
        &resolve(original, true, true),
    )
    .await;
    assert_eq!(
        closed,
        super::super::sdp_replies::missing(parent.id(), NativeRowPurpose::Seal, true)
    );
    assert!(
        route.session().header().is_ok(),
        "actual expired request body/reply physical position drained"
    );
}
