//! Pure, deterministic **discovery policy**: turn a pile of discovered peer
//! candidates into a bounded, eclipse-resistant **dial plan**.
//!
//! This crate is the single place that decides *what to dial*. The libp2p Actor
//! (catcoms-net) never auto-dials — it surfaces every signed peer record on a
//! never-dropping queue, and a higher layer feeds those records (plus PEX entries
//! and cross-session cache entries) here. The policy:
//!
//! - **unions** candidates for the same peer across sources, taking the freshest
//!   record and merging addresses,
//! - judges **freshness off the registrant's own signed sequence number** (never a
//!   server-asserted TTL — a colluding rendezvous lies about TTL), dropping a record
//!   whose seq we have already bettered (stale / replayed),
//! - **ranks** peers so a member-tag-verified peer leads, then multi-source
//!   corroboration, then a prior proven contact from the cache, then raw single-
//!   rendezvous candidates (the junk/flood) last — but *never drops* the junk, only
//!   sinks it,
//! - counts **≤ 1 trust root per rendezvous** (two colluding rendezvous cannot
//!   manufacture independent corroboration) and **round-robin interleaves** equal-rank
//!   peers across their source so **a single rendezvous** flooding thousands of
//!   records cannot dominate the dial order,
//! - **clamps** the plan to roughly the roster size (you never need to dial more
//!   distinct peers than could plausibly be members), and
//! - meters dials against a **Clock-paced, RNG-jittered budget** shared across all
//!   discovery sources, so junk costs at most `B` dials per window.
//!
//! It **ranks only — it never gates messaging** and never makes a network call. No
//! ambient time/RNG: a `Clock` and an RNG are injected on every `plan` call, exactly
//! like the rest of the stack, so the whole thing is deterministically testable.
//!
//! The round-robin guarantee is scoped to a **single** rendezvous. Distinct *colluding*
//! rendezvous each earn one front-of-line slot among equal-rank peers — but a
//! verified/cache/PEX honest peer outranks all unverified rendezvous junk by score and
//! still leads, so only an honest yet *unverified, uncorroborated, uncached* peer is
//! pushed back, and that is the documented all-rendezvous-colluding residual (answered
//! by cache + PEX + the membership tag). The caller bounds it further by admitting only
//! records from the invite's fixed rendezvous set.

use std::collections::BTreeMap;

use catcoms_rt::{Clock, CryptoRngCore};

mod cache;
mod eclipse;

pub use cache::{AddressCache, CacheConfig, CacheError, CachedPeer};
pub use eclipse::{EclipseConfig, EclipseDetector, EclipseLevel, EclipseObservation};

/// An opaque peer identifier — a libp2p `PeerId`'s bytes (or any stable id). Kept as
/// a `Vec<u8>` so this crate stays free of a libp2p dependency and fully pure.
pub type PeerKey = Vec<u8>;

/// Where a candidate came from — its **trust-root class**. Eclipse-resistance counts
/// *distinct* roots: every rendezvous is at most one root (so two colluding
/// rendezvous cannot fake corroboration); each PEX-vouching member is one root; a
/// cache entry is a prior proven contact (it becomes a counted root only via the live
/// re-proof it later enables, which is the eclipse detector's concern, not ours).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    /// Discovered at a rendezvous node, identified by its peer-id bytes.
    Rendezvous(PeerKey),
    /// Vouched for by a member over PEX, identified by the voucher's device id.
    Pex(PeerKey),
    /// Loaded from the cross-session address cache (a previously proven member).
    Cache,
}

