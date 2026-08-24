//! Sections, levels, and how much of the app is being watched.
//!
//! Two independent axes, which the previous design conflated into one on/off switch:
//!
//! 1. **Whether a diagnostic session exists at all** ([`CaptureMode`]).
//! 2. **Which parts of the app feed it, and in how much detail** ([`Section`] levels).
//!
//! Separating them is what makes "turn on diagnostics" a reasonable thing to ask a user to do.
//! With one switch, the only honest options were "capture almost nothing" and "capture everything
//! including the parts that narrate every address this device has ever seen"; the second is both
//! the bulk of the volume and the most identifying part of it, so the switch stayed off and
//! nobody had a log when they needed one.

/// How much of a diagnostic session exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum CaptureMode {
    /// No history is retained. Live health gauges still read, but nothing accumulates.
    Off,
    /// The default. Stable codes, counts, durations, and identifiers reduced to session
    /// references. No literal addresses, so a Safe report is one a user can paste in public.
    #[default]
    Safe,
    /// Safe, plus literal addresses and transport detail. For network and multi-peer failures
    /// that cannot be localised without knowing which address was actually tried.
    Enhanced,
    /// Enhanced, plus per-span protocol detail. Maintainer reproduction only, and never a
    /// preference that survives a restart.
    Full,
}

impl CaptureMode {
    /// Whether literal network addresses may be rendered.
    ///
    /// The single question that decides whether a report is publishable, so it is one function
    /// rather than a comparison repeated at each call site where it could drift.
    pub fn allows_raw_addresses(self) -> bool {
        matches!(self, CaptureMode::Enhanced | CaptureMode::Full)
    }

    /// Whether anything at all is retained.
    pub fn captures(self) -> bool {
        !matches!(self, CaptureMode::Off)
    }

    /// Whether this mode should be forgotten at the next launch.
    ///
    /// Full trace is expensive and revealing, and somebody who turned it on to reproduce one bug
    /// should not still be running it a fortnight later because they forgot.
    pub fn expires_at_restart(self) -> bool {
        matches!(self, CaptureMode::Full)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CaptureMode::Off => "off",
            CaptureMode::Safe => "safe",
            CaptureMode::Enhanced => "enhanced",
            CaptureMode::Full => "full",
        }
    }

    /// Parse a mode name, rejecting anything unrecognised.
    ///
    /// Returns `Option` rather than falling back to a default on purpose. This parses a value
    /// arriving from the webview, and the two plausible defaults are both wrong: falling back to
    /// `Safe` makes a mistyped `enhanced` look like a control that worked and did nothing, and
    /// falling back to the *current* mode hides the same mistake even better. A caller that cannot
    /// understand what it was asked for should say so.
    pub fn parse(name: &str) -> Option<CaptureMode> {
        match name {
            "off" => Some(CaptureMode::Off),
            "safe" => Some(CaptureMode::Safe),
            "enhanced" => Some(CaptureMode::Enhanced),
            "full" => Some(CaptureMode::Full),
            _ => None,
        }
    }
}

/// Severity, ordered so a filter is a comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }

    /// Parse a `tracing` level name. Anything unrecognised becomes `Info` rather than being
    /// dropped: an event with a level nobody anticipated is still an event.
    pub fn from_tracing(name: &str) -> Self {
        match name {
            "ERROR" => Level::Error,
            "WARN" => Level::Warn,
            "DEBUG" => Level::Debug,
            "TRACE" => Level::Trace,
            _ => Level::Info,
        }
    }

    /// Parse a level name for a *setting*, rejecting anything unrecognised.
    ///
    /// The opposite policy to [`Level::from_tracing`], and deliberately so: that one is classifying
    /// an event that already happened, where guessing `Info` is better than discarding it. This one
    /// is applying a control, where guessing means the user asked for one thing and got another.
    pub fn parse(name: &str) -> Option<Level> {
        match name.to_ascii_uppercase().as_str() {
            "ERROR" => Some(Level::Error),
            "WARN" => Some(Level::Warn),
            "INFO" => Some(Level::Info),
            "DEBUG" => Some(Level::Debug),
            "TRACE" => Some(Level::Trace),
            _ => None,
        }
    }
}

