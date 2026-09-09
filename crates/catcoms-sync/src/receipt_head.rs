//! Keyed P1 discovery before a requester knows a concrete epoch. Only explicitly registered
//! logical registry buckets are served. This is cooperative networking, not checkpoint admission.
use super::*;
use crate::checkpoint_exchange::CheckpointTarget;
use catcoms_replication::{
    registry::registry_document, LogicalDocument, Receipt, ReceiptHeadProof, VerifiedReceipt,
};
use catcoms_rt::Responder;
use registry_ingress::Rate;

mod detached;
mod service;
mod wire;
pub use detached::{CompletedCheckpointHead, PendingCheckpointHead};
pub use wire::ReceiptHeadAnswer;
use wire::*;

const MAX_PENDING: usize = 8;
const MAX_REQUESTERS: usize = 4096;
const QUEUE_MS: u64 = 5_000;
const REQUEST_MS: u64 = 10_000;
#[cfg(test)]
const RESPONSE_DOMAIN: &str = "catcoms/receipt-head-response/v1";

/// Exact logical registration. Unlike an operation watch it intentionally survives rotations;
/// replacing/unregistering it revokes queued requests, without resetting rate debt.
pub struct RegistryHeadWatch {
    instance: RegistrySyncInstance,
    target: CheckpointTarget,
    generation: Arc<()>,
    // Present only for a private service token; never a receive/UI registration.
    request: Option<Arc<()>>,
}
impl fmt::Debug for RegistryHeadWatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryHeadWatch { .. }")
    }
}
struct Pending {
    id: Arc<()>,
    preparing: bool,
    target: CheckpointTarget,
    generation: Arc<()>,
    inner: Vec<u8>,
    nonce: [u8; 16],
    requester: DeviceId,
    key: Vec<u8>,
    auth: RequestAuth,
    expires: u64,
    responder: Responder,
}
#[derive(Default)]
pub(super) struct HeadRequests {
    watches: BTreeMap<CheckpointTarget, Arc<()>>,
    pending: VecDeque<Pending>,
    preauth: Option<Rate>,
    service: Option<Rate>,
    requesters: BTreeMap<DeviceId, Rate>,
    now: u64,
    outbound: [std::sync::Weak<()>; 4],
    // A newly authenticated owner selection revokes older discovery contexts for this bucket.
    selections: BTreeMap<CheckpointTarget, std::sync::Weak<()>>,
    // Preparing a later attempt invalidates an earlier in-flight completion, but not an
    // already authenticated selection. A hint/failed attempt must not revoke that selection.
    attempts: BTreeMap<CheckpointTarget, std::sync::Weak<()>>,
}

/// Created only while validating the actual fresh head response, never from public answer
/// fields. Consumers must recheck the runtime/MLS/selection before using the borrowed authority.
pub(super) struct HeadSelection {
    instance: RegistrySyncInstance,
    epoch: u64,
    owner: DeviceId,
    requester: DeviceId,
    pub(super) generation: Arc<()>,
    pub(super) target: CheckpointTarget,
    pub(super) receipt: Receipt,
    pub(super) verified: VerifiedReceipt,
    pub(super) tenure: u64,
}
impl HeadRequests {
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

/// Evidence that the trusted local persistence callback saved this exact owner/MLS observation.
/// Created by explicit local preparation, NEVER by a remotely triggered source request. Later
/// membership commits invalidate it; ordinary document edits do not change the owner evidence.
#[derive(Clone)]
pub struct DurableOwnerSnapshot {
    instance: RegistrySyncInstance,
    epoch: u64,
    owner: DeviceId,
    tenure: u64,
}
impl fmt::Debug for DurableOwnerSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DurableOwnerSnapshot { .. }")
    }
}

/// One successful local reply-channel handoff of an owner proof. Private fields prevent a raw
/// receipt or constructed proof becoming completion authority. Deliberately not Clone or durable:
/// dropping this before recording completion requires an exact re-handoff after restart.
pub struct ReceiptHeadHandoff {
    snapshot: DurableOwnerSnapshot,
    target: CheckpointTarget,
    generation: Arc<()>,
    service: bool,
    receipt: Box<Receipt>,
    expires: u64,
}
impl fmt::Debug for ReceiptHeadHandoff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReceiptHeadHandoff { .. }")
    }
}

