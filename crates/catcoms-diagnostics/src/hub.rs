//! The one place observations go.
//!
//! Everything the app knows about its own behaviour arrives here: `tracing` events from the
//! protocol crates, structured records from the webview, command and event wrappers, task
//! supervision, probe results, and the pipeline's own health. One store, one sequence, one set of
//! privacy rules.
//!
//! # Why the hub owns the salt and the clock
//!
//! Both are session-scoped facts that every event needs and no caller should have to carry. The
//! salt in particular: if each subsystem derived its own references, the same peer would appear
//! under a different name in each section and correlation, the entire point, would be gone.
//!
//! # What it refuses to do
//!
//! It never blocks a caller, never grows without bound, and never logs its own failures through
//! itself. That last one matters more than it sounds: a sink failure that writes a diagnostic
//! about the sink failure through the failed sink is an unbounded loop inside the component whose
//! job is to still be working when everything else is not.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use catcoms_rt::{Clock, RngCore};

use crate::config::{CaptureConfig, CaptureGate, CaptureMode, Level, Section};
use crate::event::{DiagnosticEvent, SpanId, TraceId};
use crate::redact::{RefDomain, SessionRef, SessionSalt};
use crate::ring::{Ring, RingStats};

/// How many events the hub holds by default.
///
/// Enough to cover the run-up to a failure a user notices and then goes looking for, small enough
/// to be irrelevant beside the app's own footprint.
pub const DEFAULT_CAPACITY: usize = 8192;

struct HubInner {
    ring: Ring,
    config: CaptureConfig,
    salt: SessionSalt,
    /// Source of trace and span identifiers. A counter rather than randomness: these need to be
    /// unique within one session and nothing more, and a counter is reproducible under test.
    next_id: u64,
    /// Where this session's monotonic clock started, so elapsed time is measured from process
    /// start rather than from an arbitrary origin.
    origin_ms: u64,
}

/// The diagnostic hub. Cheap to clone; every clone refers to the same store.
///
/// # Cost on the emitting thread
///
/// This sits on paths that include actor and network work, so what it costs matters as much as
/// what it records. Diagnostics that add contention become a cause of the stalls they exist to
/// explain, and the resulting hunt is for a bug that is not there.
///
/// * An event the config excludes costs two relaxed atomic loads and no lock at all, via
///   [`CaptureGate`]. That is the common case under a debug or trace level on a busy section.
/// * An event that is recorded takes the store's lock for a bounded push: a few counter
///   increments and a `VecDeque` append, with no allocation, no I/O and no scanning.
/// * The clock is read before the lock, never under it.
/// * Field names are owned by the event rather than interned in a shared table, so building one
///   touches no global state.
#[derive(Clone)]
pub struct DiagnosticHub {
    inner: Arc<Mutex<HubInner>>,
    /// Consulted without locking, so the excluded case never contends. Kept in step with the
    /// config inside the lock on every change.
    gate: Arc<CaptureGate>,
    /// Counted outside the lock too, for the same reason.
    filtered: Arc<AtomicU64>,
    clock: Arc<dyn Clock>,
    session_id: String,
}

impl std::fmt::Debug for DiagnosticHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never renders the store or the salt. A salt in a log lets a reader recompute every
        // reference in it, and a `Debug` on a diagnostics type is exactly the sort of thing that
        // ends up in a log by accident.
        write!(f, "DiagnosticHub({})", self.session_id)
    }
}

impl DiagnosticHub {
    /// A hub with the default capacity, a random session salt, and a starting mode.
    pub fn new(clock: Arc<dyn Clock>, rng: &mut impl RngCore, mode: CaptureMode) -> Self {
        Self::with_capacity(clock, SessionSalt::random(rng), mode, DEFAULT_CAPACITY, rng)
    }

    /// A hub with an explicit salt and capacity, for tests that need reproducible references.
    pub fn with_capacity(
        clock: Arc<dyn Clock>,
        salt: SessionSalt,
        mode: CaptureMode,
        capacity: usize,
        rng: &mut impl RngCore,
    ) -> Self {
        let origin_ms = clock.monotonic_ms();
        let session_id = format!("{:08x}", rng.next_u32());
        let config = CaptureConfig::for_mode(mode);
        let gate = Arc::new(CaptureGate::new(&config));
        DiagnosticHub {
            inner: Arc::new(Mutex::new(HubInner {
                ring: Ring::new(capacity),
                config,
                salt,
                next_id: 0,
                origin_ms,
            })),
            gate,
            filtered: Arc::new(AtomicU64::new(0)),
            clock,
            session_id,
        }
    }

