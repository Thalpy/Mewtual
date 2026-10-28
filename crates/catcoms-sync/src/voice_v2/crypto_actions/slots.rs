//! Bounded native table slots, including prepaid action and packet continuations. Each table
//! has its own immutable limit and pool identity. Reservations protect future records; actual
//! rows keep their slots through retirement, even after their reservation owner closes.
use super::super::VoiceError;
use std::sync::{Arc, Mutex};

pub(crate) struct Slots {
    used: Mutex<usize>,
    limit: usize,
}
impl Default for Slots {
    fn default() -> Self {
        Self::with_limit(super::ACTIONS)
    }
}
pub(crate) struct Slot {
    pool: Arc<Slots>,
    count: usize,
    reserve: Option<Arc<ReservedSlots>>,
}
pub(crate) struct ReservedSlots {
    pool: Arc<Slots>,
    state: Mutex<State>,
}
struct State {
    credit: Slot,
    available: usize,
    closed: bool,
}
redacted_debug!(Slots, Slot, ReservedSlots);
impl Slots {
    /// The native table selects this limit. An untrusted descriptor cannot enlarge its pool,
    /// and a reserved slot from a different table cannot be loaned into this one.
    pub(crate) fn with_limit(limit: usize) -> Self {
        Self {
            used: Mutex::new(0),
            limit,
        }
    }
    pub(crate) fn charge(self: &Arc<Self>, count: usize) -> Result<Slot, VoiceError> {
        let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
        if count == 0 || count > self.limit || *used > self.limit - count {
            return Err(VoiceError::Capacity);
        }
        *used += count;
        Ok(Slot {
            pool: self.clone(),
            count,
            reserve: None,
        })
    }
    pub(crate) fn reserve(
        self: &Arc<Self>,
        count: usize,
    ) -> Result<Arc<ReservedSlots>, VoiceError> {
        let credit = self.charge(count)?;
        Ok(Arc::new(ReservedSlots {
            pool: self.clone(),
            state: Mutex::new(State {
                credit,
                available: count,
                closed: false,
            }),
        }))
    }
}
impl ReservedSlots {
    /// Pure original-pool identity, including while closed or occupied. Checking an inert
    /// preclaim must never borrow/refund positions as an authorization side effect.
    pub(crate) fn uses_pool(&self, expected: &Arc<Slots>) -> bool {
        Arc::ptr_eq(&self.pool, expected)
    }

