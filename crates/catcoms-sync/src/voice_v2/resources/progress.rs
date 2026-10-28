//! Exclusive group progression capacity, independent of weak lookup indices.
//!
//! A closed Workflow can still own real work, and a prepared successor can outlive that
//! Workflow. All those owners share this SAME claim. Neither observing strong-zero nor
//! removing a lookup entry releases it. This is capacity only, with no clock or effect API.
use super::{Credit, Resources};
use crate::voice_v2::VoiceError;
use std::sync::Arc;

/// Plain metadata payment. It must never own a Workflow, Exchange, Ready producer or claim:
/// those allocations retain this payment in one direction, without a custody cycle.
pub(in crate::voice_v2) struct ProgressFunding(Credit);
impl ProgressFunding {
    pub(in crate::voice_v2) fn new(credit: Credit) -> Arc<Self> {
        Arc::new(Self(credit))
    }
}

/// Private shared capacity custody. Every share carries an OUTER payment anchor through
/// actual cell destruction and Arc deallocation. Do not expose its inner Arc or create a
/// Weak: `Arc::into_inner` below depends on every owner following this exact drop path.
pub(in crate::voice_v2) struct GroupProgressClaim {
    inner: Option<Arc<ProgressCell>>,
    funding: Arc<ProgressFunding>,
}
struct ProgressCell {
    release: ProgressRelease,
}
struct ProgressRelease {
    pool: Arc<Resources>,
    group: [u8; 32],
    // Zero is an UNREGISTERED candidate. An address may be reused while the preceding
    // cell's final publisher is pending; a refused candidate must never clear that slot.
    key: usize,
    #[cfg(test)]
    park: Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>,
}
impl GroupProgressClaim {
    /// Acquire once, before publishing any progress owner. A failed candidate remains
    /// unarmed and drops outside Pool. Equal addresses never make acquisition idempotent.
    pub(in crate::voice_v2) fn reserve(
        pool: &Arc<Resources>,
        group: [u8; 32],
        funding: Arc<ProgressFunding>,
    ) -> Result<Self, VoiceError> {
        if !Arc::ptr_eq(pool, &funding.0.pool) || group != funding.0.group {
            return Err(VoiceError::Scope);
        }
        let mut claim = Self::candidate(pool, group, funding);
        let key = Arc::as_ptr(claim.inner.as_ref().expect("new original claim")) as usize;
        claim.register(key)?;
        Ok(claim)
    }

    fn candidate(pool: &Arc<Resources>, group: [u8; 32], funding: Arc<ProgressFunding>) -> Self {
        Self {
            inner: Some(Arc::new(ProgressCell {
                release: ProgressRelease {
                    pool: pool.clone(),
                    group,
                    key: 0,
                    #[cfg(test)]
                    park: None,
                },
            })),
            funding,
        }
    }

    // Private insertion seam also pins allocator address reuse deterministically: a busy
    // candidate carrying the occupied stamp must stay unarmed, without freeing the old row.
    fn register(&mut self, key: usize) -> Result<(), VoiceError> {
        let inner = self.inner.as_mut().expect("new original claim");
        let release = &mut Arc::get_mut(inner)
            .expect("unpublished claim has no independent handles")
            .release;
        {
            let mut state = release
                .pool
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let counts = state
                .groups
                .get_mut(&release.group)
                .expect("prepaid metadata group");
            if counts.progress != 0 {
                return Err(VoiceError::Capacity);
            }
            // Arm only on successful insertion, while holding the SAME registry fence.
            counts.progress = key;
            release.key = key;
        }
        Ok(())
    }

    /// Only genuine successor/preparation custody can share an acquired claim. There is
    /// deliberately no lookup/adopt-current-by-group operation and no public Clone trait.
    pub(in crate::voice_v2) fn share(&self) -> Self {
        Self {
            inner: Some(self.inner.as_ref().expect("live claim").clone()),
            funding: self.funding.clone(),
        }
    }

    /// Pure under Pool: validate the exact registered original capacity, not a weak count.
    pub(in crate::voice_v2) fn matches(
        &self,
        pool: &Resources,
        group: [u8; 32],
        key: usize,
    ) -> bool {
        let release = &self.inner.as_ref().expect("live claim").release;
        std::ptr::eq(pool, release.pool.as_ref())
            && group == release.group
            && key != 0
            && key == release.key
    }

    fn take_cell(&mut self) -> Option<ProgressCell> {
        self.inner.take().and_then(Arc::into_inner)
    }

    /// Park the genuine final publisher AFTER cell deallocation and BEFORE registry
    /// release. Only deterministic tests can install this hook on a sole original claim.
    #[cfg(test)]
    pub(in crate::voice_v2) fn park_release_for_test(
        &mut self,
        entered: Arc<std::sync::Barrier>,
        resume: Arc<std::sync::Barrier>,
    ) {
        Arc::get_mut(self.inner.as_mut().expect("live original claim"))
            .expect("only original final publisher remains")
            .release
            .park = Some((entered, resume));
    }

    #[cfg(test)]
    pub(in crate::voice_v2) fn metadata_bytes() -> usize {
        use std::mem::size_of;
        size_of::<ProgressFunding>()
            + size_of::<ProgressCell>()
            + 4 * size_of::<usize>() // both Arc headers
            + size_of::<usize>() // Counts' occupied registry stamp
    }

