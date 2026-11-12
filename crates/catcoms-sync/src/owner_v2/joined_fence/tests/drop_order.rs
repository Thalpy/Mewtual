//! Probe the real retained fence at the start of an actual node resource destructor.
//! The genuine MLS join is synchronous; this transport never claims network acceptance.
use super::*;

type Observation = (bool, Result<(), V2JoinedFenceRefusal>);
struct ProbeTransport {
    fence: Arc<Mutex<Option<V2JoinedSourceFence>>>,
    observed: Arc<Mutex<Option<Observation>>>,
}
impl Drop for ProbeTransport {
    fn drop(&mut self) {
        if let Some(fence) = self.fence.lock().unwrap().take() {
            let closed = fence.is_closed();
            // Drop any incorrectly admitted guard here, so the OLD order fails the
            // assertion without deadlocking the later fence destructor.
            let admission = fence.admit_final(&OWN).map(drop);
            *self.observed.lock().unwrap() = Some((closed, admission));
        }
    }
}
#[async_trait::async_trait]
impl MeshTransport for ProbeTransport {
    fn local_peer(&self) -> PeerId {
        PeerId::from_u64(0xd262)
    }
    async fn subscribe(&self, _: catcoms_rt::Topic) -> Result<(), catcoms_rt::TransportError> {
        Ok(())
    }
    async fn unsubscribe(&self, _: catcoms_rt::Topic) -> Result<(), catcoms_rt::TransportError> {
        Ok(())
    }
    async fn publish(
        &self,
        _: catcoms_rt::Topic,
        _: bytes::Bytes,
    ) -> Result<(), catcoms_rt::TransportError> {
        Ok(())
    }
    async fn request(
        &self,
        _: PeerId,
        _: catcoms_rt::ProtocolId,
        _: bytes::Bytes,
    ) -> Result<bytes::Bytes, catcoms_rt::TransportError> {
        Err(catcoms_rt::TransportError::Unreachable)
    }
    async fn request_cancellable(
        &self,
        peer: PeerId,
        protocol: catcoms_rt::ProtocolId,
        data: bytes::Bytes,
        _: catcoms_rt::RequestCancellation,
    ) -> Result<bytes::Bytes, catcoms_rt::TransportError> {
        self.request(peer, protocol, data).await
    }
    async fn next_event(&self) -> Option<catcoms_rt::TransportEvent> {
        None
    }
}

#[test]
fn node_drop_closes_joined_admission_before_transport_teardown_begins() {
    let mut f = founder(0xd261);
    let device = MlsDevice::generate().unwrap();
    let invite = f.mint_invite([0xec; 16], 10_000, Vec::new()).unwrap();
    let package = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let expected = device.key_package_reference(&package).unwrap();
    let (welcome, routing, signature) = f
        .admit_now(&invite, &serialize_key_package(&package).unwrap(), 1_000)
        .unwrap();
    let (group, routing) =
        finish_join(&device, &invite, &welcome, &signature, &routing, &expected).unwrap();
    let slot = Arc::new(Mutex::new(None));
    let observed = Arc::new(Mutex::new(None));
    let mut node = ChannelSync::new_joined(
        ProbeTransport {
            fence: slot.clone(),
            observed: observed.clone(),
        },
        group,
        device,
        ChaCha20Rng::seed_from_u64(0xd262),
        Box::new(ManualClock::new(1_000)),
        routing,
    );
    let instance = node.registry_instance();
    let (_, fence) = node.take_v2_joined_join_facts(&instance).unwrap();
    fence
        .install_selected(node.epoch(), node.group.epoch_authenticator_id(), OWN)
        .unwrap();
    drop(fence.admit_final(&OWN).unwrap());
    *slot.lock().unwrap() = Some(fence);
    // An idle fixture has no detached request/maintenance owners. The node's transport
    // field is its final strong owner, so its destructor really tests field ordering.
    assert_eq!(Arc::strong_count(&node.transport), 1);
    drop(node);
    assert_eq!(
        *observed.lock().unwrap(),
        Some((true, Err(V2JoinedFenceRefusal::Closed)))
    );
}