/// One discovered candidate, before merging. The caller (which holds `ns_secret_L`)
/// sets `tag_verified` by recomputing the member-only registration tag — an
/// unverified candidate is never dropped here, only ranked last.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The peer the record names (the record's signer).
    pub peer: PeerKey,
    /// The addresses the record advertised (as multiaddr strings; opaque here).
    pub addresses: Vec<String>,
    /// Which trust root surfaced this candidate.
    pub source: Source,
    /// The registrant's **own signed** monotonic sequence number (the libp2p
    /// `PeerRecord` seq). Freshness is judged off this, never a server TTL.
    ///
    /// INVARIANT: the caller MUST only build a `Candidate` from a `PeerRecord` whose
    /// libp2p signature it has verified, so this `seq` is the peer's *own* signed
    /// claim. libp2p verifies that signature when surfacing a discovered record, so an
    /// off-path attacker cannot mint a high `seq` for a peer it does not control. The
    /// policy folds `seq` into its anti-replay high-water map; an *unauthenticated*
    /// `seq` could pin that high-water and suppress a peer's later genuine records
    /// (an availability-only self-eclipse), which this invariant rules out.
    pub seq: u64,
    /// Whether the member-only registration tag verified (caller-checked).
    pub tag_verified: bool,
}

/// A peer the policy decided to offer for dialing, with its merged addresses, in
/// rank order. (Dialing itself is the caller's job; this is advice.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDial {
    /// The peer to dial.
    pub peer: PeerKey,
    /// The union of addresses learned for it (bounded by `max_addresses`).
    pub addresses: Vec<String>,
}

/// Tunable bounds. Defaults suit a desktop node; tests shrink them.
#[derive(Debug, Clone, Copy)]
pub struct PolicyConfig {
    /// `B`: dials granted per budget window (shared across all sources).
    pub dial_budget: u32,
    /// Budget-window length on the injected clock (ms).
    pub window_ms: u64,
    /// Max RNG jitter (ms) added to each window length, decorrelating the cadence so
    /// an observer cannot predict exactly when the budget refills.
    pub jitter_ms: u64,
    /// Extra dial slots allowed above `roster_size - 1` (headroom for stale cache /
    /// PEX entries that may no longer resolve).
    pub roster_headroom: usize,
    /// A floor on the dial slots so a tiny group (founder + a seed) can still bootstrap.
    pub min_dial_slots: usize,
    /// Cap on merged addresses retained per peer (anti-bloat).
    pub max_addresses: usize,
    /// Cap on distinct peers whose high-water seq we remember across calls (a
    /// bound on the anti-replay map; coarse eviction past this).
    pub max_tracked_peers: usize,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            dial_budget: 8,
            window_ms: 10_000,
            jitter_ms: 2_000,
            roster_headroom: 4,
            min_dial_slots: 3,
            max_addresses: 8,
            max_tracked_peers: 4096,
        }
    }
}

/// Score weights. A tag-verified member dominates everything; otherwise more
/// distinct trust roots rank higher, and a prior proven contact (cache) outranks a
/// raw single-rendezvous record (the flood). Each rendezvous counts once.
const SCORE_TAG_VERIFIED: i64 = 1_000;
const SCORE_PER_RENDEZVOUS: i64 = 30;
const SCORE_PER_PEX: i64 = 50;
const SCORE_CACHE: i64 = 50;
/// Cap distinct roots that contribute to the score, so the ordering can never be
/// inflated without bound (a peer corroborated by 4+ rendezvous is plenty).
const ROOT_CAP: usize = 4;

/// A peer after unioning every candidate that names it.
#[derive(Debug)]
struct Merged {
    peer: PeerKey,
    addresses: Vec<String>,
    /// Distinct rendezvous that surfaced this peer (each = one trust root).
    rendezvous_roots: Vec<PeerKey>,
    /// Distinct members that PEXed this peer.
    pex_roots: Vec<PeerKey>,
    from_cache: bool,
    tag_verified: bool,
    /// Highest signed seq seen for this peer this call.
    seq: u64,
}

impl Merged {
    fn new(peer: PeerKey) -> Self {
        Self {
            peer,
            addresses: Vec::new(),
            rendezvous_roots: Vec::new(),
            pex_roots: Vec::new(),
            from_cache: false,
            tag_verified: false,
            seq: 0,
        }
    }

