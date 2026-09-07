//! Bounded, opt-in registry gossip inbox. Enqueue is authentication, NOT document admission.
//! The app drains one packet through its durable typed store; legacy docs/acks never see it.

use super::*;
use catcoms_replication::epoch::MAX_SIGNED_EPOCH_OP_BYTES;
use catcoms_replication::registry::RegistryOp;
use catcoms_replication::DomainOp;

const MAX_QUEUE: usize = 16;
const MAX_RATE_ROWS: usize = 4096;
// SealedOp has a 58-byte envelope, a four-byte padding footer and a 16-byte AEAD tag.
const MAX_PACKET: usize = MAX_SIGNED_EPOCH_OP_BYTES + 78;

/// Exact installed watch generation. Same-id replacement, server restore and explicit unwatch
/// invalidate old handles. No wire authority or plaintext body is exposed by this local token.
pub struct RegistryWatch {
    bucket: u8,
    doc_id: u128,
    generation: Arc<()>,
    instance: RegistrySyncInstance,
}

impl fmt::Debug for RegistryWatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryWatch { .. }")
    }
}

pub(super) struct Watched {
    pub(super) doc_id: u128,
    generation: Arc<()>,
}

struct Incoming {
    bucket: u8,
    generation: Arc<()>,
    sealed: SealedOp,
}

/// Fixed-point token bucket: one token = 1000 units. Monotonic rollback cannot refill debt;
/// saturated arithmetic handles a very long idle interval without wrapping into fresh capacity.
#[derive(Clone, Copy)]
struct Rate {
    units: u64,
    at_ms: u64,
}
impl Rate {
    fn full(now: u64, burst: u64) -> Self {
        Self {
            units: burst * 1000,
            at_ms: now,
        }
    }
    fn refill(&mut self, now: u64, per_second: u64, burst: u64) {
        self.units = self
            .units
            .saturating_add(now.saturating_sub(self.at_ms).saturating_mul(per_second))
            .min(burst * 1000);
        self.at_ms = self.at_ms.max(now);
    }
    fn charge(&mut self, now: u64, per_second: u64, burst: u64) -> bool {
        self.refill(now, per_second, burst);
        if self.units < 1000 {
            return false;
        }
        self.units -= 1000;
        true
    }
}

#[derive(Default)]
pub(super) struct RegistryIngress {
    pub(super) watches: BTreeMap<u8, Watched>,
    queue: VecDeque<Incoming>,
    preauth: Option<Rate>,
    authors: BTreeMap<(DeviceId, u128), Rate>,
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Install a desired concrete registry watch synchronously, before any cancellable network
    /// wait. Trusted callers must derive this id from their checked store or deterministic epoch 0.
    /// The next run_once (or explicit flush) reconciles transport subscriptions. One slot/bucket.
    pub fn watch_registry(&mut self, bucket: u8, doc_id: u128) -> RegistryWatch {
        let generation = Arc::new(());
        self.registry_ingress
            .queue
            .retain(|item| item.bucket != bucket);
        self.registry_ingress.watches.insert(
            bucket,
            Watched {
                doc_id,
                generation: generation.clone(),
            },
        );
        self.needs_resync = true;
        RegistryWatch {
            bucket,
            doc_id,
            generation,
            instance: self.registry_instance(),
        }
    }

    pub fn registry_watch_is_current(&self, watch: &RegistryWatch) -> bool {
        self.matches_registry_instance(&watch.instance)
            && self
                .registry_ingress
                .watches
                .get(&watch.bucket)
                .is_some_and(|entry| {
                    entry.doc_id == watch.doc_id
                        && Arc::ptr_eq(&entry.generation, &watch.generation)
                })
    }

    /// Revoke admission and drop queued bytes immediately; async unsubscription follows on the
    /// next run_once/flush. An old handle cannot unwatch its replacement. Rate debt is retained.
    pub fn unwatch_registry(&mut self, watch: &RegistryWatch) -> Result<(), SyncError> {
        if !self.registry_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        self.registry_ingress.watches.remove(&watch.bucket);
        self.registry_ingress
            .queue
            .retain(|item| item.bucket != watch.bucket);
        self.needs_resync = true;
        Ok(())
    }

    /// Optional explicit flush so callers need not wait for an inbound event after subscribing.
    /// Error/drop leaves reconciliation armed and retains ownership of uncertain subscriptions.
    pub async fn flush_registry_subscriptions(&mut self) -> Result<(), SyncError> {
        self.resync_subscriptions().await
    }

