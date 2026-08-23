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

use crate::config::{Level, Section, SECTIONS};
use crate::event::DiagnosticEvent;

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
    events: VecDeque<DiagnosticEvent>,
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
        let at = Self::index(event.section);
        self.per_section[at] += 1;
        if event.level == Level::Error {
            self.per_section_errors[at] += 1;
        }

        let seq = event.seq;
        self.events.push_back(event);
        while self.events.len() > self.capacity {
            self.events.pop_front();
            self.stats.dropped += 1;
        }
        seq
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
    pub fn since(&self, after_seq: u64, limit: usize) -> Vec<&DiagnosticEvent> {
        self.events
            .iter()
            .filter(|e| e.seq > after_seq)
            .take(limit)
            .collect()
    }

    /// Events after `after_seq` in one section.
    pub fn section_since(
        &self,
        section: Section,
        after_seq: u64,
        limit: usize,
    ) -> Vec<&DiagnosticEvent> {
        self.events
            .iter()
            .filter(|e| e.seq > after_seq && e.section == section)
            .take(limit)
            .collect()
    }

    /// Every event belonging to one trace, which is how an operation is read end to end.
    pub fn trace(&self, trace: crate::event::TraceId) -> Vec<&DiagnosticEvent> {
        self.events.iter().filter(|e| e.trace == trace).collect()
    }

    pub fn stats(&self) -> RingStats {
        self.stats
    }

    /// How many events, and how many errors, one section has seen this session.
    pub fn section_counts(&self, section: Section) -> (u64, u64) {
        let at = Self::index(section);
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

    fn index(section: Section) -> usize {
        SECTIONS
            .iter()
            .position(|s| *s == section)
            .expect("SECTIONS lists every Section")
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

    #[test]
    fn a_zero_capacity_ring_still_works_rather_than_dividing_by_nothing() {
        let mut ring = Ring::new(0);
        ring.push(event(Section::Sync, Level::Info, "A"));
        assert_eq!(ring.capacity(), 1);
        assert_eq!(ring.held(), 1);
    }
}
