//! Admitted-call key publication and profile-neutral room discovery only.
//!
//! This owner deliberately contains no legacy packet ledger, confirmation queue or
//! crypto action factory. Tags1–3 retain their existing signed bytes: a FreshPeer
//! from this watcher proves liveness, never remote admitted support or round consent.
use super::call_activation::{ActivationObserver, Funding, RootResidence};
use super::limits::OriginLimits;
use super::presence::{FreshPeer, WatchAuthority, Watcher};
use super::resources::{Credit, ResourceKind, Resources};
use super::source::{AdmittedDiscoveryActivity, SourceActivity};
use super::*;
use catcoms_rt::{Clock, CryptoRngCore};
use std::sync::Arc;

/// Complete boxed presence, its implicit watcher, lease allocation and publication
/// result overlap. Cache records retain their original independent1536-byte fees.
pub(crate) const PRESENCE_BYTES: usize = 9 * 1024;
const RECORD_BYTES: usize = 1536;
const MAX_PENDING: usize = 32;
const RENEW_MS: u64 = 5000;

/// Keep this full retained/construction graph in the production preallocation
/// fence as well as the regression. Pinned Tokio1.53.3 watch::Shared layout uses
/// the same conservative convention as the activation graph; intrusive waiters
/// remain in their separately paid RPC/Lower futures. No inventory was trimmed
/// when the measured Windows graph exceeded the initial8KiB fee.
fn inventory_bytes() -> (usize, usize) {
    use std::mem::size_of;
    let watch_shared = 64 + 4 * size_of::<usize>() + 9 * size_of::<tokio::sync::Notify>() + 64;
    let lease_allocation = size_of::<super::LeaseState>() + 2 * size_of::<usize>() + watch_shared;
    let presence = 2 * size_of::<AdmittedPresence>() // prepared value/Box publication overlap
        + size_of::<PresenceStorage>() + lease_allocation
        + Funding::allocation_bytes()
        + size_of::<AdmittedPresenceReceipt>() + size_of::<Result<AdmittedPresenceReceipt, VoiceError>>()
        + MAX_PENDING * size_of::<CachedProof>() // fixed prepaid Vec backing, no growth
        + 2 * size_of::<SignedObject>() // shared response clone/publication overlap
        + 128;
    let proof = size_of::<ProofBody>() + 128;
    (presence, proof)
}

/// Historical proof of exact native key installation, not present-time authority.
/// End/epoch replacement can invalidate positive use immediately after this return.
/// Native must retain its original Handle and recheck it before replying positively.
pub struct AdmittedPresenceReceipt {
    // Core already owns the exact Weak<CallGrant>. Reuse that paid identity;
    // a second raw Weak here could outlive the final original funding alias.
    original: ActivationObserver,
    key: JoinKey,
}
impl AdmittedPresenceReceipt {
    pub fn matches(
        &self,
        original: &ActivationObserver,
        grant: &Arc<CallGrant>,
        join: Nonce,
        point: [u8; 65],
    ) -> bool {
        self.original.is_same(original)
            && original.matches_grant(grant)
            && self.key.join == join
            && self.key.public_key == point
    }
    pub(crate) fn installed(original: &ActivationObserver, key: &JoinKey) -> Self {
        Self {
            original: original.clone(),
            key: key.clone(),
        }
    }
}

/// The original plain payment lives in a separate shared Funding allocation,
/// OUTSIDE the Box. Escaping private watcher-child aliases carry that SAME payer
/// through their final lease/watch allocation deallocation, even after Root return.
pub(crate) struct PresenceStorage {
    body: Option<Box<AdmittedPresence>>,
    _funding: Arc<Funding>,
}
impl PresenceStorage {
    pub(crate) fn reserve(original: &ActivationObserver) -> Result<Arc<Funding>, VoiceError> {
        Self::reserve_checked(original, PRESENCE_BYTES, RECORD_BYTES)
    }
    fn reserve_checked(
        original: &ActivationObserver,
        presence_limit: usize,
        proof_limit: usize,
    ) -> Result<Arc<Funding>, VoiceError> {
        let (presence, proof) = inventory_bytes();
        if std::mem::size_of::<std::sync::RwLock<bool>>() > 64
            || presence > presence_limit
            || proof > proof_limit
        {
            return Err(VoiceError::Bounds);
        }
        // Refuse BEFORE charging or allocating Funding/lease/Watcher/Vec/Box.
        Funding::reserve_presence(original)
    }
    pub(crate) fn new(body: AdmittedPresence, funding: Arc<Funding>) -> Self {
        Self {
            body: Some(Box::new(body)),
            _funding: funding,
        }
    }
    pub(crate) fn get(&self) -> &AdmittedPresence {
        self.body.as_ref().expect("original admitted presence")
    }
    pub(crate) fn get_mut(&mut self) -> &mut AdmittedPresence {
        self.body.as_mut().expect("original admitted presence")
    }
}
impl Drop for PresenceStorage {
    fn drop(&mut self) {
        drop(self.body.take());
        // `_funding` drops only after Box::drop has freed its allocation. Escaping
        // watcher-child aliases may still retain the same original payment.
    }
}

