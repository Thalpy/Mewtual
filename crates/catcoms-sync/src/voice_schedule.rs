//! Production scheduling for V2 voice transport (docs/communication-recovery/VOICE-SIGNALLING-V2.md
//! "Bounded helper routing", VOICE-V2-CALL-DRIVER.md).
//!
//! Every protocol decision lives in the reviewed seams this module drives: kind-28 admission in
//! `voice_transport`, native endpoint controls in `voice_endpoint`, and the source/helper ledgers
//! in `voice_v2::{source, helper}`. Before this module those seams ran only when a test called them
//! by hand. This module decides only WHEN they run:
//!
//! - each sync tick, after a served kind-28 request and after each flight completion, the sole
//!   Root runs one bounded pass, but only while V2 state is live (a native join or watcher, or a
//!   retained source/helper packet): the native endpoint pump (renewed announcements, watcher
//!   challenges, join proofs: at most four public responses), then source flights and helper
//!   forwards (at most four each). Both classes share four per-group forwarding slots, so they
//!   take turns: the class owed the next opportunity prepares first and keeps that claim until it
//!   actually gets a flight. An idle attached actor pays two cheap emptiness checks;
//! - every prepared flight runs detached and connected-only, owned on `self` like the other
//!   recovery jobs, so a cancelled actor poll never drops a lower request or its credit;
//! - each completion is settled by the same Root that prepared it, which re-verifies the signed
//!   ACK, the route generation and the native owner (`complete_voice_source/forward`);
//! - while V2 state is live, a quiet-network wake keeps renewals, 500 ms hedges and retries
//!   moving without another event.
//!
//! Nothing here mints authority. Announcements alone still never admit presence, a helper's
//! queued ACK never settles a final edge, and submission is never remote receipt. No route is
//! dialed: only proven bound routes in the live connection snapshot are used.
//!
//! SERVICE IS OFF BY DEFAULT (`VoiceRouting::Disabled`): the V2 decision keeps production
//! dispatch and forwarding disabled until its remaining gates pass review. Enabling it is a
//! separate, explicit rollout decision; no production caller does so yet.

use super::*;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use std::collections::VecDeque;

/// Completions refused only for temporary ACK-verification capacity keep their exact affine
/// result for a later pass. Beyond this many, the completion is abandoned: its attempt stays
/// spent and its lower credit is released at the real drop, which is the conservative outcome.
const RETRY_BACKLOG: usize = 8;
/// Quiet-network pump cadence while V2 state is live. Shorter than the 500 ms hedge and far
/// shorter than the 5 s renewal, so neither waits on unrelated traffic; one pass is bounded.
const ACTIVE_WAKE_MS: u64 = 250;

/// Local rollout switch for V2 voice transport. It only RESTRICTS what the authenticated group
/// policy already allows: it never widens eligibility, changes group mode, or turns a live V2
/// join into a legacy one.
///
/// | Change | Effect |
/// | --- | --- |
/// | `Disabled` <-> `Forwarding` | Pauses or resumes network service only. Native joins and watchers are untouched; while paused they make no progress and their signed lifetimes run out. |
/// | anything <-> `Direct` | Changes the runtime policy view's `indirect` bit, which revokes the policy lease: every live V2 join, watcher, helper edge and pending reply on this server ends. A fresh join is required. |
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VoiceRouting {
    /// Serve no kind-28 request and schedule nothing (the default). A V2 request gets the same
    /// empty reply an unknown kind always got: unavailable, never "unsupported". Native owners
    /// may still be created exactly as before this module; they make no network progress.
    #[default]
    Disabled,
    /// Direct V2 only. This device sends and receives its own endpoint traffic over direct bound
    /// routes, but neither forwards for other members nor sends or accepts helper-routed packets.
    Direct,
    /// Direct plus the design's one-helper forwarding (the A-B-C case, B not in the room).
    Forwarding,
}

/// One detached lower request, returned to the Root that prepared it.
pub(super) enum VoiceFlight {
    Source(CompletedVoiceSource),
    Forward(CompletedVoiceForward),
}

/// The class owed the next opportunity at the shared per-group forwarding slots. Source flights
/// and helper forwards both charge `ResourceKind::Forward` (four per group), so whichever class
/// prepares first takes every freed slot. Preparing source work first unconditionally let a
/// helper's own observation traffic starve an accepted helper packet until it expired.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum FlightClass {
    #[default]
    Source,
    Helper,
}

