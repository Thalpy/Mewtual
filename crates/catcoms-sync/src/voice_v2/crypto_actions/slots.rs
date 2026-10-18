//! The per-join combined pending/terminal action limit, including prepaid SDP continuations.
//! Reservations protect future records; actual actions keep their slots through retirement.
use super::super::VoiceError;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct Slots(Mutex<usize>);
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
    pub(crate) fn charge(self: &Arc<Self>, count: usize) -> Result<Slot, VoiceError> {
        let mut used = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if count == 0 || count > super::ACTIONS || *used > super::ACTIONS - count {
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
    fn release(&mut self, count: usize) {
        assert!(
            self.reserve.is_none() && count <= self.count,
            "native action backing slot"
        );
        self.count -= count;
        *self.pool.0.lock().unwrap_or_else(|e| e.into_inner()) -= count;
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
        assert_eq!(*pool.0.lock().unwrap(), 32);
        drop((ordinary, replacement, second));
        assert_eq!(*pool.0.lock().unwrap(), 0);
    }
    #[test]
    fn abandoned_preparation_returns_to_exact_reserve_and_another_join_cannot_borrow() {
        let pool = Arc::new(Slots::default());
        let reserve = pool.reserve(2).unwrap();
        let other = Arc::new(Slots::default());
        assert_eq!(reserve.loan(&other).err(), Some(VoiceError::Scope));
        drop(reserve.loan(&pool).unwrap());
        drop(reserve.loan(&pool).unwrap());
        assert_eq!(*pool.0.lock().unwrap(), 2);
        reserve.close();
        assert_eq!(*pool.0.lock().unwrap(), 0);
    }
}