    fn absorb(&mut self, c: Candidate, max_addresses: usize) {
        self.tag_verified |= c.tag_verified;
        self.seq = self.seq.max(c.seq);
        for a in c.addresses {
            if self.addresses.len() >= max_addresses {
                break;
            }
            if !self.addresses.contains(&a) {
                self.addresses.push(a);
            }
        }
        match c.source {
            Source::Rendezvous(id) => {
                if !self.rendezvous_roots.contains(&id) {
                    self.rendezvous_roots.push(id);
                }
            }
            Source::Pex(id) => {
                if !self.pex_roots.contains(&id) {
                    self.pex_roots.push(id);
                }
            }
            Source::Cache => self.from_cache = true,
        }
    }

    fn score(&self) -> i64 {
        let mut s = 0;
        if self.tag_verified {
            s += SCORE_TAG_VERIFIED;
        }
        s += SCORE_PER_RENDEZVOUS * self.rendezvous_roots.len().min(ROOT_CAP) as i64;
        s += SCORE_PER_PEX * self.pex_roots.len().min(ROOT_CAP) as i64;
        if self.from_cache {
            s += SCORE_CACHE;
        }
        s
    }

    /// The bucket this peer round-robins within, so one rendezvous's flood cannot
    /// dominate: its source rendezvous (smallest id if several), else its PEX
    /// voucher, else the single shared cache bucket.
    fn bucket(&self) -> Vec<u8> {
        if let Some(min) = self.rendezvous_roots.iter().min() {
            let mut b = vec![0u8];
            b.extend_from_slice(min);
            b
        } else if let Some(min) = self.pex_roots.iter().min() {
            let mut b = vec![1u8];
            b.extend_from_slice(min);
            b
        } else {
            vec![2u8]
        }
    }
}

/// The stateful discovery policy. Holds the dial-budget window and the per-peer
/// high-water seq map across calls; `plan` is otherwise a pure function of its inputs.
#[derive(Debug)]
pub struct DiscoveryPolicy {
    config: PolicyConfig,
    /// High-water signed seq per peer, to drop replayed/stale records.
    best_seq: BTreeMap<PeerKey, u64>,
    /// Current budget window start (on the injected clock), or `None` before the
    /// first `plan`.
    window_start_ms: Option<u64>,
    /// This window's length (base + RNG jitter), fixed when the window opens.
    window_len_ms: u64,
    /// Dials granted so far in the current window.
    spent: u32,
}

impl DiscoveryPolicy {
    /// A policy with default bounds.
    pub fn new() -> Self {
        Self::with_config(PolicyConfig::default())
    }

    /// A policy with explicit bounds.
    pub fn with_config(config: PolicyConfig) -> Self {
        Self {
            config,
            best_seq: BTreeMap::new(),
            window_start_ms: None,
            window_len_ms: config.window_ms,
            spent: 0,
        }
    }

    /// Dials still available in the current budget window (diagnostics / tests). Does
    /// not advance the window.
    pub fn remaining_budget(&self) -> u32 {
        self.config.dial_budget.saturating_sub(self.spent)
    }

