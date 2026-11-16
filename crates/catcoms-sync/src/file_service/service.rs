//! Connected source-page service. Only the Root authenticates/signs; the blocking worker owns
//! a bounded read plan and observing authorities. No keys, mutable stores or Root cross an await.
use super::{admission::*, helper::*, read::*, routes::*, wire::*};
use crate::*;
use catcoms_rt::{ConnectionLease, RequestAuthority, Responder, SharedRequestKeepalive};
use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt};
use std::sync::Weak;
use tokio::sync::watch;
mod forwarding;
use forwarding::CompletedHelper;

const READ_MS: u64 = 2_000;

/// Trusted native attachment, never a renderer/peer capability. Replacing this configuration
/// affects new ingress only. The host must retire original account/entry/actor publishers on
/// lock, removal or stop; reattaching an equivalent healthy observer must not cancel old work.
#[derive(Clone)]
pub struct NativeFileService {
    pub reads: SourceReadRuntime,
    pub session: RequestCancellation,
    pub actor: Arc<dyn RequestAuthority>,
}

#[cfg(test)]
mod tests;
impl fmt::Debug for NativeFileService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeFileService { .. }")
    }
}

struct Registration {
    scope: Weak<Scope>,
    // Sole publisher stays on the Root, never in a worker/transport/completion.
    live: watch::Sender<bool>,
    immediate: DescriptorRef,
    original: DeviceId,
    original_key: [u8; 32],
}
pub(crate) struct FileService {
    // Drop publishers BEFORE jobs/config; actual workers retain their separate accounting.
    active: Vec<Registration>,
    pub(super) requesters: Vec<super::requester::Registration>,
    // Shared across this Root's native reads, so parallel jobs cannot overwhelm B's bounded
    // per-requester ingress window while traversing offline source batches.
    pub(super) requester_ready: Vec<(DeviceId, u64)>,
    pub(crate) jobs: FuturesUnordered<BoxFuture<'static, Completion>>,
    pub(super) native: Option<NativeFileService>,
    pub(super) admission: Arc<AdmissionRuntime>,
    pub(super) routes: RoutePool,
    helpers: HelperCapacity,
}
impl Default for FileService {
    fn default() -> Self {
        Self {
            active: Vec::new(),
            requesters: Vec::new(),
            requester_ready: Vec::new(),
            jobs: FuturesUnordered::new(),
            native: None,
            admission: AdmissionRuntime::process(),
            routes: RoutePool::default(),
            helpers: HelperCapacity::process(),
        }
    }
}

