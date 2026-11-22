//! Dedicated, bounded admission for Chat V2 requests on the original native mesh.
use catcoms_rt::{RequestAuthority, TransportEvent};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{mpsc, watch};

pub(super) type Slot = Mutex<Option<Subscription>>;

pub(super) struct Subscription {
    queue: mpsc::Sender<TransportEvent>,
    epoch: Weak<Epoch>,
}

#[derive(Debug)]
struct Epoch {
    authority: Arc<dyn RequestAuthority>,
    lifetime: watch::Receiver<()>,
}

#[async_trait::async_trait]
impl RequestAuthority for Epoch {
    fn authorized(&self) -> bool {
        self.lifetime.has_changed().is_ok() && self.authority.authorized()
    }

    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let mut lifetime = self.lifetime.clone();
        tokio::select! {
            _ = lifetime.changed() => {},
            _ = self.authority.revoked() => {},
        }
    }
}

/// Observation of one native mesh's JoinRequest lane. A clone cannot keep the
/// mesh or a subscriber alive. The caller must supply its actual original
/// session/Source negative and fund the local subscriber before attaching.
#[derive(Clone, Debug)]
pub struct NativeChatJoinIngress(pub(super) Weak<Slot>);

impl NativeChatJoinIngress {
    /// Claim the sole queue without waiting. A receiver never inherits events
    /// from an earlier registration, even on the same original mesh.
    pub fn try_attach(
        &self,
        authority: Arc<dyn RequestAuthority>,
    ) -> Option<NativeChatJoinRequests> {
        if !authority.authorized() {
            return None;
        }
        let slot = self.0.upgrade()?;
        let mut held = slot.try_lock().ok()?;
        if held.as_ref().is_some_and(|old| !old.queue.is_closed()) {
            return None;
        }
        let (queue, events) = mpsc::channel(1);
        let (owner, lifetime) = watch::channel(());
        let epoch = Arc::new(Epoch {
            authority,
            lifetime,
        });
        let previous = held.replace(Subscription {
            queue,
            epoch: Arc::downgrade(&epoch),
        });
        drop(held);
        drop(previous);
        Some(NativeChatJoinRequests {
            _owner: owner,
            events,
            epoch,
        })
    }
}

/// One original registration. Dropping it revokes the exact ingress for
/// requests that are queued, being handled, or awaiting the lower send.
pub struct NativeChatJoinRequests {
    // Close the original epoch before discarding any queued request bytes.
    _owner: watch::Sender<()>,
    events: mpsc::Receiver<TransportEvent>,
    epoch: Arc<Epoch>,
}

impl NativeChatJoinRequests {
    pub async fn next(&mut self) -> Option<TransportEvent> {
        loop {
            let event = tokio::select! {
                biased;
                _ = self.epoch.revoked() => None,
                event = self.events.recv() => event,
            };
            let event = event?;
            if matches!(&event, TransportEvent::Request { responder, .. } if responder.is_current())
            {
                return Some(event);
            }
        }
    }
}

pub(super) fn try_reserve(
    slot: &Slot,
) -> Option<(mpsc::OwnedPermit<TransportEvent>, Arc<dyn RequestAuthority>)> {
    let (queue, epoch) = {
        let slot = slot.try_lock().ok()?;
        let current = slot.as_ref()?;
        (current.queue.clone(), current.epoch.upgrade()?)
    };
    if !epoch.authorized() {
        return None;
    }
    Some((queue.try_reserve_owned().ok()?, epoch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keypair_from_seed, MeshService, NativeTcpPreparation, RR_PROTOCOL};
    use bytes::Bytes;
    use catcoms_rt::{MeshTransport, ProtocolId, TransportError, CHAT_V2_JOIN_KIND};
    use std::future::Future;
    use std::time::Duration;

    #[derive(Debug)]
    struct Current;

    #[async_trait::async_trait]
    impl RequestAuthority for Current {
        fn authorized(&self) -> bool {
            true
        }

        async fn revoked(&self) {
            std::future::pending::<()>().await;
        }
    }

    async fn bounded<F: Future>(future: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(8), future)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn native_join_transport_dispatch_preserves_original_epoch_and_ordinary_stream() {
        let mut prep = NativeTcpPreparation::new((), Arc::new(()));
        let (server, id, _) = prep
            .build(
                keypair_from_seed([117; 32]).unwrap(),
                &["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
                &[],
            )
            .unwrap();
        let (client, _) = MeshService::new_tcp(None, &[]).unwrap();
        assert!(client.native_chat_join_ingress().is_none());
        let address = bounded(server.next_listen_addr()).await.unwrap();
        bounded(client.dial(format!("{address}/p2p/{id}").parse().unwrap()))
            .await
            .unwrap();
        let from = client.local_peer();
        let peer = server.local_peer();
        bounded(client.wait_for_peer_connected(peer)).await.unwrap();
        let join = || {
            client.request_connected(
                peer,
                ProtocolId(RR_PROTOCOL),
                Bytes::from_static(&[CHAT_V2_JOIN_KIND, 7]),
            )
        };
        assert!(matches!(
            bounded(join()).await,
            Err(TransportError::NoResponse)
        ));

        let ingress = server.native_chat_join_ingress().unwrap();
        let mut original = ingress.try_attach(Arc::new(Current)).unwrap();
        assert!(ingress.try_attach(Arc::new(Current)).is_none());
        let wait = async {
            let Some(TransportEvent::Request {
                from: source,
                data,
                responder,
                ..
            }) = original.next().await
            else {
                panic!("original native ingress request");
            };
            assert_eq!(source, from);
            assert_eq!(data.as_ref(), &[CHAT_V2_JOIN_KIND, 7]);
            drop(original);
            assert!(!responder.is_current());
            assert!(responder
                .try_respond(Bytes::from_static(b"old Welcome"))
                .is_err());
        };
        let (retired, ()) = bounded(async { tokio::join!(join(), wait) }).await;
        assert!(matches!(retired, Err(TransportError::NoResponse)));

        let mut current = ingress.try_attach(Arc::new(Current)).unwrap();
        let serve = async {
            let Some(TransportEvent::Request { responder, .. }) = current.next().await else {
                panic!("replacement native ingress request");
            };
            responder
                .try_respond(Bytes::from_static(b"Welcome"))
                .unwrap();
        };
        // Keep this exact receiver live until after the actual lower reply.
        let (response, ()) = bounded(async { tokio::join!(join(), serve) }).await;
        assert_eq!(response.unwrap().as_ref(), b"Welcome");
        drop(current);

        let serve_ordinary = async {
            while let Some(event) = server.next_event().await {
                if let TransportEvent::Request {
                    data, responder, ..
                } = event
                {
                    assert_eq!(data.as_ref(), &[45]);
                    responder
                        .try_respond(Bytes::from_static(b"ordinary"))
                        .unwrap();
                    return;
                }
            }
            panic!("ordinary ingress request");
        };
        let ordinary =
            client.request_connected(peer, ProtocolId(RR_PROTOCOL), Bytes::from_static(&[45]));
        let (response, ()) = bounded(async { tokio::join!(ordinary, serve_ordinary) }).await;
        assert_eq!(response.unwrap().as_ref(), b"ordinary");
        let mut final_receiver = ingress.try_attach(Arc::new(Current)).unwrap();
        drop(server);
        assert!(bounded(final_receiver.next()).await.is_none());
        assert!(ingress.try_attach(Arc::new(Current)).is_none());
    }
}
