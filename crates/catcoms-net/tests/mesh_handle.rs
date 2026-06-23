//! `MeshHandle` — the clonable command-channel handle used by the desktop bridge to drive
//! rendezvous registration after the owning `MeshService` has been moved into a server actor.

use catcoms_net::MeshService;
use catcoms_rt::MeshTransport;

#[tokio::test]
async fn mesh_handle_shares_the_local_peer_and_queues_commands() {
    let mesh = MeshService::new_memory(None, &[]).unwrap();
    let handle = mesh.handle();
    // The handle addresses the same node as its `MeshService`.
    assert_eq!(handle.local_peer(), mesh.local_peer());
    // It clones (the whole point — the bridge keeps one per server while the actor owns the mesh).
    let clone = handle.clone();
    assert_eq!(clone.local_peer(), mesh.local_peer());

    // A fire-and-forget control verb is accepted by the live actor (the command queues; there is
    // no peer to actually connect to here). This is the path the bridge uses to register a fresh
    // invite's namespace post-spawn.
    let addr: libp2p::Multiaddr = "/memory/424242".parse().unwrap();
    handle.dial(addr).await.unwrap();
}
