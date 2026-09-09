//! Studio's typed entry points into the existing page transport. Network suspension owns no
//! Server, MLS keys or vault custody, so two reconnecting actors can still serve each other.
use super::*;
use crate::studio_exchange::StudioWatch;

/// Repeat the original heads/seed for every continuation. Requester identity is never supplied
/// by the caller; preparation signs with the current device and pins its proven endpoint peer.
pub struct StudioPageQuery<'a> {
    pub target: StudioTarget,
    pub doc_id: u128,
    pub heads: &'a [[u8; 32]],
    pub seed: Option<[u8; 32]>,
    pub cursor: Option<&'a [u8]>,
}
impl fmt::Debug for StudioPageQuery<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StudioPageQuery")
            .field("heads", &self.heads.len())
            .field("continuation", &self.cursor.is_some())
            .finish_non_exhaustive()
    }
}

/// Shares Registry's four receive slots; replacement revokes authority, not retained capacity.
pub struct StudioReceivePermit {
    watch: StudioWatch,
    _capacity: Arc<()>,
}
impl StudioReceivePermit {
    pub fn doc_id(&self) -> u128 {
        self.watch.doc_id
    }
}
impl fmt::Debug for StudioReceivePermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StudioReceivePermit { .. }")
    }
}

struct Context {
    instance: RegistrySyncInstance,
    watch: StudioWatch,
    peer: PeerId,
    provider: DeviceId,
    requester: Vec<u8>,
    group: Vec<u8>,
    query: Vec<u8>,
    auth: RequestAuth,
    expires: u64,
}

