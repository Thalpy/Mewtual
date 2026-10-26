//! Native-only bounded work production. The Host rotates existing canonical channel slots;
//! it never receives a peer/action constructor from the renderer. Actor waits belong to one
//! separately paid native-position worker per channel, so a parked old Actor cannot stop
//! service IO, another group, or a replacement join's independent channel.
use super::*;
use crate::voice_handoff::factory::native;

const CADENCE_MS: u64 = 250;
struct Cursor {
    channel: u8,
    ready_channel: u8,
    ready_rows: [u8; 64],
    ready_first: [bool; 64],
    pending: [u8; 64],
    peers: [u8; 64],
    stage: [u8; 64],
    continuations: [u8; 64],
}
impl Cursor {
    fn new() -> Self {
        Self {
            channel: 0,
            ready_channel: 0,
            ready_rows: [0; 64],
            ready_first: [true; 64],
            pending: [0; 64],
            peers: [0; 64],
            stage: [0; 64],
            continuations: [0; 64],
        }
    }
    /// Newly adopted key claims bypass the slow four-stage discovery rotation. Scan at
    /// most64 canonical routes and spawn at most4 paid attempts per cadence; every refusal
    /// advances the channel cursor too. Ordinary confirmation still receives its own turn.
    /// No Actor wait, Clock callback or Row owner crosses this synchronous scheduler scan.
    fn urgent_ready(&mut self, context: &Context, registry: &Registry) {
        let mut spawned = 0;
        for _ in 0..64 {
            let slot = self.ready_channel;
            self.ready_channel = (slot + 1) % 64;
            let Ok(route) = registry.route(slot + 1) else {
                continue;
            };
            if !route.native_ready() {
                continue;
            }
            let Some(live) = context.upgrade() else {
                return;
            };
            let pending = live
                .ledger
                .has_ready_work(route.selected(), route.session().identity());
            drop(live);
            if !pending {
                continue;
            }
            let index = usize::from(slot);
            // A failed ordinary visit still owes an actual opportunity. Retry ordinary
            // first even in this fast scan, rather than letting Ready reacquire the slot
            // immediately after its previous worker finally drops. If ordinary cannot
            // prepay (for example a snapshot), Ready may still free progress capacity.
            let admitted = if !self.ready_first[index] {
                self.ordinary_turn(context, &route, index).0
                    || self.ready_turn(context, &route, index)
            } else {
                self.ready_turn(context, &route, index)
            };
            if admitted {
                spawned += 1;
                if spawned == 4 {
                    break;
                }
            }
        }
    }
    /// Returns only a successfully spawned exact-Pending selection, for native scheduling
    /// observation/tests. This is not Root publication or network delivery evidence.
    fn turn(
        &mut self,
        context: &Context,
        registry: &Registry,
    ) -> Option<catcoms_sync::voice_v2::presence::PairSelector> {
        self.turn_observing_ready(context, registry, |actual| actual)
    }
    // Production keeps the exact native hint. Tests model a stale observation (for
    // example KeyRelease between scan and execution); the real worker still validates
    // the actual KeyClaim and cannot publish Ready from this scheduling hint alone.
    fn turn_observing_ready(
        &mut self,
        context: &Context,
        registry: &Registry,
        observe: impl Fn(bool) -> bool,
    ) -> Option<catcoms_sync::voice_v2::presence::PairSelector> {
        // A whole empty scan is bounded64; one actual ready channel gets this turn. Advance
        // on every refusal as well as success, retaining no per-peer map or retry queue.
        for _ in 0..64 {
            let slot = self.channel;
            self.channel = (self.channel + 1) % 64;
            let Ok(route) = registry.route(slot + 1) else {
                continue;
            };
            if !route.native_ready() {
                continue;
            }
            let index = usize::from(slot);
            let live = context.upgrade()?;
            let ready = observe(
                live.ledger
                    .has_ready_work(route.selected(), route.session().identity()),
            );
            drop(live);
            if ready && self.ready_first[index] {
                self.ready_turn(context, &route, index);
                return None;
            }
            return self.ordinary_turn(context, &route, index).1;
        }
        None
    }
    fn ready_turn(&mut self, context: &Context, route: &Route, index: usize) -> bool {
        // As with ordinary work, only an ACTUAL admission changes whose turn is owed.
        // In production an ordinary worker can still own NativeHold when the urgent
        // Ready scan runs in the same cadence. Discharging Ready's priority on that
        // refusal lets ordinary reacquire the free slot every next cadence forever.
        let cursor = self.ready_rows[index];
        let admitted = native::produce_ready(context, route, cursor).is_ok();
        if admitted {
            self.ready_first[index] = false;
            self.ready_rows[index] = (cursor + 1) % 32;
        }
        admitted
    }