pub(super) struct VoiceSchedule {
    routing: VoiceRouting,
    /// Persistent across passes and capacity refusals: the owed class keeps priority until it
    /// actually receives a flight, then the other class is owed. Under contention the two classes
    /// therefore alternate per freed slot; when one class has no eligible work the other still
    /// uses every slot (work-conserving), and the idle class keeps its claim.
    owed: FlightClass,
    pub(super) jobs: FuturesUnordered<BoxFuture<'static, VoiceFlight>>,
    retries: VecDeque<VoiceFlight>,
    /// Upper cancellation for every flight. Dropping it (with this owner) cancels too; lower
    /// credits stay with the transport until the actual request retires.
    cancel: tokio::sync::watch::Sender<bool>,
    /// Test-only observation of every submitted flight's next hop, for forbidden-edge oracles.
    #[cfg(test)]
    pub(super) hops: Vec<PeerId>,
    /// Passive bounded test oracle: increment ONLY after actual signed helper-completion
    /// verification succeeds. Preparing/queuing/timing out a job cannot satisfy receipt tests.
    #[cfg(test)]
    pub(super) helper_refusals: u64,
}

impl Default for VoiceSchedule {
    fn default() -> Self {
        Self {
            routing: VoiceRouting::default(),
            owed: FlightClass::default(),
            jobs: FuturesUnordered::new(),
            retries: VecDeque::new(),
            cancel: tokio::sync::watch::channel(false).0,
            #[cfg(test)]
            hops: Vec::new(),
            #[cfg(test)]
            helper_refusals: 0,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub fn voice_routing(&self) -> VoiceRouting {
        self.voice_schedule.routing
    }

    /// Change the local V2 rollout switch (see `VoiceRouting` for exactly what each change ends).
    /// Disabling cancels upper flight work at once; each lower flight keeps its own credit until
    /// the transport actually retires it, and its completion is still settled on this Root so the
    /// ledgers record the spent attempt.
    pub fn set_voice_routing(&mut self, routing: VoiceRouting) {
        if self.voice_schedule.routing == routing {
            return;
        }
        self.voice_schedule.routing = routing;
        if routing == VoiceRouting::Disabled {
            self.voice_schedule.cancel.send_replace(true);
            self.voice_schedule.cancel = tokio::sync::watch::channel(false).0;
            // Abandoning a retry is terminal: its attempt remains spent, never refunded.
            self.voice_schedule.retries.clear();
        }
        // Publish a changed `indirect` bit before anything else can use the old policy lease.
        self.reconcile_voice_actor();
    }

    /// Whether V2 may use helper routes: authenticated active PeerToPeer membership mesh, unless
    /// the local switch narrows to `Direct`. Both the runtime policy view and every V2 Context
    /// read this one function, so their `indirect` bits always agree. `Disabled` deliberately
    /// does not change it: pausing service must not end native state.
    pub(super) fn voice_indirect_allowed(&self) -> bool {
        // Voice contexts must remain bounded before their worker can reserve progress.
        // The generic member-mesh predicate materializes the complete MLS roster. Check
        // the SAME active P2P pin here, with this actual instance's borrowed local leaf;
        // another member's identity cannot stand in for local membership.
        self.voice_schedule.routing != VoiceRouting::Direct
            && self.group_mode() == GroupMode::PeerToPeer
            && self.group.is_active()
            && self.group.has_local_device(&self.device.device_id())
    }

    pub(super) fn voice_transport_enabled(&self) -> bool {
        self.voice_schedule.routing != VoiceRouting::Disabled
    }

    /// Serve one inbound kind-28 request (only called while service is enabled; otherwise the
    /// generic dispatcher answers it exactly as it answered every V2 request before this module).
    pub(super) fn serve_voice_transport(
        &mut self,
        from: PeerId,
        frame: &[u8],
        responder: catcoms_rt::Responder,
    ) where
        T: 'static,
    {
        self.handle_voice_transport_request(from, frame, responder);
        // An admitted helper packet or endpoint control is live state; a refused request leaves
        // none, so this pass then costs only the cheap emptiness checks.
        self.pump_voice_schedule();
    }

    /// Live V2 state that needs scheduling. Cheap: no reconcile, prune, directory read or route
    /// index. Retained terminal ledger records count until a pass prunes them.
    fn voice_schedule_has_work(&self) -> bool {
        let Some(voice) = self.voice_runtime.as_ref() else {
            return false;
        };
        let Some(actor) = voice.current.as_ref() else {
            return false;
        };
        voice.endpoints.needs_schedule()
            || actor.source().is_ok_and(|ledger| !ledger.is_empty())
            || (self.voice_schedule.routing == VoiceRouting::Forwarding
                && actor.helper().is_ok_and(|ledger| !ledger.is_empty()))
    }

    /// One bounded scheduling pass. Errors are local refusals (no current actor, policy, capacity,
    /// rate limit); each seam already left its state retryable or terminal, so they are traced.
    pub(super) fn pump_voice_schedule(&mut self)
    where
        T: 'static,
    {
        if self.voice_schedule.routing == VoiceRouting::Disabled {
            return;
        }
        // Completions that already own an admitted lower result go first, even with no current
        // actor (locked or detached): settling then fails terminally and releases their credit
        // instead of stranding it until unlock. One refused again goes back to the queue's end.
        for _ in 0..self.voice_schedule.retries.len() {
            match self.voice_schedule.retries.pop_front() {
                Some(flight) => self.complete_voice_flight(flight),
                None => break,
            }
        }
        if !self.voice_schedule_has_work() {
            return;
        }
        self.reconcile_voice_actor();
        if self.voice_transport_actor().is_err() {
            return;
        }
        if let Err(error) = self.pump_voice_endpoint() {
            tracing::trace!(%error, "voice endpoint pump refused");
        }
        if self.voice_schedule.routing != VoiceRouting::Forwarding {
            // Direct service has no helper class competing for the shared slots.
            self.start_voice_flights(FlightClass::Source);
            return;
        }
        let owed = self.voice_schedule.owed;
        let other = match owed {
            FlightClass::Source => FlightClass::Helper,
            FlightClass::Helper => FlightClass::Source,
        };
        // The owed class prepares first, so it gets the first freed slot. The other class may
        // still use any slots left in this same pass. The debt moves only once the owed class
        // actually received a flight; a capacity refusal or an idle owed class keeps it owed.
        if self.start_voice_flights(owed) > 0 {
            self.voice_schedule.owed = other;
        }
        self.start_voice_flights(other);
    }

    /// Prepare and start one class's flights (each preparation is bounded to four). Returns how
    /// many were started. Preparation reserves attempts and shared capacity synchronously.
    fn start_voice_flights(&mut self, class: FlightClass) -> usize
    where
        T: 'static,
    {
        let mut started = 0;
        match class {
            FlightClass::Source => match self.prepare_voice_sources() {
                Ok(pending) => {
                    for flight in pending {
                        #[cfg(test)]
                        self.voice_schedule.hops.push(flight.peer());
                        let signal = self.voice_schedule.cancel.subscribe();
                        self.voice_schedule.jobs.push(
                            async move { VoiceFlight::Source(flight.execute(signal).await) }
                                .boxed(),
                        );
                        started += 1;
                    }
                }
                Err(error) => tracing::trace!(%error, "voice source preparation refused"),
            },
            FlightClass::Helper => match self.prepare_voice_forwards() {
                Ok(pending) => {
                    for flight in pending {
                        #[cfg(test)]
                        self.voice_schedule.hops.push(flight.peer());
                        let signal = self.voice_schedule.cancel.subscribe();
                        self.voice_schedule.jobs.push(
                            async move { VoiceFlight::Forward(flight.execute(signal).await) }
                                .boxed(),
                        );
                        started += 1;
                    }
                }
                Err(error) => tracing::trace!(%error, "voice helper preparation refused"),
            },
        }
        started
    }

    /// Settle one completed flight on this Root. Only temporary verification capacity keeps the
    /// exact completion for retry; every other failure is terminal and spends its attempt.
    pub(super) fn complete_voice_flight(&mut self, flight: VoiceFlight) {
        let retry = match flight {
            VoiceFlight::Source(completed) => match self.complete_voice_source(completed) {
                Ok(_) => None,
                Err(failure) => Self::retryable(failure).map(VoiceFlight::Source),
            },
            VoiceFlight::Forward(completed) => match self.complete_voice_forward(completed) {
                Ok(_status) => {
                    #[cfg(test)]
                    if _status == VoiceTransportStatus::Refused {
                        self.voice_schedule.helper_refusals =
                            self.voice_schedule.helper_refusals.saturating_add(1);
                    }
                    None
                }
                Err(failure) => Self::retryable(failure).map(VoiceFlight::Forward),
            },
        };
        if let Some(flight) = retry {
            if self.voice_schedule.retries.len() < RETRY_BACKLOG
                && self.voice_schedule.routing != VoiceRouting::Disabled
            {
                self.voice_schedule.retries.push_back(flight);
            }
        }
    }

    fn retryable<C>(failure: VoiceCompletionFailure<C>) -> Option<C> {
        if matches!(
            failure.error(),
            voice_v2::VoiceError::Capacity | voice_v2::VoiceError::RateLimited
        ) {
            failure.into_retry()
        } else {
            None
        }
    }

    /// Relative quiet-network wake, on the voice clock, while V2 state is live or completions
    /// await a retry. None when idle or disabled.
    pub(super) fn next_voice_schedule_delay(&self) -> Option<u64> {
        if self.voice_schedule.routing == VoiceRouting::Disabled {
            return None;
        }
        (!self.voice_schedule.retries.is_empty() || self.voice_schedule_has_work())
            .then_some(ACTIVE_WAKE_MS)
    }
}
// Scheduling regressions live with the V2 transport fixtures in voice_runtime/transport_tests.rs.