    /// Identifies this run, so an excerpt someone pastes can be matched to the report it came from.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Record an event, stamping it with the sequence and both clocks.
    ///
    /// Returns the assigned sequence, or `None` when the capture config excluded it. The rejection
    /// is counted, so a silent section is distinguishable from a quiet one.
    pub fn record(&self, event: DiagnosticEvent) -> Option<u64> {
        // Before anything else, and before any lock: an event nobody asked for should cost two
        // atomic loads and go away. Under a trace level on a busy section this is most events, on
        // every thread at once, and taking a global lock to reject them would make the diagnostics
        // the bottleneck.
        if !self.gate.admits(event.section, event.level) {
            self.filtered.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        // Read the clock outside the lock. It is the one call here that could be slow, and holding
        // a global lock across a syscall is how a cheap critical section becomes a contended one.
        let at_ms = self.clock.now_ms();
        let monotonic = self.clock.monotonic_ms();
        let mut inner = self.lock();
        let mut event = event;
        event.at_ms = at_ms;
        event.monotonic_ms = monotonic.saturating_sub(inner.origin_ms);
        Some(inner.ring.push(event))
    }

    /// A fresh trace, for one user-visible operation.
    pub fn new_trace(&self) -> TraceId {
        let mut inner = self.lock();
        inner.next_id += 1;
        TraceId(inner.next_id)
    }

    /// A fresh span, for one stage inside a trace.
    pub fn new_span(&self) -> SpanId {
        let mut inner = self.lock();
        inner.next_id += 1;
        SpanId(inner.next_id)
    }

    /// Reduce an identifier to a reference under this session's salt.
    ///
    /// The only route by which an identifier reaches an event, which is what makes "a peer id
    /// cannot leak" a property of the code rather than a habit of its authors.
    pub fn reference(&self, domain: RefDomain, id: &[u8]) -> SessionRef {
        self.lock().salt.reference(domain, id)
    }

    /// Convenience for the common case of referencing something named by a string.
    pub fn reference_str(&self, domain: RefDomain, id: &str) -> SessionRef {
        self.reference(domain, id.as_bytes())
    }

    pub fn stats(&self) -> RingStats {
        let mut stats = self.lock().ring.stats();
        // Counted outside the lock, so it is folded in on the way out.
        stats.filtered = self.filtered.load(Ordering::Relaxed);
        stats
    }

    pub fn mode(&self) -> CaptureMode {
        self.lock().config.mode
    }

    /// Change what is being captured, immediately.
    ///
    /// Turning capture off takes effect now, not at the next restart. The previous design could
    /// only attach a subscriber once per process, so every change to logging waited for a relaunch
    /// and a user who wanted to stop being recorded had to quit the app to do it.
    pub fn set_mode(&self, mode: CaptureMode) {
        let mut inner = self.lock();
        inner.config = CaptureConfig::for_mode(mode);
        // Published while the lock is held, so the gate and the config can never disagree about
        // what the current setting is.
        self.gate.store(&inner.config);
    }

    /// Adjust one section's level without disturbing the others.
    pub fn set_section_level(&self, section: Section, level: Option<Level>) {
        let mut inner = self.lock();
        inner.config.set(section, level);
        self.gate.store(&inner.config);
    }

    pub fn config(&self) -> CaptureConfig {
        self.lock().config.clone()
    }

    /// Events after `after_seq`, oldest first, at most `limit`.
    pub fn since(&self, after_seq: u64, limit: usize) -> Vec<DiagnosticEvent> {
        self.lock()
            .ring
            .since(after_seq, limit)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Events after `after_seq` within one section.
    pub fn section_since(
        &self,
        section: Section,
        after_seq: u64,
        limit: usize,
    ) -> Vec<DiagnosticEvent> {
        self.lock()
            .ring
            .section_since(section, after_seq, limit)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Every stage of one operation, in order.
    pub fn trace(&self, trace: TraceId) -> Vec<DiagnosticEvent> {
        self.lock().ring.trace(trace).into_iter().cloned().collect()
    }

    /// How many events, and how many errors, one section has seen this session.
    pub fn section_counts(&self, section: Section) -> (u64, u64) {
        self.lock().ring.section_counts(section)
    }

    /// How many events are held right now.
    pub fn held(&self) -> usize {
        self.lock().ring.held()
    }

    /// Forget the held events, keeping the counters and the sequence.
    pub fn clear(&self) {
        self.lock().ring.clear();
    }

    fn lock(&self) -> MutexGuard<'_, HubInner> {
        // A panic in another thread's short critical section must not take diagnostics down with
        // it. The data behind a poisoned lock here is a queue of events and some counters, all of
        // it perfectly readable, and refusing to read it would mean losing the record of whatever
        // caused the panic.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;

    /// A deterministic RNG, so session ids and salts are reproducible in tests.
    struct StepRng(u64);
    impl RngCore for StepRng {
        fn next_u32(&mut self) -> u32 {
            self.0 = self.0.wrapping_add(0x9e37_79b9);
            self.0 as u32
        }
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            self.0
        }
        fn fill_bytes(&mut self, dest: &mut [u8]) {
            for byte in dest.iter_mut() {
                *byte = self.next_u32() as u8;
            }
        }
        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), catcoms_rt::rng::RngError> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    fn hub(mode: CaptureMode) -> (DiagnosticHub, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_787_000_000_000));
        let mut rng = StepRng(1);
        let hub = DiagnosticHub::with_capacity(
            clock.clone(),
            SessionSalt::for_tests(9),
            mode,
            64,
            &mut rng,
        );
        (hub, clock)
    }