/// One non-cloneable signed attempt. Capacity follows both the lower transport and the eventual
/// result; dropping an unpolled/cancelled future cannot refund a still-live lower stream.
pub struct PendingStudioPage<T: MeshTransport> {
    transport: Arc<T>,
    clock: Arc<dyn Clock + Send>,
    context: Context,
    request: Vec<u8>,
    capacity: Arc<()>,
}
pub struct CompletedStudioPage {
    context: Context,
    response: Result<Bytes, TransportError>,
    _capacity: Arc<()>,
}
impl<T: MeshTransport> fmt::Debug for PendingStudioPage<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingStudioPage { .. }")
    }
}
impl fmt::Debug for CompletedStudioPage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedStudioPage { .. }")
    }
}
impl<T: MeshTransport> PendingStudioPage<T> {
    /// Connected-only I/O; no automatic dialing. The fixed deadline includes time queued before
    /// polling and is checked again on completion, even if a response wins the timer race.
    pub async fn fetch(self) -> CompletedStudioPage {
        let (signal, cancelled) = tokio::sync::watch::channel(false);
        let _cancel = CancelOnDrop(signal);
        let response = if self.clock.monotonic_ms() >= self.context.expires {
            Err(TransportError::Unreachable(self.context.peer))
        } else {
            let cancellation = RequestCancellation::new(cancelled, Some(self.capacity.clone()));
            tokio::select! {
                biased;
                _ = self.clock.sleep(std::time::Duration::from_millis(self.context.expires.saturating_sub(self.clock.monotonic_ms()))) => Err(TransportError::Unreachable(self.context.peer)),
                response = self.transport.request_connected_cancellable(self.context.peer, ProtocolId(RR_PROTOCOL), Bytes::from(self.request), cancellation) => response,
            }
        };
        CompletedStudioPage {
            context: self.context,
            response,
            _capacity: self.capacity,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub fn begin_studio_receive(
        &mut self,
        watch: &StudioWatch,
    ) -> Result<StudioReceivePermit, SyncError> {
        if !self.studio_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        let slot = self
            .registry_pages
            .receivers
            .iter_mut()
            .find(|s| s.strong_count() == 0)
            .ok_or(SyncError::Malformed)?;
        let capacity = Arc::new(());
        *slot = Arc::downgrade(&capacity);
        Ok(StudioReceivePermit {
            watch: watch.copy_binding(),
            _capacity: capacity,
        })
    }
    pub fn studio_receive_is_current(&self, permit: &StudioReceivePermit) -> bool {
        self.studio_watch_is_current(&permit.watch)
    }
    pub fn prepare_studio_receive_page(
        &mut self,
        permit: &StudioReceivePermit,
        peer: PeerId,
        query: StudioPageQuery<'_>,
    ) -> Result<PendingStudioPage<T>, SyncError> {
        self.prepare_studio_page(&permit.watch, peer, query)
    }
    /// Small scheduling hint, not a source or membership authorization.
    pub fn studio_has_page_request(&self, watch: &StudioWatch) -> bool {
        self.studio_watch_is_current(watch)
            && self.registry_pages.pending.iter().any(|p| {
                p.query.scope == PageScope::Studio(watch.target)
                    && Arc::ptr_eq(&p.generation, &watch.generation)
            })
    }
    /// At most four already proven, currently connected members. Never disclose a Studio key
    /// to an unproven candidate while hoping that its answer authenticates it afterward.
    pub fn studio_page_peers(&self) -> Vec<PeerId> {
        let live: HashSet<_> = self
            .transport
            .connection_snapshot()
            .into_iter()
            .map(|c| c.peer)
            .collect();
        self.member_peers
            .iter()
            .rev()
            .filter(|p| p.bound && live.contains(&p.peer) && self.group.contains_device(&p.device))
            .map(|p| p.peer)
            .take(4)
            .collect()
    }
    pub fn serve_studio_request<E>(
        &mut self,
        watch: &StudioWatch,
        serve: impl FnOnce(
            &ServerGroup,
            &MlsDevice,
            &mut R,
            RegistryPageRequest<'_>,
        ) -> Result<RegistryPageOutcome, E>,
    ) -> Result<Option<Result<(), E>>, SyncError> {
        if !self.studio_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        self.serve_epoch_request(
            PageScope::Studio(watch.target),
            watch.doc_id,
            &watch.generation,
            serve,
        )
    }
    /// Prepare under exclusive sync ownership, then release actor/native/vault custody before
    /// awaiting fetch. Completion must return to this exact runtime and watch generation.
    pub fn prepare_studio_page(
        &mut self,
        watch: &StudioWatch,
        peer: PeerId,
        query: StudioPageQuery<'_>,
    ) -> Result<PendingStudioPage<T>, SyncError> {
        if !self.studio_watch_is_current(watch)
            || watch.target != query.target
            || watch.doc_id != query.doc_id
            || !self.registry_page_member(&self.device.public_key_bytes())
        {
            return Err(SyncError::Unauthorized);
        }
        let provider = self
            .registry_page_peer_device(peer)
            .ok_or(SyncError::Unauthorized)?;
        let scope = PageScope::Studio(query.target);
        let inner =
            encode_scoped_query(scope, query.doc_id, query.heads, query.seed, query.cursor)?;
        let (request, auth) = self.build_authed_request(scope.kind(), &inner)?;
        let slot = self
            .registry_pages
            .outbound
            .iter_mut()
            .find(|s| s.strong_count() == 0)
            .ok_or(SyncError::Malformed)?;
        let capacity = Arc::new(());
        *slot = Arc::downgrade(&capacity);
        let expires = self
            .clock
            .monotonic_ms()
            .checked_add(REQUEST_MS)
            .ok_or(SyncError::Malformed)?;
        Ok(PendingStudioPage {
            transport: self.transport.clone(),
            clock: self.clock.clone(),
            request,
            capacity,
            context: Context {
                instance: self.registry_instance(),
                watch: watch.copy_binding(),
                peer,
                provider,
                requester: self.device.public_key_bytes(),
                group: self.group.group_id(),
                query: inner,
                auth,
                expires,
            },
        })
    }
    /// Authenticate a retained response without touching storage. The caller still must gate
    /// and atomically persist every returned operation before following its opaque cursor.
    pub fn complete_studio_page(
        &mut self,
        completed: CompletedStudioPage,
    ) -> Result<Option<RegistryPageOutcome>, SyncError> {
        let c = completed.context;
        if !self.matches_registry_instance(&c.instance)
            || !self.studio_watch_is_current(&c.watch)
            || self.group.group_id() != c.group
            || self.group.epoch() != c.auth.epoch
            || self.device.public_key_bytes() != c.requester
            || !self.registry_page_member(&c.requester)
            || self.registry_page_peer_device(c.peer) != Some(c.provider)
            || self.clock.monotonic_ms() >= c.expires
        {
            return Err(SyncError::Unauthorized);
        }
        let response = completed.response?;
        if response.is_empty() {
            return Ok(None);
        }
        let (key, signature, answer) = decode_response(&response)?;
        if DeviceId::from_public_key_bytes(key) != c.provider || !self.registry_page_member(key) {
            return Err(SyncError::Unauthorized);
        }
        let scope = PageScope::Studio(c.watch.target);
        let transcript = scoped_response_transcript(
            scope.domain(),
            &c.group,
            &c.requester,
            &c.auth,
            c.peer,
            &c.query,
            answer,
        );
        if !verify_with_public_bytes(key, &transcript, &signature) {
            return Err(SyncError::Unauthorized);
        }
        Ok(Some(decode_scoped_answer(
            answer,
            scope.doc_type(),
            c.watch.doc_id,
            c.auth.epoch,
        )?))
    }
}
