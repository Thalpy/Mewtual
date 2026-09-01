//! The bounded chronological store, with per-section counts.
//!
//! One list, in the order things were observed, plus indexes over it. The review is explicit about
//! why it is not one list per subsystem: a send failure usually depends on the exact interleaving
//! of UI, IPC, actor, network, persistence and event delivery, and per-subsystem files destroy
//! precisely that. Sections are a filter over the timeline, never a separate timeline.
//!
//! Bounded, and honest about it. A ring that quietly forgets its oldest entries presents a gap as
//! a quiet period, which is the single most misleading thing a diagnostic store can do.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::config::{Level, Section};
use crate::event::DiagnosticEvent;

/// A handle to a stored event.
///
/// The store hands these out rather than copies. An event can carry thirty-two fields of owned
/// strings, and the reads that matter are not small: the console polls every second and a full
/// export pages the whole ring. Cloning that depth *under the hub's lock* meant every reader
/// blocked every producer for as long as the copy took, on paths that include actor and network
/// work.
///
/// So a read locates its range, clones handles, and lets go. The deep work, rendering an event at a
/// capture mode, happens afterwards with no lock held. Found by adversarial review (P3-012), which
/// noted that the hot-path guarantee in the hub's docs described writes and said nothing about what
/// concurrent readers cost.
pub type StoredEvent = Arc<DiagnosticEvent>;

/// What the store knows about itself.
///
/// Counted as events arrive, never derived from what is currently held: a roll-up that said "no
/// errors" because the error had aged out would be worse than having no roll-up at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RingStats {
    pub errors: u64,
    pub warnings: u64,
    /// Events evicted to stay inside the bound.
    pub dropped: u64,
    /// Events never admitted, because the capture config excluded them. Distinct from `dropped`:
    /// one is "we lost it", the other is "we were told not to look", and a user chasing a silent
    /// section needs to know which.
    pub filtered: u64,
    pub latest_seq: u64,
}

/// A bounded, chronological store of diagnostic events.
#[derive(Debug)]
pub struct Ring {
    events: VecDeque<StoredEvent>,
    capacity: usize,
    next_seq: u64,
    stats: RingStats,
    /// Per-section totals, counted on arrival like the rest.
    per_section: [u64; 22],
    per_section_errors: [u64; 22],
}

impl Ring {
    pub fn new(capacity: usize) -> Self {
        Ring {
            events: VecDeque::new(),
            capacity: capacity.max(1),
            next_seq: 0,
            stats: RingStats::default(),
            per_section: [0; 22],
            per_section_errors: [0; 22],
        }
    }

    /// Record an event, assigning its sequence number. Returns the assigned sequence.
    pub fn push(&mut self, mut event: DiagnosticEvent) -> u64 {
        self.next_seq += 1;
        event.seq = self.next_seq;
        self.stats.latest_seq = event.seq;
        match event.level {
            Level::Error => self.stats.errors += 1,
            Level::Warn => self.stats.warnings += 1,
            _ => {}
        }
        let at = event.section.index();
        self.per_section[at] += 1;
        if event.level == Level::Error {
            self.per_section_errors[at] += 1;
        }

        let seq = event.seq;
        self.events.push_back(Arc::new(event));
        while self.events.len() > self.capacity {
            self.events.pop_front();
            self.stats.dropped += 1;
        }
        seq
    }

    /// The index of the first held event numbered above `after_seq`.
    ///
    /// Arithmetic rather than a scan. Sequences are dense across the deque: `push` assigns
    /// consecutive numbers and eviction only ever removes from the front, so the event at index `i`
    /// is always numbered `front + i`. The console polls with the highest sequence it holds, which
    /// on a quiet second is the newest event in the ring, and a scan from the oldest meant walking
    /// the entire store every tick to discover there was nothing new.
    fn offset_after(&self, after_seq: u64) -> usize {
        let Some(front) = self.events.front() else {
            return 0;
        };
        // Everything held is newer than the caller: start at the beginning.
        if front.seq > after_seq {
            return 0;
        }
        // `front.seq + i > after_seq` for the first time at `i = after_seq - front.seq + 1`, and
        // that can be past the end, which the callers treat as "nothing new".
        ((after_seq - front.seq) as usize)
            .saturating_add(1)
            .min(self.events.len())
    }

    /// Note an event that the capture config excluded, so the difference between "nothing
    /// happened" and "nothing was being watched" stays visible.
    pub fn note_filtered(&mut self) {
        self.stats.filtered += 1;
    }

