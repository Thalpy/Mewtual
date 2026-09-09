//! Expected-hash checkpoint transport. Receiving owns one bounded seed under a fresh head
//! selection, not an installed epoch or a pruning grant. The durable recovery gate is separate.
use super::*;
use crate::checkpoint_exchange::CheckpointTarget;
use catcoms_replication::{
    epoch_id, registry::registry_document, Receipt, VerifiedCheckpoint, MAX_CHECKPOINT_BYTES,
};
use catcoms_rt::Responder;
use catcoms_storage::pad::{self, OP_PAD_CEILING, OP_PAD_FLOOR};
use receipt_head::{HeadSelection, ReceiptHeadAnswer};
use registry_ingress::Rate;
mod detached;
mod service;
mod wire;
pub use detached::{
    CompletedCheckpointDiscovery, CompletedCheckpointSeed, PendingCheckpointDiscovery,
    PendingCheckpointSeed,
};
use wire::*;

const MAX_PENDING: usize = 8;
const MAX_REQUESTERS: usize = 4096;
const QUEUE_MS: u64 = 5_000;
const REQUEST_MS: u64 = 10_000;
const FETCH_MS: u64 = 60_000;
#[cfg(test)]
const RESPONSE_DOMAIN: &str = "catcoms/registry-seed-response/v1";

/// Logical bucket registration. Dropping is not revocation: explicitly unwatch on vault lock.
pub struct RegistrySeedWatch {
    instance: RegistrySyncInstance,
    target: CheckpointTarget,
    generation: Arc<()>,
    request: Option<Arc<()>>,
}
impl fmt::Debug for RegistrySeedWatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistrySeedWatch { .. }")
    }
}
struct Pending {
    id: Arc<()>,
    preparing: bool,
    query: ScopedQuery,
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
    watches: BTreeMap<CheckpointTarget, Arc<()>>,
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
    attempt: Option<Arc<()>>,
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
    pub fn target(&self) -> CheckpointTarget {
        self.selection.target
    }
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

/// The actual fresh owner selection, even before its seed becomes available. A durable receiver
/// needs to seal/save conflicting receipt evidence without depending on an honest seed provider.
/// This is still a scoped synchronous borrow, not a value that can mint a deferred install pass.
pub struct RegistrySeedSelectionUse<'a> {
    pub receipt: &'a Receipt,
    pub checkpoint: Option<&'a VerifiedCheckpoint>,
    pub tenure: u64,
    pub bucket: u8,
}

/// Generic form of the same current-scope borrow. Target remains privately selected by an
/// actual fresh head response; this struct itself is not a deferred installation capability.
pub struct CheckpointSeedSelectionUse<'a> {
    pub receipt: &'a Receipt,
    pub checkpoint: Option<&'a VerifiedCheckpoint>,
    pub tenure: u64,
    pub target: CheckpointTarget,
}
impl fmt::Debug for CheckpointSeedSelectionUse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CheckpointSeedSelectionUse { .. }")
    }
}
impl fmt::Debug for RegistrySeedSelectionUse<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistrySeedSelectionUse { .. }")
    }
}

