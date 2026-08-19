//! Cross-session address cache (6e-3d-8).
//!
//! A returning node faces a **first-contact eclipse**: before it has reached any
//! member it has no peers, so a hostile rendezvous can feed it only Sybils. The cure
//! is to remember, across restarts, the **proven members** it reached last session and
//! offer them as dial candidates immediately; past any hostile rendezvous.
//!
//! This is the in-memory cache with the right semantics + a serialization seam; the
//! persistent SQLCipher backing is platform/storage-phase work (like the rest of the
//! at-rest store), so [`AddressCache::to_bytes`] / [`AddressCache::from_bytes`] carry a
//! **keyed integrity tag** that detects tampering on load (a colluding host cannot edit
//! a row undetected). Properties:
//!
//! - **Proven members only.** The caller inserts a peer only after verifying its
//!   signed record belongs to a current roster member; the cache stores opaque
//!   `record` bytes and re-offers them, but a hit **counts toward a trust root only
//!   after a fresh live re-proof** (the eclipse detector's `S`), never on the cache's
//!   say-so. (A colluding host that forged a row would fail that live re-proof.)
//! - **Freshness off the registrant's own signed seq**, never a server-asserted TTL.
//! - **RNG-jittered eviction** when over capacity; decorrelated, so an attacker
//!   cannot steer which honest entry is dropped.
//! - **Tamper-detected on load.** A flipped byte fails the keyed tag → the whole load
//!   is refused rather than trusting a doctored row.

use std::collections::BTreeMap;

use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};
use thiserror::Error;

use crate::PeerKey;

/// Domain string mixed into the serialized cache body (and thus the integrity tag).
const CACHE_DOMAIN: &str = "catcoms/addr-cache/v1";
/// A hard cap on entries a `from_bytes` will decode, independent of config, so a
/// forged length cannot drive a huge allocation before the tag is even checked.
const MAX_DECODE_ENTRIES: usize = 65_536;
/// A hard cap on addresses per entry on decode (same reasoning).
const MAX_DECODE_ADDRESSES: usize = 32;

/// Why a cache load failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CacheError {
    /// The keyed integrity tag did not verify; the bytes were tampered with.
    #[error("address cache integrity tag mismatch (tampered)")]
    Tampered,
    /// The serialized bytes were malformed.
    #[error("malformed address cache")]
    Malformed,
}

/// One cached, previously-proven member: its identity key, dialable addresses, the
/// signed `seq` it was last seen at, and the opaque signed record bytes (re-verified
/// live before the peer is ever trusted again).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPeer {
    /// The cache key; the member's **device id** (not its self-asserted transport
    /// peer id; see the 6e-3d-7 `PeerDescriptor` note).
    pub peer: PeerKey,
    /// Dialable multiaddr strings.
    pub addresses: Vec<String>,
    /// The registrant's own signed monotonic sequence (freshness; never a server TTL).
    pub seq: u64,
    /// The opaque signed record bytes, re-verified live before the peer is trusted.
    pub record: Vec<u8>,
}

/// Cache bounds.
#[derive(Debug, Clone, Copy)]
pub struct CacheConfig {
    /// Max retained entries; inserts past this evict a decorrelated (RNG-chosen) victim.
    pub max_entries: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { max_entries: 256 }
    }
}

/// An in-memory cache of proven-member dial candidates, serializable with a keyed
/// integrity tag for at-rest persistence.
#[derive(Debug)]
pub struct AddressCache {
    entries: BTreeMap<PeerKey, CachedPeer>,
    config: CacheConfig,
}

impl AddressCache {
    /// An empty cache.
    pub fn new(config: CacheConfig) -> Self {
        Self {
            entries: BTreeMap::new(),
            config,
        }
    }

    /// Number of cached peers.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up a cached peer by its device id.
    pub fn get(&self, peer: &PeerKey) -> Option<&CachedPeer> {
        self.entries.get(peer)
    }

    /// Every cached peer (the discovery layer offers these as `Source::Cache`
    /// candidates; counted toward a trust root only after a live re-proof).
    pub fn candidates(&self) -> Vec<CachedPeer> {
        self.entries.values().cloned().collect()
    }

