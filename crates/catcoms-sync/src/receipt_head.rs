//! Keyed P1 discovery before a requester knows a concrete epoch. Only explicitly registered
//! logical registry buckets are served. This is cooperative networking, not checkpoint admission.
use super::*;
use catcoms_replication::{
    registry::registry_document, LogicalDocument, Receipt, ReceiptHeadProof, VerifiedReceipt,
};
use catcoms_rt::Responder;
use registry_ingress::Rate;

mod wire;
pub use wire::ReceiptHeadAnswer;
use wire::*;

const MAX_PENDING: usize = 8;
const MAX_REQUESTERS: usize = 4096;
const QUEUE_MS: u64 = 5_000;
const REQUEST_MS: u64 = 10_000;
const RESPONSE_DOMAIN: &str = "catcoms/receipt-head-response/v1";

/// Exact logical registration. Unlike an operation watch it intentionally survives rotations;
/// replacing/unregistering it revokes queued requests, without resetting rate debt.
pub struct RegistryHeadWatch {
    instance: RegistrySyncInstance,
    bucket: u8,
    generation: Arc<()>,
}
impl fmt::Debug for RegistryHeadWatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryHeadWatch { .. }")
    }
}
struct Pending {
    bucket: u8,
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
    watches: BTreeMap<u8, Arc<()>>,
    pending: VecDeque<Pending>,
    preauth: Option<Rate>,
    service: Option<Rate>,
    requesters: BTreeMap<DeviceId, Rate>,
    now: u64,
    outbound: [std::sync::Weak<()>; 4],
    // A newly authenticated owner selection revokes older discovery contexts for this bucket.
    selections: BTreeMap<u8, Arc<()>>,
}

