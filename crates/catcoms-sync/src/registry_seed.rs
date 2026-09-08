//! Expected-hash checkpoint transport. Receiving owns one bounded seed under a fresh head
//! selection, not an installed epoch or a pruning grant. The durable recovery gate is separate.
use super::*;
use catcoms_replication::{
    epoch_id,
    registry::{registry_document, RegistryProjection},
    Receipt, VerifiedCheckpoint, MAX_CHECKPOINT_BYTES,
};
use catcoms_rt::Responder;
use catcoms_storage::pad::{self, OP_PAD_CEILING, OP_PAD_FLOOR};
use receipt_head::{HeadSelection, ReceiptHeadAnswer};
use registry_ingress::Rate;
mod wire;
use wire::*;

const MAX_PENDING: usize = 8;
const MAX_REQUESTERS: usize = 4096;
const QUEUE_MS: u64 = 5_000;
const REQUEST_MS: u64 = 10_000;
const FETCH_MS: u64 = 60_000;
const RESPONSE_DOMAIN: &str = "catcoms/registry-seed-response/v1";

/// Logical bucket registration. Dropping is not revocation: explicitly unwatch on vault lock.
pub struct RegistrySeedWatch {
    instance: RegistrySyncInstance,
    bucket: u8,
    generation: Arc<()>,
}
impl fmt::Debug for RegistrySeedWatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistrySeedWatch { .. }")
    }
}
struct Pending {
    query: Query,
    inner: Vec<u8>,
    generation: Arc<()>,
    requester: DeviceId,
    key: Vec<u8>,
    auth: RequestAuth,
    expires: u64,
    responder: Responder,
}
#[derive(Default)]
pub(super) struct SeedRequests {
    watches: BTreeMap<u8, Arc<()>>,
    pending: VecDeque<Pending>,
    preauth: Option<Rate>,
    service: Option<Rate>,
    requesters: BTreeMap<DeviceId, Rate>,
    now: u64,
    outbound: [std::sync::Weak<()>; 4],
    retained: [std::sync::Weak<()>; 4],
}
impl SeedRequests {
    fn expire(&mut self, now: u64) -> u64 {
        self.now = self.now.max(now);
        self.pending.retain(|p| self.now < p.expires);
        self.now
    }
}
struct CancelOnDrop(tokio::sync::watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

/// Only an actual fresh owner response can select a checkpoint. Hints remain useful for
/// provisional reads but never enter the seed-fetch or later durable-install authority path.
pub enum RegistrySeedDiscovery {
    Hint(ReceiptHeadAnswer),
    Selected(RegistrySeedFetch),
}
impl fmt::Debug for RegistrySeedDiscovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Hint(_) => "RegistrySeedDiscovery::Hint",
            Self::Selected(_) => "RegistrySeedDiscovery::Selected",
        })
    }
}
/// Four retained handles per runtime, each with at most one 2-MiB seed. Revocation/expiry does
/// not refund capacity while this non-Clone handle still retains memory. Three attempts total,
/// paced at one second and a fixed 60-second receiver-clock lifetime; cancellation spends one.
pub struct RegistrySeedFetch {
    selection: HeadSelection,
    _capacity: Arc<()>,
    expires: u64,
    attempts: u8,
    next_at: u64,
    seed: Option<VerifiedCheckpoint>,
}
impl fmt::Debug for RegistrySeedFetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistrySeedFetch")
            .field("attempts", &self.attempts)
            .field("fetched", &self.seed.is_some())
            .finish_non_exhaustive()
    }
}
impl RegistrySeedFetch {
    /// Bytes have been checked; this says nothing about current authority, durability or editing.
    pub fn is_fetched(&self) -> bool {
        self.seed.is_some()
    }
}
/// Borrowed during a current-scope synchronous callback, not a capability the renderer can mint.
/// Persistence must still check its own mount, receipt high-water, inventory and recovery ordering.
pub struct RegistrySeedUse<'a> {
    pub receipt: &'a Receipt,
    pub checkpoint: &'a VerifiedCheckpoint,
    pub tenure: u64,
    pub bucket: u8,
}
impl fmt::Debug for RegistrySeedUse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistrySeedUse { .. }")
    }
}