    /// Handle every registry-tagged frame, including unopened/malformed ones, without ever
    /// falling into generic Automerge ingestion or generic catch-up. Nonregistry traffic is unchanged.
    pub(super) fn on_registry_gossip(&mut self, topic: &Topic, data: &[u8]) -> bool {
        if !data.starts_with(&DocType::DocRegistry.tag().to_be_bytes()) {
            return false;
        }
        let now = self.clock.monotonic_ms();
        let inbox = &mut self.registry_ingress;
        if data.len() > MAX_PACKET
            || inbox.watches.is_empty()
            || !inbox
                .preauth
                .get_or_insert_with(|| Rate::full(now, 200))
                .charge(now, 50, 200)
            || inbox.queue.len() >= MAX_QUEUE
        {
            return true;
        }
        let Ok(sealed) = SealedOp::decode(data) else {
            return true;
        };
        let Some((&bucket, watch)) = self
            .registry_ingress
            .watches
            .iter()
            .find(|(_, watch)| watch.doc_id == sealed.doc_id)
        else {
            return true;
        };
        let generation = watch.generation.clone();
        if !self.window_labels().any(|slot| {
            self.channel_topic_for(DocType::DocRegistry, sealed.doc_id, slot)
                .as_ref()
                == Some(topic)
        }) {
            return true;
        }
        let Ok(author) = self.authenticate_registry(&sealed, bucket) else {
            return true;
        };
        // Reclaim ONLY fully refilled rows: eviction, unwatch and rotation cannot forgive debt.
        let rates = &mut self.registry_ingress.authors;
        rates.retain(|_, rate| {
            rate.refill(now, 10, 50);
            rate.units < 50_000
        });
        let key = (author, sealed.doc_id);
        if !rates.contains_key(&key) && rates.len() >= MAX_RATE_ROWS {
            return true;
        }
        if !rates
            .entry(key)
            .or_insert_with(|| Rate::full(now, 50))
            .charge(now, 10, 50)
        {
            return true;
        }
        // Decode copied the bounded ciphertext into its own Vec. Never retain the transport's
        // possibly huge backing allocation, decrypted SignedOp, or a second plaintext body.
        self.registry_ingress.queue.push_back(Incoming {
            bucket,
            generation,
            sealed,
        });
        true
    }

    fn authenticate_registry(&self, sealed: &SealedOp, bucket: u8) -> Result<DeviceId, SyncError> {
        if sealed.doc_type != DocType::DocRegistry
            || sealed.epoch != self.group.epoch()
            || sealed.blob.ciphertext.len() > MAX_SIGNED_EPOCH_OP_BYTES + 20
        {
            return Err(SyncError::Malformed);
        }
        if self
            .group
            .member_signature_key(&self.device.device_id())
            .as_deref()
            != Some(self.device.public_key_bytes().as_slice())
        {
            return Err(SyncError::Unauthorized);
        }
        let key = self
            .group
            .channel_secret(&self.device, DocType::DocRegistry, sealed.doc_id)?;
        let signed = sealed.open(&key)?;
        if signed.doc_type != sealed.doc_type
            || signed.doc_id != sealed.doc_id
            || !signed.verify()
            || self
                .group
                .member_signature_key(&signed.author_device)
                .as_deref()
                != Some(signed.author_pubkey.as_slice())
        {
            return Err(SyncError::Unauthorized);
        }
        let domain = DomainOp::decode(signed.domain_op.as_deref().ok_or(SyncError::Malformed)?)?;
        let operation = RegistryOp::decode(&domain.body)?;
        let operation_bucket = match &operation {
            RegistryOp::Put { key, .. } | RegistryOp::Tombstone { key } => key.bucket(),
        };
        if operation.domain_op(&self.group.group_id(), domain.nonce)? != domain
            || operation_bucket != bucket
        {
            return Err(SyncError::Malformed);
        }
        Ok(signed.author_device)
    }

    /// Consume at most one queued packet for this exact watch and synchronously invoke the
    /// trusted durable adapter. Revalidate current membership/MLS after queue delay, before I/O.
    /// Error/unwind consumes only volatile traffic, sends no ack and proves no saved admission.
    pub fn drain_registry_inbound<O>(
        &mut self,
        watch: &RegistryWatch,
        work: impl FnOnce(&ServerGroup, &MlsDevice, &mut R, &SealedOp) -> O,
    ) -> Result<Option<O>, SyncError> {
        if !self.registry_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        let Some(index) = self.registry_ingress.queue.iter().position(|item| {
            item.bucket == watch.bucket && Arc::ptr_eq(&item.generation, &watch.generation)
        }) else {
            return Ok(None);
        };
        let item = self
            .registry_ingress
            .queue
            .remove(index)
            .expect("bounded queue index");
        self.authenticate_registry(&item.sealed, watch.bucket)?;
        Ok(Some(work(
            &self.group,
            &self.device,
            &mut self.rng,
            &item.sealed,
        )))
    }
}

#[cfg(test)]
mod tests;
