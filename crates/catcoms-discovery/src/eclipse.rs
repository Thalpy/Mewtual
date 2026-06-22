//! Advisory eclipse detector (6e-3d-8).
//!
//! An eclipse attack isolates a member so it only ever talks to attacker-controlled
//! peers/rendezvous. Total isolation is **undecidable from inside the node** — but a
//! node *can* notice the warning signs and surface a CAUTION so the user verifies
//! out-of-band. This detector is the warning siren, and **only** a siren:
//!
//! - It **never gates messaging** and **never blocks a Remove** — it returns an
//!   advisory [`EclipseLevel`], nothing more. (Weaponizing it to block a legitimate
//!   removal — review finding H3 — is structurally impossible: it has no gate.)
//! - It works off three locally-known, mostly-unforgeable signals:
//!   - **R** = roster size ([`catcoms_mls::ServerGroup::member_count`]) — local, the
//!     attacker cannot shrink it.
//!   - **D** = distinct roster devices reached with a **live handshake** this session
//!     (including self). `(D-1)/(R-1)` is the *reach* — what fraction of the other
//!     members we can actually talk to.
//!   - **S** = distinct **trust roots** behind our discovered peers: every rendezvous
//!     counts ≤ 1 (colluding rendezvous can't fake independence), each PEX-vouching
//!     member = 1, a cache hit counts only once a live re-proof confirms it.
//! - It is **hysteretic and Clock-paced**: a node must look suspect for a continuous
//!   `grace_ms` before CAUTION is raised, and look healthy for `clear_ms` before it
//!   clears — so a transient partition doesn't flap the warning.
//!
//! `suspect = R > floor && reach < min_reach && S < min_sources`. A small group (R ≤
//! floor) is never suspect (you simply *have* few peers); good reach OR enough trust
//! roots clears suspicion (both must be low to suspect). No ambient time: a `Clock` is
//! injected on every [`EclipseDetector::observe`].

use catcoms_rt::Clock;

/// Tunable thresholds. The defaults (floor 3, reach 0.20, sources 2, 30 s grace/clear)
/// are first guesses — surface them as config and tune against staging, never hard-code.
#[derive(Debug, Clone, Copy)]
pub struct EclipseConfig {
    /// Roster sizes at or below this never raise a warning (a small group genuinely
    /// has few peers; suspicion would be all false positives).
    pub roster_floor: usize,
    /// Reach `(D-1)/(R-1)` below this is "low" — we reach too few of the other members.
    pub min_reach: f64,
    /// Fewer than this many distinct trust roots is "low" — too few independent sources.
    pub min_sources: usize,
    /// Must look suspect continuously for this long (ms, injected clock) before CAUTION.
    pub grace_ms: u64,
    /// Must look healthy continuously for this long before CAUTION clears (hysteresis).
    pub clear_ms: u64,
}

impl Default for EclipseConfig {
    fn default() -> Self {
        Self {
            roster_floor: 3,
            min_reach: 0.20,
            min_sources: 2,
            grace_ms: 30_000,
            clear_ms: 30_000,
        }
    }
}

/// The advisory level. Never anything that gates — just a hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EclipseLevel {
    /// Nothing unusual.
    Ok,
    /// Sustained signs of isolation — surface to the user; verify membership/contacts
    /// out-of-band. Messaging and removals continue unaffected.
    Caution,
}

/// One snapshot of the three signals, taken by the caller from local state.
#[derive(Debug, Clone, Copy)]
pub struct EclipseObservation {
    /// `R` — current roster size (local, unforgeable).
    pub roster_size: usize,
    /// `D` — distinct roster devices reached with a live handshake this session,
    /// **including self** (so a fully-reachable node has `D == R`).
    pub reachable_devices: usize,
    /// `S` — distinct trust roots behind our discovered peers (rendezvous ≤ 1 each).
    pub trust_roots: usize,
}

