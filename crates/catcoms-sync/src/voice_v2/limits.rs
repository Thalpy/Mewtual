//! Actor-owned original-member limits shared by every watcher, endpoint and helper route in one
//! group. Controller replacement and transport/helper churn must not reconstruct this owner.
//! Only an already-authenticated current full device identity may enter this table. Lower ingress
//! also needs its cheap pre-authentication transport budget; this is not a signature DoS defence.

use super::resources::{Credit, ResourceKind, Resources};
use super::{CallLease, Nonce, VoiceError, MAX_REMOTE_DELEGATION_REMAINING_MS};
use catcoms_crypto::DeviceId;
use std::collections::BTreeMap;
use std::sync::Arc;

const RETAIN_MS: u64 = 60_000;
const RECORD_BYTES: usize = 2048;
const MAX_BINDINGS: usize = 4;
const MAX_PINS: usize = 16;
// Readiness prepayment relies on ordinary origin records surviving the whole supported SDP
// window. Raising either deadline requires revisiting this prerequisite, not only byte math.
const _: () = assert!(
    RETAIN_MS
        > super::MAX_DELEGATION_LIFETIME_MS + super::MAX_PACKET_LIFETIME_MS + super::CLOCK_SKEW_MS
);
#[derive(Clone, Copy, Debug)]
pub enum Traffic {
    Room,
    Setup,
}
#[derive(Clone, Copy)]
pub(super) struct Bucket {
    last: u64,
    tokens: u64,
}
impl Bucket {
    pub(super) fn new(now: u64, burst: u64) -> Self {
        Self {
            last: now,
            tokens: burst,
        }
    }
    pub(super) fn spend(&mut self, now: u64, rate: u64, burst: u64) -> bool {
        // Wide intermediate preserves the refill clock on even the largest forward step.
        // Saturating elapsed*rate would repeatedly refill at the same extreme timestamp.
        let added = u128::from(now.saturating_sub(self.last)) * u128::from(rate) / 1000;
        let consumed_ms = (added * 1000 / u128::from(rate)) as u64; // <= original u64 elapsed
        self.last = self.last.saturating_add(consumed_ms);
        self.tokens = self
            .tokens
            .saturating_add(added.min(u128::from(burst)) as u64)
            .min(burst);
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}
#[derive(Clone)]
struct SeenJoin {
    join: Nonce,
    key_ref: [u8; 32],
    created: u64,
    renewal: u64,
}
#[derive(Clone)]
struct Binding {
    join: Nonce,
    key_ref: [u8; 32],
    until: u64,
    pins: Vec<CallLease>,
    renewal: u64,
}
struct Record {
    touched: u64,
    requester: Bucket,
    room: Bucket,
    setup: Bucket,
    joins: Vec<SeenJoin>,
    bindings: Vec<Binding>,
    _credit: Credit,
}
pub struct OriginLimits {
    group: [u8; 32],
    resources: Arc<Resources>,
    records: BTreeMap<DeviceId, Record>,
}
redacted_debug!(OriginLimits);
impl OriginLimits {
    #[cfg(test)]
    pub(super) fn live_pair_pins_for_test(
        &self,
        member: DeviceId,
        join: Nonce,
        key_ref: [u8; 32],
    ) -> usize {
        self.records
            .get(&member)
            .and_then(|record| {
                record
                    .bindings
                    .iter()
                    .find(|binding| binding.join == join && binding.key_ref == key_ref)
            })
            .map_or(0, |binding| {
                binding.pins.iter().filter(|pin| pin.is_live()).count()
            })
    }
    /// Prepay the existing member records before reserving an SDP workflow. This spends no
    /// requester/original allowance and resets no bucket. A fresh row's normal 60s retention
    /// exceeds the maximum 30s idle readiness plus 20s admitted work window.
    pub(super) fn prepare_exchange_members(
        &mut self,
        members: &[DeviceId],
        resources: &Arc<Resources>,
        now: u64,
    ) -> Result<(), VoiceError> {
        if !Arc::ptr_eq(resources, &self.resources) {
            return Err(VoiceError::Scope);
        }
        if members.len() > 4 {
            return Err(VoiceError::Bounds);
        }
        for member in members {
            self.record(*member, now)?;
        }
        Ok(())
    }
    pub fn new(group: [u8; 32], resources: Arc<Resources>) -> Self {
        Self {
            group,
            resources,
            records: BTreeMap::new(),
        }
    }
    pub fn group(&self) -> [u8; 32] {
        self.group
    }
    #[cfg(test)]
    pub(super) fn delegation_deadline(&self, member: DeviceId, join: Nonce) -> Option<u64> {
        self.records
            .get(&member)?
            .bindings
            .iter()
            .find(|binding| binding.join == join)
            .map(|binding| binding.until)
    }
    /// Non-touching expiry collection. Sweeping an idle/locked group must neither refill its
    /// traffic buckets nor retire a key binding still protected by an actual native pair pin.
    pub(super) fn collect_expired(&mut self, now: u64) {
        self.records.retain(|_, record| {
            for binding in &mut record.bindings {
                binding.pins.retain(CallLease::is_live);
            }
            record
                .bindings
                .retain(|binding| binding.until > now || !binding.pins.is_empty());
            now.saturating_sub(record.touched) < RETAIN_MS || !record.bindings.is_empty()
        });
    }
    fn record(&mut self, member: DeviceId, now: u64) -> Result<&mut Record, VoiceError> {
        self.collect_expired(now);
        if !self.records.contains_key(&member) {
            let credit = self
                .resources
                .charge(self.group, RECORD_BYTES, ResourceKind::Origin)?;
            self.records.insert(
                member,
                Record {
                    touched: now,
                    requester: Bucket::new(now, 40),
                    room: Bucket::new(now, 8),
                    setup: Bucket::new(now, 40),
                    joins: Vec::new(),
                    bindings: Vec::new(),
                    _credit: credit,
                },
            );
        }
        let record = self.records.get_mut(&member).expect("inserted origin");
        record.touched = record.touched.max(now);
        Ok(record)
    }
    /// Immediate requester and original issuer have independent buckets even for direct traffic.
    pub fn requester(&mut self, member: DeviceId, now: u64) -> Result<(), VoiceError> {
        if self.record(member, now)?.requester.spend(now, 20, 40) {
            Ok(())
        } else {
            Err(VoiceError::RateLimited)
        }
    }
    pub fn origin(&mut self, member: DeviceId, kind: Traffic, now: u64) -> Result<(), VoiceError> {
        let record = self.record(member, now)?;
        let allowed = match kind {
            Traffic::Room => record.room.spend(now, 2, 8),
            Traffic::Setup => record.setup.spend(now, 20, 40),
        };
        if allowed {
            Ok(())
        } else {
            Err(VoiceError::RateLimited)
        }
    }
    /// Commit original-issuer work only together with a newly retained packet owner. Callers
    /// first classify exact replays and reserve ALL packet/edge/ingress capacity. Reuse the
    /// existing bounded join/renewal validators transactionally: rejection restores logical
    /// origin allowance and protection, while requester debit and actual record custody remain.
    pub(super) fn admit_packet(
        &mut self,
        object: &super::SignedObject,
        now: u64,
        wall: u64,
    ) -> Result<(), VoiceError> {
        let member = object.issuer();
        let record = self.record(member, now)?;
        // Pruning and requester activity happen before this bounded snapshot. Never restore a
        // requester bucket or drop its Credit: failed admissions must not reset throttling.
        let backup = (
            record.room,
            record.setup,
            record.joins.clone(),
            record.bindings.clone(),
        );
        let outcome = (|| {
            let traffic = if matches!(
                object.body(),
                super::Body::Announcement { .. } | super::Body::WatchChallenge { .. }
            ) {
                Traffic::Room
            } else {
                Traffic::Setup
            };
            self.origin(member, traffic, now)?;
            if let super::Body::Announcement { key, renewal, .. } = object.body() {
                let remaining = object
                    .expires()
                    .saturating_add(super::CLOCK_SKEW_MS)
                    .saturating_sub(wall);
                self.join(
                    member,
                    key.join,
                    key.key_ref,
                    now,
                    now.saturating_add(remaining),
                )?;
                self.renewal(member, key.join, key.key_ref, *renewal, now)?;
            }
            Ok(())
        })();
        if outcome.is_err() {
            let record = self
                .records
                .get_mut(&member)
                .expect("current admission record remains touched");
            (record.room, record.setup, record.joins, record.bindings) = backup;
            // Shared CallLease revocation is irreversible even for a cloned/restored pin.
        }
        outcome
    }
    /// Sliding four-distinct-joins/minute, independent of helpers and watcher incarnations.
    /// Exact renewals consume no new join slot; rebinding a still-protected join refuses.
    pub fn join(
        &mut self,
        member: DeviceId,
        join: Nonce,
        key_ref: [u8; 32],
        now: u64,
        delegation_until: u64,
    ) -> Result<(), VoiceError> {
        if delegation_until <= now
            || delegation_until > now.saturating_add(MAX_REMOTE_DELEGATION_REMAINING_MS)
        {
            return Err(VoiceError::Freshness);
        }
        let record = self.record(member, now)?;
        record
            .joins
            .retain(|seen| now.saturating_sub(seen.created) < RETAIN_MS);
        if let Some(binding) = record
            .bindings
            .iter_mut()
            .find(|binding| binding.join == join)
        {
            if binding.key_ref != key_ref {
                return Err(VoiceError::Unauthorized);
            }
            // Renewal extends binding protection, never its original rate-window entry.
            binding.until = binding.until.max(delegation_until);
            return Ok(());
        }
        let seen = record.joins.iter().find(|seen| seen.join == join);
        if seen.is_some_and(|seen| seen.key_ref != key_ref) {
            return Err(VoiceError::Unauthorized);
        }
        if seen.is_none() && record.joins.len() >= 4 {
            return Err(VoiceError::RateLimited);
        }
        if record.bindings.len() >= MAX_BINDINGS {
            return Err(VoiceError::Capacity);
        }
        let renewal = seen.map_or(0, |seen| seen.renewal);
        if seen.is_none() {
            record.joins.push(SeenJoin {
                join,
                key_ref,
                created: now,
                renewal: 0,
            });
        }
        record.bindings.push(Binding {
            join,
            key_ref,
            until: delegation_until,
            pins: Vec::new(),
            renewal,
        });
        Ok(())
    }