/// A served response is either a provisional hint or a checked owner-proof handoff. Neither
/// variant acknowledges network delivery; only Owner can drive exact journal completion.
#[derive(Debug)]
pub enum ReceiptHeadServed {
    Hint,
    Owner(ReceiptHeadHandoff),
}

/// Trusted synchronous source context. `tenure` is present only with a still-current durable
/// snapshot permit. The source must separately persist its selected irrevocable decision and
/// check its registry/inventory before asking sync to sign a fresh proof.
pub struct ReceiptHeadSource<'a> {
    pub document: &'a LogicalDocument,
    pub requester: DeviceId,
    pub nonce: [u8; 16],
    pub tenure: Option<u64>,
}
impl fmt::Debug for ReceiptHeadSource<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReceiptHeadSource")
            .field("durable_tenure", &self.tenure.is_some())
            .finish_non_exhaustive()
    }
}

/// Source selection from a trusted vault adapter, never renderer or remote input. `prove`
/// asserts completion of that adapter's snapshot/decision barriers and nonfault checks. Sync
/// additionally verifies actual owner, observed tenure, scope and fresh request before signing.
pub struct ReceiptHeadSelection {
    pub receipt: Option<Receipt>,
    pub prove: bool,
}
impl fmt::Debug for ReceiptHeadSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReceiptHeadSelection")
            .field("receipt", &self.receipt.is_some())
            .field("prove", &self.prove)
            .finish()
    }
}