#[cfg(test)]
fn transcript(
    group: &[u8],
    key: &[u8],
    auth: &RequestAuth,
    peer: PeerId,
    query: &[u8],
    body: &[u8],
) -> Vec<u8> {
    scoped_transcript(RESPONSE_DOMAIN, group, key, auth, peer, query, body)
}
fn scoped_transcript(
    domain: &str,
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
        domain,
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
        self.watch_checkpoint_seed(CheckpointTarget::Registry(bucket))
            .expect("fixed registry bucket count")
    }
    /// Logical service registration independent of user receive watches. Studio has at most
    /// sixteen local registrations; both kinds still share all pending/rate/seed capacity.
    pub fn watch_checkpoint_seed(
        &mut self,
        target: CheckpointTarget,
    ) -> Result<RegistrySeedWatch, SyncError> {
        if matches!(target, CheckpointTarget::Studio(_))
            && !self.registry_seeds.watches.contains_key(&target)
            && self
                .registry_seeds
                .watches
                .keys()
                .filter(|t| matches!(t, CheckpointTarget::Studio(_)))
                .count()
                >= 16
        {
            return Err(SyncError::Malformed);
        }
        let generation = Arc::new(());
        self.registry_seeds
            .watches
            .insert(target, generation.clone());
        self.registry_seeds
            .pending
            .retain(|p| p.query.target != target);
        Ok(RegistrySeedWatch {
            instance: self.registry_instance(),
            target,
            generation,
            request: None,
        })
    }
    pub fn registry_seed_watch_is_current(&self, watch: &RegistrySeedWatch) -> bool {
        self.matches_registry_instance(&watch.instance)
            && (self
                .registry_seeds
                .watches
                .get(&watch.target)
                .is_some_and(|g| Arc::ptr_eq(g, &watch.generation))
                || (watch.request.is_some()
                    && self.epoch_service_generation_is_current(&watch.generation)))
    }
    pub fn unwatch_registry_seed(&mut self, watch: &RegistrySeedWatch) -> Result<(), SyncError> {
        if !self.registry_seed_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        self.registry_seeds.watches.remove(&watch.target);
        self.registry_seeds
            .pending
            .retain(|p| p.query.target != watch.target);
        Ok(())
    }
    pub(super) fn queue_registry_seed(&mut self, from: PeerId, data: &[u8], responder: Responder) {
        self.queue_checkpoint_seed(KIND_REGISTRY_SEED, from, data, responder);
    }
    pub(super) fn queue_checkpoint_seed(
        &mut self,
        kind: u8,
        from: PeerId,
        data: &[u8],
        responder: Responder,
    ) {
        let now = self.registry_seeds.expire(self.clock.monotonic_ms());
        if data.len() > MAX_QUERY + 144
            || (self.registry_seeds.watches.is_empty() && self.epoch_service.generation.is_none())
            || self.registry_seeds.pending.len() >= MAX_PENDING
            || !self
                .registry_seeds
                .preauth
                .get_or_insert_with(|| Rate::full(now, 20))
                .charge(now, 10, 20)
        {
            return;
        }
        let Some((inner, key, auth)) = self.authenticate_request(kind, data, from) else {
            return;
        };
        if auth.epoch != self.group.epoch()
            || !self.seed_member(&key)
            || !self.seed_member(&self.device.public_key_bytes())
        {
            return;
        }
        let Ok(query) = decode_scoped_query(kind, &inner, &self.group.group_id()) else {
            return;
        };
        let Some(generation) = self
            .registry_seeds
            .watches
            .get(&query.target)
            .or(self.epoch_service.generation.as_ref())
            .cloned()
        else {
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
            id: Arc::new(()),
            preparing: false,
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
        let Some(index) = self.registry_seeds.pending.iter().position(|p| {
            p.query.target == watch.target
                && watch
                    .request
                    .as_ref()
                    .is_none_or(|id| Arc::ptr_eq(id, &p.id))
        }) else {
            return Ok(None);
        };
        if watch.request.is_none()
            && !self
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
            let key = self.group.channel_secret(
                &self.device,
                item.query.target.doc_type(),
                item.query.doc_id,
            )?;
            seal_seed(&seed, &key, &mut self.rng)?
        } else {
            Vec::new()
        };
        // Disk work and sealing can outlive the admission window. This borrows sync through
        // signing/handoff, so membership/watch changes cannot interleave with source selection.
        if !self.seed_request_current(&item) || self.clock.monotonic_ms() >= item.expires {
            return Err(SyncError::Unauthorized);
        }
        let signature = self.device.sign(&scoped_transcript(
            watch.target.seed_domain(),
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
        let pending =
            self.prepare_checkpoint_discovery(peer, CheckpointTarget::Registry(bucket))?;
        let completed = pending.fetch().await;
        self.complete_checkpoint_discovery(completed)
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
        let Some(pending) = self.prepare_checkpoint_seed(pass, peer)? else {
            return Ok(true);
        };
        let completed = pending.fetch().await;
        self.complete_checkpoint_seed(pass, completed)
    }

    /// Exclusive, current-context use of a fetched seed. This local callback is NOT a storage
    /// adapter: it must still persist recovery before replacement and reject receipt rollback.
    pub fn with_registry_seed<V>(
        &mut self,
        pass: &RegistrySeedFetch,
        use_seed: impl FnOnce(&ServerGroup, &MlsDevice, &mut R, RegistrySeedUse<'_>) -> V,
    ) -> Result<V, SyncError> {
        self.with_registry_seed_selection(pass, |group, device, rng, selected| {
            let checkpoint = selected.checkpoint.ok_or(SyncError::Malformed)?;
            Ok(use_seed(
                group,
                device,
                rng,
                RegistrySeedUse {
                    receipt: selected.receipt,
                    checkpoint,
                    tenure: selected.tenure,
                    bucket: selected.bucket,
                },
            ))
        })?
    }

    /// Current selection may be used before fetching. The caller must persist verified Fault
    /// evidence independently of seed availability. Same runtime/MLS/member/expiry checks as
    /// seed use; no public receipt/answer fields can create the required private pass.
    pub fn with_registry_seed_selection<V>(
        &mut self,
        pass: &RegistrySeedFetch,
        use_selection: impl FnOnce(&ServerGroup, &MlsDevice, &mut R, RegistrySeedSelectionUse<'_>) -> V,
    ) -> Result<V, SyncError> {
        // Legacy Registry entry points cannot accidentally consume a Studio selection.
        pass.selection.target.bucket()?;
        self.with_checkpoint_seed_selection(pass, |group, device, rng, selected| {
            use_selection(
                group,
                device,
                rng,
                RegistrySeedSelectionUse {
                    receipt: selected.receipt,
                    checkpoint: selected.checkpoint,
                    tenure: selected.tenure,
                    bucket: selected.target.bucket().expect("checked registry"),
                },
            )
        })
    }
    pub fn with_checkpoint_seed_selection<V>(
        &mut self,
        pass: &RegistrySeedFetch,
        use_selection: impl FnOnce(
            &ServerGroup,
            &MlsDevice,
            &mut R,
            CheckpointSeedSelectionUse<'_>,
        ) -> V,
    ) -> Result<V, SyncError> {
        if !self.registry_seed_fetch_is_current(pass) {
            return Err(SyncError::Unauthorized);
        }
        Ok(use_selection(
            &self.group,
            &self.device,
            &mut self.rng,
            CheckpointSeedSelectionUse {
                receipt: &pass.selection.receipt,
                checkpoint: pass.seed.as_ref(),
                tenure: pass.selection.tenure,
                target: pass.selection.target,
            },
        ))
    }
}

#[cfg(test)]
mod tests;