    /// Events after `after_seq`, oldest first, at most `limit`.
    ///
    /// A caller further behind than the ring is deep gets the oldest events still held, and learns
    /// from [`RingStats::dropped`] that it missed some.
    pub fn since(&self, after_seq: u64, limit: usize) -> Vec<StoredEvent> {
        self.events
            .iter()
            .skip(self.offset_after(after_seq))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Events after `after_seq` in one section.
    ///
    /// Still a scan, but only of the range the caller has not seen: a section filter has no index
    /// of its own, and building one would be a second structure to keep in step with the timeline
    /// for a read that is bounded by the ring anyway.
    pub fn section_since(
        &self,
        section: Section,
        after_seq: u64,
        limit: usize,
    ) -> Vec<StoredEvent> {
        self.events
            .iter()
            .skip(self.offset_after(after_seq))
            .filter(|e| e.section == section)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Every event belonging to one trace, which is how an operation is read end to end.
    pub fn trace(&self, trace: crate::event::TraceId) -> Vec<StoredEvent> {
        self.events
            .iter()
            .filter(|e| e.trace == trace)
            .cloned()
            .collect()
    }

    pub fn stats(&self) -> RingStats {
        self.stats
    }

    /// How many events, and how many errors, one section has seen this session.
    pub fn section_counts(&self, section: Section) -> (u64, u64) {
        let at = section.index();
        (self.per_section[at], self.per_section_errors[at])
    }

    /// How many events are held right now, as opposed to how many were seen.
    pub fn held(&self) -> usize {
        self.events.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Forget the held events, keeping the counters and the sequence.
    ///
    /// A view-clearing operation, not a rewriting of what happened: the session totals still say
    /// twelve errors occurred, and the sequence keeps climbing so a reader cannot mistake a
    /// cleared store for a fresh session.
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::TraceId;

    fn event(section: Section, level: Level, code: &'static str) -> DiagnosticEvent {
        DiagnosticEvent::new(section, level, code)
    }

    #[test]
    fn sequence_numbers_are_dense_and_increasing() {
        let mut ring = Ring::new(10);
        let first = ring.push(event(Section::Sync, Level::Info, "A"));
        let second = ring.push(event(Section::Sync, Level::Info, "B"));
        assert_eq!((first, second), (1, 2));
        assert_eq!(ring.stats().latest_seq, 2);
    }

    /// The property the whole roll-up rests on. A user opens the console after a failure, and the
    /// header has to say "12 errors this session" even though the first of them aged out an hour
    /// ago.
    #[test]
    fn counters_survive_the_events_they_counted() {
        let mut ring = Ring::new(4);
        ring.push(event(Section::Transport, Level::Error, "NET.DIAL.FAILED"));
        for _ in 0..20 {
            ring.push(event(Section::Sync, Level::Info, "SYNC.OK"));
        }
        let stats = ring.stats();
        assert_eq!(stats.errors, 1, "still counted");
        assert!(stats.dropped >= 1, "and the loss is admitted");
        assert_eq!(ring.held(), 4);
        assert!(
            !ring
                .since(0, 100)
                .iter()
                .any(|e| e.code == "NET.DIAL.FAILED"),
            "even though the event itself is gone"
        );
    }

    /// "Nothing happened" and "nothing was being watched" are different answers, and a user
    /// staring at an empty section needs to know which one they are looking at.
    #[test]
    fn filtered_events_are_counted_separately_from_lost_ones() {
        let mut ring = Ring::new(2);
        ring.note_filtered();
        ring.note_filtered();
        ring.push(event(Section::Sync, Level::Info, "A"));
        ring.push(event(Section::Sync, Level::Info, "B"));
        ring.push(event(Section::Sync, Level::Info, "C"));
        let stats = ring.stats();
        assert_eq!(stats.filtered, 2);
        assert_eq!(stats.dropped, 1);
    }

    #[test]
    fn a_caller_is_never_told_the_same_event_twice() {
        let mut ring = Ring::new(10);
        for code in ["A", "B", "C"] {
            ring.push(event(Section::Sync, Level::Info, code));
        }
        let first = ring.since(0, 10);
        assert_eq!(first.len(), 3);
        let newest = first.last().unwrap().seq;
        assert!(ring.since(newest, 10).is_empty());
        ring.push(event(Section::Sync, Level::Warn, "D"));
        let next = ring.since(newest, 10);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].code, "D");
    }

    /// Sections are a filter over one timeline, never a separate timeline: a send failure depends
    /// on the interleaving, and per-section stores destroy exactly that.
    #[test]
    fn a_section_view_preserves_the_order_of_the_whole_timeline() {
        let mut ring = Ring::new(20);
        ring.push(event(Section::Ui, Level::Info, "UI.1"));
        ring.push(event(Section::Ipc, Level::Info, "IPC.1"));
        ring.push(event(Section::Ui, Level::Info, "UI.2"));
        ring.push(event(Section::Transport, Level::Info, "NET.1"));
        ring.push(event(Section::Ui, Level::Info, "UI.3"));

        let ui: Vec<_> = ring
            .section_since(Section::Ui, 0, 10)
            .iter()
            .map(|e| e.seq)
            .collect();
        assert_eq!(
            ui,
            [1, 3, 5],
            "sequences from the shared timeline, in order"
        );
        let all: Vec<_> = ring.since(0, 10).iter().map(|e| e.code).collect();
        assert_eq!(all, ["UI.1", "IPC.1", "UI.2", "NET.1", "UI.3"]);
    }

    #[test]
    fn per_section_counts_feed_the_rail_badges() {
        let mut ring = Ring::new(20);
        ring.push(event(Section::Transport, Level::Error, "NET.1"));
        ring.push(event(Section::Transport, Level::Warn, "NET.2"));
        ring.push(event(Section::Sync, Level::Info, "SYNC.1"));
        assert_eq!(ring.section_counts(Section::Transport), (2, 1));
        assert_eq!(ring.section_counts(Section::Sync), (1, 0));
        assert_eq!(ring.section_counts(Section::Voice), (0, 0));
    }

    /// Reading one operation end to end is the thing traces exist for.
    #[test]
    fn a_trace_gathers_every_stage_of_one_operation() {
        let mut ring = Ring::new(20);
        let mine = TraceId(0x7f2c);
        let other = TraceId(0x64aa);
        ring.push(event(Section::Ui, Level::Info, "UI.SEND").trace(mine));
        ring.push(event(Section::Ipc, Level::Info, "IPC.OTHER").trace(other));
        ring.push(event(Section::Sync, Level::Info, "SYNC.POST").trace(mine));
        ring.push(event(Section::Ipc, Level::Warn, "IPC.EMIT").trace(mine));

        let stages: Vec<_> = ring.trace(mine).iter().map(|e| e.code).collect();
        assert_eq!(stages, ["UI.SEND", "SYNC.POST", "IPC.EMIT"]);
    }

    /// Clearing is about the view. The session totals still describe the session, and the
    /// sequence keeps climbing so a cleared store cannot be mistaken for a fresh one.
    #[test]
    fn clearing_the_view_does_not_rewrite_what_happened() {
        let mut ring = Ring::new(10);
        ring.push(event(Section::Sync, Level::Error, "A"));
        ring.clear();
        assert_eq!(ring.held(), 0);
        assert_eq!(ring.stats().errors, 1);
        assert_eq!(ring.push(event(Section::Sync, Level::Info, "B")), 2);
    }

    /// A reader takes a handle, not a copy.
    ///
    /// The whole of P3-012. An event can carry thirty-two fields of owned strings, and deep-cloning
    /// a page of them happened under the hub's one mutex, so every reader blocked every producer
    /// for the length of the copy. Pointer identity is the proof: the caller and the store are
    /// looking at the same allocation, so nothing was copied to hand it over.
    #[test]
    fn a_read_hands_out_a_handle_rather_than_a_copy() {
        let mut ring = Ring::new(10);
        ring.push(
            event(Section::Sync, Level::Info, "SYNC.BIG")
                .field("payload", crate::redact::SafeText::describe("a long value")),
        );

        let first = ring.since(0, 10);
        let second = ring.since(0, 10);
        assert!(
            Arc::ptr_eq(&first[0], &second[0]),
            "two reads of one event must not produce two copies of it"
        );
        // Three live handles: the store's own, and one from each read.
        assert_eq!(Arc::strong_count(&first[0]), 3);

        // And a handle outlives eviction, so a reader that is still rendering cannot be pulled out
        // from under by a producer filling the ring.
        for _ in 0..20 {
            ring.push(event(Section::Sync, Level::Info, "SYNC.FILLER"));
        }
        assert_eq!(first[0].code, "SYNC.BIG");
        assert_eq!(
            Arc::strong_count(&first[0]),
            2,
            "the store has let go, the readers have not"
        );
    }

    /// Polling must not walk the whole store to discover there is nothing new.
    ///
    /// The console polls every second with the highest sequence it holds, which on a quiet second
    /// is the newest event in the ring. Scanning from the oldest meant traversing the entire store
    /// every tick to return nothing. Sequences are dense across the deque, so the starting point is
    /// arithmetic.
    #[test]
    fn a_poll_starts_where_the_caller_left_off() {
        let mut ring = Ring::new(100);
        for _ in 0..50 {
            ring.push(event(Section::Sync, Level::Info, "A"));
        }
        // Every boundary: before the front, at the front, mid-ring, at the newest, and past it.
        assert_eq!(ring.offset_after(0), 0, "nothing seen yet");
        assert_eq!(ring.offset_after(1), 1, "seen the first");
        assert_eq!(ring.offset_after(25), 25);
        assert_eq!(ring.offset_after(50), 50, "seen everything");
        assert_eq!(
            ring.offset_after(999),
            50,
            "and a caller ahead of the ring is clamped"
        );

        // After eviction the front is no longer sequence 1, which is where an off-by-one would
        // start returning somebody else's events.
        for _ in 0..80 {
            ring.push(event(Section::Sync, Level::Info, "B"));
        }
        let front = ring.since(0, 1)[0].seq;
        assert!(front > 1, "the ring has rolled");
        assert_eq!(
            ring.offset_after(0),
            0,
            "a caller behind the ring gets what is left"
        );
        assert_eq!(ring.offset_after(front), 1);
        let newest = ring.since(0, 1000).last().unwrap().seq;
        assert!(
            ring.since(newest, 10).is_empty(),
            "and nothing new is nothing"
        );
    }

    #[test]
    fn a_zero_capacity_ring_still_works_rather_than_dividing_by_nothing() {
        let mut ring = Ring::new(0);
        ring.push(event(Section::Sync, Level::Info, "A"));
        assert_eq!(ring.capacity(), 1);
        assert_eq!(ring.held(), 1);
    }
}
