//! Detached connected-only source/helper flights. The sole sync owner signs and validates; workers
//! have immutable bytes, a revocable route/packet authority and actual lower-driver custody.
use super::*;
use catcoms_rt::{RequestAuthority, SharedRequestKeepalive};
use futures::FutureExt;
use voice_v2::helper::{ForwardJob, HelperLedger};
use voice_v2::source::{SourceActivity, SourceHop, SourceJob, SourceLedger, SourceOutcome};
use voice_v2::CallLease;

const ROUTES: usize = 64;
const DEADLINE_MS: u64 = 3_000;
const REGISTRY_BYTES: usize = 4096;

pub(crate) struct RouteOwners {
    active: Vec<RouteRow>,
    // Plain, outside the Vec allocation. First Root attachment reserves this
    // before VoiceRuntime construction, including this owner's inline growth.
    _funding: voice_v2::resources::Credit,
}
struct RouteRow {
    gate: Option<Arc<RouteGate>>,
    // Only the ledger Flight or actual ingress Credit: the outer transport
    // FlightAuthority owns RouteGate and would create a backedge here.
    _custody: SharedRequestKeepalive,
}
impl RouteRow {
    fn gate(&self) -> &Arc<RouteGate> {
        self.gate.as_ref().expect("original route row")
    }
}
impl Drop for RouteRow {
    fn drop(&mut self) {
        // Registry owns the final Arc while detaching this row. No Weak escapes,
        // so another thread cannot acquire a new strong owner after sole-owner
        // classification. Keep the SAME payer through body AND Arc deallocation.
        drop(self.gate.take());
    }
}
impl RouteOwners {
    pub(crate) fn new(
        resources: &Arc<voice_v2::resources::Resources>,
        group: [u8; 32],
    ) -> Result<Self, VoiceError> {
        if Self::graph_bytes() > REGISTRY_BYTES
            || Self::route_metadata_bytes() > voice_v2::ROUTE_METADATA_BYTES
        {
            return Err(VoiceError::Bounds);
        }
        let funding = resources.charge(
            group,
            REGISTRY_BYTES,
            voice_v2::resources::ResourceKind::Retained,
        )?;
        Ok(Self {
            active: Vec::with_capacity(ROUTES),
            _funding: funding,
        })
    }
    fn graph_bytes() -> usize {
        // Full owner (including its new VoiceRuntime inline fields), constructor
        // overlap and exact fixed backing. Gate allocations retain their original
        // per-flight512-byte metadata payment, independently of this backing fee.
        2 * std::mem::size_of::<Self>() + ROUTES * std::mem::size_of::<RouteRow>() + 128
    }
    fn route_metadata_bytes() -> usize {
        std::mem::size_of::<RouteGate>()
            + 2 * std::mem::size_of::<usize>()
            + std::mem::size_of::<FlightAuthority>()
            + 2 * std::mem::size_of::<usize>()
            + std::mem::size_of::<RouteRow>()
            + 64
    }
    fn take_unused(&mut self) -> Option<RouteRow> {
        let index = self
            .active
            .iter()
            .position(|row| Arc::strong_count(row.gate()) == 1)?;
        Some(self.active.swap_remove(index))
    }
    pub(crate) fn prune(&mut self) {
        // Call only outside native/Group/Policy/Source guards. Detach first;
        // actual Arc/payer destructors run after the mutable Vec operation ends.
        while let Some(row) = self.take_unused() {
            drop(row);
        }
    }
    pub(crate) fn disconnected(&self, peer: PeerId) {
        for gate in self
            .active
            .iter()
            .map(RouteRow::gate)
            // Ordered events may lag a same-peer reconnect. The driver already revoked the
            // departed incarnation; an old event must not cancel a newly captured live owner.
            .filter(|gate| gate.peer == peer && !gate.connection.is_live())
        {
            gate.live.revoke();
        }
    }
}
impl Drop for RouteOwners {
    fn drop(&mut self) {
        for gate in self.active.iter().map(RouteRow::gate) {
            gate.live.revoke();
        }
        drop(std::mem::take(&mut self.active));
        // Actual Vec backing and row-owned Gate allocations are gone before the
        // plain registry funding drops. External request/reply owners can still
        // hold Gates; each retains the SAME payer after its authority instead.
    }
}
struct RouteGate {
    peer: PeerId,
    device: DeviceId,
    descriptor: [u8; 32],
    epoch: u64,
    policy: Option<[u8; 32]>,
    live: CallLease,
    connection: catcoms_rt::ConnectionLease,
    // The source/helper reserves these metadata bytes before spending its attempt. Retaining
    // the SAME flight also retains its phase/lane through any driver authority or completion;
    // cancellation alone cannot refund custody. Flight has no back-edge to this RouteGate.
    _custody: Option<SharedRequestKeepalive>,
    #[cfg(test)]
    before_deallocation: std::sync::Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
}
impl Drop for RouteGate {
    fn drop(&mut self) {
        drop(self._custody.take());
        #[cfg(test)]
        if let Some((entered, resume)) = self.before_deallocation.lock().unwrap().take() {
            let _ = entered.send(());
            let _ = resume.recv();
        }
        // Registry row's outside payer remains until Arc::drop has also freed
        // this allocation. The test pause models the old final-strong-Drop gap.
    }
}
impl fmt::Debug for RouteGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VoiceRouteGate { .. }")
    }
}
#[async_trait::async_trait]
impl RequestAuthority for RouteGate {
    fn authorized(&self) -> bool {
        self.live.is_live() && self.connection.is_live()
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        futures::future::select(
            self.live.wait_revoked().boxed(),
            self.connection.revoked().boxed(),
        )
        .await;
    }
}
struct FlightAuthority {
    route: Arc<RouteGate>,
    packet: Arc<dyn RequestAuthority>,
    clock: Arc<dyn Clock>,
    deadline: u64,
}
impl fmt::Debug for FlightAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VoiceForwardAuthority { .. }")
    }
}
#[async_trait::async_trait]
impl RequestAuthority for FlightAuthority {
    fn authorized(&self) -> bool {
        // Sample this outer Clock before invoking the packet's original clocks. Both are
        // injected callbacks; they may retire the Root, room or driver. Only then observe the
        // pure route/connection gates, so no later callback escapes an earlier owner check.
        let mono = self.clock.monotonic_ms();
        let packet = self.packet.authorized();
        packet
            && self.route.live.is_live()
            && self.route.connection.is_live()
            && mono < self.deadline
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let waits = vec![
            self.route.live.wait_revoked().boxed(),
            self.route.connection.revoked().boxed(),
            self.packet.revoked().boxed(),
            self.clock.sleep(std::time::Duration::from_millis(
                self.deadline.saturating_sub(self.clock.monotonic_ms()),
            )),
        ];
        futures::future::select_all(waits).await;
    }
}

