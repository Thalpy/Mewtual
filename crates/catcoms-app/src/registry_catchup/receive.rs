//! Receiver-owned progress, not a ciphertext outbox or finality protocol. Network waits never
//! borrow the vault. A pending page remains here until one accounted store transaction succeeds.
use super::*;
use crate::store::{epoch_budget::EpochStorageBudget, RegistryPageAdmission};
use catcoms_replication::epoch::MAX_EPOCH_OPERATIONS;
use catcoms_replication::registry_epoch::catchup::{
    RegistryFrontier, RegistryOpPage, RegistryPageCursor,
};
use catcoms_sync::registry_catchup::RegistryReceivePermit;

const PASS_LIFETIME_MS: u64 = 600_000;
const REQUEST_INTERVAL_MS: u64 = 1_000;
const MAX_RECEIVED_BYTES: usize = 16 * 1024 * 1024;

/// All completion labels describe one provider's captured prefix only, never owner finality or
/// global currency. Held states require discovery/a new pass, not legacy catch-up fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryReceiveState {
    Ready,
    PageReady,
    Paused,
    PrefixComplete,
    RestartRequired,
    CheckpointRequired,
    HistoricalAuthorizationRequired,
    Stopped,
}

/// Attempt/page counters are bounded work and durability evidence, not delivery acknowledgements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistryReceiveProgress {
    pub attempts: usize,
    pub received_pages: usize,
    pub received_operations: usize,
    pub received_bytes: usize,
    pub saved_pages: usize,
    pub persist_attempts: usize,
    pub accepted: usize,
    pub duplicates: usize,
}

/// One of four accounted receiver passes per sync runtime. It pins provider AND requester full
/// identities, watch generation, concrete epoch and physical vault mount. At most one 512-KiB
/// page is retained. Drop loses only traversal: saved pages remain durable and a new pass derives
/// its frontier from them. The provider's opaque cursor is never persisted as security state.
pub struct ServerRegistryReceive {
    permit: RegistryReceivePermit,
    mount: Arc<()>,
    server: u64,
    bucket: u8,
    requester: crate::DeviceId,
    peer: PeerId,
    provider: crate::DeviceId,
    frontier: RegistryFrontier,
    empty_fallback_used: bool,
    cursor: Option<RegistryPageCursor>,
    pending: Option<RegistryOpPage>,
    pending_epoch: u64,
    state: RegistryReceiveState,
    progress: RegistryReceiveProgress,
    now: u64,
    expires: u64,
    retry_at: u64,
    write_at: u64,
}
impl std::fmt::Debug for ServerRegistryReceive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRegistryReceive")
            .field("state", &self.state)
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}
impl ServerRegistryReceive {
    pub fn state(&self) -> RegistryReceiveState {
        self.state
    }
    pub fn progress(&self) -> RegistryReceiveProgress {
        self.progress
    }
    /// Scheduling hint on the receiver's injected monotonic clock, not the provider cursor clock.
    pub fn retry_at_ms(&self) -> u64 {
        if self.pending.is_some() {
            self.write_at
        } else {
            self.retry_at
        }
    }

    /// Explicitly retry a failed/cancelled step without forgiving attempts, pacing or lifetime.
    /// A held page retries persistence, not network. Reconciliation of uncertain storage remains
    /// the caller's responsibility; retry is not permission to ignore an invalid budget.
    pub fn retry(&mut self) {
        if self.state == RegistryReceiveState::Paused {
            self.state = if self.pending.is_some() {
                RegistryReceiveState::PageReady
            } else {
                RegistryReceiveState::Ready
            };
        }
    }