fn transcript(
    group: &[u8],
    key: &[u8],
    auth: &RequestAuth,
    peer: PeerId,
    query: &[u8],
    body: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(peer.as_bytes()).expect("peer fits");
    e.put_bytes(query).expect("query fits");
    e.put_bytes(body).expect("body bounded");
    signed_resp_transcript(
        RESPONSE_DOMAIN,
        group,
        key,
        auth.ts,
        &auth.nonce,
        auth.epoch,
        e.as_bytes(),
    )
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    fn seed_member(&self, key: &[u8]) -> bool {
        self.group
            .member_signature_key(&DeviceId::from_public_key_bytes(key))
            .as_deref()
            == Some(key)
    }
    pub fn watch_registry_seed(&mut self, bucket: u8) -> RegistrySeedWatch {
        let generation = Arc::new(());
        self.registry_seeds
            .watches
            .insert(bucket, generation.clone());
        self.registry_seeds
            .pending
            .retain(|p| p.query.bucket != bucket);
        RegistrySeedWatch {
            instance: self.registry_instance(),
            bucket,
            generation,
        }
    }
    pub fn registry_seed_watch_is_current(&self, watch: &RegistrySeedWatch) -> bool {
        self.matches_registry_instance(&watch.instance)
            && self
                .registry_seeds
                .watches
                .get(&watch.bucket)
                .is_some_and(|g| Arc::ptr_eq(g, &watch.generation))
    }
    pub fn unwatch_registry_seed(&mut self, watch: &RegistrySeedWatch) -> Result<(), SyncError> {
        if !self.registry_seed_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        self.registry_seeds.watches.remove(&watch.bucket);
        self.registry_seeds
            .pending
            .retain(|p| p.query.bucket != watch.bucket);
        Ok(())
    }
    pub(super) fn queue_registry_seed(&mut self, from: PeerId, data: &[u8], responder: Responder) {
        let now = self.registry_seeds.expire(self.clock.monotonic_ms());
        if data.len() > MAX_QUERY + 144
            || self.registry_seeds.watches.is_empty()
            || self.registry_seeds.pending.len() >= MAX_PENDING
            || !self
                .registry_seeds
                .preauth
                .get_or_insert_with(|| Rate::full(now, 20))
                .charge(now, 10, 20)
        {
            return;
        }
        let Some((inner, key, auth)) = self.authenticate_request(KIND_REGISTRY_SEED, data, from)
        else {
            return;
        };
        if auth.epoch != self.group.epoch()
            || !self.seed_member(&key)
            || !self.seed_member(&self.device.public_key_bytes())
        {
            return;
        }
        let Ok(query) = decode_query(&inner, &self.group.group_id()) else {
            return;
        };
        let Some(generation) = self.registry_seeds.watches.get(&query.bucket).cloned() else {
            return;
        };
        let requester = DeviceId::from_public_key_bytes(&key);
        if self
            .registry_seeds
            .pending
            .iter()
            .any(|p| p.requester == requester)
        {
            return;
        }
        let rates = &mut self.registry_seeds.requesters;
        rates.retain(|_, r| {
            r.refill(now, 1, 2);
            !r.is_full(2)
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
        let Some(expires) = now.checked_add(QUEUE_MS) else {
            return;
        };
        self.registry_seeds.pending.push_back(Pending {
            query,
            inner,
            generation,
            requester,
            key,
            auth,
            expires,
            responder,
        });
    }
    fn seed_request_current(&self, item: &Pending) -> bool {
        item.auth.epoch == self.group.epoch()
            && self.clock.now_ms().abs_diff(item.auth.ts) <= MAX_REQUEST_AGE_MS
            && self.seed_member(&item.key)
            && self.seed_member(&self.device.public_key_bytes())
    }
    /// One exclusive bounded source read. Callback returns only the installed opening seed for
    /// the exact `(doc_id, hash)`, never a generated next seed. None means unavailable, not empty.
    /// No source mutation/receipt publication is performed. Errors hand off no successful answer.
    pub fn serve_registry_seed<E>(
        &mut self,
        watch: &RegistrySeedWatch,
        serve: impl FnOnce(&ServerGroup, &MlsDevice, u128, [u8; 32]) -> Result<Option<Vec<u8>>, E>,
    ) -> Result<Option<Result<(), E>>, SyncError> {
        if !self.registry_seed_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        let now = self.registry_seeds.expire(self.clock.monotonic_ms());
        let Some(index) = self
            .registry_seeds
            .pending
            .iter()
            .position(|p| p.query.bucket == watch.bucket)
        else {
            return Ok(None);
        };
        if !self
            .registry_seeds
            .service
            .get_or_insert_with(|| Rate::full(now, 2))
            .charge(now, 1, 2)
        {
            return Ok(None);
        }
        let item = self
            .registry_seeds
            .pending
            .remove(index)
            .expect("located request");
        if !Arc::ptr_eq(&item.generation, &watch.generation) || !self.seed_request_current(&item) {
            return Err(SyncError::Unauthorized);
        }
        let seed = match serve(
            &self.group,
            &self.device,
            item.query.doc_id,
            item.query.hash,
        ) {
            Ok(value) => value,
            Err(error) => return Ok(Some(Err(error))),
        };
        let body = if let Some(seed) = seed {
            let key =
                self.group
                    .channel_secret(&self.device, DocType::DocRegistry, item.query.doc_id)?;
            seal_seed(&seed, &key, &mut self.rng)?
        } else {
            Vec::new()
        };
        // Disk work and sealing can outlive the admission window. This borrows sync through
        // signing/handoff, so membership/watch changes cannot interleave with source selection.
        if !self.seed_request_current(&item) || self.clock.monotonic_ms() >= item.expires {
            return Err(SyncError::Unauthorized);
        }
        let signature = self.device.sign(&transcript(
            &self.group.group_id(),
            &item.key,
            &item.auth,
            self.transport.local_peer(),
            &item.inner,
            &body,
        ))?;
        item.responder
            .respond(Bytes::from(encode_signed_commit_resp(
                &self.device.public_key_bytes(),
                &signature,
                &body,
            )));
        Ok(Some(Ok(())))
    }

    /// Reserve a retained slot BEFORE discovering. No caller-provided receipt/proof/nonce can
    /// enter this path. A fresh response to another query revokes the older bucket selection.
    pub async fn discover_registry_seed(
        &mut self,
        peer: PeerId,
        bucket: u8,
    ) -> Result<Option<RegistrySeedDiscovery>, SyncError> {
        let slot = self
            .registry_seeds
            .retained
            .iter_mut()
            .find(|s| s.strong_count() == 0)
            .ok_or(SyncError::Malformed)?;
        let capacity = Arc::new(());
        *slot = Arc::downgrade(&capacity);
        let expires = self
            .clock
            .monotonic_ms()
            .checked_add(FETCH_MS)
            .ok_or(SyncError::Malformed)?;
        let Some((answer, selection)) = self.request_registry_head_scoped(peer, bucket).await?
        else {
            return Ok(None);
        };
        Ok(Some(match selection {
            None => RegistrySeedDiscovery::Hint(answer),
            Some(selection) => RegistrySeedDiscovery::Selected(RegistrySeedFetch {
                selection,
                _capacity: capacity,
                expires,
                attempts: 0,
                next_at: 0,
                seed: None,
            }),
        }))
    }
    pub fn registry_seed_fetch_is_current(&self, pass: &RegistrySeedFetch) -> bool {
        self.head_selection_is_current(&pass.selection) && self.clock.monotonic_ms() < pass.expires
    }
    /// Attempt one exact seed fetch from any proven current member. Transport errors, malformed
    /// bodies and cancellation spend the attempt; none advances installation or writes the vault.
    pub async fn fetch_registry_seed(
        &mut self,
        pass: &mut RegistrySeedFetch,
        peer: PeerId,
    ) -> Result<bool, SyncError> {
        let now = self.clock.monotonic_ms();
        if !self.registry_seed_fetch_is_current(pass) {
            return Err(SyncError::Unauthorized);
        }
        if pass.seed.is_some() {
            return Ok(true);
        }
        if pass.attempts >= 3 || now < pass.next_at {
            return Err(SyncError::Malformed);
        }
        let expected = self
            .registry_page_peer_device(peer)
            .ok_or(SyncError::Unauthorized)?;
        let verified = &pass.selection.verified;
        let query = Query {
            bucket: pass.selection.bucket,
            doc_id: epoch_id(
                DocType::DocRegistry,
                &verified.document().logical_key,
                verified
                    .closed_epoch()
                    .checked_add(1)
                    .ok_or(SyncError::Malformed)?,
                &verified.close_record_hash(),
            ),
            hash: verified.seed_change_hash(),
        };
        let inner = encode_query(&query, &self.group.group_id())?;
        let slot = self
            .registry_seeds
            .outbound
            .iter_mut()
            .find(|s| s.strong_count() == 0)
            .ok_or(SyncError::Malformed)?;
        let permit = Arc::new(());
        *slot = Arc::downgrade(&permit);
        pass.attempts += 1;
        pass.next_at = now.checked_add(1000).ok_or(SyncError::Malformed)?;
        let (request, auth) = self.build_authed_request(KIND_REGISTRY_SEED, &inner)?;
        let expires = now
            .checked_add(REQUEST_MS)
            .ok_or(SyncError::Malformed)?
            .min(pass.expires);
        let (signal, cancelled) = tokio::sync::watch::channel(false);
        let _cancel = CancelOnDrop(signal);
        let answer = self.transport.request_cancellable(
            peer,
            ProtocolId(RR_PROTOCOL),
            Bytes::from(request),
            RequestCancellation::new(cancelled, Some(permit)),
        );
        let deadline = self.clock.sleep(std::time::Duration::from_millis(
            expires.saturating_sub(now),
        ));
        futures::pin_mut!(answer, deadline);
        let response = match futures::future::select(answer, deadline).await {
            futures::future::Either::Left((answer, _)) => answer?,
            futures::future::Either::Right(_) => {
                return Err(SyncError::Transport(TransportError::Unreachable(peer)))
            }
        };
        if !self.registry_seed_fetch_is_current(pass) || self.clock.monotonic_ms() >= expires {
            return Err(SyncError::Unauthorized);
        }
        if response.is_empty() {
            return Ok(false);
        }
        let (key, signature, body) = decode_response(&response)?;
        if auth.epoch != self.group.epoch()
            || DeviceId::from_public_key_bytes(key) != expected
            || !self.seed_member(key)
            || !verify_with_public_bytes(
                key,
                &transcript(
                    &self.group.group_id(),
                    &self.device.public_key_bytes(),
                    &auth,
                    peer,
                    &inner,
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
        let key = self
            .group
            .channel_secret(&self.device, DocType::DocRegistry, query.doc_id)?;
        let raw = open_seed(body, &key)?;
        // This checks the receipted hash/checksum BEFORE Automerge decode, then exact raw
        // change shape and the registry schema. A fileshare hash is deliberately not accepted.
        let seed =
            RegistryProjection::verify_checkpoint(&pass.selection.verified, query.bucket, &raw)?;
        if !self.registry_seed_fetch_is_current(pass) || self.clock.monotonic_ms() >= expires {
            return Err(SyncError::Unauthorized);
        }
        pass.seed = Some(seed);
        Ok(true)
    }

    /// Exclusive, current-context use of a fetched seed. This local callback is NOT a storage
    /// adapter: it must still persist recovery before replacement and reject receipt rollback.
    pub fn with_registry_seed<V>(
        &mut self,
        pass: &RegistrySeedFetch,
        use_seed: impl FnOnce(&ServerGroup, &MlsDevice, &mut R, RegistrySeedUse<'_>) -> V,
    ) -> Result<V, SyncError> {
        if !self.registry_seed_fetch_is_current(pass) {
            return Err(SyncError::Unauthorized);
        }
        let checkpoint = pass.seed.as_ref().ok_or(SyncError::Malformed)?;
        Ok(use_seed(
            &self.group,
            &self.device,
            &mut self.rng,
            RegistrySeedUse {
                receipt: &pass.selection.receipt,
                checkpoint,
                tenure: pass.selection.tenure,
                bucket: pass.selection.bucket,
            },
        ))
    }
}

#[cfg(test)]
mod tests;