/// Which part of the app an event came from.
///
/// The taxonomy the adversarial review calls for. It is finer than the six sections the console
/// shows, and deliberately so: the console's six are a *presentation* chosen so a person can find
/// the failing layer in seconds, while these are what an event is *tagged* with. One is a view
/// over the other, and collapsing them into a single list would mean either a console with
/// twenty-two rail items or a taxonomy too coarse to filter on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    /// The diagnostic pipeline's own health. Must always be cheap: it is the one section that
    /// still has to work when everything else has stopped.
    Diag,
    /// Build, process, platform, and the startup sequence.
    Startup,
    /// Svelte, the webview, rendering and navigation.
    Ui,
    /// Tauri commands and events.
    Ipc,
    /// Actors, tasks, mailboxes, timers.
    Runtime,
    /// Vault, lock state, continuity, backup.
    Vault,
    /// The blob store, manifests, integrity and repair.
    Storage,
    /// Device identity, pairing, grant lifecycle.
    Identity,
    /// MLS, membership, roles, moderation authority.
    Membership,
    /// libp2p listeners, dials and connections.
    Transport,
    /// UPnP, PCP, NAT-PMP, AutoNAT, relay, rendezvous.
    Reachability,
    /// PEX, cached records, route changes.
    Discovery,
    /// Invite, preview, admission, reply, switchboard.
    Join,
    /// Replication, catch-up, delivery, CRDT documents.
    Sync,
    /// Chat, channels, unread, inbox, delivery, jukebox.
    Channels,
    /// Wiki, status, calendar, livery, moderation.
    Documents,
    /// Uploads, downloads, media protocol, cache.
    Files,
    /// WebRTC, signalling, ICE, TURN, devices.
    Voice,
    /// MIDI, microphone and output, notifications.
    Devices,
    /// The updater and external launches.
    Updates,
    /// CPU, memory, queues, responsiveness.
    Performance,
    /// Redaction, export, issue preparation.
    Privacy,
}

/// A snapshot of the capture config that can be consulted without taking a lock.
///
/// The reason this exists rather than the config being read under the hub's mutex: an event the
/// config excludes should cost as close to nothing as possible, and taking a global lock only to
/// discover an event is unwanted turns the *filtered* case into the contended one. Under a debug
/// or trace level on a busy section that is most events, on every thread in the process at once.
///
/// Two relaxed atomic loads instead. Relaxed is right here: a capture-level change racing an
/// in-flight event can land either side of it, and neither answer is wrong.
#[derive(Debug)]
pub struct CaptureGate {
    mode: std::sync::atomic::AtomicU8,
    /// `0` means the section is off; otherwise the [`Level`] as a discriminant plus one.
    levels: [std::sync::atomic::AtomicU8; 22],
}

impl CaptureGate {
    pub fn new(config: &CaptureConfig) -> Self {
        let gate = CaptureGate {
            mode: std::sync::atomic::AtomicU8::new(0),
            levels: std::array::from_fn(|_| std::sync::atomic::AtomicU8::new(0)),
        };
        gate.store(config);
        gate
    }

    /// Replace the snapshot after a config change.
    pub fn store(&self, config: &CaptureConfig) {
        use std::sync::atomic::Ordering;
        self.mode.store(config.mode as u8, Ordering::Relaxed);
        for (at, section) in SECTIONS.iter().enumerate() {
            let encoded = config.level(*section).map_or(0, |l| l as u8 + 1);
            self.levels[at].store(encoded, Ordering::Relaxed);
        }
    }

    /// Whether an event at this section and level is captured, without taking a lock.
    pub fn admits(&self, section: Section, level: Level) -> bool {
        use std::sync::atomic::Ordering;
        if self.mode.load(Ordering::Relaxed) == CaptureMode::Off as u8 {
            return false;
        }
        let encoded = self.levels[section.index()].load(Ordering::Relaxed);
        // `Level` is ordered loudest first, so "at least as loud as the threshold" is `<=`. The
        // encoding is the level plus one, leaving zero to mean "section off", so the comparison is
        // shifted rather than the stored value.
        encoded != 0 && (level as u8) < encoded
    }
}

/// Every section, in the order reports render them.
pub const SECTIONS: [Section; 22] = [
    Section::Diag,
    Section::Startup,
    Section::Ui,
    Section::Ipc,
    Section::Runtime,
    Section::Vault,
    Section::Storage,
    Section::Identity,
    Section::Membership,
    Section::Transport,
    Section::Reachability,
    Section::Discovery,
    Section::Join,
    Section::Sync,
    Section::Channels,
    Section::Documents,
    Section::Files,
    Section::Voice,
    Section::Devices,
    Section::Updates,
    Section::Performance,
    Section::Privacy,
];

