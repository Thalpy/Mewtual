//! The shared inventory runtime against a real store, real Studio records, a manual clock and a
//! private permit pool. No receiver or network: the runtime is a state machine over the store's
//! job API, and every property in its module documentation is checked here directly.
use super::*;
use catcoms_mls::MlsDevice;
use catcoms_replication::studio::StudioTarget;
use catcoms_rt::{Clock, ManualClock};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const SERVER: u64 = 7;

struct Env {
    _root: tempfile::TempDir,
    store: ServerStore,
    device: MlsDevice,
    group: ServerGroup,
    clock: ManualClock,
    pool: Arc<Semaphore>,
    rt: InventoryRuntime,
}

impl Env {
    fn new(permits: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = ServerStore::open(
            root.path(),
            b"inventory-runtime",
            &mut ChaCha20Rng::seed_from_u64(1),
        )
        .unwrap();
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        Self {
            _root: root,
            store,
            device,
            group,
            clock: ManualClock::new(1_000),
            pool: Arc::new(Semaphore::new(permits)),
            rt: InventoryRuntime::default(),
        }
    }
    /// A real Studio source record. Its first validation is fresh, so a budgeted step parks it.
    ///
    /// Small on purpose: the receive profile this runtime scans under has a 256 KiB cold-byte
    /// limit, and the stock fixture's >256 KiB source is refused by it, which is the rail doing
    /// its job rather than anything this runtime should work around.
    fn source(&mut self, object: u8) {
        crate::store::save_studio_source_fixture_ops(
            &mut self.store,
            SERVER,
            &self.group,
            &self.device,
            StudioTarget::Flipnote {
                channel: [9; 16],
                object: [object; 16],
            },
            3,
            1_000,
        );
    }
    /// One whole visit, as the receiver's `run` makes it: the lifecycle step, one owner's turn,
    /// and the end-of-visit mark.
    fn turn(&mut self) -> Result<InventoryTurn, AppError> {
        self.rt.begin_visit(&self.store);
        self.rt
            .lifecycle(&self.store, SERVER, self.clock.monotonic_ms());
        let deadline = self.clock.monotonic_ms().saturating_add(250);
        let turn = self.rt.budget_turn(
            &mut self.store,
            SERVER,
            &self.group,
            &self.clock,
            deadline,
            &self.pool,
        );
        self.rt.end_visit(&self.store);
        turn
    }
    /// A visit in which no owner asks for a budget, such as one that only drains a packet.
    fn visit(&mut self) {
        self.rt.begin_visit(&self.store);
        self.rt
            .lifecycle(&self.store, SERVER, self.clock.monotonic_ms());
        self.rt.end_visit(&self.store);
    }
    /// A visit that returns before its lifecycle step, as `run_visit` does on a mount or
    /// numeric-server mismatch: `run` still brackets it with the check and the mark.
    fn early_return_visit(&mut self) {
        self.rt.begin_visit(&self.store);
        self.rt.end_visit(&self.store);
    }
    /// A visit in which this actor itself writes, as an explicit Save does. Custody is
    /// exclusive, so this is the only kind of write the runtime may call its own. A bare
    /// [`Self::source`] outside any visit is another actor's write.
    fn own_write(&mut self, object: u8) {
        self.rt.begin_visit(&self.store);
        self.rt
            .lifecycle(&self.store, SERVER, self.clock.monotonic_ms());
        self.source(object);
        self.rt.end_visit(&self.store);
    }
    /// Validate a detached body the way the background worker does, and hand the result back,
    /// outside custody: the runtime installs it on its next turn.
    fn validate_and_complete(&mut self, detach: InventoryDetach) {
        let InventoryDetach {
            body,
            permit,
            token,
        } = detach;
        let result = (*body).validate();
        drop(permit);
        self.rt.complete(token, Box::new(result));
    }
}

fn ready(turn: InventoryTurn) -> bool {
    match turn {
        InventoryTurn::Ready(budget) => {
            drop(budget);
            true
        }
        _ => false,
    }
}