    /// Keep only the entries `keep` returns `true` for.
    ///
    /// The cache had no removal path at all, and `insert` only ever adds, so a member who left
    /// the group stayed a dial candidate for every future launch: the node that removed them
    /// would re-dial them on startup forever, handing an ex-member a per-launch liveness and IP
    /// oracle and an inbound connection slot. Being cached is not a standing entitlement; the
    /// caller re-checks the roster and prunes on every refresh.
    pub fn retain(&mut self, keep: impl Fn(&CachedPeer) -> bool) {
        self.entries.retain(|_, cp| keep(cp));
    }

    /// Insert/refresh a proven-member entry, keeping the **freshest** by signed `seq`.
    /// When over capacity, evict a decorrelated (RNG-chosen) victim so an attacker
    /// cannot steer which honest entry is dropped. The caller must have verified the
    /// record belongs to a current member before inserting.
    pub fn insert(&mut self, peer: CachedPeer, rng: &mut impl CryptoRngCore) {
        if let Some(existing) = self.entries.get(&peer.peer) {
            if existing.seq >= peer.seq {
                return; // a stale or equal record; keep what we have
            }
        }
        self.entries.insert(peer.peer.clone(), peer);
        while self.entries.len() > self.config.max_entries {
            let n = self.entries.len();
            let idx = (rng.next_u32() as usize) % n;
            let victim = self.entries.keys().nth(idx).cloned();
            match victim {
                Some(v) => {
                    self.entries.remove(&v);
                }
                None => break,
            }
        }
    }