impl Section {
    /// This section's slot in the per-section arrays.
    ///
    /// A match rather than a scan of [`SECTIONS`]. It is called twice per recorded event while the
    /// hub's lock is held, and a linear search over twenty-two entries inside a global critical
    /// section is the kind of small cost that only shows up as unexplained jitter under load.
    pub fn index(self) -> usize {
        match self {
            Section::Diag => 0,
            Section::Startup => 1,
            Section::Ui => 2,
            Section::Ipc => 3,
            Section::Runtime => 4,
            Section::Vault => 5,
            Section::Storage => 6,
            Section::Identity => 7,
            Section::Membership => 8,
            Section::Transport => 9,
            Section::Reachability => 10,
            Section::Discovery => 11,
            Section::Join => 12,
            Section::Sync => 13,
            Section::Channels => 14,
            Section::Documents => 15,
            Section::Files => 16,
            Section::Voice => 17,
            Section::Devices => 18,
            Section::Updates => 19,
            Section::Performance => 20,
            Section::Privacy => 21,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Section::Diag => "diag",
            Section::Startup => "startup",
            Section::Ui => "ui",
            Section::Ipc => "ipc",
            Section::Runtime => "runtime",
            Section::Vault => "vault",
            Section::Storage => "storage",
            Section::Identity => "identity",
            Section::Membership => "membership",
            Section::Transport => "transport",
            Section::Reachability => "reachability",
            Section::Discovery => "discovery",
            Section::Join => "join",
            Section::Sync => "sync",
            Section::Channels => "channels",
            Section::Documents => "documents",
            Section::Files => "files",
            Section::Voice => "voice",
            Section::Devices => "devices",
            Section::Updates => "updates",
            Section::Performance => "performance",
            Section::Privacy => "privacy",
        }
    }

    /// Parse a section name, rejecting anything unrecognised.
    ///
    /// A scan of [`SECTIONS`] rather than a second twenty-two-arm match. Unlike [`Section::index`]
    /// this is never on the recording path, it runs when somebody changes a setting, so the cost is
    /// irrelevant and the property that matters is the other one: a scan reads the names from
    /// [`Section::as_str`] and therefore cannot drift out of step with them.
    pub fn parse(name: &str) -> Option<Section> {
        SECTIONS.iter().copied().find(|s| s.as_str() == name)
    }

    /// The section a `tracing` target belongs to.
    ///
    /// A crate is a coarse signal, so this is a starting point rather than the last word: an event
    /// built through [`crate::event::DiagnosticEvent`] states its own section and that always
    /// wins. This exists so the events the app already emits land somewhere sensible without every
    /// one of them being rewritten first, which is what makes the migration incremental instead of
    /// a flag day.
    pub fn from_target(target: &str) -> Section {
        match target.split("::").next().unwrap_or(target) {
            "catcoms_ui" => Section::Ui,
            "catcoms_net" => Section::Transport,
            "catcoms_discovery" => Section::Discovery,
            "catcoms_sync" => Section::Sync,
            "catcoms_mls" => Section::Membership,
            "catcoms_storage" => Section::Storage,
            "catcoms_replication" => Section::Sync,
            "catcoms_crypto" => Section::Identity,
            "catcoms_log" | "catcoms_diagnostics" => Section::Diag,
            _ => Section::Runtime,
        }
    }

    /// The console section this belongs under. See the type docs on why these are separate.
    pub fn view(self) -> ConsoleView {
        match self {
            Section::Ui => ConsoleView::Frontend,
            Section::Transport | Section::Reachability | Section::Discovery | Section::Join => {
                ConsoleView::Network
            }
            Section::Voice => ConsoleView::Voice,
            Section::Storage | Section::Files | Section::Vault => ConsoleView::Storage,
            _ => ConsoleView::Backend,
        }
    }
}

/// The six sections the debug console shows, as a view over the twenty-two above.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConsoleView {
    Overview,
    Network,
    Voice,
    Backend,
    Frontend,
    Storage,
}

impl ConsoleView {
    pub fn as_str(self) -> &'static str {
        match self {
            ConsoleView::Overview => "overview",
            ConsoleView::Network => "network",
            ConsoleView::Voice => "voice",
            ConsoleView::Backend => "backend",
            ConsoleView::Frontend => "frontend",
            ConsoleView::Storage => "storage",
        }
    }
}

/// What is being captured, and how much of it.
#[derive(Clone, Debug)]
pub struct CaptureConfig {
    pub mode: CaptureMode,
    /// The level each section is captured at. A section set to `None` is off entirely.
    levels: [Option<Level>; 22],
}

impl Default for CaptureConfig {
    fn default() -> Self {
        CaptureConfig::for_mode(CaptureMode::Safe)
    }
}

