//! Opt-in Studio gossip, separate from the legacy document map. The existing wire envelope,
//! blinded routing, subscription reconciler and one-shot transport are reused. Authentication
//! into this bounded inbox is not typed admission, byte possession, durability or delivery.

use super::*;
use catcoms_replication::epoch::MAX_SIGNED_EPOCH_OP_BYTES;
use catcoms_replication::studio::{FlipnoteOp, IndexOp, StudioTarget};
use catcoms_replication::DomainOp;
use catcoms_rt::PublishSubmission;
use registry_ingress::Rate;

const MAX_WATCHES: usize = 16;
const MAX_QUEUE: usize = 16;
const MAX_RATE_ROWS: usize = 4096;
const MAX_PACKET: usize = MAX_SIGNED_EPOCH_OP_BYTES + 78;
type Key = (DocType, [u8; 16]);

fn key(target: StudioTarget) -> Key {
    match target {
        StudioTarget::Index { channel } => (DocType::StudioIndex, channel),
        StudioTarget::Flipnote { object, .. } => (DocType::StudioObject, object),
    }
}

/// Exact local watch, not a wire or UI capability. Replacement drops queued packets immediately.
/// Callers derive the concrete epoch from their verified source, never from incoming traffic.
pub struct StudioWatch {
    target: StudioTarget,
    doc_id: u128,
    generation: Arc<()>,
    instance: RegistrySyncInstance,
}
impl fmt::Debug for StudioWatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StudioWatch { .. }")
    }
}
pub(super) struct Watched {
    target: StudioTarget,
    pub(super) doc_id: u128,
    generation: Arc<()>,
}
struct Incoming {
    key: Key,
    generation: Arc<()>,
    sealed: SealedOp,
}
#[derive(Default)]
pub(super) struct StudioExchange {
    pub(super) watches: BTreeMap<Key, Watched>,
    queue: VecDeque<Incoming>,
    preauth: Option<Rate>,
    authors: BTreeMap<(DeviceId, DocType, [u8; 16]), Rate>,
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Synchronous desired subscription; an explicit flush/run_once performs transport work.
    /// This trusted seam does not load a source or discover/authorize its physical epoch.
    pub fn watch_studio(
        &mut self,
        target: StudioTarget,
        doc_id: u128,
    ) -> Result<StudioWatch, SyncError> {
        let key = key(target);
        if !self.studio_exchange.watches.contains_key(&key)
            && self.studio_exchange.watches.len() >= MAX_WATCHES
        {
            return Err(SyncError::Malformed);
        }
        let generation = Arc::new(());
        self.studio_exchange.queue.retain(|item| item.key != key);
        self.studio_exchange.watches.insert(
            key,
            Watched {
                target,
                doc_id,
                generation: generation.clone(),
            },
        );
        self.needs_resync = true;
        Ok(StudioWatch {
            target,
            doc_id,
            generation,
            instance: self.registry_instance(),
        })
    }
    pub fn studio_watch_is_current(&self, watch: &StudioWatch) -> bool {
        self.matches_registry_instance(&watch.instance)
            && self
                .studio_exchange
                .watches
                .get(&key(watch.target))
                .is_some_and(|entry| {
                    entry.target == watch.target
                        && entry.doc_id == watch.doc_id
                        && Arc::ptr_eq(&entry.generation, &watch.generation)
                })
    }
    /// Drop alone does not unsubscribe. An old token cannot revoke a same-key replacement.
    /// Debt is keyed by full author/logical document and survives rewatch and epoch rotation.
    pub fn unwatch_studio(&mut self, watch: &StudioWatch) -> Result<(), SyncError> {
        if !self.studio_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        let key = key(watch.target);
        self.studio_exchange.watches.remove(&key);
        self.studio_exchange.queue.retain(|item| item.key != key);
        self.needs_resync = true;
        Ok(())
    }
    pub async fn flush_studio_subscriptions(&mut self) -> Result<(), SyncError> {
        self.resync_subscriptions().await
    }