    /// Independent edits commonly give the requester a head this provider has never seen.
    /// Ask for the whole prefix once, retaining the verified seed and every charged limit.
    /// Never do this after accepting a page: changing the frontier invalidates its cursor.
    fn receive_restart(&mut self) {
        if !self.empty_fallback_used
            && !self.frontier.heads.is_empty()
            && self.cursor.is_none()
            && self.progress.received_pages == 0
        {
            self.frontier.heads.clear();
            self.empty_fallback_used = true;
            self.state = RegistryReceiveState::Ready;
        } else {
            self.state = RegistryReceiveState::RestartRequired;
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Reserve bounded ownership, verify and flush the checked starting state, then capture its
    /// frontier. No file is created when epoch zero is absent. The watch must already be installed
    /// and the provider endpoint proven. No vault borrow survives into a network request.
    pub fn begin_registry_receive(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerRegistryWatch,
        peer: PeerId,
        budget: &mut EpochStorageBudget,
    ) -> Result<ServerRegistryReceive, AppError> {
        self.check_registry_watch(store, watch)?;
        let provider = self
            .sync
            .registry_page_peer_device(peer)
            .ok_or_else(|| AppError::Invalid("registry provider endpoint is not proven".into()))?;
        let permit = self.sync.begin_registry_receive(&watch.inner)?;
        let now = self.runtime_clock().monotonic_ms();
        let expires = now
            .checked_add(PASS_LIFETIME_MS)
            .ok_or_else(|| AppError::Invalid("registry receive clock exhausted".into()))?;
        let (requester, frontier) = self.sync.with_registry_context(|group, device, _, rng| {
            let (_, state) = store.ingest_registry_page(
                watch.server,
                group,
                watch.bucket,
                permit.doc_id(),
                device,
                &[],
                rng,
                budget,
            )?;
            Ok::<_, AppError>((
                device.device_id(),
                state.map_or(
                    RegistryFrontier {
                        heads: vec![],
                        seed: None,
                    },
                    |mut state| state.catchup_frontier(),
                ),
            ))
        })?;
        Ok(ServerRegistryReceive {
            permit,
            mount: store.registry_mount(),
            server: watch.server,
            bucket: watch.bucket,
            requester,
            peer,
            provider,
            frontier,
            empty_fallback_used: false,
            cursor: None,
            pending: None,
            pending_epoch: 0,
            state: RegistryReceiveState::Ready,
            progress: RegistryReceiveProgress::default(),
            now,
            expires,
            retry_at: now,
            write_at: now,
        })
    }

    fn check_registry_receive(&mut self, pass: &mut ServerRegistryReceive) -> Result<(), AppError> {
        pass.now = pass.now.max(self.runtime_clock().monotonic_ms());
        let current = self.sync.with_registry_context(|group, device, _, _| {
            device.device_id() == pass.requester
                && group.member_signature_key(&device.device_id()).as_deref()
                    == Some(device.public_key_bytes().as_slice())
        });
        if !current
            || !self.sync.registry_receive_is_current(&pass.permit)
            || self.sync.registry_page_peer_device(pass.peer) != Some(pass.provider)
        {
            pass.pending = None;
            pass.state = RegistryReceiveState::Stopped;
            return Err(AppError::Invalid(
                "registry receiver authority was replaced".into(),
            ));
        }
        if pass.now >= pass.expires {
            pass.pending = None;
            pass.state = RegistryReceiveState::RestartRequired;
        }
        Ok(())
    }

    /// Fetch at most one bounded page. Does no vault I/O or cursor advancement. A cancelled or
    /// unwound future leaves Paused with its attempt/deadline already charged, so explicit Retry
    /// is safe. This exclusive borrow prevents retry while the request is actually still running.
    pub async fn fetch_registry_receive_step(
        &mut self,
        pass: &mut ServerRegistryReceive,
    ) -> Result<RegistryReceiveState, AppError> {
        self.check_registry_receive(pass)?;
        if pass.state != RegistryReceiveState::Ready || pass.now < pass.retry_at {
            return Ok(pass.state);
        }
        if pass.progress.attempts > MAX_EPOCH_OPERATIONS {
            pass.state = RegistryReceiveState::RestartRequired;
            return Ok(pass.state);
        }
        pass.progress.attempts += 1;
        pass.retry_at = pass.now.saturating_add(REQUEST_INTERVAL_MS);
        // Arming Paused before await is deliberate: no caller can observe this pass while it is
        // borrowed by the request; after cancellation it is retryable, never stranded Fetching.
        pass.state = RegistryReceiveState::Paused;
        pass.pending_epoch = self
            .sync
            .with_registry_context(|group, _, _, _| group.epoch());
        let outcome = self
            .request_registry_page(
                pass.peer,
                RegistryPageQuery {
                    bucket: pass.bucket,
                    doc_id: pass.permit.doc_id(),
                    heads: &pass.frontier.heads,
                    seed: pass.frontier.seed,
                    cursor: pass.cursor.as_ref().map(RegistryPageCursor::as_bytes),
                },
            )
            .await?;
        self.check_registry_receive(pass)?;
        if pass.state == RegistryReceiveState::RestartRequired {
            return Ok(pass.state);
        }
        match outcome {
            None => {} // unsupported/refused is retryable, never prefix completion
            Some(RegistryPageOutcome::Restart) => pass.receive_restart(),
            Some(RegistryPageOutcome::CheckpointRequired) => {
                pass.state = RegistryReceiveState::CheckpointRequired
            }
            Some(RegistryPageOutcome::HistoricalAuthorizationRequired) => {
                pass.state = RegistryReceiveState::HistoricalAuthorizationRequired
            }
            Some(RegistryPageOutcome::Page(page)) => {
                let bytes: usize = page
                    .operations
                    .iter()
                    .map(|op| op.blob.ciphertext.len() + 62)
                    .sum();
                pass.progress.received_pages += 1;
                pass.progress.received_operations += page.operations.len();
                pass.progress.received_bytes += bytes;
                if pass.progress.received_pages > MAX_EPOCH_OPERATIONS + 1
                    || pass.progress.received_operations > MAX_EPOCH_OPERATIONS
                    || pass.progress.received_bytes > MAX_RECEIVED_BYTES
                    || page
                        .next
                        .as_ref()
                        .zip(pass.cursor.as_ref())
                        .is_some_and(|(next, old)| next.as_bytes() == old.as_bytes())
                {
                    pass.state = RegistryReceiveState::RestartRequired;
                    return Ok(pass.state);
                }
                pass.pending = Some(page);
                pass.state = RegistryReceiveState::PageReady;
            }
        }
        Ok(pass.state)
    }

    /// Save the pending page atomically, then and only then select its continuation. A failed
    /// save retains the SAME page and old cursor; retries do not spend more network work.
    /// Empty terminal pages pass all target/inventory/flush checks without creating epoch zero.
    pub fn persist_registry_receive_step(
        &mut self,
        store: &mut ServerStore,
        pass: &mut ServerRegistryReceive,
        budget: &mut EpochStorageBudget,
    ) -> Result<RegistryReceiveState, AppError> {
        if !Arc::ptr_eq(&pass.mount, &store.registry_mount()) {
            pass.pending = None;
            pass.state = RegistryReceiveState::Stopped;
            return Err(AppError::Invalid(
                "registry receiver vault mount was replaced".into(),
            ));
        }
        self.check_registry_receive(pass)?;
        if pass.state != RegistryReceiveState::PageReady {
            return Ok(pass.state);
        }
        if pass.now < pass.write_at {
            return Ok(pass.state);
        }
        if pass.progress.persist_attempts > MAX_EPOCH_OPERATIONS {
            pass.pending = None;
            pass.state = RegistryReceiveState::RestartRequired;
            return Ok(pass.state);
        }
        pass.progress.persist_attempts += 1;
        pass.write_at = pass.now.saturating_add(REQUEST_INTERVAL_MS);
        pass.state = RegistryReceiveState::Paused;
        if self
            .sync
            .with_registry_context(|group, _, _, _| group.epoch())
            != pass.pending_epoch
        {
            // These ciphertexts can never pass current-MLS admission now. Do not advertise a
            // persistence retry that will fail forever, or advance past a page we did not save.
            pass.pending = None;
            pass.state = RegistryReceiveState::RestartRequired;
            return Err(AppError::Invalid(
                "registry page MLS epoch changed; start a fresh pass".into(),
            ));
        }
        let page = pass.pending.as_ref().expect("PageReady retains its page");
        let counts = self.sync.with_registry_context(|group, device, _, rng| {
            store
                .ingest_registry_page(
                    pass.server,
                    group,
                    pass.bucket,
                    pass.permit.doc_id(),
                    device,
                    &page.operations,
                    rng,
                    budget,
                )
                .map(|(counts, _)| counts)
        })?;
        let RegistryPageAdmission {
            accepted,
            duplicates,
        } = counts;
        pass.progress.saved_pages += 1;
        pass.progress.accepted += accepted;
        pass.progress.duplicates += duplicates;
        pass.cursor = pass.pending.take().expect("saved pending page").next;
        pass.state = if pass.cursor.is_some() {
            RegistryReceiveState::Ready
        } else {
            RegistryReceiveState::PrefixComplete
        };
        Ok(pass.state)
    }
}

#[cfg(test)]
mod tests;
