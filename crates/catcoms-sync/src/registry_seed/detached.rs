//! Shared detached checkpoint discovery/fetch. The opaque handle's retained slot follows jobs
//! and results as well as the handle, so dropping a caller never refunds bytes still in flight.
use super::*;
use crate::receipt_head::{CompletedCheckpointHead, PendingCheckpointHead};

pub struct PendingCheckpointDiscovery<T: MeshTransport> {
    head: PendingCheckpointHead<T>,
    capacity: Arc<()>,
    expires: u64,
}
pub struct CompletedCheckpointDiscovery {
    head: CompletedCheckpointHead,
    capacity: Arc<()>,
    expires: u64,
}
impl<T: MeshTransport> PendingCheckpointDiscovery<T> {
    pub async fn fetch(self) -> CompletedCheckpointDiscovery {
        CompletedCheckpointDiscovery {
            head: self.head.fetch().await,
            capacity: self.capacity,
            expires: self.expires,
        }
    }
}
impl<T: MeshTransport> fmt::Debug for PendingCheckpointDiscovery<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingCheckpointDiscovery { .. }")
    }
}
impl fmt::Debug for CompletedCheckpointDiscovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedCheckpointDiscovery { .. }")
    }
}

struct Context {
    instance: RegistrySyncInstance,
    query: ScopedQuery,
    selection: Arc<()>,
    attempt: Arc<()>,
    peer: PeerId,
    provider: DeviceId,
    requester: Vec<u8>,
    group: Vec<u8>,
    inner: Vec<u8>,
    auth: RequestAuth,
    expires: u64,
}
pub struct PendingCheckpointSeed<T: MeshTransport> {
    transport: Arc<T>,
    clock: Arc<dyn Clock + Send>,
    context: Context,
    request: Vec<u8>,
    outbound: Arc<()>,
    retained: Arc<()>,
}
pub struct CompletedCheckpointSeed {
    context: Context,
    response: Result<Bytes, TransportError>,
    _outbound: Arc<()>,
    _retained: Arc<()>,
}
impl<T: MeshTransport> fmt::Debug for PendingCheckpointSeed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingCheckpointSeed { .. }")
    }
}
impl fmt::Debug for CompletedCheckpointSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedCheckpointSeed { .. }")
    }
}
impl<T: MeshTransport> PendingCheckpointSeed<T> {
    pub async fn fetch(self) -> CompletedCheckpointSeed {
        let (signal, cancelled) = tokio::sync::watch::channel(false);
        let _cancel = CancelOnDrop(signal);
        let response = if self.clock.monotonic_ms() >= self.context.expires {
            Err(TransportError::Unreachable(self.context.peer))
        } else {
            // Retained seed capacity must follow the lower stream too: cancelling this job
            // and dropping the pass does not mean the transport released its response buffer.
            let keepalive = Arc::new((self.outbound.clone(), self.retained.clone()));
            let cancellation = RequestCancellation::new(cancelled, Some(keepalive));
            tokio::select! {
                biased;
                _ = self.clock.sleep(std::time::Duration::from_millis(self.context.expires.saturating_sub(self.clock.monotonic_ms()))) => Err(TransportError::Unreachable(self.context.peer)),
                response = self.transport.request_connected_cancellable(self.context.peer, ProtocolId(RR_PROTOCOL), Bytes::from(self.request), cancellation) => response,
            }
        };
        CompletedCheckpointSeed {
            context: self.context,
            response,
            _outbound: self.outbound,
            _retained: self.retained,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Reserve the existing four retained slots before requesting a head, even for a new key.
    /// No raw receipt, public answer, caller nonce or pointer can manufacture this private pass.
    pub fn prepare_checkpoint_discovery(
        &mut self,
        peer: PeerId,
        target: CheckpointTarget,
    ) -> Result<PendingCheckpointDiscovery<T>, SyncError> {
        let capacity = self.registry_seeds.reserve_retained(false)?;
        let expires = self
            .clock
            .monotonic_ms()
            .checked_add(FETCH_MS)
            .ok_or(SyncError::Malformed)?;
        let head = self
            .prepare_checkpoint_head(peer, target)?
            .retaining(capacity.clone());
        Ok(PendingCheckpointDiscovery {
            head,
            capacity,
            expires,
        })
    }
    pub fn complete_checkpoint_discovery(
        &mut self,
        completed: CompletedCheckpointDiscovery,
    ) -> Result<Option<RegistrySeedDiscovery>, SyncError> {
        if self.clock.monotonic_ms() >= completed.expires {
            return Err(SyncError::Unauthorized);
        }
        let Some((answer, selection)) = self.complete_checkpoint_head_scoped(completed.head)?
        else {
            return Ok(None);
        };
        Ok(Some(match selection {
            None => RegistrySeedDiscovery::Hint(answer),
            Some(selection) => RegistrySeedDiscovery::Selected(RegistrySeedFetch {
                selection,
                _capacity: completed.capacity,
                expires: completed.expires,
                attempts: 0,
                next_at: 0,
                seed: None,
                attempt: None,
            }),
        }))
    }
    /// None means this exact pass already has its verified seed. A prepared attempt spends its
    /// retry even if never polled. New preparation supersedes any older result for this pass.
    pub fn prepare_checkpoint_seed(
        &mut self,
        pass: &mut RegistrySeedFetch,
        peer: PeerId,
    ) -> Result<Option<PendingCheckpointSeed<T>>, SyncError> {
        let now = self.clock.monotonic_ms();
        if !self.registry_seed_fetch_is_current(pass) {
            return Err(SyncError::Unauthorized);
        }
        if pass.seed.is_some() {
            return Ok(None);
        }
        if pass.attempts >= 3 || now < pass.next_at {
            return Err(SyncError::Malformed);
        }
        let provider = self
            .registry_page_peer_device(peer)
            .ok_or(SyncError::Unauthorized)?;
        let verified = &pass.selection.verified;
        let query = ScopedQuery {
            target: pass.selection.target,
            doc_id: epoch_id(
                pass.selection.target.doc_type(),
                &verified.document().logical_key,
                verified
                    .closed_epoch()
                    .checked_add(1)
                    .ok_or(SyncError::Malformed)?,
                &verified.close_record_hash(),
            ),
            hash: verified.seed_change_hash(),
        };
        let inner = encode_scoped_query(&query, &self.group.group_id())?;
        let slot = self
            .registry_seeds
            .outbound
            .iter_mut()
            .find(|s| s.strong_count() == 0)
            .ok_or(SyncError::Malformed)?;
        let outbound = Arc::new(());
        *slot = Arc::downgrade(&outbound);
        pass.attempts += 1;
        pass.next_at = now.checked_add(1000).ok_or(SyncError::Malformed)?;
        let (request, auth) = self.build_authed_request(query.target.seed_kind(), &inner)?;
        let expires = now
            .checked_add(REQUEST_MS)
            .ok_or(SyncError::Malformed)?
            .min(pass.expires);
        let attempt = Arc::new(());
        pass.attempt = Some(attempt.clone());
        Ok(Some(PendingCheckpointSeed {
            transport: self.transport.clone(),
            clock: self.clock.clone(),
            request,
            outbound,
            retained: pass._capacity.clone(),
            context: Context {
                instance: self.registry_instance(),
                query,
                selection: pass.selection.generation.clone(),
                attempt,
                peer,
                provider,
                requester: self.device.public_key_bytes(),
                group: self.group.group_id(),
                inner,
                auth,
                expires,
            },
        }))
    }
    pub fn complete_checkpoint_seed(
        &mut self,
        pass: &mut RegistrySeedFetch,
        completed: CompletedCheckpointSeed,
    ) -> Result<bool, SyncError> {
        let c = completed.context;
        if !self.registry_seed_fetch_is_current(pass)
            || !self.matches_registry_instance(&c.instance)
            || self.group.group_id() != c.group
            || self.group.epoch() != c.auth.epoch
            || self.device.public_key_bytes() != c.requester
            || self.registry_page_peer_device(c.peer) != Some(c.provider)
            || self.clock.monotonic_ms() >= c.expires
            || pass.selection.target != c.query.target
            || !Arc::ptr_eq(&pass.selection.generation, &c.selection)
            || !pass
                .attempt
                .as_ref()
                .is_some_and(|g| Arc::ptr_eq(g, &c.attempt))
        {
            return Err(SyncError::Unauthorized);
        }
        let response = completed.response?;
        if response.is_empty() {
            return Ok(false);
        }
        let (key, signature, body) = decode_response(&response)?;
        if DeviceId::from_public_key_bytes(key) != c.provider
            || !self.seed_member(key)
            || !verify_with_public_bytes(
                key,
                &scoped_transcript(
                    c.query.target.seed_domain(),
                    &c.group,
                    &c.requester,
                    &c.auth,
                    c.peer,
                    &c.inner,
                    body,
                ),
                &signature,
            )
        {
            return Err(SyncError::Unauthorized);
        }
        if body.is_empty() {
            return Ok(false);
        }
        let key =
            self.group
                .channel_secret(&self.device, c.query.target.doc_type(), c.query.doc_id)?;
        let raw = open_seed(body, &key)?;
        // Exact receipted Automerge hash before decode, then typed root/channel/schema checks.
        let seed = c.query.target.verify_seed(&pass.selection.verified, &raw)?;
        if !self.registry_seed_fetch_is_current(pass) || self.clock.monotonic_ms() >= c.expires {
            return Err(SyncError::Unauthorized);
        }
        pass.seed = Some(seed);
        Ok(true)
    }
}
