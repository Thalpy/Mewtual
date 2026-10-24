//! One recent bucket at a time uses the existing provider, page pass and native worker. A
//! registry hint never installs a Studio checkpoint: consumers still request an owner head.
use super::*;
use crate::registry_catchup::RegistryReceiveState;
use crate::registry_ingress::ServerRegistryWatch;
use catcoms_replication::{
    registry::{registry_document, PointerKey},
    EpochPhase,
};

fn pointer(target: StudioTarget, group: &[u8]) -> Result<PointerKey, AppError> {
    let logical = target.document(group).map_err(invalid)?;
    PointerKey::new(logical.doc_type, logical.logical_key).map_err(invalid)
}

impl CatchupRuntime {
    pub(super) fn persist_registry_page<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
    ) -> Result<bool, AppError> {
        if self
            .registry_pass
            .as_ref()
            .is_none_or(|p| p.state() != RegistryReceiveState::PageReady)
        {
            return Ok(false);
        }
        let Some(target) = self.registry_target else {
            self.registry_pass = None;
            return Ok(false);
        };
        let bucket = pointer(target, &server.group_id())?.bucket();
        if !self.prepare_registry_inventory(server, store, id, bucket)? {
            return Ok(true);
        }
        let mut budget = Self::inventory_budget(server, store, id)?;
        let key = pointer(target, &server.group_id())?;
        let hint = server.registry_maintenance_hint(
            store,
            self.registry_provider.as_mut().expect("prepared Registry"),
            &key,
            &mut budget,
        )?;
        let document = registry_document(&server.group_id(), bucket).map_err(invalid)?;
        let physical = hint.map(|h| h.doc_id).unwrap_or_else(|| {
            catcoms_replication::epoch_zero_id(document.doc_type, &document.logical_key)
        });
        if hint.is_some_and(|h| h.phase != EpochPhase::Open)
            || self
                .registry_pass
                .as_ref()
                .is_none_or(|p| p.doc_id() != physical)
        {
            // A local seal/fault may supersede a page while its network request is detached.
            // An installed successor is Open but likewise supersedes the old physical watch.
            // Discard only that stale page; do not feed it to Open-only admission and promote
            // this ordinary per-document transition into a vault-wide receive pause.
            self.registry_pass = None;
            self.registry_next_at = server.runtime_clock().monotonic_ms().saturating_add(5_000);
            return Ok(true);
        }
        let pass = self.registry_pass.as_mut().expect("held page");
        let group_id = server.group_id();
        let result =
            store.with_studio_protocol_scope(id, &group_id, &mut budget, |store, budget| {
                server.persist_registry_receive_step(store, pass, budget)
            });
        if let Err(error) = result {
            if matches!(
                pass.state(),
                RegistryReceiveState::RestartRequired | RegistryReceiveState::Stopped
            ) {
                self.registry_pass = None;
                self.registry_next_at = server.runtime_clock().monotonic_ms().saturating_add(5_000);
            } else {
                return Err(error);
            }
        }
        Ok(true)
    }

    pub(super) fn work_registry<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        server: &mut Server<T, R>,
        store: &mut ServerStore,
        id: u64,
        watches: &VecDeque<(ServerStudioWatch, u128)>,
    ) -> Result<bool, AppError> {
        let now = server.runtime_clock().monotonic_ms();
        if self
            .registry_target
            .is_some_and(|t| !watches.iter().any(|(w, _)| w.target == t))
        {
            if let Some(watch) = self.registry_watch.take() {
                let _ = server.unwatch_registry_epoch(&watch);
            }
            self.registry_pass = None;
            self.registry_target = None;
        }
        if self.in_flight
            || self.preparing
            || self.checkpoint.is_some()
            || self.discovery_plan.is_some()
            || watches.is_empty()
        {
            return Ok(false);
        }
        if let Some(pass) = &self.registry_pass {
            if pass.state() == RegistryReceiveState::Ready {
                return Ok(false);
            }
            if matches!(
                pass.state(),
                RegistryReceiveState::CheckpointRequired | RegistryReceiveState::RestartRequired
            ) {
                if let Some((watch, _)) = watches
                    .iter()
                    .find(|(w, _)| Some(w.target) == self.registry_target)
                {
                    if let Some(peer) = server.sync.studio_page_peers().first() {
                        self.schedule_discovery(store, id, watch, *peer);
                    }
                }
            }
            self.registry_pass = None;
            self.registry_target = None;
            self.registry_next_at = now.saturating_add(5_000);
            return Ok(false);
        }
        if now < self.registry_next_at {
            return Ok(false);
        }
        self.registry_next_at = now.saturating_add(5_000);
        let target = self
            .registry_target
            .unwrap_or(watches[self.registry_selection % watches.len()].0.target);
        self.registry_target = Some(target);
        let key = pointer(target, &server.group_id())?;
        let bucket = key.bucket();
        if !self.prepare(server, store, id, target)?
            || !self.prepare_registry_inventory(server, store, id, bucket)?
        {
            return Ok(true);
        }
        let mut budget = Self::inventory_budget(server, store, id)?;
        let hint = server.registry_maintenance_hint(
            store,
            self.registry_provider.as_mut().expect("Registry provider"),
            &key,
            &mut budget,
        )?;
        if hint.is_some_and(|h| h.phase == EpochPhase::Fault) {
            // Fault is scoped to this bucket, not the vault or unrelated Studio traffic.
            // Keep refusing checkpoint service and writes there until an owner repair, but
            // allow the existing Studio pass and other watched buckets to continue normally.
            self.owner_failure = Some((target, "Registry bucket needs owner repair".into()));
            self.registry_target = None;
            self.registry_selection = self.registry_selection.wrapping_add(1);
            return Ok(false);
        }
        if hint.is_some_and(|h| h.phase == EpochPhase::Closing) && self.owner_snapshot.is_none() {
            // An operation page cannot reopen a sealed epoch. Refresh its existing owner head
            // selection instead; no seed/receipt authority is inferred from a pointer.
            if let (Some((watch, _)), Some(peer)) = (
                watches.iter().find(|(w, _)| w.target == target),
                server.sync.studio_page_peers().first(),
            ) {
                self.schedule_discovery(store, id, watch, *peer);
            }
            self.registry_target = None;
            self.registry_selection = self.registry_selection.wrapping_add(1);
            return Ok(false);
        }
        let studio = server.sync.with_registry_context(|g, d, _, _| {
            store.prepared_studio_maintenance_state(id, g, target, d, &mut budget)
        })?;
        let Some((epoch, EpochPhase::Open)) = studio else {
            self.registry_target = None;
            self.registry_selection = self.registry_selection.wrapping_add(1);
            return Ok(false);
        };
        let mut physical = hint.map(|h| h.doc_id);
        if hint.is_none_or(|h| {
            h.phase == EpochPhase::Open
                && !h.pointer_deleted
                && h.pointer_epoch.is_none_or(|e| e < epoch)
        }) {
            if let Some(state) = server.sync.with_registry_context(|g, d, _, rng| {
                store.refresh_studio_registry_pointer(id, g, target, d, rng, &mut budget)
            })? {
                physical = Some(state.doc_id());
                // Actual writes invalidate read-only prepared wrappers; the next pass captures
                // this new source. No old graph can answer a query or begin a page under it.
                self.registry_provider = None;
            }
        }
        if let Some(snapshot) = self.owner_snapshot.clone() {
            let logical = registry_document(&server.group_id(), bucket).map_err(invalid)?;
            let pending = store
                .load_epoch_owner_receipts(id, &logical)?
                .pending()
                .is_some();
            if pending
                || hint.is_some_and(|h| h.phase == EpochPhase::Closing || h.close_candidate_ready)
            {
                match server.maintain_studio_registry_owner(
                    store,
                    id,
                    bucket,
                    &snapshot,
                    &mut budget,
                ) {
                    Ok(Some((_, state))) => {
                        physical = Some(state.doc_id());
                        self.registry_provider = None;
                        if state.phase() != EpochPhase::Open {
                            // Recovery acknowledgement may intentionally hold owner settlement.
                            // Receiving a tail here would turn that document hold into a pause.
                            self.registry_target = None;
                            self.registry_selection = self.registry_selection.wrapping_add(1);
                            return Ok(true);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.owner_failure =
                            Some((target, error.to_string().chars().take(256).collect()));
                        return Ok(true);
                    }
                }
            }
        }
        let logical = registry_document(&server.group_id(), bucket).map_err(invalid)?;
        let physical = physical.unwrap_or_else(|| {
            catcoms_replication::epoch_zero_id(logical.doc_type, &logical.logical_key)
        });
        // This id came from the exact checked source/absence or the just-finished durable write,
        // not from the pointer payload. Replace a watch only when its physical binding changes.
        if self
            .registry_watch
            .as_ref()
            .is_none_or(|w| w.bucket != bucket || server.check_registry_watch(store, w).is_err())
            || self.registry_watch_id != Some(physical)
        {
            if let Some(old) = self.registry_watch.take() {
                let _ = server.unwatch_registry_epoch(&old);
            }
            self.registry_watch = Some(ServerRegistryWatch {
                inner: server.sync.watch_registry(bucket, physical),
                mount: store.registry_mount(),
                server: id,
                bucket,
            });
            self.registry_watch_id = Some(physical);
        }
        if self.registry_provider.is_none() {
            return Ok(true);
        }
        let peers = server.sync.studio_page_peers();
        if !peers.is_empty() {
            let peer = peers[self.registry_selection % peers.len()];
            self.registry_pass = Some(server.begin_prepared_registry_receive(
                store,
                self.registry_watch.as_ref().expect("watch"),
                self.registry_provider.as_mut().expect("prepared source"),
                &key,
                peer,
                &mut budget,
            )?);
        } else {
            self.registry_target = None;
        }
        self.registry_selection = self.registry_selection.wrapping_add(1);
        Ok(true)
    }
}