/// Created only while validating the actual fresh head response, never from public answer
/// fields. Consumers must recheck the runtime/MLS/selection before using the borrowed authority.
pub(super) struct HeadSelection {
    instance: RegistrySyncInstance,
    epoch: u64,
    owner: DeviceId,
    requester: DeviceId,
    generation: Arc<()>,
    pub(super) bucket: u8,
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

fn transcript(
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
    pub fn watch_registry_head(&mut self, bucket: u8) -> RegistryHeadWatch {
        let generation = Arc::new(());
        self.receipt_heads
            .watches
            .insert(bucket, generation.clone());
        self.receipt_heads.pending.retain(|p| p.bucket != bucket);
        RegistryHeadWatch {
            instance: self.registry_instance(),
            bucket,
            generation,
        }
    }
    pub fn registry_head_watch_is_current(&self, watch: &RegistryHeadWatch) -> bool {
        self.matches_registry_instance(&watch.instance)
            && self
                .receipt_heads
                .watches
                .get(&watch.bucket)
                .is_some_and(|g| Arc::ptr_eq(g, &watch.generation))
    }
    pub fn unwatch_registry_head(&mut self, watch: &RegistryHeadWatch) -> Result<(), SyncError> {
        if !self.registry_head_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        self.receipt_heads.watches.remove(&watch.bucket);
        self.receipt_heads
            .pending
            .retain(|p| p.bucket != watch.bucket);
        Ok(())
    }
    fn head_member(&self, key: &[u8]) -> bool {
        self.group
            .member_signature_key(&DeviceId::from_public_key_bytes(key))
            .as_deref()
            == Some(key)
    }
    pub(super) fn queue_receipt_head(&mut self, from: PeerId, data: &[u8], responder: Responder) {
        let now = self.receipt_heads.expire(self.clock.monotonic_ms());
        if data.len() > MAX_QUERY + 144
            || self.receipt_heads.watches.is_empty()
            || self.receipt_heads.pending.len() >= MAX_PENDING
            || !self
                .receipt_heads
                .preauth
                .get_or_insert_with(|| Rate::full(now, 20))
                .charge(now, 10, 20)
        {
            return;
        }
        let Some((inner, key, auth)) = self.authenticate_request(KIND_RECEIPT_HEAD, data, from)
        else {
            return;
        };
        if auth.epoch != self.group.epoch()
            || !self.head_member(&key)
            || !self.head_member(&self.device.public_key_bytes())
        {
            return;
        }
        let Ok((document, nonce)) = decode_query(&inner, &self.group.group_id()) else {
            return;
        };
        // At most 256 fixed-size hashes, after authentication and the global preauth rail.
        // No untrusted key triggers a disk lookup or a concrete-epoch walk.
        let Some((&bucket, generation)) = self.receipt_heads.watches.iter().find(|(b, _)| {
            registry_document(&self.group.group_id(), **b).is_ok_and(|d| d == document)
        }) else {
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
            bucket,
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
        if !self.registry_head_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        let now = self.receipt_heads.expire(self.clock.monotonic_ms());
        let Some(index) = self
            .receipt_heads
            .pending
            .iter()
            .position(|p| p.bucket == watch.bucket)
        else {
            return Ok(None);
        };
        if !self
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
        let document = registry_document(&self.group.group_id(), watch.bucket)?;
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
        let answer = encode_answer(
            &ReceiptHeadAnswer {
                receipt,
                repair: None,
                proof,
            },
            &document,
        )?;
        let signature = self.device.sign(&transcript(
            &self.group.group_id(),
            &item.key,
            &item.auth,
            self.transport.local_peer(),
            &item.inner,
            &answer,
        ))?;
        item.responder
            .respond(Bytes::from(encode_signed_commit_resp(
                &self.device.public_key_bytes(),
                &signature,
                &answer,
            )));
        Ok(Some(Ok(())))
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
                .get(&selection.bucket)
                .is_some_and(|g| Arc::ptr_eq(g, &selection.generation))
    }

    pub(super) async fn request_registry_head_scoped(
        &mut self,
        peer: PeerId,
        bucket: u8,
    ) -> Result<Option<(ReceiptHeadAnswer, Option<HeadSelection>)>, SyncError> {
        if !self.head_member(&self.device.public_key_bytes()) {
            return Err(SyncError::Unauthorized);
        }
        let expected = self
            .registry_page_peer_device(peer)
            .ok_or(SyncError::Unauthorized)?;
        let document = registry_document(&self.group.group_id(), bucket)?;
        let mut nonce = [0; 16];
        self.rng.fill_bytes(&mut nonce);
        let inner = encode_query(&document, nonce)?;
        let (request, auth) = self.build_authed_request(KIND_RECEIPT_HEAD, &inner)?;
        let slot = self
            .receipt_heads
            .outbound
            .iter_mut()
            .find(|s| s.strong_count() == 0)
            .ok_or(SyncError::Transport(TransportError::Unreachable(peer)))?;
        let permit = Arc::new(());
        *slot = Arc::downgrade(&permit);
        let (signal, cancelled) = tokio::sync::watch::channel(false);
        let _cancel = CancelOnDrop(signal);
        let expires = self
            .clock
            .monotonic_ms()
            .checked_add(REQUEST_MS)
            .ok_or(SyncError::Malformed)?;
        let answer = self.transport.request_cancellable(
            peer,
            ProtocolId(RR_PROTOCOL),
            Bytes::from(request),
            RequestCancellation::new(cancelled, Some(permit)),
        );
        let deadline = self
            .clock
            .sleep(std::time::Duration::from_millis(REQUEST_MS));
        futures::pin_mut!(answer, deadline);
        let response = match futures::future::select(answer, deadline).await {
            futures::future::Either::Left((answer, _)) => answer?,
            futures::future::Either::Right(_) => {
                return Err(SyncError::Transport(TransportError::Unreachable(peer)))
            }
        };
        // A ready response can win select's first branch after local scheduling was delayed
        // beyond the timer. Its authority still expires at the original receiver deadline.
        if self.clock.monotonic_ms() >= expires {
            return Err(SyncError::Unauthorized);
        }
        if response.is_empty() {
            return Ok(None);
        }
        let (key, signature, answer) = decode_response(&response)?;
        if auth.epoch != self.group.epoch()
            || DeviceId::from_public_key_bytes(key) != expected
            || !self.head_member(key)
            || !self.head_member(&self.device.public_key_bytes())
            || !verify_with_public_bytes(
                key,
                &transcript(
                    &self.group.group_id(),
                    &self.device.public_key_bytes(),
                    &auth,
                    peer,
                    &inner,
                    answer,
                ),
                &signature,
            )
        {
            return Err(SyncError::Unauthorized);
        }
        let outcome = decode_answer(answer, &document)?;
        let selection = if let Some(proof) = &outcome.proof {
            if self.group.designated_committer() != Some(expected)
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
                &nonce,
            )?;
            let generation = Arc::new(());
            self.receipt_heads
                .selections
                .insert(bucket, generation.clone());
            Some(HeadSelection {
                instance: self.registry_instance(),
                epoch: self.group.epoch(),
                owner: expected,
                requester: self.device.device_id(),
                generation,
                bucket,
                receipt: outcome
                    .receipt
                    .as_ref()
                    .expect("proof receipt checked")
                    .clone(),
                verified,
                tenure: proof.tenure_start_group_epoch,
            })
        } else {
            None
        };
        // Even a valid current proof is a one-shot selection, never a lease or pruning grant.
        Ok(Some((outcome, selection)))
    }
}

#[cfg(test)]
mod tests;