    #[cfg(test)]
    pub(in crate::voice_v2) fn same_claim(&self, other: &Self) -> bool {
        Arc::ptr_eq(
            self.inner.as_ref().expect("live original"),
            other.inner.as_ref().expect("live successor"),
        )
    }
}
impl Drop for GroupProgressClaim {
    fn drop(&mut self) {
        // Exactly one concurrent owner obtains the cell. With NO Weak owners, its Arc
        // allocation is already deallocated when into_inner returns. Destroy the release
        // publisher now, while this outer wrapper's funding is STILL retained. A plain
        // Arc<ProgressCell> destructor would publish before deallocation/payment returned.
        if let Some(cell) = self.take_cell() {
            drop(cell);
        }
    }
}

impl Drop for ProgressRelease {
    fn drop(&mut self) {
        if self.key == 0 {
            return;
        }
        #[cfg(test)]
        if let Some((entered, resume)) = self.park.take() {
            entered.wait();
            resume.wait();
        }
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(counts) = state.groups.get_mut(&self.group) {
            if counts.progress == self.key {
                counts.progress = 0;
                if counts.bytes == 0 && counts.slots.iter().all(|count| *count == 0) {
                    state.groups.remove(&self.group);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_v2::resources::ResourceKind;
    use std::sync::Barrier;

    fn funding(pool: &Arc<Resources>, group: [u8; 32]) -> Arc<ProgressFunding> {
        ProgressFunding::new(pool.charge(group, 4096, ResourceKind::Retained).unwrap())
    }

    fn stamp(pool: &Resources, group: [u8; 32]) -> usize {
        pool.state
            .lock()
            .unwrap()
            .groups
            .get(&group)
            .map_or(0, |counts| counts.progress)
    }

    #[test]
    fn simultaneous_claims_have_one_winner_and_other_groups_still_progress() {
        let pool = Arc::new(Resources::default());
        let payment = funding(&pool, [1; 32]);
        let start = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (pool, payment, start) = (pool.clone(), payment.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    GroupProgressClaim::reserve(&pool, [1; 32], payment)
                })
            })
            .collect();
        start.wait();
        let claims: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(claims.iter().filter(|claim| claim.is_ok()).count(), 1);
        assert_eq!(
            claims
                .iter()
                .filter(|claim| matches!(claim, Err(VoiceError::Capacity)))
                .count(),
            1
        );
        let other = GroupProgressClaim::reserve(&pool, [2; 32], funding(&pool, [2; 32])).unwrap();
        assert_ne!(stamp(&pool, [1; 32]), 0);
        assert_ne!(stamp(&pool, [2; 32]), 0);
        drop((other, claims, payment));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn shared_claim_concurrent_last_drops_release_exactly_once() {
        let pool = Arc::new(Resources::default());
        let claim = GroupProgressClaim::reserve(&pool, [1; 32], funding(&pool, [1; 32])).unwrap();
        let start = Arc::new(Barrier::new(5));
        let shares: Vec<_> = (0..4)
            .map(|_| {
                let share = claim.share();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    drop(share);
                })
            })
            .collect();
        drop(claim);
        assert_eq!(pool.counts(), (4096, 0));
        start.wait();
        for handle in shares {
            handle.join().unwrap();
        }
        assert_eq!(pool.group_counts([1; 32]), None);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn deallocated_claim_keeps_payment_and_slot_until_original_release_publishes() {
        let pool = Arc::new(Resources::default());
        let payment = funding(&pool, [1; 32]);
        let mut claim = GroupProgressClaim::reserve(&pool, [1; 32], payment.clone()).unwrap();
        let occupied = stamp(&pool, [1; 32]);
        let cell = claim.take_cell().expect("sole Arc actually deallocated");
        assert_eq!(pool.counts(), (4096, 0));
        assert_eq!(stamp(&pool, [1; 32]), occupied);

        // Model exact address reuse after into_inner deallocation, before old release.
        // Busy insertion must refuse regardless of equality and leave the loser unarmed.
        let mut refused = GroupProgressClaim::candidate(&pool, [1; 32], payment.clone());
        assert_eq!(refused.register(occupied), Err(VoiceError::Capacity));
        assert_eq!(refused.inner.as_ref().unwrap().release.key, 0);
        drop(refused);
        assert_eq!(stamp(&pool, [1; 32]), occupied);
        drop(cell);
        assert_eq!(stamp(&pool, [1; 32]), 0);
        assert_eq!(pool.counts(), (4096, 0), "outer metadata still paid");
        let successor = GroupProgressClaim::reserve(&pool, [1; 32], payment.clone()).unwrap();
        drop(claim); // old outer wrapper has no release right left
        assert_ne!(stamp(&pool, [1; 32]), 0);
        drop((successor, payment));
        assert_eq!(pool.counts(), (0, 0));
    }

    #[test]
    fn capacity_requires_exact_process_and_group_payment() {
        let pool = Arc::new(Resources::default());
        let wrong_pool = Arc::new(Resources::default());
        let payment = funding(&pool, [1; 32]);
        assert_eq!(
            GroupProgressClaim::reserve(&wrong_pool, [1; 32], payment.clone()).err(),
            Some(VoiceError::Scope)
        );
        assert_eq!(
            GroupProgressClaim::reserve(&pool, [2; 32], payment.clone()).err(),
            Some(VoiceError::Scope)
        );
        assert_eq!(stamp(&pool, [1; 32]), 0);
        drop(payment);
        assert_eq!(pool.counts(), (0, 0));
    }
}