    /// Always intercept Studio tags, even when unopened or malformed. They must never reach
    /// the ungated legacy document map, its accepted counters or its delivery acknowledgements.
    pub(super) fn on_studio_gossip(&mut self, topic: &Topic, data: &[u8]) -> bool {
        if ![DocType::StudioIndex, DocType::StudioObject]
            .iter()
            .any(|t| data.starts_with(&t.tag().to_be_bytes()))
        {
            return false;
        }
        let now = self.clock.monotonic_ms();
        let inbox = &mut self.studio_exchange;
        if data.len() > MAX_PACKET
            || inbox.watches.is_empty()
            || inbox.queue.len() >= MAX_QUEUE
            || !inbox
                .preauth
                .get_or_insert_with(|| Rate::full(now, 200))
                .charge(now, 50, 200)
        {
            return true;
        }
        let Ok(sealed) = SealedOp::decode(data) else {
            return true;
        };
        let Some((&key, watch)) = self
            .studio_exchange
            .watches
            .iter()
            .find(|(key, entry)| key.0 == sealed.doc_type && entry.doc_id == sealed.doc_id)
        else {
            return true;
        };
        let target = watch.target;
        let generation = watch.generation.clone();
        if !self.window_labels().any(|slot| {
            self.channel_topic_for(sealed.doc_type, sealed.doc_id, slot)
                .as_ref()
                == Some(topic)
        }) {
            return true;
        }
        let Ok(author) = self.authenticate_studio(&sealed, target) else {
            return true;
        };
        let rates = &mut self.studio_exchange.authors;
        rates.retain(|_, rate| {
            rate.refill(now, 10, 50);
            !rate.is_full(50)
        });
        let author_key = (author, key.0, key.1);
        if !rates.contains_key(&author_key) && rates.len() >= MAX_RATE_ROWS {
            return true;
        }
        if !rates
            .entry(author_key)
            .or_insert_with(|| Rate::full(now, 50))
            .charge(now, 10, 50)
        {
            return true;
        }
        self.studio_exchange.queue.push_back(Incoming {
            key,
            generation,
            sealed,
        });
        true
    }

    /// Cheap scope/signature/body checks only. Causal change validation, channel-bound roots,
    /// prospective projection limits and durable accounting remain the store gate's job.
    fn authenticate_studio(
        &self,
        sealed: &SealedOp,
        target: StudioTarget,
    ) -> Result<DeviceId, SyncError> {
        let logical = target.document(&self.group.group_id())?;
        if sealed.doc_type != logical.doc_type
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
        let secret = self
            .group
            .channel_secret(&self.device, sealed.doc_type, sealed.doc_id)?;
        let signed = sealed.open(&secret)?;
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
        match target {
            StudioTarget::Index { .. } => {
                IndexOp::decode_domain(&logical, &domain, &signed.author_device)?;
            }
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::decode_domain(&logical, &domain)?;
            }
        }
        Ok(signed.author_device)
    }

    /// Consume at most one volatile packet; reauthenticate after queue delay before storage I/O.
    /// Failure consumes no durable intent and emits no delivery acknowledgement.
    pub fn drain_studio_inbound<O>(
        &mut self,
        watch: &StudioWatch,
        work: impl FnOnce(&ServerGroup, &MlsDevice, &mut R, &SealedOp) -> O,
    ) -> Result<Option<O>, SyncError> {
        if !self.studio_watch_is_current(watch) {
            return Err(SyncError::NoSuchDoc);
        }
        let Some(index) = self.studio_exchange.queue.iter().position(|item| {
            item.key == key(watch.target) && Arc::ptr_eq(&item.generation, &watch.generation)
        }) else {
            return Ok(None);
        };
        let item = self
            .studio_exchange
            .queue
            .remove(index)
            .expect("bounded inbox index");
        self.authenticate_studio(&item.sealed, watch.target)?;
        Ok(Some(work(
            &self.group,
            &self.device,
            &mut self.rng,
            &item.sealed,
        )))
    }

    /// One current own-author attempt from a trusted durable adapter, not a packet retry queue.
    /// The caller must keep exclusive source/lifecycle custody through await and recheck the
    /// saved Open source before every attempt. Submitted/Duplicate do not mean peer delivery.
    pub async fn publish_local_studio_once(
        &mut self,
        target: StudioTarget,
        expected_doc_id: u128,
        sealed: SealedOp,
    ) -> Result<PublishSubmission, SyncError> {
        if sealed.doc_id != expected_doc_id {
            return Err(SyncError::Malformed);
        }
        if self.authenticate_studio(&sealed, target)? != self.device.device_id() {
            return Err(SyncError::Unauthorized);
        }
        let topic = self
            .channel_topic_for(sealed.doc_type, expected_doc_id, self.routing_label)
            .ok_or(SyncError::NoSuchDoc)?;
        Ok(self
            .transport
            .publish_once(topic, Bytes::from(sealed.encode()))
            .await?)
    }
}

#[cfg(test)]
mod tests;
