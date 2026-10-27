//! Shared bounded transport and member authentication. The caller supplies its separate
//! authority or provisional provenance and keeps the retained slot through typed parsing.
use super::*;

pub(super) struct SeedTransferContext {
    pub instance: RegistrySyncInstance,
    pub query: ScopedQuery,
    pub peer: PeerId,
    pub provider: DeviceId,
    pub requester: Vec<u8>,
    pub group: Vec<u8>,
    pub inner: Vec<u8>,
    pub auth: RequestAuth,
    pub expires: u64,
}
pub(super) struct PendingSeedTransfer<T: MeshTransport> {
    pub transport: Arc<T>,
    pub clock: Arc<dyn Clock + Send>,
    pub context: SeedTransferContext,
    pub request: Vec<u8>,
    pub outbound: Arc<()>,
    pub retained: Arc<()>,
}
pub(super) struct CompletedSeedTransfer {
    pub context: SeedTransferContext,
    pub response: Result<Bytes, TransportError>,
    _outbound: Arc<()>,
    _retained: Arc<()>,
}
impl<T: MeshTransport> PendingSeedTransfer<T> {
    pub(super) async fn fetch(self) -> CompletedSeedTransfer {
        let (signal, cancelled) = tokio::sync::watch::channel(false);
        let _cancel = CancelOnDrop(signal);
        let response = if self.clock.monotonic_ms() >= self.context.expires {
            Err(TransportError::Unreachable(self.context.peer))
        } else {
            // Cancellation revokes a result, but the lower stream still owns its buffers.
            let keepalive = Arc::new((self.outbound.clone(), self.retained.clone()));
            let cancellation = RequestCancellation::new(cancelled, Some(keepalive));
            tokio::select! {
                biased;
                _ = self.clock.sleep(std::time::Duration::from_millis(self.context.expires.saturating_sub(self.clock.monotonic_ms()))) => Err(TransportError::Unreachable(self.context.peer)),
                response = self.transport.request_connected_cancellable(self.context.peer, ProtocolId(RR_PROTOCOL), Bytes::from(self.request), cancellation) => response,
            }
        };
        CompletedSeedTransfer {
            context: self.context,
            response,
            _outbound: self.outbound,
            _retained: self.retained,
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub(super) fn reserve_seed_outbound(&mut self) -> Result<Arc<()>, SyncError> {
        let slot = self
            .registry_seeds
            .outbound
            .iter_mut()
            .find(|s| s.strong_count() == 0)
            .ok_or(SyncError::Malformed)?;
        let outbound = Arc::new(());
        *slot = Arc::downgrade(&outbound);
        Ok(outbound)
    }
    pub(super) fn complete_seed_transfer(
        &self,
        completed: CompletedSeedTransfer,
    ) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, SyncError> {
        let c = &completed.context;
        if !self.matches_registry_instance(&c.instance)
            || self.group.group_id() != c.group
            || self.group.epoch() != c.auth.epoch
            || self.device.public_key_bytes() != c.requester
            || self.registry_page_peer_device(c.peer) != Some(c.provider)
            || self.clock.monotonic_ms() >= c.expires
        {
            return Err(SyncError::Unauthorized);
        }
        let response = completed.response?;
        if response.is_empty() {
            return Ok(None);
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
            return Ok(None);
        }
        let key =
            self.group
                .channel_secret(&self.device, c.query.target.doc_type(), c.query.doc_id)?;
        let raw = open_seed(body, &key)?;
        if self.clock.monotonic_ms() >= c.expires {
            return Err(SyncError::Unauthorized);
        }
        Ok(Some(raw))
    }
}