struct Scope {
    live: watch::Receiver<bool>,
    native: RequestCancellation,
    actor: Arc<dyn RequestAuthority>,
    root: Arc<dyn RequestAuthority>,
    connection: ConnectionLease,
    clock: Arc<dyn Clock>,
    deadline: u64,
    _route_slot: SharedRequestKeepalive,
}
impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileServiceScope { .. }")
    }
}
#[async_trait::async_trait]
impl RequestAuthority for Scope {
    fn authorized(&self) -> bool {
        *self.live.borrow()
            && self.live.has_changed().is_ok()
            && !self.native.is_cancelled()
            && self.actor.authorized()
            && self.root.authorized()
            && self.connection.is_live()
            && self.clock.monotonic_ms() < self.deadline
    }
    async fn revoked(&self) {
        let mut live = self.live.clone();
        let mut native = self.native.clone();
        while self.authorized() {
            tokio::select! {
                _ = live.changed() => {},
                _ = native.cancelled() => return,
                _ = self.actor.revoked() => return,
                _ = self.root.revoked() => return,
                _ = self.connection.revoked() => return,
                _ = self.clock.sleep(std::time::Duration::from_millis(self.deadline.saturating_sub(self.clock.monotonic_ms()))) => {},
            }
        }
    }
}
#[derive(Debug)]
struct ReadAuthority {
    scope: Arc<Scope>,
    reply: Arc<dyn RequestAuthority>,
    deadline: u64,
}
#[async_trait::async_trait]
impl RequestAuthority for ReadAuthority {
    fn authorized(&self) -> bool {
        self.scope.authorized()
            && self.reply.authorized()
            && self.scope.clock.monotonic_ms() < self.deadline
    }
    async fn revoked(&self) {
        while self.authorized() {
            tokio::select! {
                _ = self.scope.revoked() => return,
                _ = self.reply.revoked() => return,
                _ = self.scope.clock.sleep(std::time::Duration::from_millis(self.deadline.saturating_sub(self.scope.clock.monotonic_ms()))) => {},
            }
        }
    }
}
struct SourceFlow {
    proof: VerifiedGrant,
    responder: Responder,
    scope: Arc<Scope>,
    read_deadline: u64,
    // Release metadata before its SAME reservation; worker and final response own it too.
    custody: SharedRequestKeepalive,
}
pub(crate) struct CompletedSource {
    result: Option<FileReadCompletion>,
    flow: SourceFlow,
}
pub(crate) enum Completion {
    Source(Box<CompletedSource>),
    Helper(Box<CompletedHelper>),
}
struct SourceResponse {
    completed: Option<FileReadCompletion>,
    _flow: SharedRequestKeepalive,
}
impl fmt::Debug for SourceResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileSourceResponse { .. }")
    }
}
#[derive(Debug)]
struct PresentAuthority {
    scope: Arc<Scope>,
    response: Arc<SourceResponse>,
    deadline: u64,
}
#[async_trait::async_trait]
impl RequestAuthority for PresentAuthority {
    fn authorized(&self) -> bool {
        self.scope.authorized()
            && self.scope.clock.monotonic_ms() < self.deadline
            && self
                .response
                .completed
                .as_ref()
                .is_some_and(FileReadCompletion::storage_is_current)
    }
    async fn revoked(&self) {
        // Storage has a synchronous fence, not an async notification. Drivers MUST check
        // authorized at actual admission even if no wake was emitted for a store deletion.
        if !self.authorized() {
            return;
        }
        tokio::select! {
            _ = self.scope.revoked() => {},
            _ = self.scope.clock.sleep(std::time::Duration::from_millis(self.deadline.saturating_sub(self.scope.clock.monotonic_ms()))) => {},
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub(crate) fn complete_file_service(&mut self, completed: Completion)
    where
        T: 'static,
    {
        match completed {
            Completion::Source(source) => self.complete_file_source(*source),
            Completion::Helper(helper) => self.complete_file_helper(*helper),
        }
    }

    /// Native-only service configuration. None refuses new work; it does not claim to retire
    /// previously captured authority. The host retires its actual publishers at lifecycle edges.
    pub fn set_native_file_service(&mut self, native: Option<NativeFileService>) {
        self.file_service.native = native;
    }

    pub(crate) fn reconcile_file_service_routes(&mut self) {
        self.reconcile_file_requesters();
        self.file_service
            .active
            .retain(|row| row.scope.strong_count() != 0);
        if self.file_service.active.is_empty() {
            return;
        }
        let routes: HashMap<_, _> = self
            .voice_bound_routes()
            .into_iter()
            .map(|r| (r.device, r.peer))
            .collect();
        for row in &self.file_service.active {
            if !self.policy_allows_service()
                || !self.descriptor_ref_is_current(&row.immediate)
                || routes.get(&row.immediate.device) != Some(&row.immediate.peer)
                || self.group.member_signature_key(&row.original).as_deref()
                    != Some(row.original_key.as_slice())
            {
                row.live.send_replace(false);
            }
        }
    }

    /// Cheap current actual-sender evidence precedes the finite precrypto debit. The full
    /// uniqueness/connection-incarnation check follows that debit; claimed C cannot spend it.
    fn file_ingress_peer(&self, peer: PeerId) -> Option<ServicePeer> {
        self.member_peers.iter().find_map(|proof| {
            (proof.bound
                && proof.peer == peer
                && self.group.contains_device(&proof.device)
                && self
                    .peer_records
                    .get(&proof.device)
                    .is_some_and(|r| r.peer_id == *peer.as_bytes()))
            .then_some(ServicePeer {
                device: proof.device,
                peer,
            })
        })
    }
    fn capture_file_scope(
        &mut self,
        proof: &VerifiedGrant,
        native: &NativeFileService,
        routes: &mut RoutePlan,
    ) -> Option<Arc<Scope>> {
        self.reconcile_file_service_routes();
        if !self.policy_allows_service()
            || native.session.is_cancelled()
            || !native.actor.authorized()
        {
            return None;
        }
        let sender = proof.sender();
        let route = self
            .voice_bound_routes()
            .into_iter()
            .find(|r| r.device == sender.device && r.peer == sender.peer)?;
        let connection = self
            .transport
            .connection_lease(route.peer)
            .filter(ConnectionLease::is_live)?;
        let grant = proof.grant();
        if self
            .group
            .member_signature_key(&grant.requester())
            .as_deref()
            != Some(grant.public_key().as_slice())
        {
            return None;
        }
        let immediate = self.descriptor_ref(sender.device)?;
        let (live, signal) = watch::channel(true);
        let scope = Arc::new(Scope {
            live: signal,
            native: native.session.clone(),
            actor: native.actor.clone(),
            root: self.file_effect_authority().ok()?,
            connection,
            clock: self.file_service.admission.clock(),
            deadline: proof.deadline(),
            _route_slot: routes.take_scope()?,
        });
        if !scope.authorized() {
            return None;
        }
        self.file_service.active.push(Registration {
            scope: Arc::downgrade(&scope),
            live,
            immediate,
            original: grant.requester(),
            original_key: *grant.public_key(),
        });
        Some(scope)
    }

    /// No await: every pending responder is installed on self before the next actor poll.
    /// Unsupported/untrusted ingress stays silent. Authenticated temporary capacity failures
    /// are signed Unavailable observations, never evidence that no member holds the chunk.
    pub(crate) fn queue_file_source(&mut self, from: PeerId, frame: &[u8], responder: Responder) {
        let Some(native) = self
            .file_service
            .native
            .clone()
            .filter(|n| !n.session.is_cancelled() && n.actor.authorized())
        else {
            return;
        };
        if !self.policy_allows_service() || !responder.is_current() {
            return;
        }
        let Some(sender) = self.file_ingress_peer(from) else {
            return;
        };
        let admission = self.file_service.admission.clone();
        let Ok(ticket) = admission.admit_ingress(
            ServiceScope {
                group: &self.group.group_id(),
                epoch: self.group.epoch(),
                local: ServicePeer {
                    device: self.device.device_id(),
                    peer: self.local_peer(),
                },
            },
            sender,
            Role::Source,
            frame,
        ) else {
            return;
        };
        let Ok(delivery) = Delivery::parse(frame) else {
            return;
        };
        if self.group.member_signature_key(&sender.device).as_deref()
            != Some(delivery.public_key().as_slice())
        {
            return;
        }
        let Ok(proof) = admission.verify(ticket, frame) else {
            return;
        };
        let Some(mut routes) = self.file_service.routes.source() else {
            return;
        };
        let Some(scope) = self.capture_file_scope(&proof, &native, &mut routes) else {
            return;
        };
        if let Err(error) = admission.preflight(&proof) {
            if error == AdmissionError::Busy {
                self.respond_source_unavailable(&proof, responder, scope);
            }
            return;
        }
        let Some(deadline) = scope
            .clock
            .monotonic_ms()
            .checked_add(READ_MS)
            .map(|d| d.min(scope.deadline))
        else {
            return;
        };
        let work = Arc::new(ReadAuthority {
            scope: scope.clone(),
            reply: responder.cancellation_authority(),
            deadline,
        });
        let window = proof.grant().window();
        let task = match native.reads.prepare(
            proof.group(),
            &*self.blobs,
            &window.cid,
            window.max_cipher_bytes as usize,
            work.clone(),
        ) {
            Ok(task) => task,
            Err(_) => {
                self.respond_source_unavailable(&proof, responder, scope);
                return;
            }
        };
        // Export before start: a worker panic must not refund still-owned proof/responder data.
        let custody = task.keepalive();
        if !work.authorized() || admission.commit(&proof).is_err() {
            return;
        }
        let worker = native.reads.start(task);
        let flow = SourceFlow {
            proof,
            responder,
            scope,
            read_deadline: deadline,
            custody,
        };
        self.file_service.jobs.push(
            async move {
                let result = match futures::future::select(worker, work.revoked().boxed()).await {
                    futures::future::Either::Left((result, _)) => result,
                    futures::future::Either::Right(((), _)) => None,
                };
                Completion::Source(Box::new(CompletedSource { result, flow }))
            }
            .boxed(),
        );
    }

    fn respond_source_unavailable(
        &self,
        proof: &VerifiedGrant,
        responder: Responder,
        scope: Arc<Scope>,
    ) {
        if !scope.authorized() || !responder.is_current() {
            return;
        }
        if let Ok(bytes) = SourcePage::issue(
            &proof.grant(),
            SourceStatus::Unavailable,
            0,
            &[],
            &self.device,
        ) {
            let _ = responder.try_respond_with_authority(Bytes::from(bytes), scope);
        }
    }

    pub(crate) fn complete_file_source(&mut self, completed: CompletedSource) {
        self.reconcile_file_service_routes();
        let CompletedSource { result, flow } = completed;
        if !flow.scope.authorized() || !flow.responder.is_current() {
            return;
        }
        let grant = flow.proof.grant();
        let (status, total, page) = match result.as_ref().map(FileReadCompletion::result) {
            Some(Ok(Some(blob))) if blob.is_current() => {
                let bytes = blob.bytes();
                let offset = grant.window().offset as usize;
                if offset >= bytes.len() {
                    (SourceStatus::Unavailable, 0, &[][..])
                } else {
                    (
                        SourceStatus::Present,
                        bytes.len() as u32,
                        &bytes[offset..bytes.len().min(offset + grant.window().length as usize)],
                    )
                }
            }
            Some(Ok(None)) => (SourceStatus::Absent, 0, &[][..]),
            _ => (SourceStatus::Unavailable, 0, &[][..]),
        };
        let Ok(bytes) = SourcePage::issue(&grant, status, total, page, &self.device) else {
            return;
        };
        let response = Arc::new(SourceResponse {
            completed: result,
            _flow: flow.custody,
        });
        let effect: Arc<dyn RequestAuthority> = if status == SourceStatus::Present {
            Arc::new(PresentAuthority {
                scope: flow.scope,
                response: response.clone(),
                deadline: flow.read_deadline,
            })
        } else {
            flow.scope
        };
        // ReplyLifetime intentionally excluded: receiving this reply retires it before the
        // driver's actual send. Independent scope/storage gates and custody survive that edge.
        let _ = flow
            .responder
            .try_respond_with_custody(Bytes::from(bytes), effect, response);
    }
}