    /// Rank `candidates` into a bounded dial plan for a group whose roster has
    /// `roster_size` members (including this node). Consumes dial budget for the
    /// peers it returns. Returns peers in dial order (best first).
    pub fn plan(
        &mut self,
        candidates: Vec<Candidate>,
        roster_size: usize,
        clock: &dyn Clock,
        rng: &mut impl CryptoRngCore,
    ) -> Vec<PlannedDial> {
        // 1. Freshness: a peer is stale iff the best seq it presents THIS call is
        //    below the high-water seq we have already accepted for it (a replayed or
        //    superseded record). Drop all of a stale peer's candidates; otherwise
        //    learn the new high-water.
        let mut incoming_max: BTreeMap<PeerKey, u64> = BTreeMap::new();
        for c in &candidates {
            let e = incoming_max.entry(c.peer.clone()).or_insert(c.seq);
            *e = (*e).max(c.seq);
        }
        let mut stale: BTreeMap<PeerKey, bool> = BTreeMap::new();
        for (peer, &max_seq) in &incoming_max {
            let is_stale = matches!(self.best_seq.get(peer), Some(&b) if max_seq < b);
            stale.insert(peer.clone(), is_stale);
            if !is_stale {
                let slot = self.best_seq.entry(peer.clone()).or_insert(max_seq);
                *slot = (*slot).max(max_seq);
            } else {
                tracing::trace!("dropping stale-seq peer from discovery plan");
            }
        }
        self.evict_tracked();

        // 2. Merge surviving candidates by peer (union sources + addresses, max seq).
        let mut merged: BTreeMap<PeerKey, Merged> = BTreeMap::new();
        for c in candidates {
            if *stale.get(&c.peer).unwrap_or(&false) {
                continue;
            }
            merged
                .entry(c.peer.clone())
                .or_insert_with(|| Merged::new(c.peer.clone()))
                .absorb(c, self.config.max_addresses);
        }
        if merged.is_empty() {
            return Vec::new();
        }

        // 3. Assign each peer a within-bucket index so equal-score peers interleave
        //    round-robin across their source (a single rendezvous's Nth record sorts
        //    after every other source's first). Bucket order within is deterministic:
        //    higher score, then fresher seq, then peer bytes.
        let mut items: Vec<Merged> = merged.into_values().collect();
        items.sort_by(|a, b| {
            b.score()
                .cmp(&a.score())
                .then(b.seq.cmp(&a.seq))
                .then(a.peer.cmp(&b.peer))
        });
        let mut bucket_counts: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        let mut within: Vec<usize> = Vec::with_capacity(items.len());
        for m in &items {
            let idx = bucket_counts.entry(m.bucket()).or_insert(0);
            within.push(*idx);
            *idx += 1;
        }
        // Re-sort by (score desc, within-bucket index asc, peer asc): interleaves
        // equal-rank peers across sources while keeping higher-scored peers first.
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by(|&i, &j| {
            items[j]
                .score()
                .cmp(&items[i].score())
                .then(within[i].cmp(&within[j]))
                .then(items[i].peer.cmp(&items[j].peer))
        });

        // 4. Roster clamp: never offer more distinct peers than could plausibly be
        //    members (plus headroom), with a small floor for bootstrap.
        let clamp = (roster_size.saturating_sub(1) + self.config.roster_headroom)
            .max(self.config.min_dial_slots);

        // 5. Dial budget: meter against the Clock-paced, RNG-jittered window.
        let granted = self.grant_budget(clock, rng);
        let take = order.len().min(clamp).min(granted);
        self.spent = self.spent.saturating_add(take as u32);

        order
            .into_iter()
            .take(take)
            .map(|i| PlannedDial {
                peer: items[i].peer.clone(),
                addresses: items[i].addresses.clone(),
            })
            .collect()
    }

    /// Compute the dials available now, rolling the budget window over (with fresh RNG
    /// jitter) if the previous one has elapsed.
    fn grant_budget(&mut self, clock: &dyn Clock, rng: &mut impl CryptoRngCore) -> usize {
        let now = clock.now_ms();
        let expired = match self.window_start_ms {
            None => true,
            Some(start) => now.saturating_sub(start) >= self.window_len_ms,
        };
        if expired {
            self.window_start_ms = Some(now);
            let jitter = if self.config.jitter_ms == 0 {
                0
            } else {
                (rng.next_u32() as u64) % (self.config.jitter_ms + 1)
            };
            self.window_len_ms = self.config.window_ms + jitter;
            self.spent = 0;
        }
        self.remaining_budget() as usize
    }

