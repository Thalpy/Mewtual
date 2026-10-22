//! Reserve the complete route-record plan before admitting helper work. A fixed registry cap
//! alone could strand a fourth attempt after earlier timed-out drivers retained their scopes.
use std::fmt;
use std::sync::{Arc, Mutex};

use catcoms_rt::SharedRequestKeepalive;

const ROOT_ROUTE_SLOTS: usize = 128;
const HELPER_ROUTE_SLOTS: usize = 5; // one parent plus all four possible lower attempts

#[derive(Default)]
pub(super) struct RoutePool(Arc<Mutex<usize>>);
impl RoutePool {
    pub(super) fn source(&self) -> Option<RoutePlan> {
        self.reserve(1)
    }
    pub(super) fn helper(&self) -> Option<RoutePlan> {
        self.reserve(HELPER_ROUTE_SLOTS)
    }
    fn reserve(&self, count: usize) -> Option<RoutePlan> {
        let mut held = self.0.lock().ok()?;
        if held.checked_add(count)? > ROOT_ROUTE_SLOTS {
            return None;
        }
        *held += count;
        Some(RoutePlan {
            credit: Arc::new(Credit {
                pool: self.0.clone(),
                count,
            }),
            remaining: count,
        })
    }
}
struct Credit {
    pool: Arc<Mutex<usize>>,
    count: usize,
}
impl fmt::Debug for Credit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileRouteSlots { .. }")
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        // Poison is protective at admission; Drop still releases only this known reservation.
        let mut held = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        *held -= self.count;
    }
}
/// Non-Clone finite builder. Scope construction consumes one slot exactly once. Every minted
/// scope and this plan retain the SAME full reservation until the last actual owner disappears.
/// This grants no protocol authority or byte allowance; source/helper memory owns those too.
pub(super) struct RoutePlan {
    credit: Arc<Credit>,
    remaining: usize,
}
impl RoutePlan {
    pub(super) fn take_scope(&mut self) -> Option<SharedRequestKeepalive> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(self.credit.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admitted_four_chains_keep_all_four_child_slots_under_full_other_scope_pressure() {
        let pool = RoutePool::default();
        let mut chains: Vec<_> = (0..4).map(|_| pool.helper().unwrap()).collect();
        let parents: Vec<_> = chains.iter_mut().map(|c| c.take_scope().unwrap()).collect();
        let pressure: Vec<_> = (0..ROOT_ROUTE_SLOTS - 4 * HELPER_ROUTE_SLOTS)
            .map(|_| pool.source().unwrap())
            .collect();
        assert!(pool.source().is_none());
        let children: Vec<_> = chains
            .iter_mut()
            .flat_map(|chain| {
                (0..4)
                    .map(|_| chain.take_scope().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(chains.iter_mut().all(|chain| chain.take_scope().is_none()));
        drop((parents, chains));
        assert!(
            pool.source().is_none(),
            "held lower scopes cannot refund route records"
        );
        drop(children);
        assert!(pool.helper().is_some());
        drop(pressure);
        assert_eq!(*pool.0.lock().unwrap(), 0);
    }

    #[test]
    fn full_plan_refusal_is_atomic_and_last_scope_owns_the_entire_original_plan() {
        let pool = RoutePool::default();
        let pressure: Vec<_> = (0..ROOT_ROUTE_SLOTS - 4)
            .map(|_| pool.source().unwrap())
            .collect();
        assert!(pool.helper().is_none());
        assert_eq!(*pool.0.lock().unwrap(), ROOT_ROUTE_SLOTS - 4);
        drop(pressure);
        let mut plan = pool.helper().unwrap();
        let mut scopes: Vec<_> = (0..HELPER_ROUTE_SLOTS)
            .map(|_| plan.take_scope().unwrap())
            .collect();
        drop(plan);
        let last = scopes.pop().unwrap();
        drop(scopes);
        assert_eq!(*pool.0.lock().unwrap(), HELPER_ROUTE_SLOTS);
        drop(last);
        assert_eq!(*pool.0.lock().unwrap(), 0);
    }
}
