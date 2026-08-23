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
    pub fn for_mode(mode: CaptureMode) -> Self {
        let base = match mode {
            CaptureMode::Off => None,
            CaptureMode::Safe => Some(Level::Info),
            CaptureMode::Enhanced => Some(Level::Debug),
            CaptureMode::Full => Some(Level::Trace),
        };
        let mut config = CaptureConfig {
            mode,
            levels: [base; 22],
        };
        if mode == CaptureMode::Safe {
            config.set(Section::Transport, Some(Level::Warn));
            // The pipeline's own health is never turned down. A diagnostics system that stops
            // reporting its own failures at low verbosity has removed the one thing that explains
            // an empty log.
            config.set(Section::Diag, Some(Level::Info));
        }
        config
    }

    /// The level a section is captured at, or `None` when it is off.
    pub fn level(&self, section: Section) -> Option<Level> {
        self.levels[Self::index(section)]
    }

    pub fn set(&mut self, section: Section, level: Option<Level>) {
        let at = Self::index(section);
        self.levels[at] = level;
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
