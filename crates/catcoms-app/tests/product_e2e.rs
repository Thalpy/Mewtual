//! Product-layer end-to-end tests: the surface the desktop UI actually drives.
//!
//! Every other test suite in this workspace is shaped like a *library's*. They drive
//! `ChannelSync` and the protocol crates directly, which is why 411 of them passed while three
//! shipped features (live member presence, the phase-9g cross-session re-dial and the eclipse
//! advisory) were completely dead in the product: member peer-exchange was written, unit-tested
//! and never called from anything above `catcoms-sync`. A test that calls a primitive itself
//! passes whether or not the product ever calls it.
//!
//! So the defining property of this file is: **a test here must fail if the product path is
//! dead, even when every crate-level test passes.** Each one therefore
//!
//! - drives the `Server` facade and the `spawn`/`ServerActor`/`AppEvent` substrate, which is
//!   exactly what the Tauri bridge in `apps/desktop/src-tauri` drives, in the same order,
//! - mirrors the bridge's own call sequence (found → publish record → open channel → catch up →
//!   discovery tick) rather than reaching for whichever primitive is most convenient, and
//! - asserts something a **user** would notice: a message arriving, a dot lighting up, a file
//!   downloading byte-for-byte, a kick being refused.
//!
//! Determinism notes, which are not optional here:
//!
//! - Multi-party async flows are **never** driven with `tokio::select!`. It cancels the
//!   non-winning futures mid-tick, so a node never reaches the point where it processes the
//!   gossip it should have received and the test hangs while the implementation is fine. The
//!   actors each own their own task; the test sequences them with awaited queries.
//! - Two actors must not run a PEX pass *at the same time*: each blocks its own loop on the
//!   request it issued, so neither is in `sync_once` to answer the other and both wait out the
//!   3s per-request deadline. `discovery_pass` below serialises them the way one node's timer
//!   tick does in the real app.
//! - No ambient time or RNG: every clock is a shared `ManualClock` and every RNG a seeded
//!   `ChaCha20Rng`, so `scripts/check-no-ambient.sh` stays satisfied and reruns are identical.
//! - Test logic never keys on byte patterns of generated identities (a real device id is a
//!   BLAKE3 hash and will match any such pattern sooner or later); it uses the exact
//!   fingerprints captured off the roster.

use std::sync::Arc;
use std::time::Duration;

use catcoms_app::{
    channel_id, peer_addrs_from_snapshot, spawn, AppEvent, Profile, Server, ServerActor, ServerNet,
    ServerStore,
};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_rt::{Hub, ManualClock, PeerId};
use catcoms_storage::Cid;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// The channel the desktop app opens on found/join, addressed exactly as the UI addresses it.
fn general() -> u128 {
    channel_id("general")
}

/// Ceiling on any wait. Generous, because it is only ever hit when the product path is broken;
/// a working one converges in milliseconds. A bound (rather than an open await) means a dead
/// path *fails* the test instead of wedging CI.
const WAIT: Duration = Duration::from_secs(60);

/// A stand-in reachable address for a node's published peer record. Loopback and LAN addresses
/// are stripped inside `publish_self_record`, so a record built from them would carry none and
/// the cross-session cache would have nothing to hold; TEST-NET-3 is routable-looking and
/// unroutable in reality.
fn advertised(n: u64) -> Vec<String> {
    vec![format!("/ip4/203.0.113.{n}/tcp/9000")]
}

/// One running node: the actor handle the UI holds, its event stream, and the identity facts a
/// test needs to address it (its transport peer id and its device fingerprint).
#[derive(Debug)]
struct Node {
    actor: ServerActor,
    events: Receiver<AppEvent>,
    task: JoinHandle<()>,
    peer: PeerId,
    fp: String,
    /// This node's peer-record sequence source, mirroring the on-disk `ServerNet` block the
    /// bridge reserves per launch. A record published at a `seq` a peer has already seen is
    /// discarded by that peer forever, so the number must only ever go up.
    seq: u64,
}

impl Node {
    /// Publish this node's signed peer record, as found/join/reload do. Presence, the
    /// cross-session re-dial and the eclipse detector's reach term all read the map this fills;
    /// nothing in the product called it before `32dab2a`, which is why all three were dead.
    async fn publish_record(&mut self, addresses: Vec<String>) {
        self.seq += 65_536; // one reserved block per publish, exactly like `ServerNet`
        self.actor.publish_self_record(addresses, self.seq).await;
    }

    /// Run one discovery tick: what the bridge's per-server timer sends every minute.
    ///
    /// The trailing query is the point. `drive_discovery` is fire-and-forget, so without it the
    /// test would race the pass it just asked for; commands are FIFO on one channel, so a reply
    /// to *any* later query proves the PEX pass finished. That is also what keeps two nodes from
    /// PEX-ing each other simultaneously and deadlocking until their request deadlines expire.
    async fn discovery_pass(&self) {
        let _ = self.actor.drive_discovery().await;
        let _ = self.actor.member_count().await;
    }

    async fn shutdown(self) {
        self.actor.shutdown().await;
        let _ = self.task.await;
    }
}

/// This device's fingerprint, read the way the roster UI reads it rather than from a private
/// accessor: the one `MemberView` flagged `is_self`.
async fn my_fp(actor: &ServerActor) -> String {
    actor
        .members()
        .await
        .into_iter()
        .find(|m| m.is_self)
        .expect("this device is in its own roster")
        .fingerprint
}

/// Block until an actor has finished its start-up work (opening + subscribing the profile,
/// livery, badge, device, file, status, calendar, wiki and roles documents).
///
/// Any query does it: the command channel is FIFO behind that initialisation, so a reply proves
/// the documents are open and their topics subscribed. Without the barrier a peer can publish
/// before this node has subscribed and the op is simply never delivered, which is a hang, not a
/// failure.
async fn ready(actor: &ServerActor) {
    let _ = actor.member_count().await;
}