    #[test]
    fn a_recorded_event_is_stamped_with_both_clocks_and_a_sequence() {
        let (hub, clock) = hub(CaptureMode::Safe);
        clock.advance_ms(250);
        let seq = hub
            .record(DiagnosticEvent::info(Section::Sync, "SYNC.OK"))
            .expect("Safe captures sync at info");
        assert_eq!(seq, 1);

        let held = hub.since(0, 10);
        assert_eq!(held[0].at_ms, clock.now_ms());
        // Measured from process start, so a duration is meaningful without knowing the origin.
        assert_eq!(held[0].monotonic_ms, 250);
    }

    /// Off means off: no history, no file, nothing accumulating.
    #[test]
    fn off_records_nothing_at_all() {
        let (hub, _) = hub(CaptureMode::Off);
        assert_eq!(
            hub.record(DiagnosticEvent::error(Section::Sync, "SYNC.BAD")),
            None
        );
        assert_eq!(hub.held(), 0);
        assert_eq!(hub.stats().errors, 0);
    }

    /// The change the previous design could not make without a relaunch. A user who wants to stop
    /// being recorded should not have to quit the app to do it.
    #[test]
    fn capture_can_be_turned_off_and_on_again_while_running() {
        let (hub, _) = hub(CaptureMode::Safe);
        assert!(hub
            .record(DiagnosticEvent::info(Section::Sync, "A"))
            .is_some());

        hub.set_mode(CaptureMode::Off);
        assert!(hub
            .record(DiagnosticEvent::info(Section::Sync, "B"))
            .is_none());

        hub.set_mode(CaptureMode::Safe);
        assert!(hub
            .record(DiagnosticEvent::info(Section::Sync, "C"))
            .is_some());

        let codes: Vec<_> = hub.since(0, 10).iter().map(|e| e.code).collect();
        assert_eq!(codes, ["A", "C"]);
        assert_eq!(hub.stats().filtered, 1, "and the gap is accounted for");
    }

    #[test]
    fn a_filtered_section_is_distinguishable_from_a_quiet_one() {
        let (hub, _) = hub(CaptureMode::Safe);
        // Safe holds the transport at warn, so a debug event there is excluded by policy.
        assert!(hub
            .record(DiagnosticEvent::new(
                Section::Transport,
                Level::Debug,
                "NET.CHURN"
            ))
            .is_none());
        assert_eq!(hub.stats().filtered, 1);
        assert_eq!(hub.stats().dropped, 0, "excluded is not the same as lost");
    }

    #[test]
    fn one_session_gives_every_subsystem_the_same_name_for_the_same_peer() {
        // If each subsystem salted its own references, the same peer would appear under a
        // different name in each section and correlation would be gone.
        let (hub, _) = hub(CaptureMode::Safe);
        let from_transport = hub.reference(RefDomain::Peer, b"12D3KooWabc");
        let from_sync = hub.reference(RefDomain::Peer, b"12D3KooWabc");
        assert_eq!(from_transport, from_sync);
    }

    #[test]
    fn traces_and_spans_are_unique_within_a_session() {
        let (hub, _) = hub(CaptureMode::Safe);
        let a = hub.new_trace();
        let b = hub.new_trace();
        let span = hub.new_span();
        assert_ne!(a, b);
        assert_ne!(
            a.0, span.0,
            "spans and traces draw from one sequence, so they never collide"
        );
    }

    #[test]
    fn one_operation_is_readable_end_to_end() {
        let (hub, _) = hub(CaptureMode::Safe);
        let trace = hub.new_trace();
        let other = hub.new_trace();
        hub.record(DiagnosticEvent::info(Section::Ui, "UI.SEND").trace(trace));
        hub.record(DiagnosticEvent::info(Section::Ipc, "IPC.OTHER").trace(other));
        hub.record(DiagnosticEvent::info(Section::Sync, "SYNC.POST").trace(trace));

        let stages: Vec<_> = hub.trace(trace).iter().map(|e| e.code).collect();
        assert_eq!(stages, ["UI.SEND", "SYNC.POST"]);
    }

