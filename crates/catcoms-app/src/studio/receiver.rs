//! Runtime ownership around the existing watched inbox and durable receive adapter. No second
//! inbox, document, membership snapshot or persistence coordinator lives here.

use super::*;
use crate::studio_exchange::ServerStudioWatch;
use catcoms_replication::Admission;
use std::collections::VecDeque;
use std::sync::Arc;
mod catchup;
pub(crate) use catchup::StudioBackgroundResult;

/// Recently accessed targets, bounded by the existing sync watch rail. Reopening the same exact
/// source preserves its inbox; eviction explicitly revokes the old subscription and queued work.
#[derive(Default)]
pub(crate) struct StudioReceiver {
    watches: VecDeque<(ServerStudioWatch, u128)>,
    paused: bool,
    pause_notice: bool,
    catchup: catchup::CatchupRuntime,
    gossip_runs: usize,
}
impl StudioReceiver {
    /// Notify before any bounded event-channel await: native work never waits on the event
    /// consumer, and event backpressure must not conceal an already-queued inbox packet.
    pub(crate) fn signal<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
        signal: &tokio::sync::watch::Sender<bool>,
    ) {
        signal.send_if_modified(|pending| {
            let next = self.pending(server);
            if *pending == next {
                false
            } else {
                *pending = next;
                true
            }
        });
    }
    pub(crate) fn pending<T: MeshTransport, R: CryptoRngCore>(
        &self,
        server: &Server<T, R>,
    ) -> bool {
        !self.paused
            && (self.watches.iter().any(|(watch, _)| {
                server.sync.studio_has_inbound(&watch.inner)
                    || server.sync.studio_has_page_request(&watch.inner)
            }) || self.catchup.pending(server, &self.watches))
    }
    pub(crate) fn take_pause_notice(&mut self) -> bool {
        std::mem::take(&mut self.pause_notice)
    }

    fn observe<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &ServerStore,
        id: u64,
        target: StudioTarget,
        epoch: u128,
    ) -> Result<(), AppError> {
        // A logical object cannot acquire extra slots by changing its claimed channel.
        let logical = target.document(&server.group_id()).map_err(invalid)?;
        if let Some(i) = self.watches.iter().position(|(w, _)| {
            w.target.document(&server.group_id()).ok().as_ref() == Some(&logical)
        }) {
            let (old, held_epoch) = self.watches.remove(i).expect("watch index");
            if old.target == target
                && old.server == id
                && held_epoch == epoch
                && Arc::ptr_eq(&old.mount, &store.registry_mount())
                && server.sync.studio_watch_is_current(&old.inner)
            {
                self.watches.push_back((old, held_epoch));
                return Ok(());
            }
            let _ = server.unwatch_studio_epoch(&old);
        }
        if self.watches.len() == 16 {
            let (old, _) = self.watches.pop_front().expect("full watch rail");
            let _ = server.unwatch_studio_epoch(&old);
        }
        // The existing transaction already validated channel/member and loaded this concrete
        // epoch (or proved absence). Do not replay the full source again just to subscribe.
        let inner = server.sync.watch_studio(target, epoch)?;
        self.watches.push_back((
            ServerStudioWatch {
                inner,
                mount: store.registry_mount(),
                server: id,
                target,
            },
            epoch,
        ));
        Ok(())
    }

    /// Executed only inside the same off-executor Server/vault/native lease as ordinary Save.
    /// A busy/locked caller never reaches this method; one pass consumes at most one packet.
    pub(crate) fn run<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        request: Option<StudioRequest>,
    ) -> Result<(StudioSavedTransaction, Option<StudioTarget>), AppError> {
        if let Some(request) = request {
            let updated = request.changes_state().then(|| request.target());
            let saved = server.studio_transaction_with_publication(store, id, request)?;
            // Only explicit successful access retries a failed/over-budget background pass.
            // More inbound traffic cannot repeatedly restart expensive failed disk work.
            self.paused = false;
            self.catchup
                .explicit_retry(server.runtime_clock().monotonic_ms());
            for &(target, epoch) in &saved.observed {
                // A trusted cooperative caller can occupy the sync watch rail separately. A
                // watch refusal must not misreport an already-durable Save as failed.
                if self.observe(server, store, id, target, epoch).is_err() {
                    tracing::warn!("Studio watch admission unavailable; saved state unchanged");
                }
            }
            return Ok((saved, updated));
        }
        let empty = || StudioSavedTransaction {
            view: None,
            packets: vec![],
            observed: vec![],
        };
        if self.paused {
            return Ok((empty(), None));
        }
        let serving = self
            .watches
            .iter()
            .any(|(w, _)| server.sync.studio_has_page_request(&w.inner));
        if (serving && self.gossip_runs >= 1)
            || (self.gossip_runs >= 4 && self.catchup.pending(server, &self.watches))
        {
            self.gossip_runs = 0;
            return match self.catchup.run(server, store, id, &self.watches) {
                Ok(updated) => Ok((empty(), updated)),
                Err(error) => {
                    self.paused = true;
                    self.pause_notice = true;
                    Err(error)
                }
            };
        }
        let Some(index) = self
            .watches
            .iter()
            .position(|(w, _)| server.sync.studio_has_inbound(&w.inner))
        else {
            let result = self.catchup.run(server, store, id, &self.watches);
            return match result {
                Ok(updated) => Ok((empty(), updated)),
                Err(error) => {
                    self.paused = true;
                    self.pause_notice = true;
                    Err(error)
                }
            };
        };
        let (watch, epoch) = self.watches.remove(index).expect("pending watch index");
        self.gossip_runs = self.gossip_runs.saturating_add(1);
        // Round-robin work selection: one busy document cannot monopolize every receive pass.
        self.watches.push_back((watch, epoch));
        let watch = &self.watches.back().expect("selected watch").0;
        if watch.server != id || !Arc::ptr_eq(&watch.mount, &store.registry_mount()) {
            let _ = server.unwatch_studio_epoch(watch);
            return Err(invalid("Studio receive mount or numeric server changed"));
        }
        if !server
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(watch.target.channel()))
        {
            let _ = server.unwatch_studio_epoch(watch);
            return Err(invalid("Studio receive channel was removed"));
        }
        // Persist the current membership/device state before admitting a source that needs it on
        // restart. Exclusive native snapshot ordering remains held by the caller throughout.
        if !server.sync.check_studio_inbound(&watch.inner)? {
            return Ok((empty(), None));
        }
        let received = (|| {
            // Serving another watched target can displace this graph. Prepare the existing
            // source off-actor before draining; a healthy cold target is not a storage failure.
            // Actual absence needs no rebuild; inventory still verifies that fact before Save.
            let large_cold = server.sync.with_registry_context(|g, d, _, _| {
                if store.studio_source_is_warm(id, g, watch.target, d) {
                    Ok(false)
                } else {
                    store.studio_receive_needs_preparation(id, &g.group_id(), watch.target)
                }
            })?;
            if large_cold && !self.catchup.prepare(server, store, id, watch.target)? {
                return Ok(None);
            }
            // A warm candidate permits only bounded authentication, not stale-source use.
            // The store takes ownership and checks exact bytes again before actual ingest.
            server.sync.with_registry_context(|group, device, _, _| {
                if !store.studio_source_is_warm(id, group, watch.target, device) {
                    store.check_studio_receive_source_bound(id, &group.group_id(), watch.target)?;
                }
                Ok::<_, AppError>(())
            })?;
            // Limit before ANY source reconstruction or snapshot write. Directory order can
            // affect which small records are inspected, never produce a partial successful budget.
            let mut scan = store.scan_studio_receive_inventory()?;
            while !scan.step()?.complete {}
            let inventory = scan.finish()?;
            let snapshot = server.snapshot()?;
            server
                .sync
                .with_registry_context(|_, _, _, rng| store.save_server(id, &snapshot, rng))?;
            let mut budget = server.sync.with_registry_context(|g, _, _, _| {
                store.studio_storage_budget(id, g, &inventory)
            })?;
            server.receive_studio_step_reusing(store, watch, &mut budget)
        })();
        let received = match received {
            Ok(received) => received,
            Err(error) => {
                // A pre-drain failure retains its packet; an admission failure may consume it.
                // Neither is delivery. Hold BOTH cases instead of a peer-driven disk retry loop.
                self.paused = true;
                self.pause_notice = true;
                return Err(error);
            }
        };
        let updated = if let Some(received) = received {
            let updated = (received.admission == Admission::Accepted).then_some(watch.target);
            server.sync.with_registry_context(|group, device, _, _| {
                store.retain_received_studio_source(group, device, received.state);
            });
            updated
        } else {
            None
        };
        Ok((empty(), updated))
    }
}
