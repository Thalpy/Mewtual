//! Native helper-chain custody. This is capacity only: no membership, route, nonce, deadline or
//! read authority is granted. The eventual service must reserve after policy preflight and keep
//! the same owner in every real lower attempt and final response, not just its outer waiter.
use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use catcoms_crypto::DeviceId;
use catcoms_rt::{
    SharedRequestKeepalive, FILE_SERVICE_REQUEST_MAX_BYTES, FILE_SERVICE_RESPONSE_MAX_BYTES,
};
use thiserror::Error;

use super::wire::MAX_SOURCES;

/// Fixed explicit helper capacity, independent of source scratch and driver delivery budgets.
/// This is not a process-wide allocator/OS/socket bound; the active service must pin every
/// retained allocation to this owner and recheck the derivation against its final call paths.
pub const HELPER_CHAIN_BYTES: usize = 1024 * 1024;
/// Four application-serial attempts may all remain alive in the lower driver after upper
/// timeouts. Prepay two request copies each, four raw replies plus eight additional full reply
/// windows for transcript/signing/wrapping/copies, and 64 KiB for fixed bounded native metadata.
/// No full ciphertext chunk or source cache is admitted by this derivation.
const EXPLICIT_BOUND: usize = MAX_SOURCES * 2 * FILE_SERVICE_REQUEST_MAX_BYTES
    + (MAX_SOURCES + 8) * FILE_SERVICE_RESPONSE_MAX_BYTES
    + 64 * 1024;
const _: () = assert!(EXPLICIT_BOUND <= HELPER_CHAIN_BYTES);
const GROUP_CHAINS: usize = 4;
const PROCESS_CHAINS: usize = 8;

type Key = (Box<[u8]>, DeviceId);
#[derive(Default)]
struct Pool(Mutex<BTreeSet<Key>>);
struct Credit {
    pool: Arc<Pool>,
    key: Key,
}
impl fmt::Debug for Credit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HelperChainCredit { .. }")
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.pool
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.key);
    }
}

/// Capacity refusal is retryable only under the caller's original authority, deadline and
/// already durable execution allowance. Neither variant grants a new request or any refund.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HelperCapacityError {
    #[error("file helper group exceeds its bound")]
    Bounds,
    #[error("file helper capacity is in use")]
    Busy,
}

/// One admitted native chain with at most four monotonic lower preparation tokens. There is
/// no Clone or reset/refund API; timeout, source replacement and local refusal cannot renew it.
/// Actual lower/response owners share the underlying credit until its last real owner retires.
pub struct HelperChain {
    credit: Arc<Credit>,
    remaining: u8,
}
impl HelperChain {
    /// Spend one of the chain's four possible lower preparations BEFORE signing/handoff. The
    /// eventual private pending-request constructor must consume this affine token once; sharing
    /// accounting is not authorization to submit the same token/packet repeatedly.
    pub fn take_attempt(&mut self) -> Option<HelperAttempt> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(HelperAttempt(self.credit.clone()))
    }
    /// End preparation and transfer the SAME original accounting into final response custody.
    /// Original byte clones and the driver's lower serialization must retain it independently.
    pub fn into_response_custody(self) -> SharedRequestKeepalive {
        self.credit
    }
}
impl fmt::Debug for HelperChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HelperChain { .. }")
    }
}
/// Non-Clone token for exactly one lower preparation. It grants neither membership nor network
/// permission. Captured effects and the original deadline must still authorize actual admission.
pub struct HelperAttempt(Arc<Credit>);
impl HelperAttempt {
    /// Lend shared accounting to THIS attempt's actual transport owner. The service's affine
    /// pending-request builder must consume HelperAttempt and submit only once; this shared Arc
    /// alone cannot enforce packet/copy counts and is never a retry grant.
    pub fn keepalive(&self) -> SharedRequestKeepalive {
        self.0.clone()
    }
}
impl fmt::Debug for HelperAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HelperAttempt { .. }")
    }
}

/// Process-stable capacity facade. Replacement cannot reset its pool or duplicate a still-live
/// original-device slot. There is no waiting queue and no clock-based refund/eviction.
#[derive(Clone)]
pub struct HelperCapacity {
    pool: Arc<Pool>,
}
impl HelperCapacity {
    #[cfg(test)]
    pub(super) fn testing() -> Self {
        Self {
            pool: Arc::new(Pool::default()),
        }
    }