    /// An older still-signed delegation cannot create a newer observation. This high-water mark
    /// shares the actor-owned binding/rate history across watcher/controller/helper changes.
    /// An already-issued challenge may still finish against its immutable original delegation.
    pub fn renewal(
        &mut self,
        member: DeviceId,
        join: Nonce,
        key_ref: [u8; 32],
        sequence: u64,
        now: u64,
    ) -> Result<(), VoiceError> {
        let record = self.record(member, now)?;
        let binding = record
            .bindings
            .iter_mut()
            .find(|binding| binding.join == join && binding.key_ref == key_ref)
            .ok_or(VoiceError::Unauthorized)?;
        if sequence == 0 || sequence < binding.renewal {
            return Err(VoiceError::Freshness);
        }
        binding.renewal = sequence;
        if let Some(seen) = record.joins.iter_mut().find(|seen| seen.join == join) {
            seen.renewal = sequence;
        }
        Ok(())
    }

    /// Native Endpoint pins an established/candidate pair's key binding through its actual
    /// retirement, independently of the room heartbeat. No IPC exposes this transition.
    pub fn protect_pair(
        &mut self,
        member: DeviceId,
        join: Nonce,
        key_ref: [u8; 32],
        lease: &CallLease,
        now: u64,
    ) -> Result<(), VoiceError> {
        if !lease.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let binding = self
            .record(member, now)?
            .bindings
            .iter_mut()
            .find(|binding| binding.join == join && binding.key_ref == key_ref)
            .ok_or(VoiceError::Unauthorized)?;
        if binding.pins.iter().any(|pin| pin.is_same(lease)) {
            return Ok(());
        }
        if binding.pins.len() >= MAX_PINS {
            return Err(VoiceError::Capacity);
        }
        binding.pins.push(lease.clone());
        Ok(())
    }
    /// Release only after native retirement of this exact pair/join. An old local call lease
    /// cannot release a replacement call's pin. Still-live delegation/rate protection remains.
    pub fn retire_pair(
        &mut self,
        member: DeviceId,
        join: Nonce,
        key_ref: [u8; 32],
        lease: &CallLease,
    ) {
        if let Some(record) = self.records.get_mut(&member) {
            if let Some(binding) = record
                .bindings
                .iter_mut()
                .find(|binding| binding.join == join && binding.key_ref == key_ref)
            {
                binding.pins.retain(|pin| !pin.is_same(lease));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exchange_member_preparation_prepays_rows_without_spending_or_resetting_allowance() {
        let resources = Arc::new(Resources::default());
        let group = [9; 32];
        let mut limits = OriginLimits::new(group, resources.clone());
        limits
            .prepare_exchange_members(&[id(1), id(2)], &resources, 0)
            .unwrap();
        assert_eq!(resources.counts().0, 2 * RECORD_BYTES);
        for _ in 0..40 {
            limits.origin(id(1), Traffic::Setup, 0).unwrap();
        }
        assert_eq!(
            limits.origin(id(1), Traffic::Setup, 0),
            Err(VoiceError::RateLimited)
        );
        limits
            .prepare_exchange_members(&[id(1), id(2)], &resources, 0)
            .unwrap();
        assert_eq!(
            limits.origin(id(1), Traffic::Setup, 0),
            Err(VoiceError::RateLimited)
        );
        assert_eq!(resources.counts().0, 2 * RECORD_BYTES);
        limits.collect_expired(50_000);
        assert_eq!(
            limits.records.len(),
            2,
            "records outlive maximum readiness plus work"
        );
        let other_pool = Arc::new(Resources::default());
        assert_eq!(
            limits.prepare_exchange_members(&[id(3)], &other_pool, 0),
            Err(VoiceError::Scope)
        );
        assert_eq!(limits.records.len(), 2);
        limits.collect_expired(60_001);
        assert!(limits.records.is_empty());
        assert_eq!(resources.counts().0, 0);
    }
    fn id(seed: u8) -> DeviceId {
        DeviceId::from_bytes([seed; 32])
    }
    #[test]
    fn collection_preserves_origin_window_and_actual_live_pair_pins_without_touching() {
        let pool = Arc::new(Resources::default());
        let mut limits = OriginLimits::new([1; 32], pool.clone());
        for _ in 0..8 {
            limits.origin(id(1), Traffic::Room, 1000).unwrap();
        }
        limits.join(id(1), [2; 32], [3; 32], 1000, 2000).unwrap();
        let pin = CallLease::default();
        limits
            .protect_pair(id(1), [2; 32], [3; 32], &pin, 1000)
            .unwrap();
        let before = pool.counts();
        for _ in 0..3 {
            limits.collect_expired(60_999);
        }
        assert_eq!(pool.counts(), before);
        assert_eq!(limits.records[&id(1)].touched, 1000);
        assert_eq!(limits.records[&id(1)].room.tokens, 0);
        limits.collect_expired(61_000);
        assert_eq!(
            pool.counts(),
            before,
            "actual pair pin keeps the original key binding"
        );
        assert_eq!(
            limits.join(id(1), [2; 32], [4; 32], 61_000, 62_000),
            Err(VoiceError::Unauthorized)
        );
        // The rejected operation touched the record. Only its full later retention window may
        // expire it; a sweep itself must never move that window forward or reset the bucket.
        pin.revoke();
        limits.collect_expired(121_000);
        assert!(limits.records.is_empty());
        assert_eq!(pool.counts(), (0, 0));
    }
    #[test]
    fn fractional_refill_and_backward_time_do_not_starve_or_refund_senders() {
        let mut bucket = Bucket::new(1000, 8);
        for _ in 0..8 {
            assert!(bucket.spend(1000, 2, 8));
        }
        for time in [1100, 1200, 1300, 1400] {
            assert!(!bucket.spend(time, 2, 8));
        }
        assert!(bucket.spend(1500, 2, 8));
        assert!(!bucket.spend(1500, 2, 8));
        assert!(!bucket.spend(0, 2, 8));
        for _ in 0..8 {
            assert!(bucket.spend(u64::MAX, 2, 8));
        }
        assert!(!bucket.spend(u64::MAX, 2, 8));
    }
    #[test]
    fn requester_and_original_issuer_limits_survive_helper_changes() {
        let pool = Arc::new(Resources::default());
        let mut limits = OriginLimits::new([1; 32], pool);
        for _ in 0..8 {
            limits.requester(id(2), 1000).unwrap();
            limits.origin(id(1), Traffic::Room, 1000).unwrap();
        }
        limits.requester(id(3), 1000).unwrap();
        assert_eq!(
            limits.origin(id(1), Traffic::Room, 1000),
            Err(VoiceError::RateLimited)
        );
        limits.origin(id(4), Traffic::Room, 1000).unwrap();
        for _ in 0..40 {
            limits.origin(id(1), Traffic::Setup, 1000).unwrap();
        }
        assert_eq!(
            limits.origin(id(1), Traffic::Setup, 1000),
            Err(VoiceError::RateLimited)
        );
    }
    #[test]
    fn join_limit_is_sliding_and_renewals_cannot_reset_it_or_rebind_keys() {
        let pool = Arc::new(Resources::default());
        let mut limits = OriginLimits::new([1; 32], pool);
        for (index, time) in [(1, 1000), (2, 2000), (3, 3000), (4, 4000)] {
            limits
                .join(id(1), [index; 32], [9; 32], time, time + 35_000)
                .unwrap();
        }
        for time in [5000, 60_999] {
            limits
                .join(id(1), [1; 32], [9; 32], time, time + 35_000)
                .unwrap();
            assert_eq!(
                limits.join(id(1), [5; 32], [9; 32], time, time + 35_000),
                Err(VoiceError::RateLimited)
            );
        }
        assert_eq!(
            limits.join(id(1), [1; 32], [8; 32], 60_999, 95_999),
            Err(VoiceError::Unauthorized)
        );
        limits
            .join(id(1), [5; 32], [9; 32], 61_000, 96_000)
            .unwrap();
        assert_eq!(
            limits.join(id(1), [6; 32], [9; 32], 61_000, 96_000),
            Err(VoiceError::RateLimited)
        );
    }
    #[test]
    fn ongoing_renewals_protect_key_binding_beyond_the_original_rate_window() {
        let mut limits = OriginLimits::new([1; 32], Arc::new(Resources::default()));
        for now in [1000, 30_000, 60_000, 90_000] {
            limits
                .join(id(1), [1; 32], [9; 32], now, now + 35_000)
                .unwrap();
        }
        assert_eq!(
            limits.join(id(1), [1; 32], [8; 32], 91_000, 126_000),
            Err(VoiceError::Unauthorized)
        );
        // The original minute's rate entry may expire; this independent binding remains.
        assert_eq!(limits.records[&id(1)].joins.len(), 0);
        assert_eq!(limits.records[&id(1)].bindings.len(), 1);
    }
    #[test]
    fn pair_pins_outlive_delegation_and_old_calls_cannot_release_replacement_pins() {
        let mut limits = OriginLimits::new([1; 32], Arc::new(Resources::default()));
        let old = CallLease::default();
        let replacement = CallLease::default();
        limits.join(id(1), [1; 32], [9; 32], 1000, 36_000).unwrap();
        limits
            .protect_pair(id(1), [1; 32], [9; 32], &old, 1000)
            .unwrap();
        limits
            .protect_pair(id(1), [1; 32], [9; 32], &replacement, 1000)
            .unwrap();
        limits.retire_pair(id(1), [1; 32], [9; 32], &old);
        assert_eq!(
            limits.join(id(1), [1; 32], [8; 32], 100_000, 135_000),
            Err(VoiceError::Unauthorized)
        );
        replacement.revoke();
        limits
            .join(id(1), [1; 32], [8; 32], 100_000, 135_000)
            .unwrap();
    }
    #[test]
    fn live_pins_and_invalid_deadlines_cannot_grow_the_binding_table() {
        let mut limits = OriginLimits::new([1; 32], Arc::new(Resources::default()));
        let lease = CallLease::default();
        assert_eq!(
            limits.join(id(1), [1; 32], [9; 32], 1000, 1000),
            Err(VoiceError::Freshness)
        );
        assert_eq!(
            limits.join(id(1), [1; 32], [9; 32], 1000, 41_001),
            Err(VoiceError::Freshness)
        );
        assert!(limits.records.is_empty());
        for index in 1..=4 {
            limits
                .join(id(1), [index; 32], [9; 32], 1000, 36_000)
                .unwrap();
            limits
                .protect_pair(id(1), [index; 32], [9; 32], &lease, 1000)
                .unwrap();
        }
        assert_eq!(
            limits.join(id(1), [5; 32], [9; 32], 100_000, 135_000),
            Err(VoiceError::Capacity)
        );
        assert_eq!(limits.records[&id(1)].bindings.len(), 4);
        lease.revoke();
        limits
            .join(id(1), [5; 32], [9; 32], 100_000, 135_000)
            .unwrap();
        assert_eq!(limits.records[&id(1)].bindings.len(), 1);
    }
    #[test]
    fn live_origin_rows_are_not_evicted_to_admit_more_identities() {
        let pool = Arc::new(Resources::default());
        let mut limits = OriginLimits::new([1; 32], pool.clone());
        for member in 0..64 {
            limits.requester(id(member), 1000).unwrap();
        }
        assert_eq!(limits.requester(id(99), 1000), Err(VoiceError::Capacity));
        assert!(limits.requester(id(99), 61_000).is_ok());
        assert_eq!(limits.records.len(), 1);
        drop(limits);
        assert_eq!(pool.counts(), (0, 0));
    }
}
