//! Driver-owned aggregate connection generations. Revocation precedes every disconnect event,
//! so queued consumers need not race the sole event reader. A second path keeps the same owner;
//! complete loss and same-key reconnect can never revive one captured by earlier work.
use super::*;
type Table = Arc<std::sync::Mutex<HashMap<PeerId, catcoms_rt::ConnectionLease>>>;
#[derive(Default, Debug)]
pub(super) struct Owners {
    pub(super) table: Table,
}
impl Owners {
    pub(super) fn connected(&self, peer: PeerId) {
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        if table.len() < catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT {
            table.entry(peer).or_default();
        }
    }
    pub(super) fn disconnected(&self, peer: PeerId) {
        if let Some(lease) = self
            .table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&peer)
        {
            lease.revoke();
        }
    }
    pub(super) fn clear(&self) {
        for (_, lease) in self.table.lock().unwrap_or_else(|e| e.into_inner()).drain() {
            lease.revoke();
        }
    }
}
impl Drop for Owners {
    fn drop(&mut self) {
        self.clear();
    }
}
pub(super) fn current(table: &Table, peer: PeerId) -> Option<catcoms_rt::ConnectionLease> {
    table
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&peer)
        .filter(|lease| lease.is_live())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn driver_connection_owner_closes_before_events_and_same_peer_reconnect_is_fresh() {
        let owners = Owners::default();
        let peer = PeerId::from_u64(1);
        owners.connected(peer);
        let first = current(&owners.table, peer).unwrap();
        owners.connected(peer);
        assert!(first.is_same(&current(&owners.table, peer).unwrap()));
        owners.disconnected(peer);
        // No application event has been consumed; the driver boundary already refuses first.
        first.revoked().await;
        assert!(!first.is_live());
        owners.connected(peer);
        let next = current(&owners.table, peer).unwrap();
        assert!(next.is_live() && !first.is_same(&next) && !first.is_live());
        drop(owners);
        next.revoked().await;
        assert!(!next.is_live());
    }
    #[test]
    fn driver_connection_owner_table_is_bounded_without_evicting_live_authority() {
        let owners = Owners::default();
        for n in 0..=catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT {
            owners.connected(PeerId::from_u64(n as u64));
        }
        assert_eq!(
            owners.table.lock().unwrap().len(),
            catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT
        );
        assert!(current(
            &owners.table,
            PeerId::from_u64(catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT as u64)
        )
        .is_none());
    }
    #[derive(Debug)]
    struct LeaseAuthority(catcoms_rt::ConnectionLease);
    #[async_trait::async_trait]
    impl catcoms_rt::RequestAuthority for LeaseAuthority {
        fn authorized(&self) -> bool {
            self.0.is_live()
        }
        async fn revoked(&self) {
            self.0.revoked().await;
        }
    }
    #[tokio::test]
    async fn driver_connection_owner_rejects_old_queued_request_after_real_noise_reconnect() {
        let listen: Multiaddr = "/memory/918296".parse().unwrap();
        let mut swarm = build_memory_swarm();
        let server_id = *swarm.local_peer_id();
        swarm.listen_on(listen.clone()).unwrap();
        let server = MeshService::spawn(swarm);
        let client = MeshService::new_memory(None, &[]).unwrap();
        let bound: Multiaddr = format!("{listen}/p2p/{server_id}").parse().unwrap();
        client.dial(bound.clone()).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(3),
            client.wait_for_peer_connected(server.local_peer()),
        )
        .await
        .unwrap()
        .unwrap();
        let old = client.connection_lease(server.local_peer()).unwrap();
        server.evict_peer(client.local_peer()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), old.revoked())
            .await
            .unwrap();
        let mut snapshots = client.connection_snapshot_rx.clone();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !snapshots.borrow_and_update().is_empty() {
                snapshots.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        // Client next_event has never been consumed. Driver revocation already precedes its
        // undrained disconnect. Reconnect the same authenticated peer without draining it.
        server.unevict_peer(client.local_peer()).await.unwrap();
        client.dial(bound).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(3),
            client.wait_for_peer_connected(server.local_peer()),
        )
        .await
        .unwrap()
        .unwrap();
        let current = client.connection_lease(server.local_peer()).unwrap();
        assert!(current.is_live() && !current.is_same(&old) && !old.is_live());
        let (_publisher, signal) = watch::channel(false);
        let (reply, response) = oneshot::channel();
        // Bypass caller-side checks so this proves the actual queued driver boundary refuses.
        client
            .cmd_tx
            .send(Command::RequestConnected {
                peer: server.local_peer(),
                data: Bytes::from_static(b"stale"),
                reply,
                cancellation: Some(
                    RequestCancellation::new(signal, None)
                        .with_authority(Arc::new(LeaseAuthority(old))),
                ),
            })
            .await
            .unwrap();
        assert!(matches!(
            response.await.unwrap(),
            Err(TransportError::Cancelled)
        ));
        let (_publisher, signal) = watch::channel(false);
        let (reply, response) = oneshot::channel();
        client
            .cmd_tx
            .send(Command::RequestConnected {
                peer: server.local_peer(),
                data: Bytes::from_static(b"fresh"),
                reply,
                cancellation: Some(
                    RequestCancellation::new(signal, None)
                        .with_authority(Arc::new(LeaseAuthority(current.clone()))),
                ),
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(TransportEvent::Request {
                    data, responder, ..
                }) = server.next_event().await
                {
                    assert_eq!(&data[..], b"fresh");
                    responder.respond(Bytes::from_static(b"ok"));
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(&response.await.unwrap().unwrap()[..], b"ok");
        drop(client);
        tokio::time::timeout(Duration::from_secs(3), current.revoked())
            .await
            .unwrap();
    }
}