/// Found a server and spawn its actor, mirroring the bridge's `found_server`.
async fn found_node(
    hub: &Arc<Hub>,
    clock: &ManualClock,
    peer: PeerId,
    name: &str,
    seed: u64,
) -> Node {
    let mut server = Server::found(
        hub.join(peer),
        MlsDevice::generate().expect("a fresh MLS provider per device"),
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(clock.clone()),
        name,
    )
    .expect("found");
    server.subscribe_control().await.expect("subscribe control");
    let (actor, events, task) = spawn(server);
    actor.open_channel(general()).await;
    let fp = my_fp(&actor).await;
    Node {
        actor,
        events,
        task,
        peer,
        fp,
        seq: 0,
    }
}

/// Mint an invite the way the UI does: through the actor, single-use, non-expiring for a test.
async fn mint(actor: &ServerActor, nonce: u8) -> InviteToken {
    let bytes = actor
        .mint_invite([nonce; 16], u64::MAX, Vec::new())
        .await
        .expect("the owner may mint an invite");
    InviteToken::decode(&bytes).expect("the minted invite decodes")
}

/// Redeem an invite and spawn the joiner's actor, mirroring the bridge's `join_server` verbatim,
/// including the catch-up fan-out it issues against the inviter right after `spawn`.
///
/// This mirrors the bridge including its `subscribe_control` call. That call used to be missing
/// on both join paths while `found_server` and `reload_one` had it, and the asymmetry meant a
/// joiner never received another membership commit: a third member was invisible to it forever.
/// This suite found that, and the bridge now subscribes on join too. Keep the two in step: these
/// tests are only worth anything if they run the sequence the product runs.
async fn join_node(
    hub: &Arc<Hub>,
    clock: &ManualClock,
    peer: PeerId,
    name: &str,
    seed: u64,
    inviter: PeerId,
    invite: &InviteToken,
) -> Node {
    let server = Server::join(
        hub.join(peer),
        MlsDevice::generate().expect("a fresh MLS provider per device"),
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(clock.clone()),
        name,
        inviter,
        invite,
    )
    .await
    .expect("join");
    let mut server = server;
    server.subscribe_control().await.expect("subscribe control");
    let (actor, events, task) = spawn(server);
    actor.open_channel(general()).await;
    actor.catch_up(inviter, general()).await;
    actor.catch_up_profiles(inviter).await;
    actor.catch_up_livery(inviter).await;
    actor.catch_up_badges(inviter).await;
    actor.catch_up_files(inviter).await;
    actor.catch_up_status(inviter).await;
    actor.catch_up_calendar(inviter).await;
    actor.catch_up_wiki(inviter).await;
    actor.catch_up_roles(inviter).await;
    let fp = my_fp(&actor).await;
    Node {
        actor,
        events,
        task,
        peer,
        fp,
        seq: 0,
    }
}

/// `join_node`, with the control-topic subscription deliberately withheld.
///
/// This is how a **genuinely missed** membership commit is modelled without reaching past the
/// product surface: a node in this state never receives the broadcast admitting anybody after
/// it, which is exactly the delivery failure the recovery path exists for. Everything else is
/// the bridge's join sequence verbatim, so what recovers (or fails to) is the product's.
#[allow(clippy::too_many_arguments)]
async fn join_node_off_the_control_topic(
    hub: &Arc<Hub>,
    clock: &ManualClock,
    peer: PeerId,
    name: &str,
    seed: u64,
    inviter: PeerId,
    invite: &InviteToken,
) -> Node {
    let server = Server::join(
        hub.join(peer),
        MlsDevice::generate().expect("a fresh MLS provider per device"),
        ChaCha20Rng::seed_from_u64(seed),
        Box::new(clock.clone()),
        name,
        inviter,
        invite,
    )
    .await
    .expect("join");
    let (actor, events, task) = spawn(server);
    actor.open_channel(general()).await;
    actor.catch_up(inviter, general()).await;
    actor.catch_up_profiles(inviter).await;
    let fp = my_fp(&actor).await;
    Node {
        actor,
        events,
        task,
        peer,
        fp,
        seq: 0,
    }
}