    /// Move already-prepaid positions into a child reservation without freeing/recharging
    /// the shared table. The caller must fund the child wrapper before calling this.
    /// Unpublished abandonment returns to this exact parent; parent closure releases only
    /// positions nobody still owns. There is no parent -> child reference or lock edge.
    pub(crate) fn subreserve(self: &Arc<Self>, count: usize) -> Result<Arc<Self>, VoiceError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if count == 0 || state.closed || state.available < count {
            return Err(VoiceError::Capacity);
        }
        state.available -= count;
        drop(state);
        Ok(Arc::new(Self {
            pool: self.pool.clone(),
            state: Mutex::new(State {
                credit: Slot {
                    pool: self.pool.clone(),
                    count,
                    reserve: Some(self.clone()),
                },
                available: count,
                closed: false,
            }),
        }))
    }
    pub(crate) fn loan(self: &Arc<Self>, expected: &Arc<Slots>) -> Result<Slot, VoiceError> {
        if !Arc::ptr_eq(expected, &self.pool) {
            return Err(VoiceError::Scope);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.available == 0 {
            return Err(VoiceError::Capacity);
        }
        state.available -= 1;
        Ok(Slot {
            pool: self.pool.clone(),
            count: 1,
            reserve: Some(self.clone()),
        })
    }
    pub(crate) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return;
        }
        state.closed = true;
        let unused = std::mem::take(&mut state.available);
        state.credit.release(unused);
    }
    fn returned(&self, count: usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            state.credit.release(count);
        } else {
            state.available += count;
        }
    }
}
impl Slot {
    pub(crate) fn uses_pool(&self, expected: &Arc<Slots>) -> bool {
        Arc::ptr_eq(&self.pool, expected)
    }
    fn release(&mut self, count: usize) {
        assert!(count <= self.count, "native table backing slot");
        self.count -= count;
        if let Some(parent) = &self.reserve {
            // Child -> parent only. Parent never enters a child's mutex, so closure and
            // last actual loan return preserve the existing reservation -> pool order.
            parent.returned(count);
        } else {
            *self.pool.used.lock().unwrap_or_else(|e| e.into_inner()) -= count;
        }
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        // Never hold the shared pool lock while entering a reservation. A reservation's
        // backing slot releases only its remainder when the last actual loan disappears.
        if let Some(reserve) = self.reserve.take() {
            reserve.returned(self.count);
        } else {
            self.release(self.count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_child_first_returns_only_unused_positions_to_still_live_parent() {
        let pool = Arc::new(Slots::with_limit(2));
        let original = pool.reserve(2).unwrap();
        let child = original.subreserve(2).unwrap();
        let actual = child.loan(&pool).unwrap();
        child.close();
        assert!(pool.charge(1).is_err());
        let returned = original.loan(&pool).unwrap();
        assert!(original.loan(&pool).is_err());
        original.close();
        assert!(pool.charge(1).is_err());
        drop(actual);
        let ordinary = pool.charge(1).unwrap();
        assert!(pool.charge(1).is_err());
        drop((original, child, returned, ordinary));
        assert_eq!(*pool.used.lock().unwrap(), 0);
    }
    #[test]
    fn nested_subreservations_never_free_protected_positions_before_last_actual_loan() {
        let pool = Arc::new(Slots::with_limit(4));
        let original = pool.reserve(4).unwrap();
        let child = original.subreserve(3).unwrap();
        let grandchild = child.subreserve(2).unwrap();
        assert!(pool.charge(1).is_err());
        let first = grandchild.loan(&pool).unwrap();
        let second = grandchild.loan(&pool).unwrap();
        original.close(); // one unused original position becomes ordinary
        let ordinary = pool.charge(1).unwrap();
        child.close(); // its one unused position returns through the CLOSED original
        let next = pool.charge(1).unwrap();
        grandchild.close();
        assert!(pool.charge(1).is_err());
        drop((original, child, grandchild));
        assert!(pool.charge(1).is_err());
        drop(first);
        let reclaimed = pool.charge(1).unwrap();
        assert!(pool.charge(1).is_err());
        drop((second, ordinary, next, reclaimed));
        assert_eq!(*pool.used.lock().unwrap(), 0);
    }
    #[test]
    fn abandoned_child_returns_to_exact_parent_without_reset_or_foreign_pool_adoption() {
        let pool = Arc::new(Slots::with_limit(2));
        let original = pool.reserve(2).unwrap();
        assert_eq!(original.subreserve(0).err(), Some(VoiceError::Capacity));
        let child = original.subreserve(2).unwrap();
        assert_eq!(original.subreserve(1).err(), Some(VoiceError::Capacity));
        assert_eq!(
            child.loan(&Arc::new(Slots::with_limit(2))).err(),
            Some(VoiceError::Scope)
        );
        drop(child);
        assert!(pool.charge(1).is_err());
        let first = original.loan(&pool).unwrap();
        let second = original.loan(&pool).unwrap();
        assert!(original.loan(&pool).is_err());
        original.close();
        assert_eq!(original.subreserve(1).err(), Some(VoiceError::Capacity));
        drop((first, second, original));
        assert_eq!(*pool.used.lock().unwrap(), 0);
    }
    #[test]
    fn prepaid_continuations_survive_ordinary_pressure_and_actual_terminal_record_retirement() {
        let pool = Arc::new(Slots::default());
        let reserve = pool.reserve(2).unwrap();
        let ordinary: Vec<_> = (0..30).map(|_| pool.charge(1).unwrap()).collect();
        assert!(pool.charge(1).is_err());
        let first = reserve.loan(&pool).unwrap();
        let second = reserve.loan(&pool).unwrap();
        assert!(reserve.loan(&pool).is_err());
        reserve.close();
        assert!(pool.charge(1).is_err());
        drop(first);
        let replacement = pool.charge(1).unwrap();
        assert!(reserve.loan(&pool).is_err());
        drop(reserve);
        assert_eq!(*pool.used.lock().unwrap(), 32);
        drop((ordinary, replacement, second));
        assert_eq!(*pool.used.lock().unwrap(), 0);
    }
    #[test]
    fn abandoned_preparation_returns_to_exact_reserve_and_another_join_cannot_borrow() {
        let pool = Arc::new(Slots::default());
        let reserve = pool.reserve(2).unwrap();
        let other = Arc::new(Slots::default());
        assert_eq!(reserve.loan(&other).err(), Some(VoiceError::Scope));
        drop(reserve.loan(&pool).unwrap());
        drop(reserve.loan(&pool).unwrap());
        assert_eq!(*pool.used.lock().unwrap(), 2);
        reserve.close();
        assert_eq!(*pool.used.lock().unwrap(), 0);
    }

    #[test]
    fn smaller_packet_tables_keep_reserved_and_actual_rows_in_the_same_limit() {
        let pool = Arc::new(Slots::with_limit(3));
        let reserved = pool.reserve(2).unwrap();
        let ordinary = pool.charge(1).unwrap();
        assert!(pool.charge(1).is_err());
        let actual = reserved.loan(&pool).unwrap();
        reserved.close(); // only the other, unused reserved position is free
        let replacement = pool.charge(1).unwrap();
        assert!(pool.charge(1).is_err());
        drop(reserved);
        assert!(
            pool.charge(1).is_err(),
            "the actual row still owns its position"
        );
        drop(actual);
        let last = pool.charge(1).unwrap();
        assert!(pool.charge(1).is_err());
        drop((ordinary, replacement, last));
        assert_eq!(*pool.used.lock().unwrap(), 0);
    }
}
