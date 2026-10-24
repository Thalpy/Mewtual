//! Native-only bounded work production. The Host rotates existing canonical channel slots;
//! it never receives a peer/action constructor from the renderer. Actor waits belong to one
//! separately paid native-position worker per channel, so a parked old Actor cannot stop
//! service IO, another group, or a replacement join's independent channel.
use super::*;
use crate::voice_handoff::factory::native;

const CADENCE_MS: u64 = 250;
struct Cursor {
    channel: u8,
    pending: [u8; 64],
    peers: [u8; 64],
    stage: [u8; 64],
    continuations: [u8; 64],
}
impl Cursor {
    fn new() -> Self {
        Self {
            channel: 0,
            pending: [0; 64],
            peers: [0; 64],
            stage: [0; 64],
            continuations: [0; 64],
        }
    }
    /// Returns only a successfully spawned exact-Pending selection, for native scheduling
    /// observation/tests. This is not Root publication or network delivery evidence.
    fn turn(
        &mut self,
        context: &Context,
        registry: &Registry,
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
            let stage = self.stage[index];
            self.stage[index] = (stage + 1) % 4;
            if matches!(stage, 1 | 3) {
                let cursor = self.continuations[index];
                // Advance even when an earlier original Pending permanently refuses.
                // A worker-local cursor alone would repeatedly select the first row.
                self.continuations[index] = (cursor + 1) % 32;
                let _ = native::produce_confirmation(context, &route, cursor, stage == 3);
                return None;
            }
            let live = context.upgrade()?;
            // A discovery turn must not consume the Pending cursor. Otherwise an even
            // number of retained rows can alternate selected/discarded forever, starving
            // every second original request despite repeated successful worker drainage.
            let pending = if stage == 2 {
                None
            } else {
                live.ledger
                    .pending_derivation(route.selected(), &mut self.pending[index])
            };
            drop(live);
            if let Some(selector) = pending {
                // Observer Drop detaches, never aborts. Original native position/task/output
                // retain payment and shutdown custody through actual Actor publication.
                return native::produce_derivation(context, &route, selector)
                    .ok()
                    .map(|_observer| selector);
            } else {
                let cursor = self.peers[index];
                self.peers[index] = (cursor + 1) % 64;
                let _ = native::discover_derivation(context, &route, cursor);
            }
            return None;
        }
        None
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
    }
}

#[cfg(test)]
mod tests;