/// Poll `probe` until it yields a value, draining `events` between attempts.
///
/// The drain is load-bearing, not tidiness: the actor's event channel is bounded, so a test that
/// waits on a query while never reading events can fill it and stall the very actor it is
/// waiting for. The short inner bound also paces the loop without a sleep, which the
/// ambient-dependency gate forbids.
async fn until<F, Fut, T>(label: &str, events: &mut Receiver<AppEvent>, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    timeout(WAIT, async {
        loop {
            if let Some(value) = probe().await {
                return value;
            }
            let _ = timeout(Duration::from_millis(20), events.recv()).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the product never converged: {label}"))
}

/// Wait for the first event matching `pred`. Fails (rather than hangs) if it never arrives.
async fn wait_event(events: &mut Receiver<AppEvent>, pred: impl Fn(&AppEvent) -> bool) -> AppEvent {
    timeout(WAIT, async {
        loop {
            match events.recv().await {
                Some(ev) if pred(&ev) => return ev,
                Some(_) => continue,
                None => panic!("the actor closed before emitting the event under test"),
            }
        }
    })
    .await
    .expect("the expected event never arrived")
}

/// Every event currently queued, without waiting for more.
async fn drain(events: &mut Receiver<AppEvent>) -> Vec<AppEvent> {
    let mut out = Vec::new();
    while let Ok(Some(ev)) = timeout(Duration::from_millis(20), events.recv()).await {
        out.push(ev);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// 1. Found, invite, join, talk.
// ---------------------------------------------------------------------------------------------

/// The whole product in one line: someone founds a server, hands out an invite, someone else
/// redeems it, and the two of them can talk. If this fails, nothing else in the app matters.
///
/// It asserts convergence in **both** directions deliberately. The founder-to-joiner direction
/// alone would pass on a build where the joiner's own ops never reached the founder, which is a
/// silent half-broken chat rather than an obviously broken one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_members_found_invite_join_and_talk_both_ways() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;
    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;

    assert_eq!(alice.actor.member_count().await, 2, "the roster grew");
    assert_eq!(bob.actor.member_count().await, 2);

    // Founder to joiner, live over the channel topic the joiner subscribed on open.
    alice.actor.send_message(general(), "welcome bob").await;
    let mut alice_events = alice.events;
    let alice_actor = alice.actor;
    until(
        "the joiner receives the founder's first message",
        &mut bob.events,
        || async {
            let msgs = bob.actor.messages(general()).await;
            msgs.iter().any(|m| m.text == "welcome bob").then_some(())
        },
    )
    .await;

    // Joiner to founder. This is the direction the invite/Welcome path does not exercise.
    bob.actor.send_message(general(), "thanks alice").await;
    until(
        "the founder receives the joiner's reply",
        &mut alice_events,
        || async {
            let msgs = alice_actor.messages(general()).await;
            msgs.iter().any(|m| m.text == "thanks alice").then_some(())
        },
    )
    .await;

    // Converged, not merely "each saw the other's one message": the same ordered log on both.
    let a: Vec<String> = until(
        "the founder's log holds both messages",
        &mut alice_events,
        || async {
            let msgs = alice_actor.messages(general()).await;
            (msgs.len() == 2).then(|| msgs.into_iter().map(|m| m.text).collect())
        },
    )
    .await;
    let b: Vec<String> = until(
        "the joiner's log holds both messages",
        &mut bob.events,
        || async {
            let msgs = bob.actor.messages(general()).await;
            (msgs.len() == 2).then(|| msgs.into_iter().map(|m| m.text).collect())
        },
    )
    .await;
    assert_eq!(a, b, "both members materialize the same conversation");

    // Attribution survives the round trip: each message is authored by its sender's fingerprint,
    // which is what the roster and the profile renderer key on.
    let authors: Vec<String> = alice_actor
        .messages(general())
        .await
        .into_iter()
        .map(|m| m.author)
        .collect();
    assert!(authors.contains(&alice.fp) && authors.contains(&bob.fp));

    alice_actor.shutdown().await;
    let _ = alice.task.await;
    bob.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// 2. Presence.
// ---------------------------------------------------------------------------------------------

/// Roster online dots, at the layer that ships them.
///
/// `catcoms-app`'s own `two_members_exchange_records_and_report_each_other_online` pins the same
/// invariant one layer down, against `Server`. This one is the product form: it goes through
/// `spawn`, the `DriveDiscovery` command the bridge's timer sends, and the `ConnectivityChanged`
/// event the UI actually renders from, with a third member and a departure on top. Presence read
/// zero for the entire life of the feature because nothing at this altitude ever ran.
///
/// The asymmetry asserted below is real and worth pinning: presence is evidence of a *live
/// connection*, and over a transport with no dial verb only the node that ran the PEX pass has
/// that evidence. Alice, the hub every join went through, sees both of the others; each of them
/// sees only Alice. Over libp2p the cached-peer dial closes that gap; the product must not
/// pretend to a connection it has not made.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn presence_lights_up_across_the_roster_and_goes_dark_when_a_member_leaves() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let mut alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;

    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;

    // Before any record is published this is exactly the shipping app's state: two members that
    // have joined and can talk, and a roster with every dot dark.
    assert!(
        alice.actor.online_members().await.is_empty(),
        "the regressed bug: an empty record map means nobody is ever online"
    );

    alice.publish_record(advertised(1)).await;
    bob.publish_record(advertised(2)).await;

    // One discovery tick each, serialised (see the module note on simultaneous PEX passes).
    alice.discovery_pass().await;
    bob.discovery_pass().await;

    assert_eq!(
        alice.actor.online_members().await,
        vec![bob.fp.clone()],
        "Alice's roster shows Bob online"
    );
    assert_eq!(bob.actor.online_members().await, vec![alice.fp.clone()]);

    // The UI does not poll `online_members`; it renders whatever `ConnectivityChanged` carries.
    // A build that filled the map but never emitted the event would still ship dark dots.
    let ev = wait_event(&mut alice.events, |e| {
        matches!(e, AppEvent::ConnectivityChanged { .. })
    })
    .await;
    match ev {
        AppEvent::ConnectivityChanged { online } => assert!(online.contains(&bob.fp)),
        other => panic!("expected ConnectivityChanged, got {other:?}"),
    }

    // --- a third member joins ---
    clock.advance_ms(60_000); // the next discovery tick, an app-minute later
    let invite = mint(&alice.actor, 8).await;
    let mut carol = join_node(
        &hub,
        &clock,
        PeerId::from_u64(3),
        "carol",
        3,
        alice.peer,
        &invite,
    )
    .await;
    carol.publish_record(advertised(3)).await;
    assert_eq!(alice.actor.member_count().await, 3);

    alice.discovery_pass().await;
    carol.discovery_pass().await;

    let mut online = alice.actor.online_members().await;
    online.sort();
    let mut expected = vec![bob.fp.clone(), carol.fp.clone()];
    expected.sort();
    assert_eq!(
        online, expected,
        "the founder sees both other members online"
    );
    assert_eq!(
        carol.actor.online_members().await,
        vec![alice.fp.clone()],
        "the newest member sees the peer it actually spoke to"
    );

    // --- and one goes away ---
    //
    // Removal is the departure this transport can model faithfully: the in-memory hub has no
    // connect/disconnect signal, so a node that merely stops answering stays in the live set
    // (the real `PeerDisconnected` path is exercised over sockets in `catcoms-sync`'s TCP
    // suites). A kick is the user-visible departure that matters most anyway; the presence set
    // is filtered by current roster membership, so the dot must go out with the member.
    carol.actor.shutdown().await;
    let _ = carol.task.await;
    alice
        .actor
        .remove_member(carol.fp.clone())
        .await
        .expect("the owner may remove a member");

    let online = until(
        "the removed member's presence dot goes out",
        &mut alice.events,
        || async {
            let o = alice.actor.online_members().await;
            (!o.contains(&carol.fp)).then_some(o)
        },
    )
    .await;
    assert_eq!(online, vec![bob.fp.clone()], "only Bob is still online");
    assert_eq!(alice.actor.member_count().await, 2, "the roster shrank too");

    alice.shutdown().await;
    bob.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// 3. The eclipse advisory.
// ---------------------------------------------------------------------------------------------

/// The isolation banner must stay down for a healthy group.
///
/// With `peer_records` permanently empty the detector computed `reachable = 1` for everybody, so
/// the low-reach term was unconditionally true and every group of four or more raised CAUTION
/// about thirty seconds after startup, forever. The advisory gates nothing, which is precisely
/// why a permanent false positive is so damaging: it trains the user to ignore the one signal
/// that would tell them they are being eclipsed.
///
/// The second half covers the other half of the claim: members simply being *quiet* must not
/// raise it either. A four-person group where two people are asleep is the most ordinary
/// condition there is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_eclipse_advisory_stays_quiet_for_a_healthy_group() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let mut alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;
    alice.publish_record(advertised(1)).await;

    // Four members: one above the detector's roster floor, so suspicion is even possible.
    let mut others = Vec::new();
    for (n, nonce) in [(2u64, 11u8), (3, 12), (4, 13)] {
        let invite = mint(&alice.actor, nonce).await;
        let mut m = join_node(
            &hub,
            &clock,
            PeerId::from_u64(n),
            "member",
            n,
            alice.peer,
            &invite,
        )
        .await;
        m.publish_record(advertised(n)).await;
        others.push(m);
    }
    assert_eq!(alice.actor.member_count().await, 4);

    // Ten app-minutes of ordinary discovery ticks, well past the 30s grace window the old bug
    // fired at and past the 5-minute source-collapse sustain window.
    for _ in 0..10 {
        alice.discovery_pass().await;
        for m in &others {
            m.discovery_pass().await;
        }
        clock.advance_ms(60_000);
        for ev in drain(&mut alice.events).await {
            assert_ne!(
                ev,
                AppEvent::EclipseChanged { caution: true },
                "a healthy, fully-reachable group must never raise the isolation banner"
            );
        }
    }
    assert_eq!(
        alice.actor.online_members().await.len(),
        3,
        "the group really was healthy for the whole run, so the quiet means something"
    );

    // Now everyone else closes the app. Alice keeps ticking alone. Corroboration she already
    // earned is what keeps the banner down; the window is bounded to under the ten-minute root
    // freshness horizon because past it the in-memory transport stops being a faithful model of
    // an offline group (it has no disconnect signal, so the live set would go stale rather than
    // empty, and the observation would no longer be the one under test).
    for m in others {
        m.shutdown().await;
    }
    for _ in 0..5 {
        alice.discovery_pass().await;
        clock.advance_ms(60_000);
        for ev in drain(&mut alice.events).await {
            assert_ne!(
                ev,
                AppEvent::EclipseChanged { caution: true },
                "members being asleep is not an eclipse"
            );
        }
    }

    alice.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// 4. Channels.
// ---------------------------------------------------------------------------------------------

/// A channel one member creates is reachable by the other, backlog and all.
///
/// Named channels now travel through the shared channel directory (covered at the actor boundary).
/// This test separately pins the lower-level late-open case: a channel id is deterministic, and a
/// member opening it after messages were posted must still recover the complete backlog.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_channel_one_member_created_is_readable_by_a_member_who_opens_it_late() {
    let plans = channel_id("plans");
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let mut alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;
    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;
    ready(&bob.actor).await;

    // Bob creates a channel Alice has never heard of and fills it before she arrives.
    bob.actor.open_channel(plans).await;
    for text in ["first", "second", "third"] {
        bob.actor.send_message(plans, text).await;
    }
    until(
        "the creating member sees its own backlog",
        &mut bob.events,
        || async { (bob.actor.messages(plans).await.len() == 3).then_some(()) },
    )
    .await;

    // Alice opens it afterwards and pulls the backlog with no peer named, which is what the UI
    // does on every channel switch. A build where this regressed would show an empty channel
    // that silently starts working at the next live message.
    alice.actor.open_channel(plans).await;
    alice.actor.catch_up_any(plans).await;
    let texts: Vec<String> = until(
        "the late-opening member catches the backlog up",
        &mut alice.events,
        || async {
            let msgs = alice.actor.messages(plans).await;
            (msgs.len() == 3).then(|| msgs.into_iter().map(|m| m.text).collect())
        },
    )
    .await;
    assert_eq!(texts, vec!["first", "second", "third"], "backlog, in order");

    // And it is a live channel from then on, not a one-shot snapshot.
    alice.actor.send_message(plans, "fourth").await;
    until(
        "the channel is live after the catch-up",
        &mut bob.events,
        || async {
            let msgs = bob.actor.messages(plans).await;
            msgs.iter().any(|m| m.text == "fourth").then_some(())
        },
    )
    .await;

    // The topic rides the channel document, so it converges over the same path the messages do.
    alice
        .actor
        .set_channel_topic(plans, "what we are shipping".into())
        .await
        .expect("any member may set a channel topic");
    until("the channel topic converges", &mut bob.events, || async {
        (bob.actor.channel_topic(plans).await == "what we are shipping").then_some(())
    })
    .await;

    alice.shutdown().await;
    bob.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// 5. Files.
// ---------------------------------------------------------------------------------------------

/// Share a file on one node, download it on the other, byte for byte.
///
/// This is the only test in the workspace that drives the whole fileshare path at product
/// altitude: index gossip, the chunked download plan, the per-chunk blob fetch from a signed
/// member, decryption under the group file-wrap key, and reassembly against the whole-file
/// content address. `catcoms-app`'s in-crate test stops at "the entry appears in the other
/// member's index", which passes even if not a single byte can be fetched.
///
/// The reassembly loop below is copied from the bridge's `download_file` command rather than
/// calling a library helper, because that loop *is* the product: the actor deliberately serves
/// one chunk per command so a large download cannot freeze it, which leaves the bridge owning
/// reassembly and the final integrity check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_shared_by_one_member_downloads_byte_for_byte_on_another() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;
    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;
    ready(&bob.actor).await;

    // Deliberately not a round number and not compressible-looking: a truncation or an
    // off-by-one in the chunk walk has to show up as a byte mismatch, not as a plausible prefix.
    let data: Vec<u8> = (0..40_000u32)
        .map(|i| (i.wrapping_mul(31) % 251) as u8)
        .collect();
    let cid_hex = alice
        .actor
        .add_file(
            "notes.bin".into(),
            "application/octet-stream".into(),
            "docs".into(),
            data.clone(),
        )
        .await
        .expect("the founder holds the group file key and can share");

    // Bob learns the listing over the file-index document (live; the bridge also catches it up
    // on join, which `join_node` already did).
    let entry = until(
        "the shared file appears in the other member's index",
        &mut bob.events,
        || async {
            bob.actor
                .files()
                .await
                .into_iter()
                .find(|f| f.name == "notes.bin")
        },
    )
    .await;
    assert_eq!(entry.size, data.len() as u64);
    assert_eq!(entry.path, "docs", "the folder the uploader filed it under");
    assert_eq!(entry.author, alice.fp, "attributed to its uploader");
    assert_eq!(
        hex_of(&entry.cid),
        cid_hex,
        "the handle the UI downloads by"
    );
    assert!(
        !bob.actor.file_available(entry.cid.clone()).await,
        "listed, but not a byte of it is held locally yet"
    );

    // The bridge's download: plan, then one chunk per actor command, then verify the whole-file
    // content address over the reassembled plaintext.
    let (chunks, size) = bob
        .actor
        .file_download_plan(entry.cid.clone())
        .await
        .expect("a listed file with a decodable reference is downloadable");
    assert_eq!(size, data.len() as u64);
    assert!(chunks >= 1);
    let mut out = Vec::with_capacity(size as usize);
    for i in 0..chunks {
        let (bytes, provider) = bob
            .actor
            .fetch_file_chunk(entry.cid.clone(), i)
            .await
            .unwrap_or_else(|e| panic!("chunk {i} of {chunks} could not be fetched: {e}"));
        // The provider is the *signed* responder that served the bytes, which is what the
        // Downloads tab shows; an unauthenticated fetch would have nobody to name.
        assert_eq!(
            provider.as_deref(),
            Some(alice.fp.as_str()),
            "chunk {i} came from the member that shared it"
        );
        out.extend_from_slice(&bytes);
    }
    assert_eq!(out, data, "the downloaded bytes are the uploaded bytes");
    assert_eq!(
        Cid::of(&out).as_bytes(),
        entry.cid.as_slice(),
        "and they hash to the address the file was listed under"
    );
    assert!(
        bob.actor.file_available(entry.cid.clone()).await,
        "the file now opens with no further network fetch"
    );

    alice.shutdown().await;
    bob.shutdown().await;
}

/// Lowercase hex, matching what the bridge hands the UI as a file's download handle.
fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------------------------
// 6. Restart and re-find.
// ---------------------------------------------------------------------------------------------

/// Close the app, reopen it, and find the group again with no fresh invite.
///
/// This is the phase-9g claim, which was silently a no-op: the re-dial read `peer_records`, and
/// nothing in the product ever wrote to that map, so a reload re-dialled exactly nothing. The
/// test therefore asserts both halves at product altitude:
///
/// - state recovery, through the real sealed `ServerStore` (vault, snapshot, address cache and
///   the `ServerNet` sequence block), read back after the store is closed and reopened with the
///   passphrase, exactly like the desktop unlock; and
/// - re-contact, through `cache_known_records` + `dial_cached_peers`, the roster-checked,
///   policy-ranked, budget-capped path the bridge's `reload_one` runs.
///
/// The in-memory transport's `dial_addr` is inert, so the dial *plan* is what is asserted there;
/// rejoining the hub under the same peer id models the connection a real dial establishes, which
/// is itself a product invariant (a node that regenerated its network identity per launch
/// invalidated every invite ever issued, fixed in `0af1583`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restarted_server_recovers_its_state_and_re_finds_its_peers_without_a_fresh_invite() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut rng = ChaCha20Rng::seed_from_u64(99);
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let alice_peer = PeerId::from_u64(1);

    let mut alice = found_node(&hub, &clock, alice_peer, "alice", 1).await;
    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;
    let alice_fp = alice.fp.clone();

    // A `ServerNet` is the per-server identity + sequence-block record the bridge keeps sealed
    // beside the snapshot. Both launches draw their record `seq` from it, which is the only
    // thing that stops a restarted node's records being discarded by every peer that holds the
    // old ones.
    let mut net = ServerNet {
        key_seed: [3u8; 32],
        port: 20_001,
        advertise: String::new(),
        relay: String::new(),
        rendezvous: String::new(),
        record_seq: 0,
    };
    alice.seq = net.reserve_record_seq_block() - 65_536;
    alice.publish_record(advertised(1)).await;
    bob.publish_record(advertised(2)).await;
    alice.discovery_pass().await;
    bob.discovery_pass().await;
    assert_eq!(
        alice.actor.online_members().await,
        vec![bob.fp.clone()],
        "the session being persisted actually had a proven peer"
    );

    alice
        .actor
        .send_message(general(), "before the restart")
        .await;
    until(
        "the founder's pre-restart message lands",
        &mut alice.events,
        || async { (alice.actor.messages(general()).await.len() == 1).then_some(()) },
    )
    .await;

    // --- close the app ---
    let store = ServerStore::open(dir.path(), b"correct horse battery", &mut rng).expect("vault");
    let cache_key = store.address_cache_key().expect("cache key");
    let snapshot = alice.actor.snapshot().await.expect("snapshot");
    let cache = alice
        .actor
        .address_cache_bytes(cache_key)
        .await
        .expect("address cache");
    store.save_server(1, &snapshot, &mut rng).expect("seal");
    store
        .save_address_cache(1, &cache, &mut rng)
        .expect("seal the cache");
    store
        .save_server_net(1, &net, &mut rng)
        .expect("seal the network record");
    alice.shutdown().await;
    drop(store);

    // The 9g claim, checked directly on the bytes that reached the disk: the snapshot carries
    // somewhere to dial. This returned an empty list for the entire life of the feature.
    let addrs = peer_addrs_from_snapshot(&snapshot).expect("the snapshot decodes");
    assert!(
        addrs.contains(&"/ip4/203.0.113.2/tcp/9000".to_string()),
        "the reload has Bob's address to dial, got {addrs:?}"
    );

    // Bob keeps talking while Alice is gone; this is the backlog she must recover.
    bob.actor
        .send_message(general(), "while you were out")
        .await;
    until(
        "the peer's offline-window message lands",
        &mut bob.events,
        || async { (bob.actor.messages(general()).await.len() == 2).then_some(()) },
    )
    .await;

    // --- reopen it: passphrase, unseal, restore onto a fresh transport ---
    let store = ServerStore::open(dir.path(), b"correct horse battery", &mut rng).expect("unlock");
    let sealed = store.load_server(1).expect("the snapshot unseals");
    let mut net = store
        .load_server_net(1)
        .expect("the network record unseals")
        .expect("it was written");
    let mut alice = Server::restore(
        &sealed,
        hub.join(alice_peer),
        ChaCha20Rng::seed_from_u64(1),
        Box::new(clock.clone()),
        "alice",
    )
    .expect("restore");
    alice.subscribe_control().await.expect("subscribe control");

    // State recovery, offline: history and identity are back before a single packet moves.
    assert_eq!(
        alice.my_fingerprint(),
        alice_fp,
        "the same device, restored"
    );
    assert_eq!(alice.member_count(), 2);
    let texts: Vec<String> = alice
        .messages(general())
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(
        texts,
        vec!["before the restart"],
        "history is readable with no network at all"
    );

    // Re-contact, the bridge's `reload_one` sequence: republish on this launch's reserved block,
    // reload the sealed cache, fold the restored records in, and offer them to the dial policy.
    let seq = net.reserve_record_seq_block();
    alice
        .publish_self_record(advertised(1), seq)
        .expect("republish");
    let cached = store.load_address_cache(1).expect("the cache unseals");
    assert!(
        alice.load_address_cache(&cached, &cache_key),
        "the sealed address cache carries its own integrity tag and must verify"
    );
    assert_eq!(
        alice.cached_peer_count(),
        1,
        "the previously-proven member survived the restart"
    );
    alice.cache_known_records();
    assert_eq!(
        alice.dial_cached_peers().await,
        1,
        "the phase-9g re-dial planned a dial at a peer it has never been re-invited to"
    );

    // Nothing above involved an invite, and none is minted or redeemed anywhere below. From the
    // peer's side the conversation simply resumes on the topics the restored node re-subscribed
    // out of its own snapshot.
    let (actor, mut events, task) = spawn(alice);
    actor.open_channel(general()).await;
    bob.actor.send_message(general(), "welcome back").await;

    // The UI catches a channel up from the best known peer every time it opens one, and it has to
    // here: the ops the peer broadcast while the app was closed were never received, so the live
    // message that causally depends on them is buffered by automerge rather than applied. Closing
    // a gap it slept through is exactly what a reload has to do, and the catch-up is how; the
    // retry loop is because the first attempt can precede the inbound gossip that teaches this
    // node which peer to ask.
    let texts: Vec<String> = until(
        "the restarted node recovers the backlog it missed",
        &mut events,
        || async {
            actor.catch_up_any(general()).await;
            let msgs = actor.messages(general()).await;
            (msgs.len() == 3).then(|| msgs.into_iter().map(|m| m.text).collect())
        },
    )
    .await;
    assert!(
        texts.contains(&"while you were out".to_string()),
        "the message sent while the app was closed was recovered, got {texts:?}"
    );
    assert!(
        texts.contains(&"welcome back".to_string()),
        "and live traffic flows again, got {texts:?}"
    );

    actor.shutdown().await;
    let _ = task.await;
    bob.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// 7. Profiles and roster.
// ---------------------------------------------------------------------------------------------

/// Rename yourself and your friend sees it.
///
/// Messages carry only the author's device fingerprint; every name, colour and avatar in the UI
/// is resolved from the shared profile document at render time. So a profile that does not
/// replicate is not a cosmetic defect, it is a chat where everyone is a hex string.
///
/// The second edit is the important one: it asserts a *live* change propagates over the profile
/// document's own gossip topic, with no catch-up requested. The bridge only issues a profile
/// catch-up once, on join, so anything after that has to arrive on its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_profile_change_on_one_node_reaches_the_other_members_roster() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;
    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;

    // The founding display name is seeded into the profile document on spawn, so Bob knows who
    // Alice is from the moment he catches the document up.
    let seeded = until(
        "the joiner learns the founder's seeded profile",
        &mut bob.events,
        || async { bob.actor.profiles().await.get(&alice.fp).cloned() },
    )
    .await;
    assert_eq!(seeded.name, "alice", "the seeded founding name");

    // Alice customizes herself. No catch-up is requested for this one.
    alice
        .actor
        .set_profile(Profile {
            name: "Alice of the Cat Cafe".into(),
            color: "#ffcc00".into(),
            font: "serif".into(),
            effect: "wave".into(),
            description: "runs the espresso machine".into(),
            ..Default::default()
        })
        .await;

    let profile = until(
        "the joiner sees the founder's edited profile",
        &mut bob.events,
        || async {
            bob.actor
                .profiles()
                .await
                .get(&alice.fp)
                .filter(|p| p.name == "Alice of the Cat Cafe")
                .cloned()
        },
    )
    .await;
    assert_eq!(profile.color, "#ffcc00");
    assert_eq!(profile.effect, "wave");
    assert_eq!(profile.description, "runs the espresso machine");

    // And the roster Bob renders that profile against still holds both members, each correctly
    // flagged, so the name attaches to the right row.
    let roster = bob.actor.members().await;
    assert_eq!(roster.len(), 2);
    assert!(roster
        .iter()
        .any(|m| m.fingerprint == alice.fp && !m.is_self));
    assert!(roster.iter().any(|m| m.fingerprint == bob.fp && m.is_self));

    alice.shutdown().await;
    bob.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// 8. Wiki, status and events.
// ---------------------------------------------------------------------------------------------

/// The per-server surfaces that are not chat still replicate.
///
/// They ride their own document types with their own topics, and are opened by the actor at
/// startup rather than on demand, so a subscription regression would leave everyone but the
/// author with a stale surface rather than producing an obvious crash.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiki_status_and_events_written_on_one_node_reach_the_other() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;
    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;
    ready(&bob.actor).await;

    let queued = alice
        .actor
        .write_wiki_page("Opening Hours", "Tuesday to Sunday, 08:00 to 18:00.")
        .await
        .expect("the owner may write");
    assert!(
        !queued,
        "with review off an owner's save applies immediately"
    );

    alice.actor.post_status("the new grinder arrived").await;
    let event_id = alice
        .actor
        .create_event(
            "Launch night".into(),
            "Bring snacks".into(),
            1_700_003_600_000,
            0,
            "".into(),
        )
        .await
        .expect("any member may create an event");

    let body = until(
        "the wiki page reaches the other member",
        &mut bob.events,
        || async {
            let b = bob.actor.read_wiki_page("Opening Hours").await;
            (!b.is_empty()).then_some(b)
        },
    )
    .await;
    assert_eq!(body, "Tuesday to Sunday, 08:00 to 18:00.");
    assert!(
        bob.actor
            .wiki_pages()
            .await
            .contains(&"Opening Hours".to_string()),
        "and it is listed in the page index, not just readable by name"
    );

    let posts = until(
        "the status post reaches the other member",
        &mut bob.events,
        || async {
            let s = bob.actor.statuses().await;
            (!s.is_empty()).then_some(s)
        },
    )
    .await;
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].text, "the new grinder arrived");
    assert_eq!(posts[0].author, alice.fp);

    let events = until(
        "the event reaches the other member",
        &mut bob.events,
        || async {
            let events = bob.actor.events().await;
            events
                .iter()
                .any(|event| event.id == event_id)
                .then_some(events)
        },
    )
    .await;
    let launch = events
        .iter()
        .find(|event| event.id == event_id)
        .expect("the replicated event remains in the calendar");
    assert_eq!(launch.title, "Launch night");
    assert_eq!(launch.body, "Bring snacks");

    // A member editing the page converges back the other way (the wiki is char-level merged, so
    // this is a real edit of Alice's text, not a replacement document).
    bob.actor
        .write_wiki_page("Opening Hours", "Tuesday to Sunday, 08:00 to 20:00.")
        .await
        .expect("a member may write with review off");
    let mut alice_events = alice.events;
    let alice_actor = alice.actor;
    until(
        "the member's wiki edit reaches the founder",
        &mut alice_events,
        || async {
            (alice_actor.read_wiki_page("Opening Hours").await
                == "Tuesday to Sunday, 08:00 to 20:00.")
                .then_some(())
        },
    )
    .await;

    alice_actor.shutdown().await;
    let _ = alice.task.await;
    bob.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// 9. Roles.
// ---------------------------------------------------------------------------------------------

/// Owner-only actions are refused for a non-owner, at the layer the UI calls.
///
/// The owner is derived from the MLS designated committer, so every member computes it
/// identically with no roles op present, and the refusals below are the product's answer, not a
/// UI affordance: the bridge surfaces whatever error string comes back from these commands.
/// Membership removal is additionally protocol-enforced (the committer ignores a remove request
/// that is not from the owner), so this asserts the honest-client half of THREAT-MODEL R1.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_only_actions_are_refused_for_a_non_owner() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;
    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;

    // Both members agree on who the owner is, with no roles document written yet.
    let roles = until(
        "both members agree on the roles map",
        &mut bob.events,
        || async {
            let r = bob.actor.roles().await;
            (r.len() == 2).then_some(r)
        },
    )
    .await;
    assert_eq!(roles.get(&alice.fp).map(String::as_str), Some("owner"));
    assert_eq!(roles.get(&bob.fp).map(String::as_str), Some("member"));

    // A plain member may not grant admin, may not kick, and may not invite.
    let err = bob
        .actor
        .set_admin(bob.fp.clone(), true)
        .await
        .expect_err("a member cannot promote itself");
    assert!(err.contains("owner"), "the refusal names the reason: {err}");
    let err = bob
        .actor
        .remove_member(alice.fp.clone())
        .await
        .expect_err("a member cannot kick the owner");
    assert!(err.contains("owner"), "the refusal names the reason: {err}");
    assert!(
        bob.actor
            .mint_invite([9u8; 16], u64::MAX, Vec::new())
            .await
            .is_err(),
        "a plain member cannot invite strangers into someone else's server"
    );

    // The refusals were refusals, not silent failures: nothing changed for anybody.
    assert_eq!(alice.actor.member_count().await, 2);
    assert_eq!(bob.actor.member_count().await, 2);
    assert_eq!(
        bob.actor.roles().await.get(&bob.fp).map(String::as_str),
        Some("member")
    );

    // The owner can do it, and the grant reaches the member it was made about.
    alice
        .actor
        .set_admin(bob.fp.clone(), true)
        .await
        .expect("the owner may grant admin");
    until(
        "the admin grant reaches the member it names",
        &mut bob.events,
        || async {
            let r = bob.actor.roles().await;
            (r.get(&bob.fp).map(String::as_str) == Some("admin")).then_some(())
        },
    )
    .await;

    // …and an admin may now invite, which is the whole point of the grant.
    bob.actor
        .mint_invite([9u8; 16], u64::MAX, Vec::new())
        .await
        .expect("an admin may mint an invite");

    alice.shutdown().await;
    bob.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// A defect this suite found, now fixed. The test stays as the regression guard.
// ---------------------------------------------------------------------------------------------

/// A member who joined earlier must see a member who joins later.
///
/// **This suite found this broken.** Inviting a third person to a two-person server left the
/// *second* person's client showing a roster of two forever, with every message the newcomer sent
/// invisible to them. Only the founder and the newcomer saw a group of three. That is about as
/// fundamental a breakage as a group chat can have, and no crate-level test could see it, because
/// it lived in the product's own call sequence rather than in any protocol layer.
///
/// **Cause, fixed:** a joiner never subscribed the control topic. The bridge called
/// `subscribe_control()` in `found_server` and in `reload_one` but in neither join path, so
/// `ChannelSync::control_subscribed` stayed `false` for the whole life of a joined server and
/// `desired_routing_topics()` omitted the control topics. The joiner therefore never received the
/// membership-commit broadcast admitting anybody after it, and sat at its join epoch forever.
/// `join_node` above mirrors the fixed sequence; keep the two in step.
///
/// **The recovery half of the same defect is covered by the next test, not this one**: a
/// subscribed joiner receives the commit live and never reaches the recovery path at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_third_member_is_visible_to_the_member_who_joined_before_them() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;

    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;

    let invite = mint(&alice.actor, 8).await;
    let carol = join_node(
        &hub,
        &clock,
        PeerId::from_u64(3),
        "carol",
        3,
        alice.peer,
        &invite,
    )
    .await;

    // The founder and the newcomer agree there are three of them.
    assert_eq!(alice.actor.member_count().await, 3);
    assert_eq!(carol.actor.member_count().await, 3);

    // The newcomer says hello, which is what a newcomer does.
    carol.actor.send_message(general(), "hi, I'm carol").await;

    // The member who was already here must see both the message and the roster change. Today it
    // sees neither: its roster reads 2 and its channel is empty.
    until(
        "the earlier joiner sees the new member's first message",
        &mut bob.events,
        || async {
            let msgs = bob.actor.messages(general()).await;
            msgs.iter().any(|m| m.text == "hi, I'm carol").then_some(())
        },
    )
    .await;
    assert_eq!(
        bob.actor.member_count().await,
        3,
        "and the roster shows all three"
    );

    alice.shutdown().await;
    bob.shutdown().await;
    carol.shutdown().await;
}

/// A **genuinely missed** membership commit heals on its own, with no member that predates it
/// having to say a word.
///
/// This is the recovery half of P14 (`docs/design-zeroconf-reachability.md`), and the property
/// the whole recovery path exists for. The lagging member does notice the gap: the newcomer's
/// first op arrives sealed under a future epoch, `ingest_future` queues a commit catch-up, and
/// it drains on the next tick. What was broken was **whom it asked**. `remember_peer(from)` runs
/// on that same inbound gossip event, so the newcomer is the most-recently-seen peer at the
/// exact moment the drain chooses, and `pick_catchup_peer` prefers the most recent; there is no
/// proven `member_peers` entry to outrank it, because only a commit catch-up promotes and a
/// document catch-up does not. So the peer asked was *by construction* the member whose arrival
/// caused the gap, and a member that joined at epoch N holds an empty `commit_log`. Its empty
/// bundle returned `Ok(0)` and marked nothing failed, so the next op repeated the identical
/// choice, forever. It healed only when a member that *was* present for the commit happened to
/// post a document op after the newcomer.
///
/// Hence the shape of this test: after Carol joins, **Alice never sends anything**. She answers
/// what she is asked and nothing else, so a pass means the recovery aimed itself at a peer that
/// could actually serve the gap rather than being rescued by an older member speaking.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_genuinely_missed_membership_commit_heals_without_an_older_member_speaking() {
    let hub = Hub::new();
    let clock = ManualClock::new(1_700_000_000_000);
    let alice = found_node(&hub, &clock, PeerId::from_u64(1), "alice", 1).await;

    // Bob joins, but off the control topic: the commit admitting anyone after him is lost.
    let invite = mint(&alice.actor, 7).await;
    let mut bob = join_node_off_the_control_topic(
        &hub,
        &clock,
        PeerId::from_u64(2),
        "bob",
        2,
        alice.peer,
        &invite,
    )
    .await;
    assert_eq!(bob.actor.member_count().await, 2);

    let invite = mint(&alice.actor, 8).await;
    let carol = join_node(
        &hub,
        &clock,
        PeerId::from_u64(3),
        "carol",
        3,
        alice.peer,
        &invite,
    )
    .await;
    assert_eq!(alice.actor.member_count().await, 3, "the roster grew");
    // Bob is deliberately not asserted to still read 2 here: the commit did not reach him over
    // the control topic (he is not on it), but Carol publishes her profile the moment she is up,
    // and that op is already enough to reveal the gap and drive the recovery. Whether the
    // recovery has finished by this line is a race; that it finishes at all is the property.

    // Only the newcomer speaks from here.
    carol.actor.send_message(general(), "hi, I'm carol").await;

    until(
        "the lagging member recovers the commit it missed",
        &mut bob.events,
        || async { (bob.actor.member_count().await == 3).then_some(()) },
    )
    .await;
    until(
        "and can then read what the newcomer said",
        &mut bob.events,
        || async {
            let msgs = bob.actor.messages(general()).await;
            msgs.iter().any(|m| m.text == "hi, I'm carol").then_some(())
        },
    )
    .await;

    alice.shutdown().await;
    bob.shutdown().await;
    carol.shutdown().await;
}