    #[test]
    fn a_section_level_can_be_changed_without_disturbing_the_others() {
        let (hub, _) = hub(CaptureMode::Safe);
        hub.set_section_level(Section::Transport, Some(Level::Debug));
        assert!(hub
            .record(DiagnosticEvent::new(
                Section::Transport,
                Level::Debug,
                "NET.CHURN"
            ))
            .is_some());
        assert!(
            hub.record(DiagnosticEvent::new(
                Section::Voice,
                Level::Debug,
                "VOICE.X"
            ))
            .is_none(),
            "the others keep the mode's default"
        );
    }

    /// A salt in a log lets a reader recompute every reference in it, and a `Debug` impl on a
    /// diagnostics type is precisely the sort of thing that ends up in a log by accident.
    #[test]
    fn the_hub_never_renders_its_own_salt_or_contents() {
        let (hub, _) = hub(CaptureMode::Safe);
        hub.record(DiagnosticEvent::error(Section::Sync, "SYNC.SECRETISH"));
        let rendered = format!("{hub:?}");
        assert!(rendered.starts_with("DiagnosticHub("));
        assert!(!rendered.contains("SYNC.SECRETISH"));
        assert!(!rendered.contains("Salt"));
    }

    /// The property the whole design rests on when things get busy: many threads emitting at once
    /// must produce a coherent record and must not deadlock. Every thread here contends for the
    /// same store, which is the shape the real app has.
    #[test]
    fn concurrent_emitters_produce_one_coherent_record() {
        let (hub, _) = hub(CaptureMode::Safe);
        let threads = 8;
        let each = 500;
        std::thread::scope(|scope| {
            for _ in 0..threads {
                let hub = hub.clone();
                scope.spawn(move || {
                    for _ in 0..each {
                        hub.record(DiagnosticEvent::info(Section::Sync, "SYNC.CONCURRENT"));
                    }
                });
            }
        });

        let stats = hub.stats();
        assert_eq!(
            stats.latest_seq,
            (threads * each) as u64,
            "every event got exactly one sequence number"
        );
        // The ring holds 64 in this fixture, so the rest were evicted rather than lost silently.
        assert_eq!(stats.dropped, (threads * each) as u64 - hub.held() as u64);

        // Sequences are dense and increasing across every thread's contributions.
        let held = hub.since(0, 1000);
        for pair in held.windows(2) {
            assert!(pair[1].seq > pair[0].seq, "the timeline stayed ordered");
        }
    }

    /// An excluded event must not take the store's lock. Under a trace level on a busy section
    /// that is most events on every thread at once, and locking only to reject them would make the
    /// diagnostics the bottleneck rather than the observer.
    ///
    /// Contention is not directly observable from a test, so this pins the consequence: rejection
    /// is counted without the ring's sequence moving, which is only possible off the lock path.
    #[test]
    fn rejecting_an_event_leaves_the_store_untouched() {
        let (hub, _) = hub(CaptureMode::Off);
        for _ in 0..1000 {
            assert!(hub
                .record(DiagnosticEvent::info(Section::Sync, "SYNC.NOPE"))
                .is_none());
        }
        let stats = hub.stats();
        assert_eq!(stats.filtered, 1000);
        assert_eq!(stats.latest_seq, 0, "the store never advanced");
        assert_eq!(hub.held(), 0);
    }

    /// A level change has to be visible to the lock-free gate, or capture would keep following the
    /// old setting until something happened to take the lock.
    #[test]
    fn a_config_change_reaches_the_lock_free_gate_immediately() {
        let (hub, _) = hub(CaptureMode::Safe);
        assert!(hub
            .record(DiagnosticEvent::new(
                Section::Voice,
                Level::Debug,
                "VOICE.X"
            ))
            .is_none());
        hub.set_section_level(Section::Voice, Some(Level::Debug));
        assert!(hub
            .record(DiagnosticEvent::new(
                Section::Voice,
                Level::Debug,
                "VOICE.X"
            ))
            .is_some());
        hub.set_section_level(Section::Voice, None);
        assert!(hub
            .record(DiagnosticEvent::error(Section::Voice, "VOICE.BAD"))
            .is_none());
    }

    #[test]
    fn clones_share_one_store() {
        let (hub, _) = hub(CaptureMode::Safe);
        let other = hub.clone();
        other.record(DiagnosticEvent::info(Section::Sync, "A"));
        assert_eq!(hub.held(), 1, "a clone is a handle, not a copy");
        assert_eq!(hub.session_id(), other.session_id());
    }
}