impl CaptureConfig {
    /// The default section levels for a mode.
    ///
    /// `Transport` is the one that varies most, and for a reason: at debug it narrates every
    /// connection, stream and address the node sees. That is simultaneously the bulk of the
    /// volume, the most identifying part of a report, and the only thing that explains why a node
    /// cannot connect. So Safe keeps it at warn and Enhanced opens it up, which is the trade a
    /// user is actually making when they turn Enhanced on.
    ///
    /// # Safe is a content decision, not a verbosity one
    ///
    /// Safe used to hold every section at `Info`, which quietly made the mode do two unrelated
    /// jobs. The correlation stages that answer "which of the ten stages failed" are recorded at
    /// `Debug`, because there is one per stage of every command and they are of no interest until
    /// something goes wrong. Holding the default mode at `Info` therefore threw all of them away
    /// before anything could ask, so the default record could say that a send failed and never
    /// which stage.
    ///
    /// The two axes are meant to be independent, so they are: what a mode changes is which values
    /// may be rendered literally, and whether the transport firehose is on. How much of the app
    /// speaks is the per-section levels, which the user can move separately.
    pub fn for_mode(mode: CaptureMode) -> Self {
        let base = match mode {
            CaptureMode::Off => None,
            CaptureMode::Safe | CaptureMode::Enhanced => Some(Level::Debug),
            CaptureMode::Full => Some(Level::Trace),
        };
        let mut config = CaptureConfig {
            mode,
            levels: [base; 22],
        };
        if mode == CaptureMode::Safe {
            // The one section a Safe report holds back, and the whole of what Safe costs in
            // coverage. Its warnings still speak, so a dial failure is recorded; what is dropped is
            // the per-connection churn, which is both the bulk of the volume and the most
            // identifying part of a report a user is about to share.
            config.set(Section::Transport, Some(Level::Warn));
        }
        config
    }

    /// The level a section is captured at, or `None` when it is off.
    pub fn level(&self, section: Section) -> Option<Level> {
        self.levels[section.index()]
    }

    pub fn set(&mut self, section: Section, level: Option<Level>) {
        self.levels[section.index()] = level;
    }

