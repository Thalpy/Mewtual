//! Product-layer end-to-end over **real OS sockets**: the one thing `product_e2e.rs` cannot do.
//!
//! Everything else in this crate's product suite runs on the in-memory mesh, which is faster,
//! deterministic and sufficient. It has one blind spot that happens to sit exactly on top of a
//! feature that shipped broken: `Hub` models no connections, so it never emits `PeerConnected`
//! or `PeerDisconnected`, and **presence is defined in terms of those two events**. Over the
//! memory transport a node that closes its app stays lit in everyone's roster forever, which
//! would make an in-memory "the dot goes out" test assert nothing at all.
//!
//! So this file founds a server on a real TCP listener, joins it from a second real libp2p node,
//! runs the product's own discovery tick until presence lights up, and then closes the second
//! node's app for real and watches the dot go out. It drives `spawn`/`ServerActor`/`AppEvent`
//! like the rest of the product suite, not the sync layer.
//!
//! This also exercises the LAN-only restart path. Private/loopback addresses remain absent from
//! signed peer records, but a joiner now seals the exact outbound direct route that completed the
//! Noise handshake and retries it after roster revalidation. Loopback is therefore the right
//! deterministic stand-in: it has the same "not publishable through PEX" property as a home LAN.

use std::sync::Arc;
use std::time::Duration;

use catcoms_app::{channel_id, spawn, AppEvent, Server, ServerActor};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::MeshService;
use catcoms_rt::{MeshTransport, OsCryptoRng, SystemClock, TransportEvent};
use catcoms_storage::Cid;
use libp2p::Multiaddr;
use tokio::time::timeout;

/// Ceiling on every wait. Real sockets and a real swarm, so this is loose enough that only a
/// genuinely dead path reaches it, and bounded so that a dead path fails rather than wedges CI.
const WAIT: Duration = Duration::from_secs(60);

/// This device's fingerprint, read off the roster the way the UI reads it.
async fn my_fp(actor: &ServerActor) -> String {
    actor
        .members()
        .await
        .into_iter()
        .find(|m| m.is_self)
        .expect("this device is in its own roster")
        .fingerprint
}

/// One discovery tick, then a query to prove it finished. Serialising the two nodes' passes this
/// way is required, not stylistic: an actor running a PEX pass is blocked on its own request and
/// is therefore not in `sync_once` to answer anyone else's, so two simultaneous passes wait each
/// other out to the per-request deadline.
async fn discovery_pass(actor: &ServerActor) {
    let _ = actor.drive_discovery().await;
    let _ = actor.member_count().await;
}