    /// Return actual worker admission separately from the optional exact-Pending metadata.
    /// Refusal keeps its ordinary stage; a busy Ready worker must not advance/discard it.
    fn ordinary_turn(
        &mut self,
        context: &Context,
        route: &Route,
        index: usize,
    ) -> (bool, Option<catcoms_sync::voice_v2::presence::PairSelector>) {
        let stage = self.stage[index];
        let (admitted, selected) = if matches!(stage, 1 | 3) {
            let cursor = self.continuations[index];
            let admitted = native::produce_confirmation(context, route, cursor, stage == 3).is_ok();
            if admitted {
                self.continuations[index] = (cursor + 1) % 32;
            }
            (admitted, None)
        } else {
            let Some(live) = context.upgrade() else {
                return (false, None);
            };
            // Discovery never consumes the exact-Pending cursor. The original native row
            // remains the authority; no asynchronous factory is selected by a renderer hint.
            let mut proposed = self.pending[index];
            let pending = if stage == 2 {
                None
            } else {
                live.ledger
                    .pending_derivation(route.selected(), &mut proposed)
            };
            drop(live);
            if let Some(selector) = pending {
                let admitted = native::produce_derivation(context, route, selector).is_ok();
                // Busy Ready work must not consume every other original selection.
                // Commit rotation only when its actual native worker acquired the slot.
                if admitted {
                    self.pending[index] = proposed;
                }
                (admitted, admitted.then_some(selector))
            } else {
                let cursor = self.peers[index];
                let admitted = native::discover_derivation(context, route, cursor).is_ok();
                if admitted {
                    self.peers[index] = (cursor + 1) % 64;
                }
                (admitted, None)
            }
        };
        if admitted {
            self.stage[index] = (stage + 1) % 4;
            self.ready_first[index] = true;
        }
        (admitted, selected)
    }
}

/// Synchronous maintenance executes while both pinned service/timer futures remain live.
/// Count its actual native wrappers in addition to those complete futures. The producer
/// prep/task and peer Vec/RPC have their independent8KiB/4KiB original payments; do not
/// silently borrow spare service bytes to pay those graphs as well.
pub(super) fn turn_layout_bytes() -> usize {
    std::mem::size_of::<crate::voice_handoff::context::LiveContext>()
        + std::mem::size_of::<Route>()
        + 2 * std::mem::size_of::<catcoms_sync::voice_v2::presence::PairSelector>()
        + std::mem::size_of::<Option<native::NativeWaiter>>()
        + crate::voice_handoff::ledger::Ledger::collection_layout_bytes()
        + 256 // bounded registry/index/state guards and synchronous call wrappers
}

pub(super) async fn run(context: Option<&Context>, registry: &Registry, clock: &dyn Clock) {
    let Some(context) = context else {
        return std::future::pending().await;
    };
    let mut cursor = Cursor::new();
    loop {
        // Each actual completed turn gets one new interval. Clock jumps never create a
        // catch-up burst, and this future never waits on an Actor or admitted producer.
        let Some(deadline) = clock.monotonic_ms().checked_add(CADENCE_MS) else {
            // Never wrap/saturate a new cadence into an immediately-ready sleep loop.
            // Exhausted protocol time disables production; IO/Host Stop stays pollable.
            return std::future::pending().await;
        };
        super::expire_at(clock, deadline).await;
        let _ = cursor.turn(context, registry);
        cursor.urgent_ready(context, registry);
    }
}

#[cfg(test)]
mod tests;
