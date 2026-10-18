//! Native endpoint lifecycle and public controls. Normal kind28 dispatch stays disabled until encrypted ingress,
//! recoverable pull/confirmation/negotiation and the browser adapter are integrated and reviewed.
//! The sole Root owns every Watcher/LocalJoin; detached jobs borrow only revocable capabilities.
use super::*;
use voice_v2::presence::{LocalJoin, Watcher};
use voice_v2::resources::{Credit, ResourceKind};
use voice_v2::{CallGrant, CallLease, JoinKey, Scope, VoiceError, WatchLease};

const WATCHES: usize = 8;
mod ingress;

/// Affine native reply custody for one observer. The Root holds no strong reference to this
/// grant: losing its reply/last native owner closes observation even if the stop command is lost.
/// Parent consent is bound before any lease clone escapes the factory.
pub struct VoiceWatchGrant {
    id: u64,
    scope: Scope,
    lease: WatchLease,
    instance: Arc<()>,
    _credit: Credit,
}
impl fmt::Debug for VoiceWatchGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VoiceWatchGrant { .. }")
    }
}
impl VoiceWatchGrant {
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub fn is_live(&self) -> bool {
        self.lease.is_live()
    }
    pub fn revoke(&self) {
        self.lease.revoke();
    }
}
impl Drop for VoiceWatchGrant {
    fn drop(&mut self) {
        self.lease.revoke();
    }
}
struct Observer {
    scope: Scope,
    lease: WatchLease,
    watcher: Watcher,
}
struct Participant {
    // Only the native caller owns affine call custody. Root may recognize that exact owner,
    // but retaining a strong Arc here would keep an abandoned native call alive indefinitely.
    grant: std::sync::Weak<CallGrant>,
    lease: CallLease,
    watch: Arc<VoiceWatchGrant>,
    join: LocalJoin,
}
pub(super) struct EndpointOwners {
    serial: u64,
    observers: BTreeMap<u64, Observer>,
    participant: Option<Participant>,
    revision: tokio::sync::watch::Sender<u64>,
    public: ingress::PublicInbox,
}
impl Default for EndpointOwners {
    fn default() -> Self {
        Self {
            serial: 0,
            observers: BTreeMap::new(),
            participant: None,
            revision: tokio::sync::watch::channel(0).0,
            public: ingress::PublicInbox::default(),
        }
    }
}
impl EndpointOwners {
    pub(super) fn collect_expired(&mut self, clock: &dyn Clock) {
        self.prune();
        self.public.prune(clock);
        let mut changed = false;
        for observer in self.observers.values_mut() {
            changed |= observer.watcher.collect_expired(clock);
        }
        if let Some(participant) = &mut self.participant {
            participant.join.collect_expired(clock);
        }
        if changed {
            self.changed();
        }
    }
    #[cfg(test)]
    pub(crate) fn implicit_watch_for_test(&self) -> Option<Arc<VoiceWatchGrant>> {
        // Inspection only: this returns the already factory-issued implicit watcher. Tests do
        // not fabricate an observer, participant, route proof or native ownership capability.
        self.participant
            .as_ref()
            .map(|participant| participant.watch.clone())
    }
    fn changed(&self) {
        // A coalesced metadata wake cannot block the sole actor on a slow/missing UI consumer.
        // Actual pull payloads and exact input retry custody are separate integration work.
        self.revision
            .send_modify(|revision| *revision = revision.saturating_add(1));
    }
    pub(super) fn prune(&mut self) {
        let before = self.observers.len();
        self.observers
            .retain(|_, observer| observer.lease.is_live());
        let retired = self.participant.as_ref().is_some_and(|participant| {
            participant.grant.strong_count() == 0
                || !participant.lease.is_live()
                || !participant.watch.is_live()
        });
        if retired {
            self.participant = None;
        }
        if retired || before != self.observers.len() {
            self.changed();
        }
    }
    pub(super) fn retire_member(&mut self, member: DeviceId) {
        for observer in self.observers.values_mut() {
            observer.watcher.retire_member(member);
        }
        if let Some(participant) = &mut self.participant {
            participant.join.retire_member(member);
        }
        self.changed();
    }
    fn recognize(&self, grant: &VoiceWatchGrant, instance: &Arc<()>) -> Result<(), VoiceError> {
        let observer = self
            .observers
            .get(&grant.id)
            .ok_or(VoiceError::Unauthorized)?;
        if !Arc::ptr_eq(instance, &grant.instance)
            || !grant.lease.is_live()
            || !observer.lease.is_same(&grant.lease)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Trusted native factory. Parent is the actual controller for an explicit observer, or the
    /// actual call grant for an implicit call watcher. Caller rechecks native UI binding after
    /// awaits; the affine reply closes on abandonment without relying on another actor command.
    pub fn start_voice_watch(
        &mut self,
        room: u128,
        parent: CallLease,
    ) -> Result<Arc<VoiceWatchGrant>, VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        let context = self.voice_v2_context(room)?;
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.endpoints.prune();
        if voice.endpoints.observers.len() >= WATCHES {
            return Err(VoiceError::Capacity);
        }
        let id = voice
            .endpoints
            .serial
            .checked_add(1)
            .ok_or(VoiceError::Reservation)?;
        let owner = voice.current.as_ref().ok_or(VoiceError::Unauthorized)?;
        let credit = voice
            .resources
            .charge(context.scope.group, 256, ResourceKind::Retained)?;
        let lease = voice.directory.watch_lease(owner, context.scope)?;
        let lease = match lease.clone().with_parent(parent) {
            Ok(lease) => lease,
            Err(error) => {
                lease.revoke();
                return Err(error);
            }
        };
        let grant = Arc::new(VoiceWatchGrant {
            id,
            scope: context.scope,
            lease: lease.clone(),
            instance: self.blob_fetch_instance.clone(),
            _credit: credit,
        });
        let watcher = Watcher::new(
            &context,
            lease.clone(),
            voice.resources.clone(),
            &mut self.rng,
        )?;
        // Publication follows every fallible step. A lost reply drops grant and revokes watcher.
        if !grant.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        voice.endpoints.serial = id;
        voice.endpoints.observers.insert(
            id,
            Observer {
                scope: context.scope,
                lease,
                watcher,
            },
        );
        voice.endpoints.changed();
        Ok(grant)
    }
    pub fn validate_voice_watch(&mut self, grant: &VoiceWatchGrant) -> Result<(), VoiceError> {
        self.reconcile_voice_actor();
        self.observe_voice_directory();
        if self.voice_v2_context(grant.scope.room)?.scope != grant.scope {
            return Err(VoiceError::Scope);
        }
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        voice.directory.authority(grant.scope)?;
        voice.endpoints.recognize(grant, &self.blob_fetch_instance)
    }
    /// Install only the recognized current call grant and its native join nonce. The browser
    /// contributes a structurally checked SEC1 public key; the WebView's P-256 import must also
    /// validate the curve point before derivation/confirmation. Its private key stays there. Exact
    /// retries are idempotent and cannot allocate a second ledger or change a join's key.
    pub fn start_voice_endpoint(
        &mut self,
        grant: Arc<CallGrant>,
        join: voice_v2::Nonce,
        public_key: [u8; 65],
    ) -> Result<(), VoiceError> {
        self.validate_voice_call_grant(&grant)?;
        let key = JoinKey::new(join, public_key)?;
        let context = self.voice_v2_context(grant.scope().room)?;
        let voice = self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if let Some(participant) = &voice.endpoints.participant {
            if std::sync::Weak::ptr_eq(&participant.grant, &Arc::downgrade(&grant)) {
                return if participant.join.key() == &key {
                    Ok(())
                } else {
                    Err(VoiceError::RetryMismatch)
                };
            }
        }
        let watch = self.start_voice_watch(context.scope.room, grant.lease())?;
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        let local = LocalJoin::new(&context, key, grant.lease(), voice.resources.clone())?;
        if !grant.lease().is_live() || !watch.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        voice.endpoints.participant = Some(Participant {
            grant: Arc::downgrade(&grant),
            lease: grant.lease(),
            watch,
            join: local,
        });
        voice.endpoints.prune();
        voice.endpoints.changed();
        Ok(())
    }
    pub fn voice_endpoint_changes(
        &mut self,
    ) -> Result<tokio::sync::watch::Receiver<u64>, VoiceError> {
        self.reconcile_voice_actor();
        self.voice_transport_actor()?.helper()?;
        Ok(self
            .voice_runtime
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .endpoints
            .revision
            .subscribe())
    }
}

#[cfg(test)]
mod tests;
