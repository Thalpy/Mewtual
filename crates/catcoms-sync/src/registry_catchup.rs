//! Authenticated, bounded P1 registry page exchange. Networking owns only small pending
//! requests; a cooperative synchronous callback reads the durable source under the app's gate.
//! Returned pages are transport claims, NOT admitted operations or proof of document currency.
use super::*;
use catcoms_replication::epoch::MAX_SIGNED_EPOCH_OP_BYTES;
use catcoms_replication::registry_epoch::catchup::{
    RegistryOpPage, RegistryPageCursor, RegistryPageOutcome, RegistryPageRequest,
    MAX_REGISTRY_PAGE_BYTES, MAX_REGISTRY_PAGE_HEADS, MAX_REGISTRY_PAGE_OPS,
};
use catcoms_rt::Responder;
use registry_ingress::Rate;

mod wire;
pub use wire::RegistryPageQuery;
use wire::*;

const MAX_PENDING: usize = 8;
const MAX_REQUESTERS: usize = 4096;
const QUEUE_TTL_MS: u64 = 5_000;
const REQUEST_MS: u64 = 10_000;
// Auth framing adds 144 bytes; cap before decode_authed_request makes any Vec copies.
const MAX_REQUEST: usize = MAX_QUERY + 144;
const RESPONSE_DOMAIN: &str = "catcoms/registry-page-response/v1";

struct Pending {
    query: OwnedQuery,
    inner: Vec<u8>,
    requester: DeviceId,
    key: Vec<u8>,
    auth: RequestAuth,
    generation: Arc<()>,
    expires: u64,
    responder: Responder,
}

#[derive(Default)]
pub(super) struct RegistryRequests {
    pending: VecDeque<Pending>,
    preauth: Option<Rate>,
    service: Option<Rate>,
    requesters: BTreeMap<DeviceId, Rate>,
    now: u64,
    // Weak slots follow the lifetime of transport-owned accounting, not the caller future.
    outbound: [std::sync::Weak<()>; 4],
}