#[cfg(test)]
fn transcript(
    group: &[u8],
    key: &[u8],
    auth: &RequestAuth,
    peer: PeerId,
    query: &[u8],
    answer: &[u8],
) -> Vec<u8> {
    scoped_transcript(RESPONSE_DOMAIN, group, key, auth, peer, query, answer)
}
fn scoped_transcript(
    domain: &str,
    group: &[u8],
    key: &[u8],
    auth: &RequestAuth,
    peer: PeerId,
    query: &[u8],
    answer: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(peer.as_bytes()).expect("peer fits");
    e.put_bytes(query).expect("query fits");
    e.put_bytes(answer).expect("answer fits");
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
    /// Explicit local preparation; serializes legacy whole-server state, which is NOT covered
    /// by the small head-query byte budget. Never call this automatically for a remote request.
    /// Save failure yields no permit. The app wrapper must bind its mount/numeric server too.
    pub fn prepare_receipt_head_snapshot<E>(
        &mut self,
        save: impl FnOnce(&[u8], &mut R) -> Result<(), E>,
    ) -> Result<Result<DurableOwnerSnapshot, E>, SyncError> {
        let tenure = self
            .observed_owner_tenure_start()
            .ok_or(SyncError::Unauthorized)?;
        if self.group.designated_committer() != Some(self.device.device_id())
            || !self.head_member(&self.device.public_key_bytes())
        {
            return Err(SyncError::Unauthorized);
        }
        let snapshot = self.snapshot()?;
        if let Err(error) = save(&snapshot, &mut self.rng) {
            return Ok(Err(error));
        }
        Ok(Ok(DurableOwnerSnapshot {
            instance: self.registry_instance(),
            epoch: self.group.epoch(),
            owner: self.device.device_id(),
            tenure,
        }))
    }
    fn head_snapshot_is_current(&self, permit: &DurableOwnerSnapshot) -> bool {
        self.matches_registry_instance(&permit.instance)
            && permit.epoch == self.group.epoch()
            && self.device.device_id() == permit.owner
            && self.group.designated_committer() == Some(permit.owner)
            && self.observed_owner_tenure_start() == Some(permit.tenure)
    }
    /// Admit a finite local owner transaction under the exact persisted MLS/tenure snapshot.
    /// An observed tenure alone is insufficient: a restart must not restore authority behind
    /// a newly irrevocable decision. The app additionally binds the physical mount and server.
    pub fn with_durable_owner_snapshot<V>(
        &mut self,
        permit: &DurableOwnerSnapshot,
        use_owner: impl FnOnce(&ServerGroup, &MlsDevice, &mut R, u64) -> V,
    ) -> Result<V, SyncError> {
        if !self.head_snapshot_is_current(permit)
            || !self.head_member(&self.device.public_key_bytes())
        {
            return Err(SyncError::Unauthorized);
        }
        Ok(use_owner(
            &self.group,
            &self.device,
            &mut self.rng,
            permit.tenure,
        ))
    }

    /// Consume a short-lived handoff immediately under exclusive runtime ownership. The app
    /// retains its mount/server/store borrow from serving through the accounted journal write.
    /// Failure here cannot unsend a reply already accepted by the local forwarding channel.
    pub fn with_receipt_head_handoff<V>(
        &mut self,
        handoff: ReceiptHeadHandoff,
        complete: impl FnOnce(&Receipt, &mut R) -> V,
    ) -> Result<V, SyncError> {
        if self.clock.monotonic_ms() >= handoff.expires
            || !self.head_snapshot_is_current(&handoff.snapshot)
            || !self.head_member(&self.device.public_key_bytes())
            || !(self
                .receipt_heads
                .watches
                .get(&handoff.target)
                .is_some_and(|g| Arc::ptr_eq(g, &handoff.generation))
                || (handoff.service
                    && self.epoch_service_generation_is_current(&handoff.generation)))
            || handoff.receipt.document != handoff.target.document(&self.group.group_id())?
        {
            return Err(SyncError::Unauthorized);
        }
        handoff
            .receipt
            .verify_current_owner(&self.group, handoff.snapshot.tenure)?;
        Ok(complete(&handoff.receipt, &mut self.rng))
    }
    pub fn watch_registry_head(&mut self, bucket: u8) -> RegistryHeadWatch {
        self.watch_checkpoint_head(CheckpointTarget::Registry(bucket))
            .expect("fixed registry bucket count")
    }
    /// Register a logical checkpoint service independently of the concrete operation watch.
    /// Studio registrations have sixteen slots; replacing one never refunds request rate debt.
    pub fn watch_checkpoint_head(
        &mut self,
        target: CheckpointTarget,
    ) -> Result<RegistryHeadWatch, SyncError> {
        if matches!(target, CheckpointTarget::Studio(_))
            && !self.receipt_heads.watches.contains_key(&target)
            && self
                .receipt_heads
                .watches
                .keys()
                .filter(|t| matches!(t, CheckpointTarget::Studio(_)))
                .count()
                >= 16
        {
            return Err(SyncError::Malformed);
        }
        let generation = Arc::new(());
        self.receipt_heads
            .watches
            .insert(target, generation.clone());
        self.receipt_heads.pending.retain(|p| p.target != target);
        Ok(RegistryHeadWatch {
            instance: self.registry_instance(),
            target,
            generation,
            request: None,
        })
    }
    pub fn registry_head_watch_is_current(&self, watch: &RegistryHeadWatch) -> bool {
        self.matches_registry_instance(&watch.instance)
            && (self
                .receipt_heads
                .watches
                .get(&watch.target)
                .is_some_and(|g| Arc::ptr_eq(g, &watch.generation))
                || (watch.request.is_some()
                    && self.epoch_service_generation_is_current(&watch.generation)))
    }
    pub fn unwatch_registry_head(&mut self, watch: &RegistryHeadWatch) -> Result<(), SyncError> {
        if !self.registry_head_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        self.receipt_heads.watches.remove(&watch.target);
        self.receipt_heads
            .pending
            .retain(|p| p.target != watch.target);
        Ok(())
    }
    fn head_member(&self, key: &[u8]) -> bool {
        self.group
            .member_signature_key(&DeviceId::from_public_key_bytes(key))
            .as_deref()
            == Some(key)
    }
    pub(super) fn queue_receipt_head(&mut self, from: PeerId, data: &[u8], responder: Responder) {
        self.queue_checkpoint_head(KIND_RECEIPT_HEAD, from, data, responder);
    }
    pub(super) fn queue_checkpoint_head(
        &mut self,
        kind: u8,
        from: PeerId,
        data: &[u8],
        responder: Responder,
    ) {
        let now = self.receipt_heads.expire(self.clock.monotonic_ms());
        if data.len() > MAX_QUERY + 144
            || (self.receipt_heads.watches.is_empty() && self.epoch_service.generation.is_none())
            || self.receipt_heads.pending.len() >= MAX_PENDING
            || !self
                .receipt_heads
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
            || !self.head_member(&key)
            || !self.head_member(&self.device.public_key_bytes())
        {
            return;
        }
        let Ok((target, nonce)) = decode_scoped_query(kind, &inner, &self.group.group_id()) else {
            return;
        };
        // At most 256 fixed-size hashes, after authentication and the global preauth rail.
        // No untrusted key triggers a disk lookup or a concrete-epoch walk.
        let Some(generation) = self
            .receipt_heads
            .watches
            .get(&target)
            .or(self.epoch_service.generation.as_ref())
        else {
            return;
        };
        let generation = generation.clone();
        let requester = DeviceId::from_public_key_bytes(&key);
        if self
            .receipt_heads
            .pending
            .iter()
            .any(|p| p.requester == requester)
        {
            return;
        }
        let rates = &mut self.receipt_heads.requesters;
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
        self.receipt_heads.pending.push_back(Pending {
            id: Arc::new(()),
            preparing: false,
            target,
            generation,
            inner,
            nonce,
            requester,
            key,
            auth,
            expires,
            responder,
        });
    }

    /// Drain one bounded request while keeping exclusive sync ownership through source barriers
    /// and responder handoff. No reply token or snapshot escapes across await/cancellation.
    /// The callback must enforce durable source selection/fault checks; errors send no success.
    pub fn serve_receipt_head<E>(
        &mut self,
        watch: &RegistryHeadWatch,
        snapshot: Option<&DurableOwnerSnapshot>,
        serve: impl FnOnce(
            &ServerGroup,
            &MlsDevice,
            &mut R,
            ReceiptHeadSource<'_>,
        ) -> Result<ReceiptHeadSelection, E>,
    ) -> Result<Option<Result<(), E>>, SyncError> {
        Ok(self
            .serve_receipt_head_with_handoff(watch, snapshot, serve)?
            .map(|result| result.map(|_| ())))
    }

    /// Same bounded serving transaction, retaining exact completion evidence for the trusted
    /// caller. Only a successful checked local-channel send of an owner proof yields a handoff.
    /// Source errors, hints, expired requests and dropped receivers cannot complete a journal.
    pub fn serve_receipt_head_with_handoff<E>(
        &mut self,
        watch: &RegistryHeadWatch,
        snapshot: Option<&DurableOwnerSnapshot>,
        serve: impl FnOnce(
            &ServerGroup,
            &MlsDevice,
            &mut R,
            ReceiptHeadSource<'_>,
        ) -> Result<ReceiptHeadSelection, E>,
    ) -> Result<Option<Result<ReceiptHeadServed, E>>, SyncError> {
        if !self.registry_head_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        let now = self.receipt_heads.expire(self.clock.monotonic_ms());
        let Some(index) = self.receipt_heads.pending.iter().position(|p| {
            p.target == watch.target
                && watch
                    .request
                    .as_ref()
                    .is_none_or(|id| Arc::ptr_eq(id, &p.id))
        }) else {
            return Ok(None);
        };
        if watch.request.is_none()
            && !self
                .receipt_heads
                .service
                .get_or_insert_with(|| Rate::full(now, 4))
                .charge(now, 2, 4)
        {
            return Ok(None);
        }
        let item = self
            .receipt_heads
            .pending
            .remove(index)
            .expect("located request");
        if !Arc::ptr_eq(&item.generation, &watch.generation) || !self.head_request_current(&item) {
            return Err(SyncError::Unauthorized);
        }
        let document = watch.target.document(&self.group.group_id())?;
        let tenure = snapshot
            .filter(|p| self.head_snapshot_is_current(p))
            .map(|p| p.tenure);
        let selected = match serve(
            &self.group,
            &self.device,
            &mut self.rng,
            ReceiptHeadSource {
                document: &document,
                requester: item.requester,
                nonce: item.nonce,
                tenure,
            },
        ) {
            Ok(value) => value,
            Err(error) => return Ok(Some(Err(error))),
        };
        // Synchronous disk work can consume the request's lifetime. Never send stale success.
        if !self.head_request_current(&item) || self.clock.monotonic_ms() >= item.expires {
            return Err(SyncError::Unauthorized);
        }
        let receipt = selected.receipt;
        if receipt.as_ref().is_some_and(|r| r.document != document) {
            return Err(SyncError::Malformed);
        }
        let proof = if selected.prove {
            let receipt = receipt.as_ref().ok_or(SyncError::Malformed)?;
            let tenure = tenure.ok_or(SyncError::Unauthorized)?;
            if self.observed_owner_tenure_start() != Some(tenure) {
                return Err(SyncError::Unauthorized);
            }
            receipt.verify_current_owner(&self.group, tenure)?;
            Some(ReceiptHeadProof::sign(
                receipt,
                item.requester,
                item.nonce,
                &self.device,
            )?)
        } else {
            None
        };
        let answer = ReceiptHeadAnswer {
            receipt,
            repair: None,
            proof,
        };
        let bytes = encode_answer(&answer, &document)?;
        let signature = self.device.sign(&scoped_transcript(
            watch.target.head_domain(),
            &self.group.group_id(),
            &item.key,
            &item.auth,
            self.transport.local_peer(),
            &item.inner,
            &bytes,
        ))?;
        if !self.head_request_current(&item) || self.clock.monotonic_ms() >= item.expires {
            return Err(SyncError::Unauthorized);
        }
        item.responder
            .try_respond(Bytes::from(encode_signed_commit_resp(
                &self.device.public_key_bytes(),
                &signature,
                &bytes,
            )))?;
        let served = if answer.proof.is_some() {
            ReceiptHeadServed::Owner(ReceiptHeadHandoff {
                snapshot: snapshot.expect("proved durable snapshot").clone(),
                target: watch.target,
                generation: watch.generation.clone(),
                service: watch.request.is_some(),
                receipt: Box::new(answer.receipt.expect("proved receipt")),
                expires: item.expires,
            })
        } else {
            ReceiptHeadServed::Hint
        };
        Ok(Some(Ok(served)))
    }
    fn head_request_current(&self, item: &Pending) -> bool {
        item.auth.epoch == self.group.epoch()
            && self.clock.now_ms().abs_diff(item.auth.ts) <= MAX_REQUEST_AGE_MS
            && self.head_member(&item.key)
            && self.head_member(&self.device.public_key_bytes())
    }

    /// One keyed discovery query to an already proven current member endpoint. A fresh nonce
    /// comes from the injected RNG, never the caller. Returned hints are NOT installed epochs.
    /// The receiver must verify a seed by expected hash and preserve its existing provisional work.
    pub async fn request_registry_head(
        &mut self,
        peer: PeerId,
        bucket: u8,
    ) -> Result<Option<ReceiptHeadAnswer>, SyncError> {
        Ok(self
            .request_registry_head_scoped(peer, bucket)
            .await?
            .map(|(answer, _)| answer))
    }

    pub(super) fn head_selection_is_current(&self, selection: &HeadSelection) -> bool {
        self.matches_registry_instance(&selection.instance)
            && self.group.epoch() == selection.epoch
            && self.group.designated_committer() == Some(selection.owner)
            && self.device.device_id() == selection.requester
            && self.head_member(&self.device.public_key_bytes())
            && self
                .receipt_heads
                .selections
                .get(&selection.target)
                .and_then(std::sync::Weak::upgrade)
                .is_some_and(|g| Arc::ptr_eq(&g, &selection.generation))
    }

    pub(super) async fn request_registry_head_scoped(
        &mut self,
        peer: PeerId,
        bucket: u8,
    ) -> Result<Option<(ReceiptHeadAnswer, Option<HeadSelection>)>, SyncError> {
        let pending = self.prepare_checkpoint_head(peer, CheckpointTarget::Registry(bucket))?;
        let completed = pending.fetch().await;
        self.complete_checkpoint_head_scoped(completed)
    }
}

#[cfg(test)]
mod tests;