// Both paths use the same driver ownership and ACK verifier. The source owns its original packet
// and the helper owns a delegated edge; their independent ledgers must never settle each other.
enum Work {
    Helper {
        job: ForwardJob,
        ledger: Arc<HelperLedger>,
    },
    Source {
        // Packet ciphertext remains in the original SourceJob. Only this immutable fixed
        // delegation is new; never clone a full SDP merely to remember its transport role.
        // Drop this Box BEFORE job's possibly sole Lower payment, including a preparation
        // refusal before RouteGate has captured another keepalive owner.
        permit: Option<Box<voice_v2::RoutePermit>>,
        job: SourceJob,
        ledger: Arc<SourceLedger>,
    },
}

/// Private provenance-bound ACK view. Only Work below constructs it from the SAME actual
/// immutable ledger job, so completion verification may reuse the already-computed hash.
/// A public ACK verifier or untrusted frame cannot pair a copied hash with another object.
pub(super) struct RetainedEnvelope<'a> {
    envelope: voice_v2::EnvelopeRef<'a>,
    hash: [u8; 32],
    progress: voice_v2::source::SourceProgress<'a>,
}
impl RetainedEnvelope<'_> {
    pub(super) fn progress(&self) -> voice_v2::source::SourceProgress<'_> {
        self.progress
    }
    pub(super) fn object(&self) -> &voice_v2::SignedObject {
        self.envelope.object()
    }
    pub(super) fn hash(&self) -> [u8; 32] {
        self.hash
    }
    pub(super) fn authorize(
        &self,
        requester: DeviceId,
        context: &voice_v2::Context,
        member: impl Fn(DeviceId) -> bool,
    ) -> Result<voice_v2::Disposition, VoiceError> {
        self.envelope
            .authorize_retained(requester, context, member, self.hash)
    }
}
impl Work {
    fn retained(&self) -> RetainedEnvelope<'_> {
        RetainedEnvelope {
            envelope: self.envelope(),
            hash: self.object_hash(),
            progress: match self {
                Self::Source { job, .. } => job.progress(),
                Self::Helper { .. } => voice_v2::source::SourceProgress::Ordinary,
            },
        }
    }
    fn peer(&self) -> PeerId {
        match self {
            Self::Helper { job, .. } => job.peer(),
            Self::Source { job, .. } => job.peer(),
        }
    }
    fn target(&self) -> DeviceId {
        match self {
            Self::Helper { job, .. } => job.target(),
            Self::Source { job, .. } => job.hop().device(),
        }
    }
    fn envelope(&self) -> voice_v2::EnvelopeRef<'_> {
        match self {
            Self::Helper { job, .. } => job.envelope().as_ref(),
            Self::Source {
                job, permit: None, ..
            } => voice_v2::EnvelopeRef::Direct(job.object()),
            Self::Source {
                job,
                permit: Some(permit),
                ..
            } => voice_v2::EnvelopeRef::ToHelper {
                object: job.object(),
                permit,
            },
        }
    }
    fn object_hash(&self) -> [u8; 32] {
        match self {
            Self::Helper { job, .. } => job.object_hash(),
            Self::Source { job, .. } => job.object_hash(),
        }
    }
    fn authority(&self) -> Arc<dyn RequestAuthority> {
        match self {
            Self::Helper { job, .. } => job.authority(),
            Self::Source { job, .. } => job.authority(),
        }
    }
    fn keepalive(&self) -> SharedRequestKeepalive {
        match self {
            Self::Helper { job, .. } => job.keepalive(),
            Self::Source { job, .. } => job.keepalive(),
        }
    }
    fn current(&self, actor: &voice_v2::runtime::ActorOwner) -> Result<(), VoiceError> {
        let current = match self {
            Self::Helper { ledger, .. } => Arc::ptr_eq(ledger, &actor.helper()?),
            Self::Source { ledger, .. } => Arc::ptr_eq(ledger, &actor.source()?),
        };
        if current {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
    fn complete(self, status: Option<Status>) -> Result<Option<SourceOutcome>, VoiceError> {
        match self {
            Self::Helper { job, ledger } => {
                ledger.complete(job.finish(status == Some(Status::EndpointAdmitted)))?;
                Ok(None)
            }
            Self::Source { job, ledger, .. } => ledger.complete(job.finish(status)).map(Some),
        }
    }
}

/// One immutable helper request; neither signing keys nor mutable MLS/product owners leave the
/// actor. Dropping an unsubmitted preparation conservatively burns its already reserved attempt.
pub struct PendingVoiceForward<T: MeshTransport> {
    transport: Arc<T>,
    request: Bytes,
    job: Work,
    authority: Arc<FlightAuthority>,
    instance: Arc<()>,
    // Last even on an abandoned, unexecuted preparation after Root retirement:
    // `job` alone drops before `authority` and cannot sponsor its final Arc tail.
    _custody: SharedRequestKeepalive,
}
/// Bounded untrusted ACK plus the exact affine request/edge it can complete.
pub struct CompletedVoiceForward {
    request: Bytes,
    job: Work,
    authority: Arc<FlightAuthority>,
    instance: Arc<()>,
    response: Result<Bytes, TransportError>,
    _custody: SharedRequestKeepalive,
}
/// Native source submission with the same connected-driver custody as delegated work. The
/// wrapper keeps source completion distinct from a helper edge at the public Rust boundary.
pub struct PendingVoiceSource<T: MeshTransport>(PendingVoiceForward<T>);
pub struct CompletedVoiceSource(CompletedVoiceForward);

/// A local verification-capacity refusal preserves the exact affine completion, its actual
/// flight/copy credits and original deadline. The actor retries it fairly without another send,
/// signature, nonce or attempt debit. Other errors are terminal and carry no retry owner.
pub struct VoiceCompletionFailure<T> {
    error: VoiceError,
    retry: Option<Box<T>>,
}
impl<T> fmt::Debug for VoiceCompletionFailure<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VoiceCompletionFailure")
            .field("error", &self.error)
            .field("retryable", &self.retry.is_some())
            .finish()
    }
}
impl<T> VoiceCompletionFailure<T> {
    fn terminal(error: VoiceError) -> Self {
        Self { error, retry: None }
    }
    fn map_retry<U>(self, map: impl FnOnce(T) -> U) -> VoiceCompletionFailure<U> {
        VoiceCompletionFailure {
            error: self.error,
            retry: self.retry.map(|retry| Box::new(map(*retry))),
        }
    }
    pub fn error(&self) -> &VoiceError {
        &self.error
    }
    pub fn into_retry(self) -> Option<T> {
        self.retry.map(|retry| *retry)
    }
    /// Explicitly abandon any retry custody. Callers normally use this only for terminal errors.
    pub fn into_error(self) -> VoiceError {
        self.error
    }
}
impl<T: MeshTransport> fmt::Debug for PendingVoiceSource<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingVoiceSource { .. }")
    }
}
impl fmt::Debug for CompletedVoiceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedVoiceSource { .. }")
    }
}
impl<T: MeshTransport> PendingVoiceSource<T> {
    #[cfg(test)]
    pub(crate) fn keepalive_for_test(&self) -> SharedRequestKeepalive {
        // Observe the SAME actual prepared flight, as a retained transport cancellation or
        // completed output does. Never fabricate a separate credit or retirement witness.
        self.0.job.keepalive()
    }
    pub fn peer(&self) -> PeerId {
        self.0.peer()
    }
    pub async fn execute(self, signal: tokio::sync::watch::Receiver<bool>) -> CompletedVoiceSource {
        CompletedVoiceSource(self.0.execute(signal).await)
    }
}
impl<T: MeshTransport> fmt::Debug for PendingVoiceForward<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingVoiceForward { .. }")
    }
}
impl fmt::Debug for CompletedVoiceForward {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompletedVoiceForward { .. }")
    }
}
impl<T: MeshTransport> PendingVoiceForward<T> {
    #[cfg(test)]
    pub(crate) fn authority_with_clock_for_test(
        &mut self,
        clock: Arc<dyn Clock>,
    ) -> Arc<dyn RequestAuthority> {
        // Keep the actual prepared job, route, connection and custody. Isolate the outer Clock
        // callback from packet clocks at the same ManualClock origin, before any driver clone.
        Arc::get_mut(&mut self.authority)
            .expect("unsubmitted native fixture")
            .clock = clock;
        self.authority.clone()
    }
    pub fn peer(&self) -> PeerId {
        self.job.peer()
    }
    /// Cancellation and deadlines stop upper work. The transport owns this same flight's credit
    /// and in-flight flag until its command/actual request retires, including uncertain timeout.
    pub async fn execute(
        self,
        signal: tokio::sync::watch::Receiver<bool>,
    ) -> CompletedVoiceForward {
        let custody = self.job.keepalive();
        let cancellation = RequestCancellation::new(signal, Some(custody.clone()))
            .with_authority(self.authority.clone());
        let response = match futures::future::select(
            Box::pin(self.transport.request_connected_cancellable(
                self.job.peer(),
                ProtocolId(RR_PROTOCOL),
                self.request.clone(),
                cancellation,
            )),
            Box::pin(self.authority.revoked()),
        )
        .await
        {
            futures::future::Either::Left((response, _)) => response,
            futures::future::Either::Right(((), _)) => Err(TransportError::Cancelled),
        }
        .and_then(|response| {
            if response.len() <= 512 {
                Ok(response)
            } else {
                Err(TransportError::NoResponse)
            }
        });
        CompletedVoiceForward {
            request: self.request,
            job: self.job,
            authority: self.authority,
            instance: self.instance,
            response,
            _custody: custody,
        }
    }
}
impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Register an inbound reply's original route BEFORE packet/origin admission. The fixed
    /// verification allowance covers this small metadata owner. Root control hooks retire it
    /// for epoch/proof/descriptor/policy changes, while the driver owns connection retirement.
    /// A later retry captures a new gate; it cannot repair this reply's original authority.
    pub(crate) fn capture_voice_reply_route(
        &mut self,
        route: BoundRoute,
        custody: SharedRequestKeepalive,
    ) -> Result<Arc<dyn RequestAuthority>, VoiceError> {
        self.capture_voice_route(route, custody)
            .map(|gate| gate as Arc<dyn RequestAuthority>)
    }
    fn capture_voice_route(
        &mut self,
        route: BoundRoute,
        custody: SharedRequestKeepalive,
    ) -> Result<Arc<RouteGate>, VoiceError> {
        let context = self.voice_v2_context(0)?;
        let descriptor = peer_descriptor_hash(
            self.peer_records
                .get(&route.device)
                .ok_or(VoiceError::Unauthorized)?,
        );
        let connection = self
            .transport
            .connection_lease(route.peer)
            .filter(|lease| lease.is_live())
            .ok_or(VoiceError::Unauthorized)?;
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        voice.routes.prune();
        if voice.routes.active.len() >= ROUTES {
            return Err(VoiceError::Capacity);
        }
        let gate = Arc::new(RouteGate {
            peer: route.peer,
            device: route.device,
            descriptor,
            epoch: context.epoch,
            policy: context.scope.policy,
            live: CallLease::default(),
            connection,
            _custody: Some(custody.clone()),
            #[cfg(test)]
            before_deallocation: std::sync::Mutex::new(None),
        });
        voice.routes.active.push(RouteRow {
            gate: Some(gate.clone()),
            _custody: custody,
        });
        Ok(gate)
    }

    /// Release unused route metadata BEFORE asking for another Crypto/Forward
    /// lane. An idle unjoined Root may own only old verification-credit rows.
    pub(crate) fn prune_voice_routes(&mut self) {
        if let Some(voice) = &mut self.voice_runtime {
            voice.routes.prune();
        }
    }

    /// Observe every real root/member/policy/descriptor/proof transition before another await.
    /// Strong registry rows stay counted while actual RouteGate owners exist. The separate lower Flight
    /// credit stays charged until real retirement even after route metadata has been released.
    pub(crate) fn reconcile_voice_routes(&mut self) {
        self.prune_voice_routes();
        // File reads have their own Root/member lifetime even when voice is never attached.
        // This common observed-transition hook must precede every optional voice early return.
        self.reconcile_file_effects();
        self.reconcile_file_service_routes();
        self.reconcile_recovery_submissions();
        let gates: Vec<_> = self
            .voice_runtime
            .as_ref()
            .map(|voice| {
                voice
                    .routes
                    .active
                    .iter()
                    .map(|row| row.gate().clone())
                    .collect()
            })
            .unwrap_or_default();
        if gates.is_empty() {
            return;
        }
        let context = self.voice_v2_context(0).ok();
        let routes: HashMap<_, _> = self
            .voice_bound_routes()
            .into_iter()
            .map(|route| (route.device, route.peer))
            .collect();
        for gate in gates {
            if context.as_ref().is_none_or(|context| {
                context.epoch != gate.epoch || context.scope.policy != gate.policy
            }) || routes.get(&gate.device) != Some(&gate.peer)
                || self
                    .peer_records
                    .get(&gate.device)
                    .is_none_or(|record| peer_descriptor_hash(record) != gate.descriptor)
            {
                gate.live.revoke();
            }
        }
        self.prune_voice_routes();
    }
    /// Prepare at most four current helper edges. `voice_schedule` calls this only while the
    /// local switch is `Forwarding`; it executes and settles each edge on this same Root.
    pub fn prepare_voice_forwards(&mut self) -> Result<Vec<PendingVoiceForward<T>>, VoiceError> {
        self.reconcile_voice_actor();
        let free_routes = self.available_voice_routes()?;
        let helper = self.voice_transport_actor()?.helper()?;
        let jobs = helper.take_ready_bounded(
            self.voice_transport_actor()?,
            &self.voice_bound_routes(),
            free_routes,
        )?;
        let mut pending = Vec::new();
        for job in jobs {
            // take_ready reserves the attempt before signing. Local preparation failure burns
            // it conservatively; no retry path can turn ambiguous handoff into a free attempt.
            if let Ok(prepared) = self.prepare_voice_request(Work::Helper {
                job,
                ledger: helper.clone(),
            }) {
                pending.push(prepared);
            }
        }
        Ok(pending)
    }
    /// Trusted native seam only. The activity is minted by the actual LocalJoin/Watcher; the
    /// Root supplies current directory authority rather than accepting a caller's room lease.
    /// No WebView command may accept arbitrary SignedObject/SourceActivity values.
    pub fn queue_voice_source(
        &mut self,
        object: voice_v2::SignedObject,
        activity: SourceActivity,
    ) -> Result<bool, VoiceError> {
        self.reconcile_voice_actor();
        // Classify the actual local effect, not a caller's activity label. Only a
        // real WatchChallenge can remain a watcher source; SourceActivity::check
        // also authenticates that sealed producer/body pairing. Helper forwarding
        // uses its independent ledger and does not pass through this local queue.
        if self.has_admitted_voice_activation()
            && !matches!(object.body(), voice_v2::Body::WatchChallenge { .. })
        {
            return Err(VoiceError::Unauthorized);
        }
        let room = self.voice_room_authority(object.scope().room)?;
        self.voice_transport_actor()?.source()?.admit(
            self.voice_transport_actor()?,
            object,
            activity,
            room,
        )
    }
    /// Driven by `voice_schedule` within the sync tick's fair rotation. Reserve each
    /// immutable source attempt before framing, and sign a permit only for its selected helper.
    pub fn prepare_voice_sources(&mut self) -> Result<Vec<PendingVoiceSource<T>>, VoiceError> {
        self.reconcile_voice_actor();
        if self.has_admitted_voice_activation() {
            // Include already queued off-Root Existing values. Retirement is
            // permanent, so clearing the admitted call cannot resurrect them.
            // It occurs before take_ready can debit a new submission attempt.
            self.voice_transport_actor()?
                .source()?
                .retire_existing_endpoint_sources();
        }
        let free_routes = self.available_voice_routes()?;
        let source = self.voice_transport_actor()?.source()?;
        let jobs = source.take_ready_bounded(
            self.voice_transport_actor()?,
            &self.voice_bound_routes(),
            free_routes,
        )?;
        let mut pending = Vec::new();
        for job in jobs {
            let permit = match job.hop() {
                SourceHop::Direct(_) => None,
                SourceHop::Helper(helper) => {
                    let now = self.voice_v2_clock().now_ms();
                    // Delegation never extends the original lifetime, even for a later hedge.
                    let expires = job
                        .object()
                        .expires()
                        .min(now.saturating_add(voice_v2::MAX_PACKET_LIFETIME_MS));
                    let Ok(permit) = voice_v2::RoutePermit::issue(
                        &self.device,
                        job.object(),
                        helper,
                        now,
                        expires,
                    ) else {
                        continue;
                    };
                    Some(Box::new(permit))
                }
            };
            if let Ok(prepared) = self.prepare_voice_request(Work::Source {
                job,
                ledger: source.clone(),
                permit,
            }) {
                pending.push(PendingVoiceSource(prepared));
            }
        }
        Ok(pending)
    }
    fn available_voice_routes(&mut self) -> Result<usize, VoiceError> {
        let voice = self
            .voice_runtime
            .as_mut()
            .ok_or(VoiceError::Unauthorized)?;
        // Revocation suppresses effects; only actual last external-owner drop
        // permits detaching the registry's final strong row and original payer.
        // Root's exclusive borrow spans scheduling and all synchronous preparations below, so
        // another caller cannot insert a gate between this count and this bounded batch.
        voice.routes.prune();
        Ok(ROUTES.saturating_sub(voice.routes.active.len()))
    }
    pub(crate) fn admitted_route_capacity(&mut self) -> usize {
        self.available_voice_routes().unwrap_or(0)
    }
    fn prepare_voice_request(&mut self, job: Work) -> Result<PendingVoiceForward<T>, VoiceError> {
        let context = self.voice_v2_context(job.envelope().object().scope().room)?;
        let route = self
            .voice_bound_route(job.peer())
            .ok_or(VoiceError::Unauthorized)?;
        if route.device != job.target() || !job.authority().authorized() {
            return Err(VoiceError::Unauthorized);
        }
        let gate = self.capture_voice_route(route, job.keepalive())?;
        let authority = Arc::new(FlightAuthority {
            route: gate,
            packet: job.authority(),
            clock: self.voice_v2_clock(),
            deadline: self
                .voice_v2_clock()
                .monotonic_ms()
                .saturating_add(DEADLINE_MS),
        });
        let inner = job.envelope().encode();
        let key = self.device.public_key_bytes();
        let mut nonce = [0; 16];
        self.rng.fill_bytes(&mut nonce);
        let auth = RequestAuth {
            ts: self.voice_v2_clock().now_ms(),
            nonce,
            epoch: context.epoch,
        };
        let transcript = catchup_auth_transcript(
            &self.group.group_id(),
            KIND,
            &inner,
            &key,
            auth.ts,
            &auth.nonce,
            auth.epoch,
            Some(&self.local_peer()),
        );
        let signature = self
            .device
            .sign(&transcript)
            .map_err(|_| VoiceError::Signature)?;
        drop(transcript);
        // Encode directly into the final frame. Building an encoded body and then extending
        // a second frame would retain an avoidable complete SDP copy at this boundary.
        let frame = encode_authed_request_frame(
            KIND,
            &inner,
            &key,
            auth.ts,
            &auth.nonce,
            auth.epoch,
            &signature,
        );
        if !authority.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(PendingVoiceForward {
            transport: self.transport.clone(),
            request: Bytes::from(frame),
            _custody: job.keepalive(),
            job,
            authority,
            instance: self.blob_fetch_instance.clone(),
        })
    }
    /// Only this originating current root can settle its retained helper edge. A cryptographically
    /// valid late ACK after any owner/route change cannot install RemoteAdmitted or health evidence.
    pub fn complete_voice_forward(
        &mut self,
        completed: CompletedVoiceForward,
    ) -> Result<VoiceTransportStatus, VoiceCompletionFailure<CompletedVoiceForward>> {
        if !matches!(completed.job, Work::Helper { .. }) {
            return Err(VoiceCompletionFailure::terminal(VoiceError::Unauthorized));
        }
        self.complete_voice_request(completed)
            .map(|(status, _)| status)
    }
    /// HelperQueued remains pending/unconfirmed; only an exact final endpoint's direct ACK
    /// may settle a targeted source packet. Transport failure does not refund the submission.
    pub fn complete_voice_source(
        &mut self,
        completed: CompletedVoiceSource,
    ) -> Result<(VoiceTransportStatus, SourceOutcome), VoiceCompletionFailure<CompletedVoiceSource>>
    {
        let (status, outcome) = self
            .complete_voice_request(completed.0)
            .map_err(|failure| failure.map_retry(CompletedVoiceSource))?;
        outcome
            .map(|outcome| (status, outcome))
            .ok_or_else(|| VoiceCompletionFailure::terminal(VoiceError::Unauthorized))
    }
    fn complete_voice_request(
        &mut self,
        completed: CompletedVoiceForward,
    ) -> Result<
        (VoiceTransportStatus, Option<SourceOutcome>),
        VoiceCompletionFailure<CompletedVoiceForward>,
    > {
        self.reconcile_voice_actor();
        if !Arc::ptr_eq(&completed.instance, &self.blob_fetch_instance)
            || !completed.authority.authorized()
        {
            return Err(VoiceCompletionFailure::terminal(VoiceError::Unauthorized));
        }
        completed
            .job
            .current(
                self.voice_transport_actor()
                    .map_err(VoiceCompletionFailure::terminal)?,
            )
            .map_err(VoiceCompletionFailure::terminal)?;
        let status = if let Ok(response) = &completed.response {
            match self.verify_voice_response(
                completed.job.peer(),
                &completed.request,
                completed.job.object_hash(),
                response,
                Some(completed.job.retained()),
            ) {
                Ok(status) => Some(status),
                Err(error @ (VoiceError::Capacity | VoiceError::RateLimited)) => {
                    return Err(VoiceCompletionFailure {
                        error,
                        retry: Some(Box::new(completed)),
                    })
                }
                Err(_) => None,
            }
        } else {
            None
        };
        // Keep the flight alive while validating. Fresh context, native lock and permanent route
        // generation must still agree at commit, including revocation during signature work.
        if !completed.authority.authorized() {
            return Err(VoiceCompletionFailure::terminal(VoiceError::Unauthorized));
        }
        let outcome = completed
            .job
            .complete(status)
            .map_err(VoiceCompletionFailure::terminal)?;
        status
            .map(|status| (status, outcome))
            .ok_or_else(|| VoiceCompletionFailure::terminal(VoiceError::Unauthorized))
    }
}

#[cfg(test)]
mod tests;