/// The hysteretic, Clock-paced detector. Advisory only.
#[derive(Debug)]
pub struct EclipseDetector {
    config: EclipseConfig,
    level: EclipseLevel,
    /// When we first looked suspect in the current healthy spell (`None` if healthy now).
    suspect_since: Option<u64>,
    /// When we first looked healthy in the current CAUTION spell (`None` if suspect now).
    clear_since: Option<u64>,
}

impl EclipseDetector {
    /// A detector starting in the healthy state.
    pub fn new(config: EclipseConfig) -> Self {
        Self {
            config,
            level: EclipseLevel::Ok,
            suspect_since: None,
            clear_since: None,
        }
    }

    /// The current advisory level (no state change).
    pub fn level(&self) -> EclipseLevel {
        self.level
    }

    /// Whether the snapshot looks suspect *right now* (before hysteresis): a
    /// big-enough roster we can reach little of, behind too few trust roots.
    fn instantaneously_suspect(&self, o: &EclipseObservation) -> bool {
        if o.roster_size <= self.config.roster_floor {
            return false;
        }
        let reach = if o.roster_size <= 1 {
            1.0
        } else {
            (o.reachable_devices.saturating_sub(1) as f64) / ((o.roster_size - 1) as f64)
        };
        reach < self.config.min_reach && o.trust_roots < self.config.min_sources
    }

