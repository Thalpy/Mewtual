//! Detached connected-only source/helper flights. The sole sync owner signs and validates; workers
//! have immutable bytes, a revocable route/packet authority and actual lower-driver custody.
use super::*;
use catcoms_rt::{RequestAuthority, SharedRequestKeepalive};
use futures::FutureExt;
use std::sync::Weak;
use voice_v2::helper::{ForwardJob, HelperLedger};
use voice_v2::source::{SourceActivity, SourceHop, SourceJob, SourceLedger, SourceOutcome};
use voice_v2::CallLease;

const ROUTES: usize = 64;
const DEADLINE_MS: u64 = 3_000;

#[derive(Default)]
pub(crate) struct RouteOwners {
    active: Vec<Weak<RouteGate>>,
}
impl RouteOwners {
    pub(crate) fn disconnected(&self, peer: PeerId) {
        for gate in self
            .active
            .iter()
            .filter_map(Weak::upgrade)
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
        for gate in self.active.iter().filter_map(Weak::upgrade) {
            gate.live.revoke();
        }
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
    _custody: SharedRequestKeepalive,
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
        self.route.live.is_live()
            && self.route.connection.is_live()
            && self.packet.authorized()
            && self.clock.monotonic_ms() < self.deadline
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
        job: SourceJob,
        ledger: Arc<SourceLedger>,
        envelope: Box<Envelope>,
    },
}
impl Work {
    fn exchange(&self) -> Option<Arc<voice_v2::negotiation::Exchange>> {
        match self {
            Self::Source { job, .. } => job.exchange(),
            Self::Helper { .. } => None,
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
    fn envelope(&self) -> &Envelope {
        match self {
            Self::Helper { job, .. } => job.envelope(),
            Self::Source { envelope, .. } => envelope,
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
    /// Observe every real root/member/policy/descriptor/proof transition before another await.
    /// Weak rows stay counted while actual RouteGate owners exist. The separate lower Flight
    /// credit stays charged until real retirement even after route metadata has been released.
    pub(crate) fn reconcile_voice_routes(&mut self) {
        self.reconcile_recovery_submissions();
        let gates: Vec<_> = self
            .voice_runtime
            .as_ref()
            .map(|voice| {
                voice
                    .routes
                    .active
                    .iter()
                    .filter_map(Weak::upgrade)
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
        if let Some(voice) = &mut self.voice_runtime {
            voice.routes.active.retain(|gate| gate.strong_count() > 0);
        }
    }
    /// Prepare at most four current helper edges. This is still an inert integration seam:
    /// production actor scheduling/request dispatch remains disabled until endpoint ownership.
    pub fn prepare_voice_forwards(&mut self) -> Result<Vec<PendingVoiceForward<T>>, VoiceError> {
        self.reconcile_voice_actor();
        let helper = self.voice_transport_actor()?.helper()?;
        let jobs = helper.take_ready(self.voice_transport_actor()?, &self.voice_bound_routes())?;
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
        let room = self.voice_room_authority(object.scope().room)?;
        self.voice_transport_actor()?.source()?.admit(
            self.voice_transport_actor()?,
            object,
            activity,
            room,
        )
    }
    /// Still inert until endpoint admission and actor fairness are integrated. Reserve each
    /// immutable source attempt before framing, and sign a permit only for its selected helper.
    pub fn prepare_voice_sources(&mut self) -> Result<Vec<PendingVoiceSource<T>>, VoiceError> {
        self.reconcile_voice_actor();
        let source = self.voice_transport_actor()?.source()?;
        let jobs = source.take_ready(self.voice_transport_actor()?, &self.voice_bound_routes())?;
        let mut pending = Vec::new();
        for job in jobs {
            let envelope = match job.hop() {
                SourceHop::Direct(_) => Envelope::Direct(job.object().clone()),
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
                    Envelope::ToHelper {
                        object: job.object().clone(),
                        permit,
                    }
                }
            };
            if let Ok(prepared) = self.prepare_voice_request(Work::Source {
                job,
                ledger: source.clone(),
                envelope: Box::new(envelope),
            }) {
                pending.push(PendingVoiceSource(prepared));
            }
        }
        Ok(pending)
    }
    fn prepare_voice_request(&mut self, job: Work) -> Result<PendingVoiceForward<T>, VoiceError> {
        let context = self.voice_v2_context(job.envelope().object().scope().room)?;
        let route = self
            .voice_bound_route(job.peer())
            .ok_or(VoiceError::Unauthorized)?;
        if route.device != job.target() || !job.authority().authorized() {
            return Err(VoiceError::Unauthorized);
        }
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
        voice.routes.active.retain(|gate| gate.strong_count() > 0);
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
            _custody: job.keepalive(),
        });
        voice.routes.active.push(Arc::downgrade(&gate));
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
        let mut frame = vec![KIND];
        frame.extend(encode_authed_request(
            &inner,
            &key,
            auth.ts,
            &auth.nonce,
            auth.epoch,
            &signature,
        ));
        if !authority.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(PendingVoiceForward {
            transport: self.transport.clone(),
            request: Bytes::from(frame),
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
        let progress = completed.job.exchange();
        let status = if let Ok(response) = &completed.response {
            match self.verify_voice_response(
                completed.job.peer(),
                &completed.request,
                completed.job.object_hash(),
                response,
                Some(completed.job.envelope()),
                progress.as_ref(),
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
