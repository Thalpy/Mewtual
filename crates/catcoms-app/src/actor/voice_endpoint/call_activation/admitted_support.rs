//! Exact typed Query/Support turns on the original Actor's paid send fence.
//! These observations create no Availability, round, SDK or RTC permission.
use super::*;
use catcoms_sync::voice_v2::admitted_control::{
    DirectHelperSupportObservation, DirectHelperSupportProbe, EndpointSupportObservation,
    EndpointSupportQuery, SupportQueryIdentity,
};

/// A whole Root outcome. Endpoint and helper evidence remain separately typed;
/// native callers must never replace authenticated observation with a queued job.
pub enum VoiceAdmittedSupportResult {
    EndpointQuery(Result<EndpointSupportQuery, VoiceError>),
    DirectHelperProbe(Result<DirectHelperSupportProbe, VoiceError>),
    EndpointObservation(Result<EndpointSupportObservation, VoiceError>),
    DirectHelperObservation(Result<DirectHelperSupportObservation, VoiceError>),
    PresencePeers(PeerSnapshot),
}
impl std::fmt::Debug for VoiceAdmittedSupportResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceAdmittedSupportResult { .. }")
    }
}
/// Concrete native cells/oneshots/publishers require separate prepaid storage.
/// Callbacks run outside send/Root/Group/Policy guards and may not call Root,
/// Clock, UI admission, SDK or an await. An entered/panicking callback is unknown,
/// never a proven unstarted refusal. Lost ensure replies recover Root's SAME
/// current query; only an explicit refresh with its opaque previous may replace it.
pub trait VoiceAdmittedSupportPublisher: Send {
    fn accepted(&mut self, work: VoiceAdmittedSupportWork) -> Result<(), VoiceEndpointError>;
    fn completed(
        self: Box<Self>,
        result: VoiceAdmittedSupportResult,
        work: VoiceAdmittedSupportWork,
    );
    fn refused(
        self: Box<Self>,
        error: VoiceEndpointError,
        work: Option<VoiceAdmittedSupportWork>,
        payment: Option<ActivationRpcPermit>,
    );
}
// No raw Arc/Weak escapes. EVERY funding alias uses into_inner: concurrent final
// drops free the funding allocation/header before returning its plain permit.
struct FundingOwner(Option<Arc<ActivationRpcPermit>>);
impl Clone for FundingOwner {
    fn clone(&self) -> Self {
        Self(Some(
            self.0.as_ref().expect("original support funding").clone(),
        ))
    }
}
impl Drop for FundingOwner {
    fn drop(&mut self) {
        if let Some(permit) = self.0.take().and_then(Arc::into_inner) {
            drop(permit);
        }
    }
}
/// Lifecycle metadata, not current Root authority. Every alias retains the SAME
/// outside permit through actual invocation body AND Arc header deallocation.
#[derive(Clone)]
pub struct VoiceAdmittedSupportWork {
    state: Option<Arc<InvocationState<()>>>,
    _funding: FundingOwner,
}
impl Drop for VoiceAdmittedSupportWork {
    fn drop(&mut self) {
        drop(self.state.take());
    }
}
impl VoiceAdmittedSupportWork {
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(self.state.as_ref().unwrap(), other.state.as_ref().unwrap())
    }
    pub fn is_retired(&self) -> bool {
        self.state.as_ref().unwrap().is_retired()
    }
    pub async fn wait_retired(&self) {
        self.state.as_ref().unwrap().wait_retired().await;
    }
}
struct Retirement {
    marker: Option<InvocationRetirement<()>>,
    // Even the marker's final Arc tail keeps the original sponsor outside it.
    _funding: FundingOwner,
}
impl Drop for Retirement {
    fn drop(&mut self) {
        drop(self.marker.take());
    }
}
impl std::fmt::Debug for VoiceAdmittedSupportWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceAdmittedSupportWork { .. }")
    }
}
enum Action {
    EnsureEndpoint(DeviceId),
    RefreshEndpoint(DeviceId, SupportQueryIdentity),
    EnsureHelper(DeviceId),
    RefreshHelper(DeviceId, SupportQueryIdentity),
    ObserveEndpoint(EndpointSupportQuery),
    ObserveHelper(DirectHelperSupportProbe),
    PresencePeers(Option<PeerSnapshotPermit>),
}
struct Body {
    grant: Arc<CallGrant>,
    original: ActivationObserver,
    action: Action,
    publisher: Option<Box<dyn VoiceAdmittedSupportPublisher>>,
    work: Option<VoiceAdmittedSupportWork>,
}
impl Drop for Body {
    fn drop(&mut self) {
        if let Some(publisher) = self.publisher.take() {
            publisher.refused(VoiceEndpointError::Stopped, self.work.take(), None);
        }
    }
}
struct Arguments {
    body: Option<Box<Body>>,
    // Outside the Box: actual argument ALLOCATION return precedes this marker.
    _retirement: Option<Retirement>,
}
impl Drop for Arguments {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}
// One boxed body keeps the enclosing existing command enum small. Its SAME
// payer is OUTSIDE that Box and declared last, including failed/abandoned sends.
pub(in crate::actor::voice_endpoint) struct Submission(Paid<Arguments, VoiceAdmittedSupportWork>);
type Preparing = Paid<Arguments, Option<ActivationRpcPermit>>;

pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut submission: Submission,
    server: &mut Server<T, R>,
) {
    let body = submission
        .0
        .arguments
        .body
        .as_mut()
        .expect("original support body");
    // Take BEFORE Root entry: unwind cannot turn entered work into unstarted.
    let publisher = body.publisher.take().expect("original support publisher");
    let result = match &mut body.action {
        Action::EnsureEndpoint(peer) => VoiceAdmittedSupportResult::EndpointQuery(
            server
                .sync
                .ensure_admitted_endpoint_support(&body.grant, &body.original, *peer),
        ),
        Action::RefreshEndpoint(peer, previous) => VoiceAdmittedSupportResult::EndpointQuery(
            server.sync.refresh_admitted_endpoint_support(
                &body.grant,
                &body.original,
                *peer,
                previous,
            ),
        ),
        Action::EnsureHelper(peer) => VoiceAdmittedSupportResult::DirectHelperProbe(
            server
                .sync
                .ensure_admitted_direct_helper_support(&body.grant, &body.original, *peer),
        ),
        Action::RefreshHelper(peer, previous) => VoiceAdmittedSupportResult::DirectHelperProbe(
            server.sync.refresh_admitted_direct_helper_support(
                &body.grant,
                &body.original,
                *peer,
                previous,
            ),
        ),
        Action::ObserveEndpoint(query) => VoiceAdmittedSupportResult::EndpointObservation(
            server
                .sync
                .admitted_endpoint_support_observation(&body.grant, &body.original, query),
        ),
        Action::ObserveHelper(query) => VoiceAdmittedSupportResult::DirectHelperObservation(
            server.sync.admitted_direct_helper_support_observation(
                &body.grant,
                &body.original,
                query,
            ),
        ),
        Action::PresencePeers(permit) => VoiceAdmittedSupportResult::PresencePeers(
            server.sync.admitted_voice_presence_peer_snapshot(
                &body.grant,
                &body.original,
                permit.take().expect("one original snapshot permit"),
            ),
        ),
    };
    publisher.completed(result, submission.0.payment.clone());
}
impl VoiceActivationControl {
    /// Ensure/recover this original's current endpoint query; absence of a reply
    /// never authorizes replacement or a fresh deadline.
    pub fn try_queue_ensure_admitted_endpoint_support(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        peer: DeviceId,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedSupportPublisher>,
    ) -> Result<VoiceAdmittedSupportWork, VoiceEndpointError> {
        self.queue_admitted_support(
            grant,
            original,
            Action::EnsureEndpoint(peer),
            payment,
            publisher,
        )
    }
    /// Explicitly replace only the exact opaque previous endpoint query.
    #[allow(clippy::too_many_arguments)] // Exact original, previous identity and independent payer stay explicit.
    pub fn try_queue_refresh_admitted_endpoint_support(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        peer: DeviceId,
        previous: SupportQueryIdentity,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedSupportPublisher>,
    ) -> Result<VoiceAdmittedSupportWork, VoiceEndpointError> {
        self.queue_admitted_support(
            grant,
            original,
            Action::RefreshEndpoint(peer, previous),
            payment,
            publisher,
        )
    }
    /// Probe an actually connected helper without treating it as a room endpoint.
    pub fn try_queue_ensure_admitted_direct_helper_support(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        peer: DeviceId,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedSupportPublisher>,
    ) -> Result<VoiceAdmittedSupportWork, VoiceEndpointError> {
        self.queue_admitted_support(
            grant,
            original,
            Action::EnsureHelper(peer),
            payment,
            publisher,
        )
    }
    /// Explicitly replace only the exact opaque previous direct-helper probe.
    #[allow(clippy::too_many_arguments)] // Exact original, previous identity and independent payer stay explicit.
    pub fn try_queue_refresh_admitted_direct_helper_support(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        peer: DeviceId,
        previous: SupportQueryIdentity,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedSupportPublisher>,
    ) -> Result<VoiceAdmittedSupportWork, VoiceEndpointError> {
        self.queue_admitted_support(
            grant,
            original,
            Action::RefreshHelper(peer, previous),
            payment,
            publisher,
        )
    }
    /// Ask Root to authenticate observation of this SAME current endpoint query.
    pub fn try_queue_observe_admitted_endpoint_support(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        query: EndpointSupportQuery,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedSupportPublisher>,
    ) -> Result<VoiceAdmittedSupportWork, VoiceEndpointError> {
        self.queue_admitted_support(
            grant,
            original,
            Action::ObserveEndpoint(query),
            payment,
            publisher,
        )
    }
    /// Direct-helper evidence remains a distinct observation and grants no call.
    pub fn try_queue_observe_admitted_direct_helper_support(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        query: DirectHelperSupportProbe,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedSupportPublisher>,
    ) -> Result<VoiceAdmittedSupportWork, VoiceEndpointError> {
        self.queue_admitted_support(
            grant,
            original,
            Action::ObserveHelper(query),
            payment,
            publisher,
        )
    }
    /// Consume the independently paid snapshot permit under the same admitted
    /// residence. Shared FreshPeer evidence alone says nothing about support.
    pub fn try_queue_admitted_presence_peer_snapshot(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        permit: PeerSnapshotPermit,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedSupportPublisher>,
    ) -> Result<VoiceAdmittedSupportWork, VoiceEndpointError> {
        self.queue_admitted_support(
            grant,
            original,
            Action::PresencePeers(Some(permit)),
            payment,
            publisher,
        )
    }
    fn queue_admitted_support(
        &self,
        grant: Arc<CallGrant>,
        original: ActivationObserver,
        action: Action,
        payment: ActivationRpcPermit,
        publisher: Box<dyn VoiceAdmittedSupportPublisher>,
    ) -> Result<VoiceAdmittedSupportWork, VoiceEndpointError> {
        // Check BEFORE allocating Body, InvocationState or any new queued work.
        let error = if !payment.matches(&original) || !self.matches(&original) {
            Some(VoiceEndpointError::Protocol(VoiceError::Scope))
        } else if !original.is_active() || !original.matches_grant(&grant) {
            Some(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
        } else if voice_admitted_support_rpc_bytes() > ACTIVATION_RPC_BYTES {
            Some(VoiceEndpointError::Protocol(VoiceError::Bounds))
        } else {
            None
        };
        if let Some(error) = error {
            publisher.refused(error, None, Some(payment));
            return Err(error);
        }
        let Some(owner) = self.upgrade() else {
            publisher.refused(VoiceEndpointError::Stopped, None, Some(payment));
            return Err(VoiceEndpointError::Stopped);
        };
        let Some(sender) = owner.owner.sender.upgrade() else {
            publisher.refused(VoiceEndpointError::Stopped, None, Some(payment));
            return Err(VoiceEndpointError::Stopped);
        };
        let permit = match sender.try_reserve() {
            Ok(permit) => permit,
            Err(error) => {
                let error = match error {
                    mpsc::error::TrySendError::Full(()) => {
                        VoiceEndpointError::Protocol(VoiceError::Capacity)
                    }
                    mpsc::error::TrySendError::Closed(()) => VoiceEndpointError::Stopped,
                };
                publisher.refused(error, None, Some(payment));
                return Err(error);
            }
        };
        let mut preparing = Preparing {
            arguments: Arguments {
                body: Some(Box::new(Body {
                    grant,
                    original,
                    action,
                    publisher: Some(publisher),
                    work: None,
                })),
                _retirement: None,
            },
            payment: Some(payment),
        };
        let funding = FundingOwner(Some(Arc::new(preparing.payment.take().unwrap())));
        let work = VoiceAdmittedSupportWork {
            state: Some(Arc::new(InvocationState::new(()))),
            _funding: funding.clone(),
        };
        preparing.arguments._retirement = Some(Retirement {
            marker: Some(InvocationRetirement(work.state.as_ref().unwrap().clone())),
            _funding: funding,
        });
        let body = preparing.arguments.body.as_mut().unwrap();
        body.work = Some(work.clone());
        if let Err(error) = body.publisher.as_mut().unwrap().accepted(work.clone()) {
            body.publisher
                .take()
                .unwrap()
                .refused(error, Some(work.clone()), None);
            return Err(error);
        }
        let envelope = Envelope {
            trace: self.trace,
            command: AppCommand::VoiceEndpoint(VoiceEndpointCommand(
                super::super::Command::CallActivation(Command::AdmittedSupport(Submission(Paid {
                    arguments: preparing.arguments,
                    payment: work.clone(),
                }))),
            )),
        };
        {
            let open = owner.owner.send.lock().unwrap_or_else(|e| e.into_inner());
            if *open {
                permit.send(envelope);
                return Ok(work);
            }
        }
        drop(permit);
        drop(envelope);
        Err(VoiceEndpointError::Stopped)
    }
}
/// Complete Actor body/Box/marker/result/preparation graph. Native publisher
/// cells/oneshots require separate measured payment, not this RPC's remainder.
pub fn voice_admitted_support_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Command>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<Submission>()
        + 2 * std::mem::size_of::<Body>()
        // Preserve the earlier (larger) invocation allowance after moving its
        // permit outside; add every new funding/retirement wrapper, no cuts.
        + std::mem::size_of::<(usize, usize, InvocationState<ActivationRpcPermit>)>()
        + 2 * std::mem::size_of::<FundingOwner>() + std::mem::size_of::<Retirement>()
        + 4 * std::mem::size_of::<VoiceAdmittedSupportWork>()
        + std::mem::size_of::<VoiceAdmittedSupportResult>()
        + std::mem::size_of::<(usize, usize, ActivationRpcPermit)>()
        + 2 * std::mem::size_of::<VoiceActivationControl>()
        + catcoms_sync::voice_v2::runtime::admitted_epoch_wake_bytes()
        + 512
}

#[cfg(test)]
mod tests;