#[test]
fn a_vault_with_nothing_to_validate_mints_in_one_turn() {
    let mut env = Env::new(1);
    assert!(ready(env.turn().unwrap()));
    assert!(matches!(env.rt.state, State::Idle));
    assert_eq!(env.pool.available_permits(), 1, "the permit was returned");
}

/// The whole park path. The permit is reserved before the step that parks, travels with the body
/// into the detach, and is only returned when the worker drops it; the job cannot step while its
/// body is out; and the turn after the result is installed completes and mints.
#[test]
fn a_parked_record_is_detached_with_its_permit_and_its_result_completes_the_job() {
    let mut env = Env::new(1);
    env.source(1);
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    assert!(matches!(env.rt.state, State::Parked { .. }));
    assert!(env.rt.pending(), "a parked body needs a visit");
    assert_eq!(
        env.pool.available_permits(),
        0,
        "the parked body holds the permit"
    );

    let detach = env.rt.take_detach().expect("a parked body is detachable");
    assert!(matches!(env.rt.state, State::Validating { .. }));
    assert!(!env.rt.pending(), "the result wakes the actor, not polling");
    assert!(
        matches!(env.turn().unwrap(), InventoryTurn::NotYet),
        "a job whose body is out does not step"
    );
    assert_eq!(
        env.pool.available_permits(),
        0,
        "the worker still owns the permit"
    );

    env.validate_and_complete(detach);
    assert_eq!(env.pool.available_permits(), 1);
    assert!(matches!(env.rt.state, State::Installing { .. }));
    // Only an owner's turn installs it, and owners are paced. Advertising it as driver work
    // would hold the driver at its active cadence until that turn came round.
    assert!(
        !env.rt.pending(),
        "a result waiting for an owner's turn was reported as driver work"
    );
    assert!(
        ready(env.turn().unwrap()),
        "the installed record completes the job"
    );
}

/// No permit, no step: a full pool leaves the job untouched and no body resident.
#[test]
fn a_full_pool_means_no_step_and_no_resident_body() {
    let mut env = Env::new(1);
    env.source(1);
    let held = env.pool.clone().try_acquire_owned().unwrap();
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    assert!(
        matches!(env.rt.state, State::Stepping(_)),
        "without a permit the job must not have parked a body"
    );
    drop(held);
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    assert!(matches!(env.rt.state, State::Parked { .. }));
}

/// Results are routed by token. Another token's result is ignored; a job released while its body
/// was out does not come back when its result arrives; and that old result, arriving while a
/// **new** job's body is out, is not installed into the new job.
#[test]
fn stale_and_abandoned_results_are_dropped() {
    let mut env = Env::new(1);
    env.source(1);
    env.turn().unwrap();
    let detach = env.rt.take_detach().unwrap();
    let token = detach.token;
    let result = (*detach.body).validate();
    drop(detach.permit);
    env.rt.complete(
        token.wrapping_add(1),
        Box::new(Err(invalid("not this one"))),
    );
    assert!(
        matches!(env.rt.state, State::Validating { .. }),
        "another token's result was installed"
    );

    env.rt.release();
    assert!(matches!(env.rt.state, State::Idle));
    let stale = result.unwrap();
    // A fresh job parks and detaches the same record under a new token.
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    let fresh = env.rt.take_detach().expect("a fresh job began");
    assert_ne!(fresh.token, token, "tokens are never reused");
    // The released job's result arrives now. It must not be installed into the new job.
    env.rt.complete(token, Box::new(Ok(stale)));
    assert!(
        matches!(&env.rt.state, State::Validating { token: t, .. } if *t == fresh.token),
        "a released job's result was installed into its successor"
    );
    env.validate_and_complete(fresh);
    assert!(
        ready(env.turn().unwrap()),
        "the new job's own result completes it"
    );
}