    /// Bound the anti-replay seq map: past the cap, drop the lowest-keyed entries
    /// (coarse but deterministic; at worst re-admits one already-seen record).
    fn evict_tracked(&mut self) {
        while self.best_seq.len() > self.config.max_tracked_peers {
            let Some(lowest) = self.best_seq.keys().next().cloned() else {
                break;
            };
            self.best_seq.remove(&lowest);
        }
    }
}

impl Default for DiscoveryPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::ManualClock;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn rng() -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(0)
    }

    fn peer(n: u8) -> PeerKey {
        vec![n; 32]
    }

    fn rdv(n: u8) -> PeerKey {
        // A distinct rendezvous-id per `n` (16 bytes, well clear of the peer keys).
        vec![0xA0u8.wrapping_add(n); 16]
    }

    fn cand(p: u8, source: Source, seq: u64, tag_verified: bool) -> Candidate {
        Candidate {
            peer: peer(p),
            addresses: vec![format!("/ip4/10.0.0.{p}/tcp/4001")],
            source,
            seq,
            tag_verified,
        }
    }

    /// A generous budget + window so the budget never interferes with ranking tests.
    fn ranking_policy() -> DiscoveryPolicy {
        DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 1_000,
            roster_headroom: 64,
            min_dial_slots: 64,
            ..PolicyConfig::default()
        })
    }

    #[test]
    fn a_tag_verified_member_and_corroborated_peers_rank_above_junk() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidates = vec![
            // peer 9: junk — single rendezvous, unverified.
            cand(9, Source::Rendezvous(rdv(1)), 1, false),
            // peer 5: corroborated by two distinct rendezvous.
            cand(5, Source::Rendezvous(rdv(1)), 1, false),
            cand(5, Source::Rendezvous(rdv(2)), 1, false),
            // peer 3: tag-verified member (single rendezvous).
            cand(3, Source::Rendezvous(rdv(2)), 1, true),
            // peer 7: cache-only (a prior proven contact).
            cand(7, Source::Cache, 1, false),
        ];
        let plan = pol.plan(candidates, 5, &clock, &mut r);
        let order: Vec<PeerKey> = plan.iter().map(|p| p.peer.clone()).collect();
        assert_eq!(order[0], peer(3), "tag-verified member leads");
        assert_eq!(order[1], peer(5), "two-rendezvous corroboration next");
        assert_eq!(
            order[2],
            peer(7),
            "cache (prior proven contact) beats raw junk"
        );
        assert_eq!(
            *order.last().unwrap(),
            peer(9),
            "single-rendezvous junk ranks last"
        );
    }

    #[test]
    fn a_single_rendezvous_flood_cannot_dominate_dial_order() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let mut candidates = Vec::new();
        // Rendezvous A floods 200 distinct junk peers.
        for p in 0..200u32 {
            candidates.push(Candidate {
                peer: p.to_be_bytes().to_vec(),
                addresses: vec!["/ip4/10.0.0.1/tcp/1".into()],
                source: Source::Rendezvous(rdv(1)),
                seq: 1,
                tag_verified: false,
            });
        }
        // Rendezvous B offers a single (equally unverified) peer.
        let b_peer = vec![0xBB; 8];
        candidates.push(Candidate {
            peer: b_peer.clone(),
            addresses: vec!["/ip4/10.0.0.2/tcp/2".into()],
            source: Source::Rendezvous(rdv(2)),
            seq: 1,
            tag_verified: false,
        });
        let plan = pol.plan(candidates, 5, &clock, &mut r);
        // B's lone peer must surface in the first couple of slots, not be buried
        // behind A's flood (round-robin interleave across the source bucket).
        let pos = plan.iter().position(|d| d.peer == b_peer);
        assert!(
            matches!(pos, Some(p) if p <= 1),
            "B's peer should interleave to the front, got {pos:?}"
        );
    }

    #[test]
    fn a_flood_under_a_small_roster_is_clamped() {
        let mut pol = DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 1_000, // budget high, so the CLAMP (not the budget) bounds it
            roster_headroom: 4,
            min_dial_slots: 3,
            ..PolicyConfig::default()
        });
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidates: Vec<Candidate> = (0..500u32)
            .map(|p| Candidate {
                peer: p.to_be_bytes().to_vec(),
                addresses: vec!["/ip4/10.0.0.1/tcp/1".into()],
                source: Source::Rendezvous(rdv(1)),
                seq: 1,
                tag_verified: false,
            })
            .collect();
        let plan = pol.plan(candidates, 4, &clock, &mut r);
        // roster 4 → (4-1) + headroom 4 = 7 dial slots; 500 is clamped to 7.
        assert_eq!(plan.len(), 7, "500 candidates under roster 4 clamp to 7");
    }

    #[test]
    fn a_stale_seq_record_is_dropped_across_calls() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        // First sighting: peer 5 at seq 10.
        let plan1 = pol.plan(
            vec![cand(5, Source::Rendezvous(rdv(1)), 10, false)],
            5,
            &clock,
            &mut r,
        );
        assert_eq!(plan1.len(), 1);
        // A replayed older record (seq 3) for the same peer is dropped.
        let plan2 = pol.plan(
            vec![cand(5, Source::Rendezvous(rdv(1)), 3, false)],
            5,
            &clock,
            &mut r,
        );
        assert!(plan2.is_empty(), "stale-seq record must be dropped");
        // A fresher record (seq 11) is accepted.
        let plan3 = pol.plan(
            vec![cand(5, Source::Rendezvous(rdv(1)), 11, false)],
            5,
            &clock,
            &mut r,
        );
        assert_eq!(plan3.len(), 1, "a newer seq is accepted");
    }

    #[test]
    fn a_cache_only_peer_is_still_offered() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let plan = pol.plan(vec![cand(7, Source::Cache, 1, false)], 5, &clock, &mut r);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].peer, peer(7));
    }

    #[test]
    fn the_dial_budget_caps_dials_per_window_and_refills_after_it() {
        let mut pol = DiscoveryPolicy::with_config(PolicyConfig {
            dial_budget: 3,
            window_ms: 1_000,
            jitter_ms: 0, // deterministic window for the assertion
            roster_headroom: 64,
            min_dial_slots: 64,
            ..PolicyConfig::default()
        });
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidates: Vec<Candidate> = (0..10u8)
            .map(|p| cand(p, Source::Cache, 1, false))
            .collect();
        // First call: budget 3 caps the plan to 3, even though 10 are offered/clamped.
        let plan1 = pol.plan(candidates.clone(), 64, &clock, &mut r);
        assert_eq!(plan1.len(), 3, "budget caps dials this window");
        assert_eq!(pol.remaining_budget(), 0);
        // Same window: nothing more is granted.
        let plan2 = pol.plan(candidates.clone(), 64, &clock, &mut r);
        assert!(plan2.is_empty(), "budget exhausted within the window");
        // After the window elapses, the budget refills.
        clock.advance_ms(1_001);
        let plan3 = pol.plan(candidates, 64, &clock, &mut r);
        assert_eq!(plan3.len(), 3, "budget refills after the window");
    }

    #[test]
    fn merging_unions_addresses_and_sources_for_one_peer() {
        let mut pol = ranking_policy();
        let clock = ManualClock::new(0);
        let mut r = rng();
        let candidates = vec![
            Candidate {
                peer: peer(5),
                addresses: vec!["/ip4/1.1.1.1/tcp/1".into()],
                source: Source::Rendezvous(rdv(1)),
                seq: 4,
                tag_verified: false,
            },
            Candidate {
                peer: peer(5),
                addresses: vec!["/ip4/2.2.2.2/tcp/2".into()],
                source: Source::Cache,
                seq: 7,
                tag_verified: false,
            },
        ];
        let plan = pol.plan(candidates, 5, &clock, &mut r);
        assert_eq!(plan.len(), 1, "the two records merge into one peer");
        assert_eq!(plan[0].addresses.len(), 2, "addresses union");
    }
}