    /// Whether an event at this section and level is captured.
    pub fn admits(&self, section: Section, level: Level) -> bool {
        if !self.mode.captures() {
            return false;
        }
        // `Level` is ordered loudest first, so "at least as loud as the threshold" is `<=`.
        self.level(section)
            .is_some_and(|threshold| level <= threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_appears_exactly_once_in_the_render_order() {
        // The array is indexed into by position, so a duplicate or omission would silently make
        // two sections share a level.
        let mut seen: Vec<&str> = SECTIONS.iter().map(|s| s.as_str()).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "a section is listed twice");
        assert_eq!(count, 22);
    }

    #[test]
    fn off_captures_nothing_whatever_the_section_levels_say() {
        let mut config = CaptureConfig::for_mode(CaptureMode::Off);
        config.set(Section::Transport, Some(Level::Trace));
        assert!(
            !config.admits(Section::Transport, Level::Error),
            "off means off"
        );
    }

    /// The trade Enhanced exists to offer, pinned. Safe holds the transport quiet because it is
    /// the most identifying and highest-volume part of a report; Enhanced is the user deciding
    /// that a connection problem is worth the detail.
    #[test]
    fn safe_holds_the_transport_quiet_and_enhanced_opens_it_up() {
        let safe = CaptureConfig::for_mode(CaptureMode::Safe);
        assert!(safe.admits(Section::Transport, Level::Warn));
        assert!(!safe.admits(Section::Transport, Level::Debug));
        assert!(
            safe.admits(Section::Sync, Level::Info),
            "product layers still speak"
        );

        let enhanced = CaptureConfig::for_mode(CaptureMode::Enhanced);
        assert!(enhanced.admits(Section::Transport, Level::Debug));
        // Transport is the *only* section the two modes disagree about. What else separates them is
        // how a value renders, not how much of the app speaks.
        for section in SECTIONS {
            if section == Section::Transport {
                continue;
            }
            assert_eq!(
                safe.level(section),
                enhanced.level(section),
                "{section:?} should not vary with the mode"
            );
        }
    }

    /// The correlation stages have to survive the default mode.
    ///
    /// They are recorded at `Debug`, because there is one per stage of every command and none of
    /// them is interesting until something goes wrong. Safe used to hold every section at `Info`,
    /// which threw all of them away before anything could ask: the default record could say a send
    /// failed and never which of its stages did. That is the exact question the trace exists for.
    #[test]
    fn the_default_mode_still_records_which_stage_of_an_operation_failed() {
        let safe = CaptureConfig::for_mode(CaptureMode::Safe);
        assert!(safe.admits(Section::Ipc, Level::Debug), "command stages");
        assert!(safe.admits(Section::Runtime, Level::Debug), "actor stages");
        assert!(
            safe.admits(Section::Channels, Level::Debug),
            "the send itself"
        );
        assert!(
            safe.admits(Section::Ui, Level::Debug),
            "and the webview's half"
        );
    }

    /// A diagnostics pipeline that goes quiet about itself has removed the only thing that could
    /// explain an empty log.
    #[test]
    fn the_pipeline_never_stops_reporting_on_itself_while_capture_is_on() {
        for mode in [CaptureMode::Safe, CaptureMode::Enhanced, CaptureMode::Full] {
            let config = CaptureConfig::for_mode(mode);
            assert!(config.admits(Section::Diag, Level::Info), "{mode:?}");
            assert!(config.admits(Section::Diag, Level::Error), "{mode:?}");
        }
    }

    #[test]
    fn only_deliberately_chosen_modes_reveal_a_literal_address() {
        assert!(!CaptureMode::Off.allows_raw_addresses());
        assert!(!CaptureMode::Safe.allows_raw_addresses());
        assert!(CaptureMode::Enhanced.allows_raw_addresses());
        assert!(CaptureMode::Full.allows_raw_addresses());
    }

    #[test]
    fn full_trace_does_not_survive_a_restart() {
        assert!(CaptureMode::Full.expires_at_restart());
        assert!(!CaptureMode::Safe.expires_at_restart());
    }

    #[test]
    fn a_section_can_be_turned_off_on_its_own() {
        let mut config = CaptureConfig::for_mode(CaptureMode::Safe);
        config.set(Section::Ui, None);
        assert!(!config.admits(Section::Ui, Level::Error));
        assert!(
            config.admits(Section::Sync, Level::Error),
            "the others are untouched"
        );
    }

    #[test]
    fn targets_land_in_the_section_they_belong_to() {
        assert_eq!(Section::from_target("catcoms_net"), Section::Transport);
        assert_eq!(Section::from_target("catcoms_ui"), Section::Ui);
        assert_eq!(Section::from_target("catcoms_sync::join"), Section::Sync);
        // Anything unrecognised is still captured, under the section least likely to mislead.
        assert_eq!(Section::from_target("some_dependency"), Section::Runtime);
    }

    /// A control that silently does something other than what it was asked is worse than one that
    /// refuses, because the user has no way to tell it did not work. Every name that round-trips
    /// must parse, and nothing else may.
    #[test]
    fn a_setting_that_cannot_be_understood_is_refused_rather_than_guessed() {
        for mode in [
            CaptureMode::Off,
            CaptureMode::Safe,
            CaptureMode::Enhanced,
            CaptureMode::Full,
        ] {
            assert_eq!(CaptureMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(CaptureMode::parse("enhaced"), None, "a typo is not Safe");
        assert_eq!(CaptureMode::parse("SAFE"), None, "and not case-insensitive");
        assert_eq!(CaptureMode::parse(""), None);

        for level in [
            Level::Error,
            Level::Warn,
            Level::Info,
            Level::Debug,
            Level::Trace,
        ] {
            assert_eq!(Level::parse(level.as_str()), Some(level));
        }
        assert_eq!(
            Level::parse("warn"),
            Some(Level::Warn),
            "a level is spelled either way"
        );
        assert_eq!(Level::parse("verbose"), None);

        for section in SECTIONS {
            assert_eq!(Section::parse(section.as_str()), Some(section));
        }
        assert_eq!(
            Section::parse("network"),
            None,
            "a console view is not a section"
        );
        assert_eq!(
            Section::parse("catcoms_net"),
            None,
            "nor is a tracing target"
        );
    }

    #[test]
    fn the_console_view_covers_every_section() {
        for section in SECTIONS {
            // A section with no view would vanish from the console entirely, which is the failure
            // this whole exercise exists to stop.
            let _: ConsoleView = section.view();
        }
        assert_eq!(Section::Transport.view(), ConsoleView::Network);
        assert_eq!(Section::Join.view(), ConsoleView::Network);
        assert_eq!(Section::Ui.view(), ConsoleView::Frontend);
        assert_eq!(Section::Voice.view(), ConsoleView::Voice);
        assert_eq!(Section::Files.view(), ConsoleView::Storage);
        assert_eq!(Section::Membership.view(), ConsoleView::Backend);
    }
}