    /// Feed a fresh observation and advance the hysteresis. Returns the (possibly
    /// updated) advisory level. Call periodically on the injected clock.
    ///
    /// The grace/clear windows use `now - since` elapsed timing, so they assume a
    /// roughly-monotonic `Clock` (the same assumption as the dial budget and the PEX
    /// rate limit). A backward wall-clock step only *defers* a raise or a clear — both
    /// fail-safe directions for an advisory that gates nothing.
    pub fn observe(&mut self, o: EclipseObservation, clock: &dyn Clock) -> EclipseLevel {
        let now = clock.now_ms();
        let suspect = self.instantaneously_suspect(&o);
        match self.level {
            EclipseLevel::Ok => {
                if suspect {
                    self.clear_since = None;
                    let since = *self.suspect_since.get_or_insert(now);
                    if now.saturating_sub(since) >= self.config.grace_ms {
                        self.level = EclipseLevel::Caution;
                        self.suspect_since = None;
                        tracing::warn!(
                            "eclipse detector raised CAUTION (sustained isolation signs)"
                        );
                    }
                } else {
                    self.suspect_since = None;
                }
            }
            EclipseLevel::Caution => {
                if suspect {
                    self.clear_since = None;
                } else {
                    self.suspect_since = None;
                    let since = *self.clear_since.get_or_insert(now);
                    if now.saturating_sub(since) >= self.config.clear_ms {
                        self.level = EclipseLevel::Ok;
                        self.clear_since = None;
                        tracing::info!("eclipse detector cleared CAUTION");
                    }
                }
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;

    fn obs(r: usize, d: usize, s: usize) -> EclipseObservation {
        EclipseObservation {
            roster_size: r,
            reachable_devices: d,
            trust_roots: s,
        }
    }

    #[test]
    fn a_big_all_attacker_roster_is_suspect_only_after_the_grace_window() {
        let mut det = EclipseDetector::new(EclipseConfig::default());
        let clock = ManualClock::new(0);
        // R=20, reach (1-1)/19 = 0, S=1 — instantaneously suspect, but held under grace.
        assert_eq!(det.observe(obs(20, 1, 1), &clock), EclipseLevel::Ok);
        clock.advance_ms(29_999);
        assert_eq!(det.observe(obs(20, 1, 1), &clock), EclipseLevel::Ok);
        clock.advance_ms(1);
        assert_eq!(
            det.observe(obs(20, 1, 1), &clock),
            EclipseLevel::Caution,
            "raised once suspect for the full grace window"
        );
    }

    #[test]
    fn a_small_group_is_never_suspect() {
        let mut det = EclipseDetector::new(EclipseConfig::default());
        let clock = ManualClock::new(0);
        // R=2 ≤ floor(3): a tiny group genuinely has few peers.
        assert_eq!(det.observe(obs(2, 1, 0), &clock), EclipseLevel::Ok);
        clock.advance_ms(10 * 60_000);
        assert_eq!(det.observe(obs(2, 1, 0), &clock), EclipseLevel::Ok);
    }

    #[test]
    fn a_well_reached_partition_is_not_suspect() {
        let mut det = EclipseDetector::new(EclipseConfig::default());
        let clock = ManualClock::new(0);
        // R=20 but reach 17/19 ≈ 0.89 ≥ 0.20 — even with one trust root, not suspect
        // (both reach AND sources must be low to suspect).
        assert_eq!(det.observe(obs(20, 18, 1), &clock), EclipseLevel::Ok);
        clock.advance_ms(120_000);
        assert_eq!(det.observe(obs(20, 18, 1), &clock), EclipseLevel::Ok);
    }

    #[test]
    fn enough_trust_roots_clears_suspicion_even_at_low_reach() {
        let mut det = EclipseDetector::new(EclipseConfig::default());
        let clock = ManualClock::new(0);
        // Low reach, but S=2 ≥ min_sources — diverse sources, so not suspect.
        assert_eq!(det.observe(obs(20, 1, 2), &clock), EclipseLevel::Ok);
        clock.advance_ms(120_000);
        assert_eq!(det.observe(obs(20, 1, 2), &clock), EclipseLevel::Ok);
    }

    #[test]
    fn a_single_low_source_with_low_reach_is_suspect() {
        let mut det = EclipseDetector::new(EclipseConfig::default());
        let clock = ManualClock::new(0);
        assert_eq!(det.observe(obs(10, 2, 1), &clock), EclipseLevel::Ok); // reach 1/9 ≈ 0.11
        clock.advance_ms(30_000);
        assert_eq!(det.observe(obs(10, 2, 1), &clock), EclipseLevel::Caution);
    }

    #[test]
    fn caution_clears_hysteretically_after_sustained_recovery() {
        let mut det = EclipseDetector::new(EclipseConfig::default());
        let clock = ManualClock::new(0);
        // Raise CAUTION.
        det.observe(obs(20, 1, 1), &clock);
        clock.advance_ms(30_000);
        assert_eq!(det.observe(obs(20, 1, 1), &clock), EclipseLevel::Caution);
        // Recovery starts; the warning holds through the clear window…
        clock.advance_ms(1);
        assert_eq!(det.observe(obs(20, 20, 3), &clock), EclipseLevel::Caution);
        clock.advance_ms(29_999);
        assert_eq!(det.observe(obs(20, 20, 3), &clock), EclipseLevel::Caution);
        // …then clears once recovery has been sustained for clear_ms.
        clock.advance_ms(1);
        assert_eq!(det.observe(obs(20, 20, 3), &clock), EclipseLevel::Ok);
    }

    #[test]
    fn a_transient_suspect_blip_does_not_flap_the_warning() {
        let mut det = EclipseDetector::new(EclipseConfig::default());
        let clock = ManualClock::new(0);
        assert_eq!(det.observe(obs(20, 1, 1), &clock), EclipseLevel::Ok);
        clock.advance_ms(10_000); // suspect, but under grace
        assert_eq!(det.observe(obs(20, 1, 1), &clock), EclipseLevel::Ok);
        clock.advance_ms(1_000); // recovers before grace elapses
        assert_eq!(det.observe(obs(20, 20, 3), &clock), EclipseLevel::Ok);
        // The grace timer reset, so a fresh suspect spell starts from scratch.
        clock.advance_ms(29_999);
        assert_eq!(det.observe(obs(20, 1, 1), &clock), EclipseLevel::Ok);
    }
}
