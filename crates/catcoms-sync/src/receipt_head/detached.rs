//! Splitting prepare/I/O/complete leaves the actor free to answer a simultaneous peer request.
//! All cryptographic and selection authority remains in the exclusive synchronous phases.
use super::*;

struct Context {
    instance: RegistrySyncInstance,
    target: CheckpointTarget,
    attempt: Arc<()>,
    peer: PeerId,
    provider: DeviceId,
    requester: Vec<u8>,
    group: Vec<u8>,
    query: Vec<u8>,
    nonce: [u8; 16],
    auth: RequestAuth,
    expires: u64,
}

/// Authenticated member delivery only. The candidate receipt has been decoded, not verified
/// as current or historical owner authority. Only provisional discovery may retain this type.
pub(crate) struct AuthenticatedCheckpointHint {
    context: Context,
    receipt: Receipt,
}
impl AuthenticatedCheckpointHint {
    pub(crate) fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub(crate) fn peer(&self) -> PeerId {
        self.context.peer
    }
    pub(crate) fn provider(&self) -> DeviceId {
        self.context.provider
    }
}

/// One signed, non-cloneable request, containing no Server/MLS/vault access. Four slots are
/// shared across Studio and Registry; capacity follows unpolled jobs, completions and the driver.
pub struct PendingCheckpointHead<T: MeshTransport> {
    transport: Arc<T>,
    clock: Arc<dyn Clock + Send>,
    context: Context,
    request: Vec<u8>,
    capacity: Arc<()>,
    retained: Option<Arc<()>>,
}
pub struct CompletedCheckpointHead {
    context: Context,
    response: Result<Bytes, TransportError>,
    _capacity: Arc<()>,
    _retained: Option<Arc<()>>,
}
impl<T: MeshTransport> fmt::Debug for PendingCheckpointHead<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingCheckpointHead { .. }")
    }
}
impl fmt::Debug for CompletedCheckpointHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedCheckpointHead { .. }")
    }
}
impl<T: MeshTransport> PendingCheckpointHead<T> {
    pub(crate) fn retaining(mut self, capacity: Arc<()>) -> Self {
        self.retained = Some(capacity);
        self
    }
    /// No dialing from a logical key. The receiver-clock deadline starts at preparation, not
    /// polling; cancellation keeps the lower stream's capacity until it actually releases it.
    pub async fn fetch(self) -> CompletedCheckpointHead {
        let (signal, cancelled) = tokio::sync::watch::channel(false);
        let _cancel = CancelOnDrop(signal);
        let response = if self.clock.monotonic_ms() >= self.context.expires {
            Err(TransportError::Unreachable(self.context.peer))
        } else {
            let keepalive = Arc::new((self.capacity.clone(), self.retained.clone()));
            let cancellation = RequestCancellation::new(cancelled, Some(keepalive));
            tokio::select! {
                biased;
                _ = self.clock.sleep(std::time::Duration::from_millis(self.context.expires.saturating_sub(self.clock.monotonic_ms()))) => Err(TransportError::Unreachable(self.context.peer)),
                response = self.transport.request_connected_cancellable(self.context.peer, ProtocolId(RR_PROTOCOL), Bytes::from(self.request), cancellation) => response,
            }
        };
        CompletedCheckpointHead {
            context: self.context,
            response,
            _capacity: self.capacity,
            _retained: self.retained,
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub fn prepare_checkpoint_head(
        &mut self,
        peer: PeerId,
        target: CheckpointTarget,
    ) -> Result<PendingCheckpointHead<T>, SyncError> {
        if !self.head_member(&self.device.public_key_bytes()) {
            return Err(SyncError::Unauthorized);
        }
        let provider = self
            .registry_page_peer_device(peer)
            .ok_or(SyncError::Unauthorized)?;
        let mut nonce = [0; 16];
        self.rng.fill_bytes(&mut nonce);
        let inner = encode_scoped_query(target, &self.group.group_id(), nonce)?;
        let (request, auth) = self.build_authed_request(target.head_kind(), &inner)?;
        let slot = self
            .receipt_heads
            .outbound
            .iter_mut()
            .find(|s| s.strong_count() == 0)
            .ok_or(SyncError::Transport(TransportError::Unreachable(peer)))?;
        let capacity = Arc::new(());
        *slot = Arc::downgrade(&capacity);
        let expires = self
            .clock
            .monotonic_ms()
            .checked_add(REQUEST_MS)
            .ok_or(SyncError::Malformed)?;
        let attempt = Arc::new(());
        // Weak maps cannot grow with the number of arbitrary object keys requested over time.
        // Only the four in-flight heads and four retained seed selections keep entries alive.
        self.receipt_heads
            .attempts
            .retain(|_, g| g.strong_count() != 0);
        self.receipt_heads
            .selections
            .retain(|_, g| g.strong_count() != 0);
        self.receipt_heads
            .attempts
            .insert(target, Arc::downgrade(&attempt));
        Ok(PendingCheckpointHead {
            transport: self.transport.clone(),
            clock: self.clock.clone(),
            request,
            capacity,
            retained: None,
            context: Context {
                instance: self.registry_instance(),
                target,
                attempt,
                peer,
                provider,
                requester: self.device.public_key_bytes(),
                group: self.group.group_id(),
                query: inner,
                nonce,
                auth,
                expires,
            },
        })
    }
    /// Raw answer fields remain hints to callers. Only the seed-discovery wrapper retains the
    /// private current-owner selection minted by this same verification path.
    pub fn complete_checkpoint_head(
        &mut self,
        completed: CompletedCheckpointHead,
    ) -> Result<Option<ReceiptHeadAnswer>, SyncError> {
        Ok(self
            .complete_checkpoint_head_scoped(completed)?
            .map(|(answer, _)| answer))
    }
    pub(crate) fn complete_checkpoint_head_scoped(
        &mut self,
        completed: CompletedCheckpointHead,
    ) -> Result<Option<(ReceiptHeadAnswer, Option<HeadSelection>)>, SyncError> {
        let Some((c, outcome)) = self.authenticate_checkpoint_head(completed)? else {
            return Ok(None);
        };
        let selection = if let Some(proof) = &outcome.proof {
            if self.group.designated_committer() != Some(c.provider)
                || self
                    .observed_owner_tenure_start()
                    .is_some_and(|t| t != proof.tenure_start_group_epoch)
            {
                return Err(SyncError::Unauthorized);
            }
            let verified = proof.verify(
                &self.group,
                outcome.receipt.as_ref().ok_or(SyncError::Malformed)?,
                self.device.device_id(),
                &c.nonce,
            )?;
            let generation = Arc::new(());
            self.receipt_heads
                .selections
                .retain(|_, g| g.strong_count() != 0);
            self.receipt_heads
                .selections
                .insert(c.target, Arc::downgrade(&generation));
            Some(HeadSelection {
                instance: self.registry_instance(),
                epoch: self.group.epoch(),
                owner: c.provider,
                requester: self.device.device_id(),
                generation,
                target: c.target,
                receipt: outcome
                    .receipt
                    .as_ref()
                    .expect("verified proof receipt")
                    .clone(),
                verified,
                tenure: proof.tenure_start_group_epoch,
            })
        } else {
            None
        };
        // Fresh proof is a one-shot selection, never a lease or a pruning grant.
        Ok(Some((outcome, selection)))
    }
    /// Does not mint or supersede an owner selection, even if a proof was supplied. Owner-proof
    /// and repair answers need the normal authoritative path, with a fresh discovery request.
    pub(crate) fn complete_checkpoint_hint(
        &mut self,
        completed: CompletedCheckpointHead,
    ) -> Result<Option<AuthenticatedCheckpointHint>, SyncError> {
        let Some((context, answer)) = self.authenticate_checkpoint_head(completed)? else {
            return Ok(None);
        };
        if answer.proof.is_some() || answer.repair.is_some() {
            return Ok(None);
        }
        Ok(answer
            .receipt
            .map(|receipt| AuthenticatedCheckpointHint { context, receipt }))
    }
    /// Rechecks provenance, not a lifetime extension. The provisional custodian must separately
    /// enforce its fixed receiver-clock lifetime and its copied Studio watch/mount bindings.
    pub(crate) fn checkpoint_hint_is_current(&self, hint: &AuthenticatedCheckpointHint) -> bool {
        self.head_context_is_current(&hint.context)
    }
    fn head_context_is_current(&self, c: &Context) -> bool {
        self.matches_registry_instance(&c.instance)
            && self.group.group_id() == c.group
            && self.group.epoch() == c.auth.epoch
            && self.device.public_key_bytes() == c.requester
            && self.head_member(&c.requester)
            && self.registry_page_peer_device(c.peer) == Some(c.provider)
            && self
                .receipt_heads
                .attempts
                .get(&c.target)
                .and_then(std::sync::Weak::upgrade)
                .is_some_and(|g| Arc::ptr_eq(&g, &c.attempt))
    }
    fn authenticate_checkpoint_head(
        &mut self,
        completed: CompletedCheckpointHead,
    ) -> Result<Option<(Context, ReceiptHeadAnswer)>, SyncError> {
        let c = completed.context;
        if !self.head_context_is_current(&c) || self.clock.monotonic_ms() >= c.expires {
            return Err(SyncError::Unauthorized);
        }
        let response = completed.response?;
        if response.is_empty() {
            return Ok(None);
        }
        let (key, signature, answer) = decode_response(&response)?;
        if DeviceId::from_public_key_bytes(key) != c.provider
            || !self.head_member(key)
            || !verify_with_public_bytes(
                key,
                &scoped_transcript(
                    c.target.head_domain(),
                    &c.group,
                    &c.requester,
                    &c.auth,
                    c.peer,
                    &c.query,
                    answer,
                ),
                &signature,
            )
        {
            return Err(SyncError::Unauthorized);
        }
        let outcome = decode_answer(answer, &c.target.document(&c.group)?)?;
        Ok(Some((c, outcome)))
    }
}