struct CancelOnDrop(tokio::sync::watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
impl RegistryRequests {
    pub(super) fn drop_bucket(&mut self, bucket: u8) {
        self.pending.retain(|item| item.query.bucket != bucket);
    }
    fn expire(&mut self, now: u64) -> u64 {
        self.now = self.now.max(now);
        self.pending.retain(|item| self.now < item.expires);
        self.now
    }
}

fn response_transcript(
    group: &[u8],
    requester: &[u8],
    auth: &RequestAuth,
    provider_peer: PeerId,
    query: &[u8],
    answer: &[u8],
) -> Vec<u8> {
    let mut bound = Encoder::new();
    bound
        .put_bytes(provider_peer.as_bytes())
        .expect("peer fits");
    bound.put_bytes(query).expect("bounded query");
    bound.put_bytes(answer).expect("bounded answer");
    signed_resp_transcript(
        RESPONSE_DOMAIN,
        group,
        requester,
        auth.ts,
        &auth.nonce,
        auth.epoch,
        bound.as_bytes(),
    )
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    fn registry_page_member(&self, key: &[u8]) -> bool {
        self.group
            .member_signature_key(&DeviceId::from_public_key_bytes(key))
            .as_deref()
            == Some(key)
    }

    /// Enqueue only for an exact locally installed watch. All rejection paths drop the reply
    /// handle: no unsigned "empty document" or legacy history response is fabricated.
    pub(super) fn queue_registry_page_request(
        &mut self,
        from: PeerId,
        data: &[u8],
        responder: Responder,
    ) {
        let now = self.registry_pages.expire(self.clock.monotonic_ms());
        if data.len() > MAX_REQUEST
            || self.registry_ingress.watches.is_empty()
            || self.registry_pages.pending.len() >= MAX_PENDING
            || !self
                .registry_pages
                .preauth
                .get_or_insert_with(|| Rate::full(now, 20))
                .charge(now, 10, 20)
        {
            return;
        }
        let Some((inner, key, auth)) = self.authenticate_request(KIND_REGISTRY_PAGE, data, from)
        else {
            return;
        };
        if auth.epoch != self.group.epoch()
            || !self.registry_page_member(&key)
            || !self.registry_page_member(&self.device.public_key_bytes())
        {
            return;
        }
        let Ok(query) = decode_query(&inner) else {
            return;
        };
        let Some(watch) = self.registry_ingress.watches.get(&query.bucket) else {
            return;
        };
        if watch.doc_id != query.doc_id {
            return;
        }
        let generation = watch.generation.clone();
        let requester = DeviceId::from_public_key_bytes(&key);
        // One queued request/full identity across ALL buckets. Watch replacement cannot clear
        // identity debt. Only completely refilled rows may be evicted; Sybils hit the global rails.
        if self
            .registry_pages
            .pending
            .iter()
            .any(|item| item.requester == requester)
        {
            return;
        }
        let rates = &mut self.registry_pages.requesters;
        rates.retain(|_, rate| {
            rate.refill(now, 1, 2);
            !rate.is_full(2)
        });
        if !rates.contains_key(&requester) && rates.len() >= MAX_REQUESTERS {
            return;
        }
        if !rates
            .entry(requester)
            .or_insert_with(|| Rate::full(now, 2))
            .charge(now, 1, 2)
        {
            return;
        }
        let Some(expires) = now.checked_add(QUEUE_TTL_MS) else {
            return;
        };
        self.registry_pages.pending.push_back(Pending {
            query,
            inner,
            requester,
            key,
            auth,
            generation,
            expires,
            responder,
        });
    }

    /// Serve at most one queued request for an exact watch. Re-authenticate current authority,
    /// epoch and freshness BEFORE the source callback. Global service debt bounds costly vault
    /// rebuilds at 2/sec (burst 4), independently of requester count. No await/cancellation gap
    /// exists between source read and response signing; errors consume the volatile request.
    /// The callback must be a trusted bounded durable-source adapter, not UI-provided data.
    /// Nested result distinguishes a source error from a sync/framing error; neither sends success.
    pub fn serve_registry_request<E>(
        &mut self,
        watch: &RegistryWatch,
        serve: impl FnOnce(
            &ServerGroup,
            &MlsDevice,
            &mut R,
            RegistryPageRequest<'_>,
        ) -> Result<RegistryPageOutcome, E>,
    ) -> Result<Option<Result<(), E>>, SyncError> {
        if !self.registry_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        let now = self.registry_pages.expire(self.clock.monotonic_ms());
        let Some(index) = self
            .registry_pages
            .pending
            .iter()
            .position(|item| item.query.bucket == watch.bucket)
        else {
            return Ok(None);
        };
        // Leave bounded ownership on self when a service token is unavailable.
        if !self
            .registry_pages
            .service
            .get_or_insert_with(|| Rate::full(now, 4))
            .charge(now, 2, 4)
        {
            return Ok(None);
        }
        let item = self
            .registry_pages
            .pending
            .remove(index)
            .expect("located request");
        if !Arc::ptr_eq(&item.generation, &watch.generation)
            || item.query.doc_id != watch.doc_id
            || item.auth.epoch != self.group.epoch()
            || self.clock.now_ms().abs_diff(item.auth.ts) > MAX_REQUEST_AGE_MS
            || !self.registry_page_member(&item.key)
            || !self.registry_page_member(&self.device.public_key_bytes())
        {
            return Err(SyncError::Unauthorized);
        }
        let outcome = match serve(
            &self.group,
            &self.device,
            &mut self.rng,
            item.query.request(item.requester),
        ) {
            Ok(outcome) => outcome,
            Err(error) => return Ok(Some(Err(error))),
        };
        let answer = encode_answer(&outcome, watch.doc_id, self.group.epoch())?;
        let transcript = response_transcript(
            &self.group.group_id(),
            &item.key,
            &item.auth,
            self.transport.local_peer(),
            &item.inner,
            &answer,
        );
        let signature = self.device.sign(&transcript)?;
        item.responder
            .respond(Bytes::from(encode_signed_commit_resp(
                &self.device.public_key_bytes(),
                &signature,
                &answer,
            )));
        Ok(Some(Ok(())))
    }

    /// Request one page, without admitting operations or advancing a durable cursor. `None`
    /// means unsupported/refused, never "empty" and NEVER permission for legacy full catch-up.
    /// A caller must persist each operation through its gate before accepting `next`. An outer
    /// signature authenticates the provider's claim, not each operation's author or projection.
    pub async fn request_registry_page(
        &mut self,
        peer: PeerId,
        query: RegistryPageQuery<'_>,
    ) -> Result<Option<RegistryPageOutcome>, SyncError> {
        if !self.registry_page_member(&self.device.public_key_bytes()) {
            return Err(SyncError::Unauthorized);
        }
        // Registry identifiers/heads are private metadata. Do not send them to a candidate and
        // hope its reply proves membership afterward. Bootstrap a bound proof separately.
        let expected = self
            .member_peers
            .iter()
            .find(|proof| {
                proof.peer == peer && proof.bound && self.group.contains_device(&proof.device)
            })
            .map(|proof| proof.device)
            .ok_or(SyncError::Unauthorized)?;
        let inner = encode_query(&query)?;
        let (request, auth) = self.build_authed_request(KIND_REGISTRY_PAGE, &inner)?;
        let slot = self
            .registry_pages
            .outbound
            .iter_mut()
            .find(|slot| slot.strong_count() == 0)
            .ok_or(SyncError::Transport(TransportError::Unreachable(peer)))?;
        let permit = Arc::new(());
        *slot = Arc::downgrade(&permit);
        let (signal, cancelled) = tokio::sync::watch::channel(false);
        let _cancel = CancelOnDrop(signal);
        // Once queued, the driver retains the permit until it REALLY releases its stream. A
        // cancelled caller can neither lose the cancellation signal nor recycle that capacity.
        let cancellation = RequestCancellation::new(cancelled, Some(permit));
        let response = {
            let answer = self.transport.request_cancellable(
                peer,
                ProtocolId(RR_PROTOCOL),
                Bytes::from(request),
                cancellation,
            );
            let deadline = self
                .clock
                .sleep(std::time::Duration::from_millis(REQUEST_MS));
            futures::pin_mut!(answer, deadline);
            match futures::future::select(answer, deadline).await {
                futures::future::Either::Left((answer, _)) => answer?,
                futures::future::Either::Right(_) => {
                    return Err(SyncError::Transport(TransportError::Unreachable(peer)))
                }
            }
        };
        if response.is_empty() {
            return Ok(None);
        }
        let (key, signature, answer) = decode_response(&response)?;
        if auth.epoch != self.group.epoch()
            || DeviceId::from_public_key_bytes(key) != expected
            || !self.registry_page_member(key)
            || !self.registry_page_member(&self.device.public_key_bytes())
        {
            return Err(SyncError::Unauthorized);
        }
        let transcript = response_transcript(
            &self.group.group_id(),
            &self.device.public_key_bytes(),
            &auth,
            peer,
            &inner,
            answer,
        );
        if !verify_with_public_bytes(key, &transcript, &signature) {
            return Err(SyncError::Unauthorized);
        }
        let outcome = decode_answer(answer, query.doc_id, auth.epoch)?;
        self.promote_member_peer_bound(peer, DeviceId::from_public_key_bytes(key), true);
        Ok(Some(outcome))
    }
}

#[cfg(test)]
mod tests;