/// A member's presence dot follows a real connection: it lights when the two nodes have
/// exchanged records over a live link, and it goes out when that node closes its app. Both
/// clients are then closed and restored, proving the retained LAN route survives the exact
/// lifecycle that originally stranded an established group.
///
/// Both halves shipped broken. The lighting half was dead because nothing in the product ever
/// published or requested a peer record, so `connected_member_fingerprints` read an empty map;
/// the going-out half was dead earlier because `PeerDisconnected` was dropped on the floor, so
/// the live set only ever grew. A test that cannot disconnect a peer cannot see the second bug,
/// which is why this one uses sockets.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn presence_follows_a_real_connection_and_goes_dark_when_the_peer_closes_its_app() {
    catcoms_log::init_test();
    let general = channel_id("general");
    let reconnect = channel_id("reconnect");

    // --- Alice founds on a real, ephemeral TCP loopback port ---
    let a_key = libp2p::identity::Keypair::ed25519_from_bytes([0xA1; 32])
        .expect("deterministic founder transport key");
    let a_listen: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let (a_mesh, a_id, _) =
        MeshService::new_tcp_with_key(a_key.clone(), std::slice::from_ref(&a_listen), &[])
            .expect("bind a TCP listener");
    let a_addr: Multiaddr = timeout(WAIT, a_mesh.next_listen_addr())
        .await
        .expect("listen-addr timeout")
        .expect("Alice bound a TCP address");
    let a_peer = a_mesh.local_peer();
    assert_eq!(a_peer, catcoms_net::phase0_peer_id(&a_id));

    let mut alice = Server::found(
        a_mesh,
        MlsDevice::generate().expect("a fresh MLS provider per device"),
        OsCryptoRng,
        Box::new(SystemClock),
        "alice",
    )
    .expect("found");
    alice.subscribe_control().await.expect("subscribe control");
    let (alice, mut alice_events, alice_task) = spawn(alice);
    alice.open_channel(general).await;
    alice.open_channel(reconnect).await;
    let alice_fp = my_fp(&alice).await;
    // Production publishes this before minting/joining. The direct joiner immediately requests
    // this signed descriptor so an immediate close cannot leave its sealed socket without the
    // roster-backed transport claim required at restart.
    alice
        .publish_self_record(vec![a_addr.to_string()], 65_536)
        .await;

    let invite = alice
        .mint_invite([7u8; 16], u64::MAX, vec![a_addr.to_string()])
        .await
        .expect("the owner may mint an invite");
    let invite = InviteToken::decode(&invite).expect("the minted invite decodes");

    // --- Bob dials the address out of the invite and joins over the wire ---
    let b_key = libp2p::identity::Keypair::ed25519_from_bytes([0xB2; 32])
        .expect("deterministic test transport key");
    let b_listen: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let (b_mesh, b_id, _) = MeshService::new_tcp_with_key(
        b_key.clone(),
        std::slice::from_ref(&b_listen),
        std::slice::from_ref(&a_addr),
    )
    .expect("build the joiner node");
    let b_handle = b_mesh.handle();
    let b_mesh = Arc::new(b_mesh);
    timeout(WAIT, async {
        loop {
            match b_mesh.next_event().await {
                Some(TransportEvent::PeerConnected(p)) if p == a_peer => break,
                Some(_) => continue,
                None => panic!("the joiner's transport closed before it connected"),
            }
        }
    })
    .await
    .expect("the joiner did not connect to the founder over TCP");
    let b_mesh = Arc::try_unwrap(b_mesh).expect("sole owner once the connect wait is done");

    let mut bob = Server::join(
        b_mesh,
        MlsDevice::generate().expect("a fresh MLS provider per device"),
        OsCryptoRng,
        Box::new(SystemClock),
        "bob",
        a_peer,
        &invite,
    )
    .await
    .expect("join over real sockets");
    assert!(
        bob.request_pex(a_peer).await.expect("immediate direct PEX") > 0,
        "direct admission persists the inviter's signed transport claim before the first timer"
    );
    let bob_snapshot = bob
        .snapshot()
        .expect("snapshot Bob immediately after direct admission");
    let (bob, _bob_events, bob_task) = spawn(bob);
    bob.open_channel(general).await;
    bob.catch_up(a_peer, general).await;
    let bob_fp = my_fp(&bob).await;
    assert_eq!(
        alice.member_count().await,
        2,
        "the roster grew over the wire"
    );

    // Give the founder an ordinary authenticated product event from Bob before checking the
    // presence projection. This deliberately lands in `general` *after* the immediate Bob
    // snapshot; post-restart messaging uses the separate `reconnect` document so restoring that
    // exact snapshot cannot reuse an Automerge actor sequence.
    bob.send_message(general, "presence proof").await;
    let saw_bob_online_event = timeout(WAIT, async {
        let mut saw_online = false;
        loop {
            if alice
                .messages(general)
                .await
                .iter()
                .any(|message| message.text == "presence proof")
            {
                return saw_online;
            }
            if let Ok(Some(event)) = timeout(Duration::from_millis(20), alice_events.recv()).await {
                saw_online |= matches!(
                    &event.event,
                    AppEvent::ConnectivityChanged { online } if online.contains(&bob_fp)
                );
            }
        }
    })
    .await
    .expect("the founder never received the joiner's pre-close product event");

    // --- presence lights up ---
    //
    // Both nodes publish a record with the address they are actually reachable on. Loopback is
    // stripped as non-routable, so these records carry no dialable address at all; that is the
    // documented LAN-only shape, and presence is unaffected because it matches on the `peer_id`
    // each member signed into its **own** record, not on an address.
    bob.publish_self_record(Vec::new(), 65_536).await;
    discovery_pass(&bob).await;
    discovery_pass(&alice).await;

    assert_eq!(
        alice.online_members().await,
        vec![bob_fp.clone()],
        "the founder's roster lights the joiner's dot"
    );
    assert_eq!(bob.online_members().await, vec![alice_fp.clone()]);

    let reconnect_routes: Vec<_> = b_handle
        .authenticated_dial_routes()
        .into_iter()
        .filter(|route| route.peer == a_peer)
        .map(|route| (route.peer, route.address))
        .collect();
    assert_eq!(
        reconnect_routes.len(),
        1,
        "the joiner's exact outbound Noise-authenticated route is available to seal"
    );
    // The desktop keeps this handle inside the same `ServerEntry` that is dropped on shutdown.
    // Drop the test's extra clone too, or it would intentionally keep the swarm alive and prevent
    // the founder from observing `PeerDisconnected`.
    drop(b_handle);

    // The UI renders presence off the event, not off a poll, so the event has to fire too.
    if !saw_bob_online_event {
        timeout(WAIT, async {
            loop {
                match alice_events.recv().await {
                    Some(ev) if matches!(&ev.event, AppEvent::ConnectivityChanged { online } if online.contains(&bob_fp)) => {
                        return
                    }
                    Some(_) => continue,
                    None => panic!("the founder's actor closed"),
                }
            }
        })
        .await
        .expect("no ConnectivityChanged carried the joiner's fingerprint");
    }

    // --- and goes out when the peer closes its app ---
    //
    // Shutting the actor down drops the `Server`, which drops the `MeshService`, which closes
    // the swarm and with it the TCP connection. The founder learns about it the only way anyone
    // ever does: a `PeerDisconnected` off a real socket.
    bob.shutdown().await;
    let _ = bob_task.await;

    timeout(WAIT, async {
        loop {
            match alice_events.recv().await {
                Some(ev)
                    if matches!(&ev.event, AppEvent::ConnectivityChanged { online } if online.is_empty()) =>
                {
                    return
                }
                Some(_) => continue,
                None => panic!("the founder's actor closed"),
            }
        }
    })
    .await
    .expect("the departed member's dot never went out");
    assert!(
        alice.online_members().await.is_empty(),
        "and the query agrees with the event"
    );
    assert_eq!(
        alice.member_count().await,
        2,
        "going offline is not leaving: the roster is unchanged, only the dot"
    );

    // Close the founder too. Rebinding its exact listener before Bob returns exercises the
    // reported both-apps-closed case, not merely a transient joiner restart while an old actor
    // remains alive in memory.
    let alice_snapshot = alice.snapshot().await.expect("snapshot Alice before close");
    alice.shutdown().await;
    let _ = alice_task.await;

    let (a_mesh, restarted_a_id, _) =
        MeshService::new_tcp_with_key(a_key, std::slice::from_ref(&a_addr), &[])
            .expect("rebind Alice's persisted listener without a fresh invite");
    assert_eq!(
        restarted_a_id, a_id,
        "the founder transport identity survives"
    );
    assert_eq!(
        timeout(WAIT, a_mesh.next_listen_addr())
            .await
            .expect("restarted founder listen-addr timeout")
            .expect("restarted founder bound its persisted address"),
        a_addr,
        "the founder rebinds the route sealed by the joiner"
    );
    let mut restarted_alice = Server::restore(
        &alice_snapshot,
        a_mesh,
        OsCryptoRng,
        Box::new(SystemClock),
        "alice",
    )
    .expect("restore Alice's group state");
    restarted_alice
        .subscribe_control()
        .await
        .expect("restore Alice's control subscription");
    let (alice, mut alice_events, alice_task) = spawn(restarted_alice);
    alice.open_channel(general).await;
    alice.open_channel(reconnect).await;

    // --- recreate the closed joiner with the same identity and no fresh invite ---
    let (b_mesh, restarted_b_id, _) =
        MeshService::new_tcp_with_key(b_key.clone(), std::slice::from_ref(&b_listen), &[])
            .expect("rebuild Bob's transport without an initial dial");
    let restarted_b_addr = timeout(WAIT, b_mesh.next_listen_addr())
        .await
        .expect("restarted joiner listen-addr timeout")
        .expect("restarted joiner bound a listener");
    assert_eq!(
        restarted_b_id, b_id,
        "the persisted transport identity survives"
    );
    let mut restarted_bob = Server::restore(
        &bob_snapshot,
        b_mesh,
        OsCryptoRng,
        Box::new(SystemClock),
        "bob",
    )
    .expect("restore Bob's group state");
    restarted_bob
        .subscribe_control()
        .await
        .expect("restore the control subscription");
    restarted_bob.set_local_reconnect_routes(reconnect_routes);
    assert_eq!(
        restarted_bob.dial_local_reconnect_routes().await,
        1,
        "the sealed LAN route enters a fresh peer-bound socket dial"
    );
    let (bob, mut restarted_events, restarted_task) = spawn(restarted_bob);
    bob.open_channel(general).await;
    bob.open_channel(reconnect).await;

    timeout(WAIT, async {
        loop {
            if bob.online_members().await.contains(&alice_fp) {
                return;
            }
            let _ = timeout(Duration::from_millis(20), restarted_events.recv()).await;
        }
    })
    .await
    .expect("the restored sealed route never became a roster-bound live connection");

    bob.send_message(reconnect, "after reopening").await;
    timeout(WAIT, async {
        loop {
            if alice
                .messages(reconnect)
                .await
                .iter()
                .any(|m| m.text == "after reopening")
            {
                return;
            }
            let _ = timeout(Duration::from_millis(20), alice_events.recv()).await;
        }
    })
    .await
    .expect("the reopened client could not send over its recovered LAN route");

    alice.send_message(reconnect, "welcome back").await;
    timeout(WAIT, async {
        loop {
            if bob
                .messages(reconnect)
                .await
                .iter()
                .any(|m| m.text == "welcome back")
            {
                return;
            }
            let _ = timeout(Duration::from_millis(20), restarted_events.recv()).await;
        }
    })
    .await
    .expect("the reopened client could not receive over its recovered LAN route");

    // Exercise the file-index gossip and authenticated chunk request/response after the restart
    // too. The bytes are deterministic and the final CID assertion detects truncation, reordering
    // and corruption across the recovered connection.
    let file_bytes: Vec<u8> = (0..12_345u32)
        .map(|i| (i.wrapping_mul(29) % 251) as u8)
        .collect();
    let cid_hex = alice
        .add_file(
            "tcp-check.bin".into(),
            "application/octet-stream".into(),
            "acceptance".into(),
            file_bytes.clone(),
        )
        .await
        .expect("the restored founder can publish the deterministic test file");
    let entry = timeout(WAIT, async {
        loop {
            if let Some(entry) = bob
                .files()
                .await
                .into_iter()
                .find(|entry| entry.name == "tcp-check.bin")
            {
                return entry;
            }
            let _ = timeout(Duration::from_millis(20), restarted_events.recv()).await;
        }
    })
    .await
    .expect("the restored joiner never received the file listing over TCP");
    assert_eq!(
        entry
            .cid
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        cid_hex,
        "the listing carries the uploader's content address"
    );
    let (chunks, size) = bob
        .file_download_plan(entry.cid.clone())
        .await
        .expect("the received listing has a valid download plan");
    assert_eq!(size, file_bytes.len() as u64);
    let mut downloaded = Vec::with_capacity(size as usize);
    for index in 0..chunks {
        let (bytes, provider) = bob
            .fetch_file_chunk(entry.cid.clone(), index)
            .await
            .unwrap_or_else(|error| panic!("TCP file chunk {index}/{chunks} failed: {error}"));
        assert_eq!(provider.as_deref(), Some(alice_fp.as_str()));
        downloaded.extend_from_slice(&bytes);
    }
    assert_eq!(downloaded, file_bytes, "the file crosses TCP byte-for-byte");
    assert_eq!(
        Cid::of(&downloaded).as_bytes(),
        entry.cid.as_slice(),
        "the downloaded plaintext verifies against its content address"
    );

    // --- changed listener recovery code -----------------------------------------------------
    // Bring up Bob's replacement listener while the old one is still bound, which guarantees the
    // address really changes rather than relying on an OS ephemeral-port allocation coincidence.
    // The restored server intentionally receives no sealed fallback route, so neither side has a
    // route that can reconnect until Alice applies Bob's current-member-signed code.
    let changed_snapshot = bob
        .snapshot()
        .await
        .expect("snapshot before address change");
    let (changed_mesh, changed_b_id, _) =
        MeshService::new_tcp_with_key(b_key, std::slice::from_ref(&b_listen), &[])
            .expect("bind Bob's replacement listener");
    assert_eq!(changed_b_id, b_id, "the transport key remains stable");
    let changed_b_addr = timeout(WAIT, changed_mesh.next_listen_addr())
        .await
        .expect("changed joiner listen-addr timeout")
        .expect("changed joiner bound a listener");
    assert_ne!(
        changed_b_addr, restarted_b_addr,
        "the recovery test must actually replace the listener address"
    );

    bob.shutdown().await;
    let _ = restarted_task.await;
    let mut changed_bob = Server::restore(
        &changed_snapshot,
        changed_mesh,
        OsCryptoRng,
        Box::new(SystemClock),
        "bob",
    )
    .expect("restore Bob on the changed listener");
    changed_bob
        .subscribe_control()
        .await
        .expect("subscribe the changed listener");
    // A recovery route must terminate in the same transport identity that the signed member
    // record claims. `next_listen_addr` reports only the socket, so append the stable peer id just
    // as the desktop's advertised listener collector does before minting a code.
    let changed_b_route = format!("{changed_b_addr}/p2p/{changed_b_id}");
    let recovery = changed_bob
        .mint_member_recovery_code(vec![changed_b_route])
        .expect("mint a member-signed code for the changed listener");
    let (bob, mut changed_events, changed_task) = spawn(changed_bob);
    bob.open_channel(reconnect).await;

    let applied = alice
        .apply_member_recovery(recovery.encode())
        .await
        .expect("Alice verifies and submits Bob's recovery code");
    assert_eq!(applied.submitted_routes, 1);
    timeout(WAIT, async {
        loop {
            if alice.online_members().await.contains(&bob_fp)
                && bob.online_members().await.contains(&alice_fp)
            {
                return;
            }
            let _ = timeout(Duration::from_millis(20), changed_events.recv()).await;
            let _ = timeout(Duration::from_millis(20), alice_events.recv()).await;
        }
    })
    .await
    .expect("the changed listener recovery code never produced an authenticated member path");

    bob.send_message(reconnect, "after changing address").await;
    timeout(WAIT, async {
        loop {
            if alice
                .messages(reconnect)
                .await
                .iter()
                .any(|message| message.text == "after changing address")
            {
                return;
            }
            let _ = timeout(Duration::from_millis(20), alice_events.recv()).await;
        }
    })
    .await
    .expect("messaging did not resume after changed-address recovery");

    bob.shutdown().await;
    let _ = changed_task.await;

    alice.shutdown().await;
    let _ = alice_task.await;
}