/// A cancelled validation waiter drops the job without charging a restart.
#[test]
fn a_cancelled_validation_drops_the_job_and_the_next_turn_starts_fresh() {
    let mut env = Env::new(1);
    env.source(1);
    env.turn().unwrap();
    let detach = env.rt.take_detach().unwrap();
    env.rt.cancelled(detach.token.wrapping_add(1));
    assert!(
        matches!(env.rt.state, State::Validating { .. }),
        "another token cancelled it"
    );
    env.rt.cancelled(detach.token);
    assert!(matches!(env.rt.state, State::Idle));
    drop(detach);
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    assert!(matches!(env.rt.state, State::Parked { .. }));
}

/// A validation error is surfaced once, on the next turn, and then a fresh job starts.
#[test]
fn a_validation_error_is_surfaced_once() {
    let mut env = Env::new(1);
    env.source(1);
    env.turn().unwrap();
    let detach = env.rt.take_detach().unwrap();
    env.rt
        .complete(detach.token, Box::new(Err(invalid("validation failed"))));
    drop(detach);
    let error = env.turn().err().expect("the error is surfaced");
    assert!(error.to_string().contains("validation failed"));
    assert!(
        matches!(env.turn().unwrap(), InventoryTurn::NotYet),
        "then a fresh job"
    );
}

/// Overtake the job once per cycle with `write` while each body is out, until it is `Unstable`.
/// Returns how many bodies were detached on the way and when the backoff ends.
fn storm(env: &mut Env, write: fn(&mut Env, u8)) -> (usize, u64) {
    env.source(1);
    let mut detached = 0;
    loop {
        // Each turn installs the previous cycle's overtaken result (a restart), steps the
        // restarted job, and parks the newest uncached record again.
        if let InventoryTurn::Unstable = env.turn().unwrap() {
            let until = env
                .rt
                .backoff_until_for_test()
                .expect("Unstable means a backoff");
            return (detached, until);
        }
        let Some(detach) = env.rt.take_detach() else {
            panic!("expected a park every cycle");
        };
        detached += 1;
        // A five-family write while the body is out: the install is overtaken.
        write(env, 1 + detached as u8);
        env.validate_and_complete(detach);
        assert!(detached < 16, "the restart budget never ran out");
    }
}

/// Another actor's writes landing while each body is out restart the job until the store's
/// budget is spent, every one of them charged; then the runtime backs off for the first backoff
/// interval, refuses until it ends, is not driver work at any point, and starts a fresh job after.
#[test]
fn a_restart_storm_backs_off_and_recovers() {
    let mut env = Env::new(1);
    let (detached, retry_at) = storm(&mut env, |env, object| env.source(object));
    assert_eq!(
        detached,
        crate::store::MAX_INVENTORY_RESTARTS + 1,
        "a foreign write went uncharged"
    );
    assert_eq!(env.rt.own_restarts, 0);
    assert!(matches!(env.rt.state, State::Backoff { .. }));
    assert_eq!(retry_at, env.clock.monotonic_ms() + BACKOFF_FIRST_MS);
    assert!(!env.rt.pending());
    env.clock.advance_ms(BACKOFF_FIRST_MS - 1);
    assert!(
        matches!(env.turn().unwrap(), InventoryTurn::Unstable),
        "a turn inside the backoff must be refused"
    );
    assert_eq!(
        env.rt.backoff_until_for_test(),
        Some(retry_at),
        "a refused turn moved the backoff"
    );

    env.clock.advance_ms(1);
    // A run-out backoff is cleared by any visit and is never driver work: if no owner wants the
    // job any more, nothing should keep the driver awake for it.
    assert!(
        !env.rt.pending(),
        "a passed backoff was reported as driver work"
    );
    env.visit();
    assert!(matches!(env.rt.state, State::Idle));
    assert!(
        matches!(env.turn().unwrap(), InventoryTurn::NotYet),
        "a fresh job began"
    );
}