    /// Serialize the cache with a keyed integrity tag appended, for at-rest storage.
    /// `integrity_key` is a per-device key (e.g. an HKDF subkey of the at-rest key); a
    /// load with the same key detects any tampering of the body.
    pub fn to_bytes(&self, integrity_key: &[u8; 32]) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_str(CACHE_DOMAIN).expect("domain fits");
        e.put_u32(self.entries.len() as u32);
        for cp in self.entries.values() {
            e.put_bytes(&cp.peer).expect("peer fits");
            e.put_u64(cp.seq);
            e.put_u32(cp.addresses.len() as u32);
            for a in &cp.addresses {
                e.put_str(a).expect("addr fits");
            }
            e.put_bytes(&cp.record).expect("record fits");
        }
        let mut body = e.finish();
        let tag = blake3::keyed_hash(integrity_key, &body);
        body.extend_from_slice(tag.as_bytes());
        body
    }

    /// Load a cache serialized by [`AddressCache::to_bytes`], verifying the keyed
    /// integrity tag first; a tampered body is rejected wholesale, not partially
    /// trusted. `config` bounds the loaded set.
    pub fn from_bytes(
        bytes: &[u8],
        integrity_key: &[u8; 32],
        config: CacheConfig,
    ) -> Result<Self, CacheError> {
        if bytes.len() < 32 {
            return Err(CacheError::Malformed);
        }
        let (body, tag) = bytes.split_at(bytes.len() - 32);
        let expected = blake3::keyed_hash(integrity_key, body);
        // Constant-time compare: `tag` is exactly 32 bytes, so converting it to a fixed
        // array drives blake3's `PartialEq<[u8; 32]> for Hash` impl, which is
        // unconditionally constant-time (no early-out on the first differing byte, so a
        // colluding at-rest host cannot time the load to forge a valid tag byte by byte).
        let tag: [u8; 32] = tag.try_into().expect("split_at left exactly 32 bytes");
        if expected != tag {
            return Err(CacheError::Tampered);
        }
        let mut d = Decoder::new(body);
        let domain = d.get_str().map_err(|_| CacheError::Malformed)?;
        if domain != CACHE_DOMAIN {
            return Err(CacheError::Malformed);
        }
        let count = d.get_u32().map_err(|_| CacheError::Malformed)? as usize;
        if count > MAX_DECODE_ENTRIES {
            return Err(CacheError::Malformed);
        }
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            let peer = d.get_bytes().map_err(|_| CacheError::Malformed)?.to_vec();
            let seq = d.get_u64().map_err(|_| CacheError::Malformed)?;
            let n_addr = d.get_u32().map_err(|_| CacheError::Malformed)? as usize;
            if n_addr > MAX_DECODE_ADDRESSES {
                return Err(CacheError::Malformed);
            }
            let mut addresses = Vec::with_capacity(n_addr);
            for _ in 0..n_addr {
                addresses.push(d.get_str().map_err(|_| CacheError::Malformed)?.to_string());
            }
            let record = d.get_bytes().map_err(|_| CacheError::Malformed)?.to_vec();
            entries.insert(
                peer.clone(),
                CachedPeer {
                    peer,
                    addresses,
                    seq,
                    record,
                },
            );
        }
        d.finish().map_err(|_| CacheError::Malformed)?;
        // Respect the configured cap: drop the lowest-keyed entries, retaining a
        // deterministic suffix (the highest-keyed proven members). They are equivalently
        // trustworthy, so which subset survives a shrink does not matter.
        while entries.len() > config.max_entries {
            let victim = entries.keys().next().cloned();
            match victim {
                Some(v) => {
                    entries.remove(&v);
                }
                None => break,
            }
        }
        Ok(Self { entries, config })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn rng() -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(0)
    }

    fn cached(p: u8, seq: u64) -> CachedPeer {
        CachedPeer {
            peer: vec![p; 32],
            addresses: vec![format!("/ip4/10.0.0.{p}/tcp/4001")],
            seq,
            record: vec![p; 80],
        }
    }

    #[test]
    fn a_cached_member_survives_a_serialize_load_round_trip() {
        // Session 1 caches a proven member; session 2 loads it and can offer it as a
        // candidate immediately; reaching it past a hostile rendezvous.
        let key = [42u8; 32];
        let mut c1 = AddressCache::new(CacheConfig::default());
        c1.insert(cached(7, 1), &mut rng());
        let bytes = c1.to_bytes(&key);

        let c2 = AddressCache::from_bytes(&bytes, &key, CacheConfig::default()).unwrap();
        assert_eq!(c2.len(), 1);
        assert!(c2.get(&vec![7u8; 32]).is_some());
        assert_eq!(c2.candidates().len(), 1);
    }

    #[test]
    fn a_tampered_row_is_rejected_on_load() {
        let key = [42u8; 32];
        let mut c1 = AddressCache::new(CacheConfig::default());
        c1.insert(cached(7, 1), &mut rng());
        let mut bytes = c1.to_bytes(&key);
        // Flip a byte in the body (not the trailing tag).
        bytes[10] ^= 0x01;
        assert!(matches!(
            AddressCache::from_bytes(&bytes, &key, CacheConfig::default()),
            Err(CacheError::Tampered)
        ));
        // A wrong integrity key (a different device) is likewise rejected.
        let fresh = AddressCache::new(CacheConfig::default()).to_bytes(&key);
        assert!(matches!(
            AddressCache::from_bytes(&fresh, &[7u8; 32], CacheConfig::default()),
            Err(CacheError::Tampered)
        ));
    }

    #[test]
    fn retain_drops_entries_the_caller_no_longer_vouches_for() {
        // A member leaving the group must leave the cache with them, or the node that removed
        // them keeps re-dialling them on every launch.
        let mut c = AddressCache::new(CacheConfig::default());
        let mut r = rng();
        for p in 1..=4u8 {
            c.insert(cached(p, 1), &mut r);
        }
        assert_eq!(c.len(), 4);
        c.retain(|cp| cp.peer[0] % 2 == 0);
        assert_eq!(c.len(), 2);
        assert!(c.get(&vec![2u8; 32]).is_some());
        assert!(c.get(&vec![3u8; 32]).is_none());
        // And it survives the serialization round trip as the pruned set.
        let key = [1u8; 32];
        let back = AddressCache::from_bytes(&c.to_bytes(&key), &key, CacheConfig::default())
            .expect("round trip");
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn insert_keeps_the_freshest_by_seq() {
        let mut c = AddressCache::new(CacheConfig::default());
        c.insert(cached(7, 5), &mut rng());
        c.insert(cached(7, 3), &mut rng()); // stale; ignored
        assert_eq!(c.get(&vec![7u8; 32]).unwrap().seq, 5);
        c.insert(cached(7, 9), &mut rng()); // fresher; replaces
        assert_eq!(c.get(&vec![7u8; 32]).unwrap().seq, 9);
    }

    #[test]
    fn the_cache_is_bounded_with_rng_jittered_eviction() {
        let mut c = AddressCache::new(CacheConfig { max_entries: 4 });
        let mut r = rng();
        for p in 0..20u8 {
            c.insert(cached(p, 1), &mut r);
        }
        assert_eq!(c.len(), 4, "the cache stays bounded under inserts");
    }
}