    /// Sole production constructor. Deterministic tests create isolated private pools directly.
    pub fn process() -> Self {
        static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
        Self {
            pool: POOL.get_or_init(|| Arc::new(Pool::default())).clone(),
        }
    }
    /// Native admission must already have authenticated the full original device/group. This
    /// atomically reserves all capacity or none, before any network handoff or nonce/work commit.
    pub fn reserve(
        &self,
        group: &[u8],
        original: DeviceId,
    ) -> Result<HelperChain, HelperCapacityError> {
        if group.is_empty() || group.len() > 256 {
            return Err(HelperCapacityError::Bounds);
        }
        let key = (Box::<[u8]>::from(group), original);
        let mut held = self.pool.0.lock().unwrap_or_else(|e| e.into_inner());
        if held.len() >= PROCESS_CHAINS
            || held.contains(&key)
            || held.iter().filter(|(g, _)| g.as_ref() == group).count() >= GROUP_CHAINS
        {
            return Err(HelperCapacityError::Busy);
        }
        held.insert(key.clone());
        Ok(HelperChain {
            credit: Arc::new(Credit {
                pool: self.pool.clone(),
                key,
            }),
            remaining: MAX_SOURCES as u8,
        })
    }
}
impl fmt::Debug for HelperCapacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HelperCapacity { .. }")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capacity() -> HelperCapacity {
        HelperCapacity {
            pool: Arc::new(Pool::default()),
        }
    }
    fn device(value: u8) -> DeviceId {
        DeviceId::from_bytes([value; 32])
    }

    #[test]
    fn group_pressure_leaves_other_group_its_share_and_process_remains_bounded() {
        let capacity = capacity();
        let mut held = Vec::new();
        for value in 0..4 {
            held.push(capacity.reserve(b"same-prefix/A", device(value)).unwrap());
        }
        assert!(matches!(
            capacity.reserve(b"same-prefix/A", device(8)),
            Err(HelperCapacityError::Busy)
        ));
        for value in 0..4 {
            held.push(capacity.reserve(b"same-prefix/B", device(value)).unwrap());
        }
        assert!(matches!(
            capacity.reserve(b"third", device(8)),
            Err(HelperCapacityError::Busy)
        ));
        assert_eq!(capacity.pool.0.lock().unwrap().len(), PROCESS_CHAINS);
        drop(held);
        assert!(capacity.pool.0.lock().unwrap().is_empty());
    }

    #[test]
    fn actual_last_lower_and_response_owners_retain_original_slot_after_runtime_replacement() {
        let capacity = capacity();
        let replacement = capacity.clone();
        let mut chain = capacity.reserve(b"g", device(1)).unwrap();
        let lowers: Vec<_> = (0..MAX_SOURCES)
            .map(|_| chain.take_attempt().unwrap().keepalive())
            .collect();
        assert!(
            chain.take_attempt().is_none(),
            "all four lower preparations remain spent even without an outer waiter"
        );
        let response = chain.into_response_custody();
        drop(capacity);
        for lower in lowers {
            assert!(matches!(
                replacement.reserve(b"g", device(1)),
                Err(HelperCapacityError::Busy)
            ));
            drop(lower);
        }
        assert!(matches!(
            replacement.reserve(b"g", device(1)),
            Err(HelperCapacityError::Busy)
        ));
        drop(response);
        let new = replacement.reserve(b"g", device(1)).unwrap();
        drop(new);
        assert!(replacement.pool.0.lock().unwrap().is_empty());
    }

    #[test]
    fn actual_lower_owners_alone_retain_capacity_after_final_response_and_outer_drop() {
        let capacity = capacity();
        let mut chain = capacity.reserve(b"g", device(1)).unwrap();
        let mut lowers: Vec<_> = (0..MAX_SOURCES)
            .map(|_| chain.take_attempt().unwrap().keepalive())
            .collect();
        assert!(chain.take_attempt().is_none());
        let response = chain.into_response_custody();
        drop(response);
        while lowers.len() > 1 {
            assert!(
                matches!(
                    capacity.reserve(b"g", device(1)),
                    Err(HelperCapacityError::Busy)
                ),
                "lower request ownership must not be masked by a retained response"
            );
            drop(lowers.pop());
        }
        assert!(
            matches!(
                capacity.reserve(b"g", device(1)),
                Err(HelperCapacityError::Busy)
            ),
            "last actual lower still owns the original-device slot"
        );
        drop(lowers);
        let fresh = capacity.reserve(b"g", device(1)).unwrap();
        drop(fresh);
        assert!(capacity.pool.0.lock().unwrap().is_empty());
    }

    #[test]
    fn full_group_and_device_keys_release_only_the_exact_owner() {
        let capacity = capacity();
        let a = capacity.reserve(b"same-prefix/A", device(1)).unwrap();
        let b = capacity.reserve(b"same-prefix/B", device(1)).unwrap();
        assert!(matches!(
            capacity.reserve(b"same-prefix/A", device(1)),
            Err(HelperCapacityError::Busy)
        ));
        drop(a);
        let a = capacity.reserve(b"same-prefix/A", device(1)).unwrap();
        assert!(matches!(
            capacity.reserve(b"same-prefix/B", device(1)),
            Err(HelperCapacityError::Busy)
        ));
        drop((a, b));
    }

    #[test]
    fn invalid_groups_refuse_without_capacity_and_memory_derivation_keeps_all_lower_attempts() {
        let capacity = capacity();
        for group in [&[][..], &[1; 257][..]] {
            assert!(matches!(
                capacity.reserve(group, device(1)),
                Err(HelperCapacityError::Bounds)
            ));
        }
        assert!(capacity.pool.0.lock().unwrap().is_empty());
        assert_eq!(EXPLICIT_BOUND, 944 * 1024);
        assert_eq!(HELPER_CHAIN_BYTES, 1024 * 1024);
        assert!(Arc::ptr_eq(
            &HelperCapacity::process().pool,
            &HelperCapacity::process().pool
        ));
    }

    #[test]
    fn concurrent_same_original_admission_is_atomic_and_never_creates_two_owners() {
        let capacity = capacity();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let capacity = capacity.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    capacity.reserve(b"g", device(1))
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(HelperCapacityError::Busy)))
                .count(),
            1
        );
        drop(results);
        assert!(capacity.pool.0.lock().unwrap().is_empty());
    }
}