/// Run turns, validating every body the runtime detaches, until the job mints. Returns how many
/// bodies it detached on the way.
fn detaches_until_ready(env: &mut Env) -> usize {
    let mut detaches = 0;
    loop {
        match env.turn().unwrap() {
            InventoryTurn::Ready(budget) => {
                drop(budget);
                return detaches;
            }
            InventoryTurn::NotYet => {
                if let Some(detach) = env.rt.take_detach() {
                    detaches += 1;
                    env.validate_and_complete(detach);
                }
            }
            InventoryTurn::Unstable => panic!("a quiet vault reported unstable"),
        }
        assert!(detaches < 16, "the job never minted");
    }
}

/// C-3 runtime design 14.3, through the runtime (implementation review M-1). An own write
/// overtakes the job while a body is out, and the uncharged refresh that follows hands the
/// result to the store before it discards the cursor. The restarted job then reuses that record
/// instead of parking it again.
///
/// The overtaking write only rotates the token. A real source write would not do: the fixture's
/// writer runs its own unbudgeted inventory first, which validates and caches every source
/// already present, so the record would be warm whether or not the refresh kept its result.
/// Counted against a baseline: a fresh job over the same vault needs some number of detaches,
/// and the refreshed one must need exactly one fewer, because the record it already validated
/// is now a hit. A refresh that dropped the result would need the same number as the baseline.
#[test]
fn an_own_write_refresh_memoizes_the_overtaken_result() {
    let mut baseline = Env::new(1);
    baseline.source(1);
    let fresh = detaches_until_ready(&mut baseline);
    assert!(
        fresh >= 1,
        "precondition: the cold source parks in a fresh job"
    );

    let mut env = Env::new(1);
    env.source(1);
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    let detach = env.rt.take_detach().expect("the cold source parked");
    // An own write: inside this actor's visit, so the refresh is uncharged (N-M1).
    env.rt.begin_visit(&env.store);
    env.rt
        .lifecycle(&env.store, SERVER, env.clock.monotonic_ms());
    env.store.overtake_inventory_for_test();
    env.rt.end_visit(&env.store);
    env.validate_and_complete(detach);
    let after = detaches_until_ready(&mut env);
    assert_eq!(
        env.rt.own_restarts, 1,
        "precondition: the job was refreshed as an uncharged own-write restart"
    );
    assert_eq!(
        after,
        fresh - 1,
        "the refreshed job parked the overtaken record again, so its result was dropped"
    );
}

/// N-M1. This actor's own writes (inside its own visits) restart the job without charging the
/// store's budget, up to [`OWN_RESTARTS`]; after that they are charged like anyone's.
#[test]
fn own_writes_are_uncharged_up_to_their_cap() {
    let mut env = Env::new(1);
    let (detached, _) = storm(&mut env, |env, object| env.own_write(object));
    assert_eq!(
        detached,
        crate::store::MAX_INVENTORY_RESTARTS + OWN_RESTARTS + 1,
        "own writes were charged, or uncharged without a cap"
    );
    assert_eq!(env.rt.own_restarts, OWN_RESTARTS);
}

/// N-M1. A foreign write anywhere since the cursor began taints it, even when the gap just before
/// the turn that notices was quiet and the visits in between asked for nothing: the proof is
/// "no foreign write since this cursor began", not "none in the last gap".
#[test]
fn a_foreign_write_in_an_earlier_gap_is_still_charged() {
    let mut env = Env::new(1);
    env.source(1);
    env.turn().unwrap();
    let detach = env.rt.take_detach().unwrap();
    env.source(2); // between visits: another actor's
    env.visit(); // notices the moved token, steps nothing
    env.visit(); // a quiet gap since the last visit
    env.validate_and_complete(detach);
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    assert_eq!(
        env.rt.own_restarts, 0,
        "a restart another actor caused went uncharged"
    );

    // And the converse in the same job: a fresh cursor owes nothing to writes before it began,
    // so the next overtaking, by this actor's own write, is uncharged.
    let detach = env.rt.take_detach().unwrap();
    env.own_write(3);
    env.validate_and_complete(detach);
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    assert_eq!(
        env.rt.own_restarts, 1,
        "an own write after a restart was charged"
    );
}

