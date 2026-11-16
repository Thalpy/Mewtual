//! One-hop opaque helper service. Each preparation token is consumed by the private pending
//! builder once; all four possible actual lower owners retain the original chain reservation.
use super::*;

struct HelperFlow {
    proof: VerifiedGrant,
    responder: Responder,
    parent: Arc<Scope>,
    next: usize,
    unavailable: bool,
    routes: RoutePlan,
    // Metadata/route plans precede the mandatory original capacity owner in drop order.
    chain: HelperChain,
}
struct PendingHelper<T: MeshTransport> {
    transport: Arc<T>,
    frame: Bytes,
    peer: PeerId,
    authority: Arc<ReadAuthority>,
    custody: SharedRequestKeepalive,
}
struct LowerResponseBytes {
    data: Bytes,
    // Even a final signing/admission refusal may retire the outer flow first. Received bytes
    // and any clones therefore retain their own SAME charge, with data dropped before credit.
    _custody: SharedRequestKeepalive,
}
impl AsRef<[u8]> for LowerResponseBytes {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}
impl<T: MeshTransport> PendingHelper<T> {
    fn new(
        transport: Arc<T>,
        frame: Vec<u8>,
        peer: PeerId,
        authority: Arc<ReadAuthority>,
        attempt: HelperAttempt,
    ) -> Self {
        // There is no public constructor/Clone/retry API. The affine attempt is consumed here;
        // shared accounting alone never authorizes another request or another packet copy.
        Self {
            transport,
            frame: Bytes::from(frame),
            peer,
            authority,
            custody: attempt.keepalive(),
        }
    }
    async fn execute(self) -> Result<Bytes, TransportError> {
        let (_wait_owner, signal) = watch::channel(false);
        let cancellation = RequestCancellation::new(signal, Some(self.custody.clone()))
            .with_authority(self.authority.clone());
        let result = match futures::future::select(
            self.transport
                .request_connected_cancellable(
                    self.peer,
                    ProtocolId(RR_PROTOCOL),
                    self.frame,
                    cancellation,
                )
                .boxed(),
            self.authority.revoked().boxed(),
        )
        .await
        {
            futures::future::Either::Left((response, _)) => response,
            futures::future::Either::Right(((), _)) => Err(TransportError::Cancelled),
        };
        result.and_then(|bytes| {
            if bytes.len() <= MAX_SOURCE_REPLY_BYTES {
                Ok(Bytes::from_owner(LowerResponseBytes {
                    data: bytes,
                    _custody: self.custody,
                }))
            } else {
                Err(TransportError::NoResponse)
            }
        })
    }
}
pub(crate) struct CompletedHelper {
    response: Result<Bytes, TransportError>,
    source: DeviceId,
    key: [u8; 32],
    child: Arc<Scope>,
    flow: HelperFlow,
}
#[cfg(test)]
impl CompletedHelper {
    pub(super) fn response_clone_for_test(&self) -> Option<Bytes> {
        self.response.as_ref().ok().cloned()
    }
}
#[derive(Debug)]
struct FoundAuthority {
    parent: Arc<Scope>,
    source: Arc<Scope>,
}
#[async_trait::async_trait]
impl RequestAuthority for FoundAuthority {
    fn authorized(&self) -> bool {
        self.parent.authorized() && self.source.authorized()
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        tokio::select! { _ = self.parent.revoked() => {}, _ = self.source.revoked() => {} }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    pub(crate) fn queue_file_helper(&mut self, from: PeerId, frame: &[u8], responder: Responder)
    where
        T: 'static,
    {
        let Some(native) = self
            .file_service
            .native
            .clone()
            .filter(|n| !n.session.is_cancelled() && n.actor.authorized())
        else {
            tracing::debug!(stage = "native", "file helper request not admitted");
            return;
        };
        if !self.policy_allows_service() || !responder.is_current() {
            tracing::debug!(
                stage = "policy-or-request",
                "file helper request not admitted"
            );
            return;
        }
        let Some(sender) = self.file_ingress_peer(from) else {
            tracing::debug!(stage = "bound-sender", "file helper request not admitted");
            return;
        };
        let admission = self.file_service.admission.clone();
        let ticket = match admission.admit_ingress(
            ServiceScope {
                group: &self.group.group_id(),
                epoch: self.group.epoch(),
                local: ServicePeer {
                    device: self.device.device_id(),
                    peer: self.local_peer(),
                },
            },
            sender,
            Role::Helper,
            frame,
        ) {
            Ok(ticket) => ticket,
            Err(error) => {
                tracing::debug!(
                    stage = "ingress",
                    ?error,
                    "file helper request not admitted"
                );
                return;
            }
        };
        let Ok(grant) = Grant::parse(frame) else {
            return;
        };
        if self.group.member_signature_key(&sender.device).as_deref()
            != Some(grant.public_key().as_slice())
        {
            return;
        }
        let Ok(proof) = admission.verify(ticket, frame) else {
            return;
        };
        // Reserve the COMPLETE parent+four-child route plan before irreversible admission.
        // Prior timed-out drivers may still hold all their scopes; no child needs a new slot.
        let Some(mut routes) = self.file_service.routes.helper() else {
            tracing::debug!(stage = "route-capacity", "file helper request not admitted");
            return;
        };
        let Some(parent) = self.capture_file_scope(&proof, &native, &mut routes) else {
            tracing::debug!(stage = "scope", "file helper request not admitted");
            return;
        };
        if let Err(error) = admission.preflight(&proof) {
            if error == AdmissionError::Busy {
                self.respond_helper_unavailable(&proof, responder, parent);
            }
            return;
        }
        let chain = match self
            .file_service
            .helpers
            .reserve(proof.group(), proof.grant().requester())
        {
            Ok(chain) => chain,
            Err(_) => {
                self.respond_helper_unavailable(&proof, responder, parent);
                return;
            }
        };
        if !parent.authorized() || !responder.is_current() || admission.commit(&proof).is_err() {
            return;
        }
        self.advance_file_helper(HelperFlow {
            proof,
            responder,
            parent,
            next: 0,
            unavailable: false,
            routes,
            chain,
        });
    }

    fn capture_helper_child(
        &mut self,
        flow: &mut HelperFlow,
        source: DeviceId,
    ) -> Option<(Arc<Scope>, [u8; 32])> {
        self.reconcile_file_service_routes();
        if !flow.parent.authorized() {
            return None;
        }
        let route = self
            .voice_bound_routes()
            .into_iter()
            .find(|r| r.device == source)?;
        let key: [u8; 32] = self.group.member_signature_key(&source)?.try_into().ok()?;
        let immediate = self.descriptor_ref(source)?;
        let connection = self
            .transport
            .connection_lease(route.peer)
            .filter(ConnectionLease::is_live)?;
        let deadline = flow
            .parent
            .clock
            .monotonic_ms()
            .checked_add(READ_MS)?
            .min(flow.parent.deadline);
        let (live, signal) = watch::channel(true);
        let child = Arc::new(Scope {
            live: signal,
            native: flow.parent.native.clone(),
            actor: flow.parent.actor.clone(),
            root: flow.parent.clone(),
            connection,
            clock: flow.parent.clock.clone(),
            deadline,
            _route_slot: flow.routes.take_scope()?,
        });
        if !child.authorized() {
            return None;
        }
        let grant = flow.proof.grant();
        self.file_service.active.push(Registration {
            scope: Arc::downgrade(&child),
            live,
            immediate,
            original: grant.requester(),
            original_key: *grant.public_key(),
        });
        Some((child, key))
    }

    fn advance_file_helper(&mut self, mut flow: HelperFlow)
    where
        T: 'static,
    {
        if !flow.parent.authorized() || !flow.responder.is_current() {
            return;
        }
        while let Some(source) = flow.proof.grant().sources().get(flow.next).copied() {
            // Device positions are monotonic even when a route was skipped or later reconnected.
            // Reconciliation can never restart the source list or replenish the four tokens.
            flow.next += 1;
            let Some((child, key)) = self.capture_helper_child(&mut flow, source) else {
                flow.unavailable = true;
                continue;
            };
            let Some(attempt) = flow.chain.take_attempt() else {
                flow.unavailable = true;
                break;
            };
            let peer = self
                .descriptor_ref(source)
                .expect("captured descriptor")
                .peer;
            let frame = match Delivery::issue(&flow.proof.grant(), source, peer, &self.device) {
                Ok(frame) => frame,
                Err(_) => {
                    flow.unavailable = true;
                    continue;
                }
            };
            let work = Arc::new(ReadAuthority {
                scope: child.clone(),
                reply: flow.responder.cancellation_authority(),
                deadline: child.deadline,
            });
            let pending = PendingHelper::new(self.transport.clone(), frame, peer, work, attempt);
            self.file_service.jobs.push(
                async move {
                    let response = pending.execute().await;
                    Completion::Helper(Box::new(CompletedHelper {
                        response,
                        source,
                        key,
                        child,
                        flow,
                    }))
                }
                .boxed(),
            );
            return;
        }
        let status = if flow.unavailable {
            HelperStatus::Unavailable
        } else {
            HelperStatus::Absent
        };
        self.finish_file_helper(flow, status, None, None);
    }

    pub(super) fn complete_file_helper(&mut self, completed: CompletedHelper)
    where
        T: 'static,
    {
        self.reconcile_file_service_routes();
        let CompletedHelper {
            response,
            source,
            key,
            child,
            mut flow,
        } = completed;
        if !flow.parent.authorized() || !flow.responder.is_current() {
            return;
        }
        let mut absent = false;
        if child.authorized()
            && self.group.member_signature_key(&source).as_deref() == Some(key.as_slice())
        {
            if let Ok(bytes) = &response {
                let grant = flow.proof.grant();
                if let Ok(page) = SourcePage::parse(bytes, &grant) {
                    // Grant membership alone is insufficient: another named A cannot answer
                    // this selected source's attempt or substitute its key before wrapping.
                    if page.source() == source
                        && page.public_key() == &key
                        && page.verify_signature()
                        && child.authorized()
                    {
                        if page.status() == SourceStatus::Present {
                            self.finish_file_helper(
                                flow,
                                HelperStatus::Found,
                                Some(&page),
                                Some(child),
                            );
                            return;
                        }
                        absent = page.status() == SourceStatus::Absent;
                    }
                }
            }
        }
        if !absent {
            flow.unavailable = true;
        }
        self.advance_file_helper(flow);
    }

    fn finish_file_helper(
        &self,
        flow: HelperFlow,
        status: HelperStatus,
        source: Option<&SourcePage<'_>>,
        child: Option<Arc<Scope>>,
    ) {
        if !flow.parent.authorized() || !flow.responder.is_current() {
            return;
        }
        let Ok(bytes) = HelperPage::issue(&flow.proof.grant(), status, source, &self.device) else {
            return;
        };
        let effect: Arc<dyn RequestAuthority> = match child {
            Some(source) => Arc::new(FoundAuthority {
                parent: flow.parent,
                source,
            }),
            None => flow.parent,
        };
        // The actual response byte clones AND pending lower serialization retain the SAME chain
        // as all up-to-four actual source requests. Reply-wait retirement is not final authority.
        let _ = flow.responder.try_respond_with_custody(
            Bytes::from(bytes),
            effect,
            flow.chain.into_response_custody(),
        );
    }
    fn respond_helper_unavailable(
        &self,
        proof: &VerifiedGrant,
        responder: Responder,
        scope: Arc<Scope>,
    ) {
        if !scope.authorized() || !responder.is_current() {
            return;
        }
        if let Ok(bytes) = HelperPage::issue(
            &proof.grant(),
            HelperStatus::Unavailable,
            None,
            &self.device,
        ) {
            let _ = responder.try_respond_with_authority(Bytes::from(bytes), scope);
        }
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;

    #[test]
    fn actual_fixed_helper_metadata_fits_the_prepaid_metadata_envelope() {
        // Closed metadata has no caller-sized strings, paths or unbounded vectors. Include
        // all five scope/registration pairs, four simultaneous lower continuation layouts,
        // the flow and a generous16KiB for bounded watch/Arc/group/key allocation overhead.
        // Request/response serialized buffers have their separate944KiB derivation in helper.rs;
        // codec maximum-frame tests pin those lengths. Driver/allocator/OS costs stay separate.
        let explicit = std::mem::size_of::<HelperFlow>()
            + 5 * (std::mem::size_of::<Scope>() + std::mem::size_of::<Registration>())
            + 4 * (std::mem::size_of::<CompletedHelper>()
                + std::mem::size_of::<PendingHelper<catcoms_rt::MemNetwork>>()
                + std::mem::size_of::<ReadAuthority>())
            + 16 * 1024;
        assert!(
            explicit <= 64 * 1024,
            "helper metadata requires {explicit} bytes"
        );
    }
}