struct ProofBody {
    challenge: SignedObject,
    proof: SignedObject,
    deadline: u64,
}
struct CachedProof {
    body: Option<Box<ProofBody>>,
    _credit: Credit,
}
impl CachedProof {
    fn get(&self) -> &ProofBody {
        self.body.as_ref().expect("original discovery proof")
    }
}
impl Drop for CachedProof {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}
pub(crate) struct AdmittedPresence {
    id: u64,
    scope: Scope,
    local: DeviceId,
    key: JoinKey,
    lease: CallLease,
    watch: WatchLease,
    watcher: Watcher,
    resources: Arc<Resources>,
    renewal: u64,
    next_renewal: u64,
    announcement: Option<SignedObject>,
    // Fixed capacity is paid by PresenceStorage before allocation. Each record's
    // SAME plain payment remains outside its own Box through actual deallocation.
    proofs: Vec<CachedProof>,
}
impl AdmittedPresence {
    #[allow(clippy::too_many_arguments)] // Exact residence, tuple and independent watcher/RNG inputs stay explicit.
    pub(crate) fn new(
        residence: &RootResidence,
        context: &Context,
        id: u64,
        key: JoinKey,
        funding: &Arc<Funding>,
        resources: Arc<Resources>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, VoiceError> {
        let original = residence.original();
        if !original.is_active() || original.scope() != context.scope {
            return Err(VoiceError::Unauthorized);
        }
        key.validate()?;
        let watch = residence.watch_lease(funding.clone())?;
        let watcher = Watcher::new(context, watch.clone(), resources.clone(), rng)?;
        if !original.is_active() || !watch.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Self {
            id,
            scope: context.scope,
            local: context.local,
            key,
            lease: original.original_lease().clone(),
            watch,
            watcher,
            resources,
            renewal: 0,
            next_renewal: 0,
            announcement: None,
            proofs: Vec::with_capacity(MAX_PENDING),
        })
    }
    pub(crate) fn id(&self) -> u64 {
        self.id
    }
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }
    pub(crate) fn key(&self) -> &JoinKey {
        &self.key
    }
    pub(crate) fn watch(&self) -> &WatchLease {
        &self.watch
    }
    pub(crate) fn lease(&self) -> &CallLease {
        &self.lease
    }
    pub(crate) fn is_live(&self) -> bool {
        self.lease.admitted_active() && self.watch.is_live()
    }
    pub(crate) fn targets_proof(&self, object: &SignedObject) -> bool {
        self.is_live() && self.watcher.targets_proof(object)
    }
    fn current(
        &self,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
    ) -> Result<(), VoiceError> {
        if !self.is_live()
            || context.scope != self.scope
            || context.local != self.local
            || !member(self.local)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    fn signer(&self, signer: &dyn VoiceSigner) -> Result<(), VoiceError> {
        if DeviceId::from_public_key_bytes(&signer.voice_public_key()?) != self.local {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    pub(crate) fn activity(&self) -> Result<AdmittedDiscoveryActivity, VoiceError> {
        if !self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(AdmittedDiscoveryActivity::new(SourceActivity::joined(
            self.scope,
            self.local,
            self.key.clone(),
            self.lease.clone(),
        )))
    }
    pub(crate) fn announce(
        &mut self,
        signer: &dyn VoiceSigner,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SignedObject, VoiceError> {
        self.current(context, member)?;
        self.signer(signer)?;
        if let Some(object) = &self.announcement {
            if clock.monotonic_ms() < self.next_renewal
                && object.check_freshness(clock.now_ms()).is_ok()
                && matches!(object.body(), Body::Announcement { epoch, .. } if *epoch == context.epoch)
            {
                self.current(context, member)?;
                return Ok(object.clone());
            }
        }
        let renewal = self.renewal.checked_add(1).ok_or(VoiceError::Capacity)?;
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        let issued = clock.now_ms();
        let expires = issued
            .checked_add(MAX_DELEGATION_LIFETIME_MS)
            .ok_or(VoiceError::Freshness)?;
        let object = SignedObject::issue(
            self.scope,
            signer,
            id,
            issued,
            expires,
            Body::Announcement {
                key: self.key.clone(),
                epoch: context.epoch,
                renewal,
            },
        )?;
        let next = clock.monotonic_ms().saturating_add(RENEW_MS);
        self.current(context, member)?;
        self.renewal = renewal;
        self.next_renewal = next;
        self.announcement = Some(object.clone());
        Ok(object)
    }
    pub(crate) fn answer(
        &mut self,
        challenge: &SignedObject,
        signer: &dyn VoiceSigner,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<SignedObject, VoiceError> {
        self.current(context, member)?;
        self.signer(signer)?;
        if challenge.scope() != self.scope {
            return Err(VoiceError::Scope);
        }
        challenge.check_freshness(clock.now_ms())?;
        let Body::WatchChallenge {
            watcher,
            target,
            join,
            key_ref,
            challenge: nonce,
        } = challenge.body()
        else {
            return Err(VoiceError::Malformed);
        };
        if *target != self.local
            || *join != self.key.join
            || *key_ref != self.key.key_ref
            || !member(challenge.issuer())
        {
            return Err(VoiceError::Unauthorized);
        }
        let mono = clock.monotonic_ms();
        self.proofs.retain(|cached| {
            mono < cached.get().deadline && member(cached.get().challenge.issuer())
        });
        let cache_id = (challenge.issuer(), challenge.id());
        if let Some(cached) = self
            .proofs
            .iter()
            .map(CachedProof::get)
            .find(|cached| (cached.challenge.issuer(), cached.challenge.id()) == cache_id)
        {
            if cached.challenge != *challenge {
                return Err(VoiceError::RetryMismatch);
            }
            cached.proof.check_freshness(clock.now_ms())?;
            self.current(context, member)?;
            return Ok(cached.proof.clone());
        }
        if self.proofs.len() >= MAX_PENDING {
            return Err(VoiceError::Capacity);
        }
        let credit =
            self.resources
                .charge(self.scope.group, RECORD_BYTES, ResourceKind::Retained)?;
        let issued = clock.now_ms();
        let expires = challenge
            .expires()
            .min(issued.saturating_add(MAX_PACKET_LIFETIME_MS));
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        let proof = SignedObject::issue(
            self.scope,
            signer,
            id,
            issued,
            expires,
            Body::JoinProof {
                join: self.key.join,
                key_ref: self.key.key_ref,
                watcher: challenge.issuer(),
                watcher_incarnation: *watcher,
                challenge: *nonce,
                challenge_id: challenge.id(),
            },
        )?;
        let deadline = clock
            .monotonic_ms()
            .saturating_add(expires.saturating_sub(issued).saturating_add(CLOCK_SKEW_MS));
        self.current(context, member)?;
        self.proofs.push(CachedProof {
            body: Some(Box::new(ProofBody {
                challenge: challenge.clone(),
                proof: proof.clone(),
                deadline,
            })),
            _credit: credit,
        });
        Ok(proof)
    }
    #[allow(clippy::too_many_arguments)] // Existing shared discovery takes current context/member/limits separately.
    pub(crate) fn challenge(
        &mut self,
        object: &SignedObject,
        signer: &dyn VoiceSigner,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
        limits: &mut OriginLimits,
        clock: &dyn Clock,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(SignedObject, SourceActivity), VoiceError> {
        self.current(context, member)?;
        let authority = WatchAuthority {
            context,
            lease: &self.watch,
            member,
        };
        let response = self
            .watcher
            .challenge(object, signer, &authority, limits, clock, rng)?;
        self.current(context, member)?;
        Ok((response, self.watcher.source_activity(&authority)?))
    }
    pub(crate) fn prove(
        &mut self,
        object: &SignedObject,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
        clock: &dyn Clock,
    ) -> Result<(), VoiceError> {
        self.current(context, member)?;
        self.watcher.prove(
            object,
            &WatchAuthority {
                context,
                lease: &self.watch,
                member,
            },
            clock,
        )?;
        self.current(context, member)
    }
    pub(crate) fn peers(
        &mut self,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
        clock: &dyn Clock,
    ) -> Result<Vec<FreshPeer>, VoiceError> {
        self.current(context, member)?;
        let peers = self.watcher.peers(
            &WatchAuthority {
                context,
                lease: &self.watch,
                member,
            },
            clock,
        )?;
        self.current(context, member)?;
        Ok(peers)
    }
    pub(crate) fn collect_expired(&mut self, clock: &dyn Clock) -> bool {
        let mono = clock.monotonic_ms();
        self.proofs
            .retain(|cached| self.lease.admitted_active() && mono < cached.get().deadline);
        self.watcher.collect_expired(clock)
    }
    pub(crate) fn retire_member(&mut self, member: DeviceId) {
        if member == self.local {
            // Called under Root's Group transition guard. The original admitted
            // issuer wake is deferred by that transition; do not run a new waker
            // here. Every watcher observation/source also checks this parent.
            self.lease.invalidate();
            self.watch.invalidate();
            return;
        }
        self.proofs
            .retain(|cached| cached.get().challenge.issuer() != member);
        self.watcher.retire_member(member);
    }
}
impl Drop for AdmittedPresence {
    fn drop(&mut self) {
        self.watch.revoke();
    }
}
redacted_debug!(AdmittedPresenceReceipt);

#[cfg(test)]
mod tests;