/// Review of step 2, LOW-1. A visit that returns early still checks before it marks: if it only
/// marked, the foreign write in the gap before it would read as quiet at the next visit and its
/// restart would go uncharged.
#[test]
fn an_early_return_visit_does_not_launder_a_foreign_write() {
    let mut env = Env::new(1);
    env.source(1);
    env.turn().unwrap();
    let detach = env.rt.take_detach().unwrap();
    env.source(2); // another actor's, between visits
    env.early_return_visit();
    env.validate_and_complete(detach);
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    assert_eq!(
        env.rt.own_restarts, 0,
        "an early-return visit laundered a foreign write into an uncharged restart"
    );
}

/// Review of step 2, LOW-3. A cursor that never stepped (its turn found the pool full) loses
/// nothing to a restart, so refreshing it charges neither budget: a storm that follows gets the
/// whole of the store's restart budget, exactly as for a job that had just begun.
#[test]
fn a_cursor_that_never_stepped_is_refreshed_for_free() {
    let mut env = Env::new(1);
    let held = env.pool.clone().try_acquire_owned().unwrap();
    assert!(matches!(env.turn().unwrap(), InventoryTurn::NotYet));
    assert!(matches!(env.rt.state, State::Stepping(_)));
    drop(held);
    // `storm` begins with a foreign write, which overtakes the cursor that never stepped.
    let (detached, _) = storm(&mut env, |env, object| env.source(object));
    assert_eq!(
        detached,
        crate::store::MAX_INVENTORY_RESTARTS + 1,
        "refreshing a cursor that had done nothing was charged"
    );
    assert_eq!(
        env.rt.own_restarts, 0,
        "and it was counted as an own restart"
    );
}

/// A changed numeric server drops the job.
#[test]
fn a_changed_numeric_server_drops_the_job() {
    let mut env = Env::new(1);
    env.source(1);
    let held = env.pool.clone().try_acquire_owned().unwrap();
    env.turn().unwrap();
    assert!(matches!(env.rt.state, State::Stepping(_)));
    env.rt
        .lifecycle(&env.store, SERVER + 1, env.clock.monotonic_ms());
    assert!(
        matches!(env.rt.state, State::Idle),
        "another numeric server kept the job"
    );
    drop(held);
}

/// Idleness is time since an owner last asked, not a visit count. Every inbound packet is a
/// visit, so a burst of them between two of an owner's paced turns must not drop the job that
/// owner is working through: that restarts it every time and the owner never finishes.
#[test]
fn a_burst_of_visits_keeps_a_wanted_job_and_time_drops_an_unwanted_one() {
    let mut env = Env::new(1);
    env.source(1);
    let held = env.pool.clone().try_acquire_owned().unwrap();
    env.turn().unwrap();
    assert!(matches!(env.rt.state, State::Stepping(_)));
    for _ in 0..1_000 {
        env.visit();
    }
    assert!(
        matches!(env.rt.state, State::Stepping(_)),
        "a burst of packet visits dropped a job its owner still wants"
    );
    env.clock.advance_ms(IDLE_MS);
    env.visit();
    assert!(matches!(env.rt.state, State::Stepping(_)), "dropped early");
    env.clock.advance_ms(1);
    env.visit();
    assert!(
        matches!(env.rt.state, State::Idle),
        "an unwanted job outlived the idle limit"
    );
    drop(held);

    // A result no owner comes back for is dropped the same way.
    env.turn().unwrap();
    let detach = env.rt.take_detach().unwrap();
    env.validate_and_complete(detach);
    assert!(matches!(env.rt.state, State::Installing { .. }));
    env.clock.advance_ms(IDLE_MS + 1);
    env.visit();
    assert!(
        matches!(env.rt.state, State::Idle),
        "an uninstalled result outlived the idle limit"
    );
}
