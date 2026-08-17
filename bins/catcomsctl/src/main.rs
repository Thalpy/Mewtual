//! `catcomsctl`; a dev CLI that drives the Mewtual stack end to end.
//!
//! The `demo` command runs the whole pipeline in one process: Alice founds a
//! server, mints a single-use invite, Bob redeems it and joins the MLS group,
//! both open a channel over the (in-memory) mesh transport, exchange end-to-end
//! encrypted chat messages, and converge. It composes every layer; identity,
//! MLS groups, invites, encrypted CRDT replication, and channel sync; and is
//! fully scriptable for testing. `--debug` additionally writes a verbose
//! `debug_log_<timestamp>.txt`.

use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, AutomergeError, ObjType, ReadDoc, Value, ROOT};
use catcoms_discovery::{Candidate, DiscoveryPolicy, PolicyConfig, Source};
use catcoms_mls::{InviteLedger, InviteToken, MlsDevice, ServerGroup};
use catcoms_net::{
    build_relay_swarm, build_relay_swarm_with_key, build_rendezvous_swarm,
    build_rendezvous_swarm_with_key, phase0_peer_id, run_relay, run_rendezvous,
    validate_rendezvous_addrs, MeshService,
};
use catcoms_rt::{
    Clock, Hub, MemNetwork, MeshTransport, OsCryptoRng, PeerId, RngCore, SystemClock,
    TransportEvent,
};
use catcoms_sync::{join_namespace, ChannelSync, SyncStats};
use catcoms_wire::DocType;
use clap::{Parser, Subcommand};
use libp2p::Multiaddr;
use tokio::time::timeout;

/// The channel all demo messages go to.
const GENERAL: u128 = 1;

#[derive(Parser)]
#[command(
    name = "catcomsctl",
    about = "Drive the Mewtual stack from the command line"
)]
struct Cli {
    /// Write a verbose debug log to <log-dir>/debug_log_<timestamp>.txt.
    #[arg(long, global = true)]
    debug: bool,
    /// Directory for debug logs.
    #[arg(long, global = true, default_value = "logs")]
    log_dir: PathBuf,
    /// Print each node's sync diagnostic counters (SyncStats) on completion.
    #[arg(long, global = true)]
    stats: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the full end-to-end demo (found server -> invite -> join -> chat).
    Demo,
    /// Demonstrate membership-commit recovery (6d-1b): a member misses a commit
    /// and self-heals via ordered commit catch-up. A scriptable debug harness for
    /// the recovery path.
    Recover,
    /// Found a server, listen on TCP over real libp2p, write a join invite to a
    /// file, and serve indefinitely. Pair with `join` from another process/machine.
    Serve {
        /// TCP port to listen on.
        #[arg(long, default_value_t = 9000)]
        port: u16,
        /// Where to write the (hex) invite token.
        #[arg(long, default_value = "catcoms-invite.txt")]
        invite_file: PathBuf,
        /// Host advertised in the bootstrap address (use the server's reachable IP).
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Reserve a circuit slot on this relay and advertise the relayed address
        /// (for reachability behind NAT). e.g. /ip4/1.2.3.4/tcp/4000/p2p/<relay-id>.
        #[arg(long)]
        relay: Option<String>,
        /// Register at this rendezvous (a direct /ip4|dns/.../p2p/<id> multiaddr) under
        /// the invite's pre-join `join_ns`, so a joiner can discover this server with no
        /// hard-coded address (the invite then carries the rendezvous, not a bootstrap).
        #[arg(long)]
        rendezvous: Option<String>,
    },
    /// Run a zero-knowledge circuit-relay-v2 server: forward Noise+MLS ciphertext
    /// between peers that cannot connect directly. Print its dialable address.
    Relay {
        /// TCP port to listen on.
        #[arg(long, default_value_t = 4000)]
        port: u16,
        /// Persist the relay identity to this file (created if absent), so the peer id; and
        /// therefore every invite that embeds the relay's multiaddr; survives restarts.
        #[arg(long)]
        identity: Option<PathBuf>,
    },
    /// Run a zero-knowledge rendezvous server: members register their signed peer
    /// records under blinded namespaces and discover each other. Print its address.
    Rendezvous {
        /// TCP port to listen on.
        #[arg(long, default_value_t = 5000)]
        port: u16,
        /// Persist the rendezvous identity to this file (created if absent), for a stable peer id
        /// across restarts (so invites carrying the rendezvous address keep working).
        #[arg(long)]
        identity: Option<PathBuf>,
    },
    /// Join a server using an invite file written by `serve`, over real libp2p, then
    /// catch up the channel and print it.
    Join {
        /// The (hex) invite file written by `serve`.
        #[arg(long, default_value = "catcoms-invite.txt")]
        invite_file: PathBuf,
    },
    /// Print the version.
    Version,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let _log_guard = catcoms_log::init_debug(cli.debug, &cli.log_dir);

    match cli.command {
        Command::Version => println!("catcomsctl {}", env!("CARGO_PKG_VERSION")),
        Command::Demo => run_demo(cli.stats).await?,
        Command::Recover => run_recover(cli.stats).await?,
        Command::Serve {
            port,
            invite_file,
            host,
            relay,
            rendezvous,
        } => run_serve(port, invite_file, host, relay, rendezvous).await?,
        Command::Join { invite_file } => run_join(invite_file).await?,
        Command::Relay { port, identity } => run_relay_node(port, identity).await?,
        Command::Rendezvous { port, identity } => run_rendezvous_node(port, identity).await?,
    }
    Ok(())
}

/// Load a persisted libp2p identity from `path`, or generate one and write it there (protobuf
/// encoding). Gives an infra node a **stable peer id** across restarts; without it libp2p mints a
/// fresh identity each run, changing the printed `/p2p/<id>` and breaking already-shared invites.
fn load_or_create_identity(
    path: &std::path::Path,
) -> Result<libp2p::identity::Keypair, Box<dyn Error>> {
    if path.exists() {
        let bytes = std::fs::read(path)?;
        Ok(libp2p::identity::Keypair::from_protobuf_encoding(&bytes)?)
    } else {
        let key = libp2p::identity::Keypair::generate_ed25519();
        std::fs::write(path, key.to_protobuf_encoding()?)?;
        println!("[identity] created new keypair at {}", path.display());
        Ok(key)
    }
}

/// Run a circuit-relay-v2 server node.
async fn run_relay_node(port: u16, identity: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    let listen: Multiaddr = format!("/ip4/0.0.0.0/tcp/{port}").parse()?;
    let mut swarm = match identity {
        Some(path) => build_relay_swarm_with_key(load_or_create_identity(&path)?)?,
        None => build_relay_swarm()?,
    };
    let relay_id = *swarm.local_peer_id();
    swarm
        .listen_on(listen)
        .map_err(|e| format!("relay listen failed: {e}"))?;
    println!("== Mewtual relay ==");
    println!("[relay] running on tcp/{port} (peer {relay_id})");
    println!("[relay] dialable as /ip4/<this-host-ip>/tcp/{port}/p2p/{relay_id}");
    println!("[relay] forwarding ciphertext only; Ctrl-C to stop");
    run_relay(swarm).await; // runs until the process is killed
    Ok(())
}

/// Run a rendezvous server node.
async fn run_rendezvous_node(port: u16, identity: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    let listen: Multiaddr = format!("/ip4/0.0.0.0/tcp/{port}").parse()?;
    let mut swarm = match identity {
        Some(path) => build_rendezvous_swarm_with_key(load_or_create_identity(&path)?)?,
        None => build_rendezvous_swarm()?,
    };
    let rz_id = *swarm.local_peer_id();
    swarm
        .listen_on(listen)
        .map_err(|e| format!("rendezvous listen failed: {e}"))?;
    println!("== Mewtual rendezvous ==");
    println!("[rendezvous] running on tcp/{port} (peer {rz_id})");
    println!("[rendezvous] dialable as /ip4/<this-host-ip>/tcp/{port}/p2p/{rz_id}");
    println!("[rendezvous] members register/discover under blinded namespaces; Ctrl-C to stop");
    run_rendezvous(swarm).await; // runs until the process is killed
    Ok(())
}

/// Found a server over real libp2p TCP, write a join invite, and serve forever.
/// With `--relay`, also reserve a circuit slot and advertise the relayed address so
/// joiners behind NAT can reach the server through the relay.
async fn run_serve(
    port: u16,
    invite_file: PathBuf,
    host: String,
    relay: Option<String>,
    rendezvous: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // Validate a rendezvous address up front (reject circuit / no-peer-id / duplicate).
    let rz_target = match &rendezvous {
        Some(rz) => Some(
            validate_rendezvous_addrs(std::slice::from_ref(rz))?
                .into_iter()
                .next()
                .expect("one validated target"),
        ),
        None => None,
    };
    let listen: Multiaddr = format!("/ip4/0.0.0.0/tcp/{port}").parse()?;
    let relay_dial: Vec<Multiaddr> = match &relay {
        Some(r) => vec![r.parse().map_err(|e| format!("bad --relay address: {e}"))?],
        None => Vec::new(),
    };
    let (mesh, libp2p_id) = MeshService::new_tcp(Some(listen), &relay_dial)?;

    // Bootstrap addresses advertised in the invite: the direct address, plus (if a
    // relay was given) the relayed circuit address once the reservation is granted.
    let mut bootstrap = vec![format!("/ip4/{host}/tcp/{port}/p2p/{libp2p_id}")];
    if let Some(r) = &relay {
        // Wait for the relay connection (TCP+Noise+identify) before reserving, so
        // the relay-client transport has a connection to reserve over.
        timeout(Duration::from_secs(20), async {
            loop {
                if let Some(TransportEvent::PeerConnected(_)) = mesh.next_event().await {
                    break;
                }
            }
        })
        .await
        .map_err(|_| "could not connect to the relay")?;
        let circuit: Multiaddr = format!("{r}/p2p-circuit").parse()?;
        mesh.listen_on(circuit).await?;
        let circuit_addr = timeout(Duration::from_secs(20), async {
            loop {
                match mesh.next_listen_addr().await {
                    Some(a) if a.to_string().contains("p2p-circuit") => return Some(a),
                    Some(_) => continue,
                    None => return None,
                }
            }
        })
        .await
        .map_err(|_| "relay reservation timed out")?
        .ok_or("relay reservation failed")?;
        println!("[serve] reserved relay circuit: {circuit_addr}");
        bootstrap.insert(0, circuit_addr.to_string()); // prefer the relayed address
    }

    let server = MlsDevice::generate()?;
    let server_group = ServerGroup::create(&server)?;
    let mut sync = ChannelSync::new(
        mesh,
        server_group,
        server,
        OsCryptoRng,
        Box::new(SystemClock),
    );
    sync.subscribe_control().await?;
    sync.open_channel(DocType::Channel, GENERAL).await?;
    sync.post(DocType::Channel, GENERAL, |d| {
        append_message(
            d,
            "server",
            "Welcome! You joined a Mewtual server over libp2p.",
        )
    })
    .await?;

    let now = SystemClock.now_ms();
    let nonce = random_nonce();
    let invite = match &rz_target {
        Some(t) => sync.mint_invite_with_rendezvous(
            nonce,
            now + 3_600_000,
            bootstrap.clone(),
            vec![t.addr.to_string()],
        )?,
        None => sync.mint_invite(nonce, now + 3_600_000, bootstrap.clone())?,
    };
    let blob = hex::encode(invite.encode());
    tokio::fs::write(&invite_file, &blob).await?;

    // If a rendezvous was given, connect to it and register the server under the
    // invite's pre-join join_ns (so a joiner discovers us with no hard-coded address)
    // and under our steady-state member namespace(s).
    if let Some(t) = &rz_target {
        let rz_phase0 = phase0_peer_id(&t.peer);
        sync.transport().dial(t.addr.clone()).await?;
        timeout(Duration::from_secs(20), async {
            loop {
                match sync.transport().next_event().await {
                    Some(TransportEvent::PeerConnected(p)) if p == rz_phase0 => break,
                    Some(_) => continue,
                    None => break,
                }
            }
        })
        .await
        .map_err(|_| "could not connect to the rendezvous")?;
        sync.transport()
            .add_external_address(format!("/ip4/{host}/tcp/{port}").parse()?)
            .await?;
        let join_ns = join_namespace(&invite.group_id, &invite.invite_nonce, &t.peer.to_bytes());
        sync.transport()
            .rendezvous_register(&join_ns, t.peer)
            .await?;
        for ns in sync.rendezvous_namespaces(&t.peer.to_bytes()) {
            sync.transport().rendezvous_register(&ns, t.peer).await?;
        }
        println!(
            "[serve] registered at rendezvous {} (join_ns + member namespace)",
            t.peer
        );
    }

    println!("== Mewtual server ==");
    println!("[serve] listening on tcp/{port} (peer {libp2p_id})");
    for b in &bootstrap {
        println!("[serve] bootstrap: {b}");
    }
    println!("[serve] invite written to {}", invite_file.display());
    println!("[serve] serving; run `catcomsctl join` elsewhere; Ctrl-C to stop\n");

    // Serve indefinitely: admit joiners, answer catch-up, apply membership commits.
    while sync.run_once().await? {}
    Ok(())
}

/// Join a server from an invite file over real libp2p, catch up, and print the chat.
/// If the invite carries `rendezvous` addresses, discover the inviter there (no
/// hard-coded server address); otherwise dial the bootstrap address directly.
async fn run_join(invite_file: PathBuf) -> Result<(), Box<dyn Error>> {
    let blob = tokio::fs::read_to_string(&invite_file).await?;
    let invite = InviteToken::decode(&hex::decode(blob.trim())?)?;
    // Authenticate the pasted token LOCALLY before acting on any of its fields; dialing
    // its (attacker-nameable) rendezvous addresses or deriving join_ns from its group_id
    // and nonce. request_join re-verifies the Welcome too, but failing fast here keeps a
    // forged token from spending dial budget or leaking join interest to a rendezvous.
    if !invite.verify_self() {
        return Err("invite failed self-verification (forged or corrupt token)".into());
    }
    println!("== Mewtual join ==");
    if !invite.rendezvous.is_empty() {
        return run_join_via_rendezvous(invite).await;
    }

    let boot = invite
        .bootstrap
        .first()
        .ok_or("invite carries neither a rendezvous nor a bootstrap address")?;
    let addr: Multiaddr = boot.parse()?;
    // The server's peer id is the LAST /p2p/ component (past the relay, for a
    // circuit address), so we wait for the SERVER specifically; not the relay we
    // hop through to reach it.
    let server_lp =
        catcoms_net::target_peer_in_multiaddr(&addr).ok_or("bootstrap address has no peer id")?;
    let inviter = phase0_peer_id(&server_lp);
    println!("[join] dialing {addr}");
    let (mesh, _) = MeshService::new_tcp(None, std::slice::from_ref(&addr))?;
    wait_for_peer(&mesh, inviter, "the server").await?;
    join_and_converge(mesh, inviter, &invite).await
}

/// Discover the inviter at a rendezvous under the invite's pre-join `join_ns`, let the
/// `DiscoveryPolicy` decide what to dial (the net Actor never auto-dials), dial it, and
/// join; with no hard-coded server address.
async fn run_join_via_rendezvous(invite: InviteToken) -> Result<(), Box<dyn Error>> {
    let targets = validate_rendezvous_addrs(&invite.rendezvous)?;
    let rz = targets
        .into_iter()
        .next()
        .ok_or("invite has no rendezvous address")?;
    let rz_phase0 = phase0_peer_id(&rz.peer);
    println!("[join] discovering the inviter via rendezvous {}", rz.peer);

    let (mesh, _) = MeshService::new_tcp(None, std::slice::from_ref(&rz.addr))?;
    wait_for_peer(&mesh, rz_phase0, "the rendezvous").await?;

    let join_ns = join_namespace(&invite.group_id, &invite.invite_nonce, &rz.peer.to_bytes());
    mesh.rendezvous_discover(&join_ns, rz.peer).await?;
    let discovered = timeout(Duration::from_secs(20), mesh.next_discovered())
        .await
        .map_err(|_| "rendezvous discovery timed out")?
        .ok_or("rendezvous discovery returned nothing")?;
    let inviter = phase0_peer_id(&discovered.peer);

    // The DiscoveryPolicy is the only thing that decides what to dial (no auto-dial).
    let mut policy = DiscoveryPolicy::with_config(PolicyConfig::default());
    let candidate = Candidate {
        peer: discovered.peer.to_bytes(),
        addresses: discovered.addresses.iter().map(|a| a.to_string()).collect(),
        source: Source::Rendezvous(rz.peer.to_bytes()),
        seq: 1,
        tag_verified: false,
    };
    let mut rng = OsCryptoRng;
    let dialed = policy
        .plan(vec![candidate], 2, &SystemClock, &mut rng)
        .into_iter()
        .next()
        .ok_or("the discovery policy offered no peer to dial")?;
    println!(
        "[join] discovered the inviter; dialing {:?}",
        dialed.addresses
    );
    for a in &dialed.addresses {
        mesh.dial(a.parse()?).await?;
    }
    wait_for_peer(&mesh, inviter, "the server").await?;
    join_and_converge(mesh, inviter, &invite).await
}

/// Block (with a timeout) until `mesh` reports a connection to `target`.
async fn wait_for_peer(
    mesh: &MeshService,
    target: PeerId,
    what: &str,
) -> Result<(), Box<dyn Error>> {
    timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                if p == target {
                    return;
                }
            }
        }
    })
    .await
    .map_err(|_| format!("timed out connecting to {what}"))?;
    Ok(())
}

/// Once connected to `inviter`, run the MLS join handshake, build the sync node, catch
/// up the channel, and print the converged transcript.
async fn join_and_converge(
    mesh: MeshService,
    inviter: PeerId,
    invite: &InviteToken,
) -> Result<(), Box<dyn Error>> {
    println!("[join] connected; requesting to join…");
    let device = MlsDevice::generate()?;
    let (group, routing) = timeout(
        Duration::from_secs(20),
        catcoms_sync::request_join(&mesh, inviter, &device, invite),
    )
    .await
    .map_err(|_| "join timed out")??;
    println!("[join] joined the server (epoch {})", group.epoch());

    let mut sync = ChannelSync::new_joined(
        mesh,
        group,
        device,
        OsCryptoRng,
        Box::new(SystemClock),
        routing,
    );
    sync.open_channel(DocType::Channel, GENERAL).await?;
    let applied = timeout(
        Duration::from_secs(20),
        sync.request_catchup(inviter, DocType::Channel, GENERAL),
    )
    .await
    .map_err(|_| "catch-up timed out")??;
    println!("[join] caught up {applied} message(s):\n");
    print_transcript("server", &sync);
    println!("\n[OK] joined and converged over libp2p");
    Ok(())
}

/// Print a node's sync diagnostics in a compact, greppable line.
fn print_stats(label: &str, s: &SyncStats) {
    println!(
        "  [stats] {label:<6} epoch-applied={} buffered={} served={} \
commit-catchups={} ops={} past-epoch-recovered={} future-dropped={} \
old-dropped={} doc-catchups={} | gauges: past-keys={} pending={} log={} peers={}",
        s.commits_applied,
        s.commits_buffered,
        s.commits_served,
        s.commit_catchups_requested,
        s.ops_ingested,
        s.ops_recovered_past_epoch,
        s.ops_dropped_future_epoch,
        s.ops_dropped_old_epoch,
        s.doc_catchups_requested,
        s.past_keys_retained,
        s.pending_commits,
        s.commit_log_len,
        s.known_peers,
    );
}

fn random_nonce() -> [u8; 16] {
    let mut nonce = [0u8; 16];
    let mut rng = OsCryptoRng;
    rng.fill_bytes(&mut nonce);
    nonce
}

/// Append a `{author, text}` message to the channel document's `messages` list.
fn append_message(doc: &mut AutoCommit, author: &str, text: &str) -> Result<(), AutomergeError> {
    let list = match doc.get(ROOT, "messages")? {
        Some((Value::Object(ObjType::List), id)) => id,
        _ => doc.put_object(ROOT, "messages", ObjType::List)?,
    };
    let index = doc.length(&list);
    let msg = doc.insert_object(&list, index, ObjType::Map)?;
    doc.put(&msg, "author", author)?;
    doc.put(&msg, "text", text)?;
    Ok(())
}

/// Print the channel transcript from one member's converged view.
fn print_transcript<T: MeshTransport>(label: &str, sync: &ChannelSync<T, OsCryptoRng>) {
    println!("--- {label}'s view ---");
    let Some(enc) = sync.doc(DocType::Channel, GENERAL) else {
        println!("  (no channel)");
        return;
    };
    let doc = enc.doc();
    if let Ok(Some((Value::Object(ObjType::List), list))) = doc.get(ROOT, "messages") {
        for i in 0..doc.length(&list) {
            if let Ok(Some((Value::Object(ObjType::Map), msg))) = doc.get(&list, i) {
                let author = field(doc, &msg, "author");
                let text = field(doc, &msg, "text");
                println!("  {author}: {text}");
            }
        }
    }
}

fn field(doc: &AutoCommit, obj: &automerge::ObjId, key: &str) -> String {
    doc.get(obj, key)
        .ok()
        .flatten()
        .and_then(|(v, _)| v.into_string().ok())
        .unwrap_or_default()
}

async fn run_demo(stats: bool) -> Result<(), Box<dyn Error>> {
    println!("== Mewtual end-to-end demo ==\n");
    let now = SystemClock.now_ms();

    // 1. Alice founds a server.
    let alice = MlsDevice::generate()?;
    let mut alice_group = ServerGroup::create(&alice)?;
    let gid = alice_group.group_id();
    println!(
        "[1] Alice founded server  (group {}…, epoch {})",
        hex::encode(&gid[..6]),
        alice_group.epoch()
    );

    // 2. Alice mints a single-use, device-bound invite (expires in 1h).
    let invite = alice_group.mint_invite(&alice, random_nonce(), now + 3_600_000, vec![])?;
    let invite_blob = hex::encode(invite.encode());
    println!(
        "[2] Alice minted invite   (paste-token {}…, {} bytes)",
        &invite_blob[..24],
        invite_blob.len() / 2
    );

    // 3. Bob parses the invite, mints a bound KeyPackage, Alice admits him.
    let bob = MlsDevice::generate()?;
    let parsed = InviteToken::decode(&hex::decode(&invite_blob)?)?;
    let bob_kp = bob.key_package_for_invite(&parsed.group_id, parsed.invite_nonce)?;
    let mut ledger = InviteLedger::new();
    let welcome = alice_group
        .add_member_via_invite(&alice, bob_kp, &parsed, &mut ledger, now)?
        .welcome;
    let bob_group = ServerGroup::join(&bob, &welcome)?;
    println!(
        "[3] Bob joined via invite (members {}, epoch {})",
        alice_group.member_count(),
        bob_group.epoch()
    );

    // 4. Wire both members to the mesh and open the #general channel.
    let hub = Hub::new();
    let mut alice_sync = ChannelSync::new(
        hub.join(PeerId::from_u64(1)),
        alice_group,
        alice,
        OsCryptoRng,
        Box::new(SystemClock),
    );
    let mut bob_sync = ChannelSync::new(
        hub.join(PeerId::from_u64(2)),
        bob_group,
        bob,
        OsCryptoRng,
        Box::new(SystemClock),
    );
    alice_sync.open_channel(DocType::Channel, GENERAL).await?;
    bob_sync.open_channel(DocType::Channel, GENERAL).await?;
    println!("[4] Both opened #general over the mesh\n");

    // 5. End-to-end encrypted chat.
    alice_sync
        .post(DocType::Channel, GENERAL, |d| {
            append_message(d, "alice", "Hey Bob - welcome to the server!")
        })
        .await?;
    bob_sync.run_once().await?; // Bob ingests Alice's gossiped op

    bob_sync
        .post(DocType::Channel, GENERAL, |d| {
            append_message(d, "bob", "Thanks! End-to-end encrypted and it just works.")
        })
        .await?;
    alice_sync.run_once().await?; // Alice ingests Bob's gossiped op

    // 6. Show both converged views.
    print_transcript("Alice", &alice_sync);
    print_transcript("Bob", &bob_sync);

    let alice_text = field_join(&alice_sync);
    let bob_text = field_join(&bob_sync);
    println!();
    if stats {
        print_stats("alice", &alice_sync.stats());
        print_stats("bob", &bob_sync.stats());
        println!();
    }
    if alice_text == bob_text && !alice_text.is_empty() {
        println!("[OK] both members converged on an identical, encrypted transcript");
    } else {
        return Err("members did not converge".into());
    }
    Ok(())
}

/// Drive the 6d-1b commit-recovery path end to end so it can be exercised and
/// inspected from the command line. Alice founds a server and admits Bob, Carol
/// and Dave. Bob is offline for the control topic when Carol joins, so he *misses*
/// that membership commit; when he later sees Dave's (future) commit he detects
/// the gap, fetches the missing commits in order, and converges; without an
/// explicit catch-up call.
async fn run_recover(stats: bool) -> Result<(), Box<dyn Error>> {
    println!("== Mewtual membership-recovery demo (6d-1b) ==\n");
    let hub = Hub::new();
    let alice_peer = PeerId::from_u64(1);

    // Alice founds the server and listens for membership commits.
    let alice = MlsDevice::generate()?;
    let alice_group = ServerGroup::create(&alice)?;
    let mut alice_sync = ChannelSync::new(
        hub.join(alice_peer),
        alice_group,
        alice,
        OsCryptoRng,
        Box::new(SystemClock),
    );
    alice_sync.subscribe_control().await?;
    println!("[1] Alice founded server (epoch {})", alice_sync.epoch());

    // Bob joins over the wire (epoch 0 -> 1).
    let bob = MlsDevice::generate()?;
    let bob_net = hub.join(PeerId::from_u64(2));
    let invite_b = alice_sync.mint_invite(random_nonce(), u64::MAX, vec![])?;
    let (bob_group, _) = tokio::join!(
        catcoms_sync::request_join(&bob_net, alice_peer, &bob, &invite_b),
        alice_sync.run_once(),
    );
    let (bob_group, bob_routing) = bob_group?;
    let mut bob_sync = ChannelSync::new_joined(
        bob_net,
        bob_group,
        bob,
        OsCryptoRng,
        Box::new(SystemClock),
        bob_routing,
    );
    println!(
        "[2] Bob joined (Alice epoch {}, Bob epoch {}) -- Bob is offline for control",
        alice_sync.epoch(),
        bob_sync.epoch()
    );

    // Carol joins (epoch 1 -> 2). Bob is not on the control topic, so he MISSES it.
    let carol = MlsDevice::generate()?;
    let carol_net = hub.join(PeerId::from_u64(3));
    let invite_c = alice_sync.mint_invite(random_nonce(), u64::MAX, vec![])?;
    let (_carol_group, _) = tokio::join!(
        catcoms_sync::request_join(&carol_net, alice_peer, &carol, &invite_c),
        alice_sync.run_once(),
    );
    bob_sync.subscribe_control().await?; // Bob comes online, but already behind
    println!(
        "[3] Carol joined (Alice epoch {}) -- Bob ({}) MISSED this commit",
        alice_sync.epoch(),
        bob_sync.epoch()
    );

    // Dave joins (epoch 2 -> 3). Bob receives this *future* commit.
    let dave = MlsDevice::generate()?;
    let dave_net = hub.join(PeerId::from_u64(4));
    let invite_d = alice_sync.mint_invite(random_nonce(), u64::MAX, vec![])?;
    let (_dave_group, _) = tokio::join!(
        catcoms_sync::request_join(&dave_net, alice_peer, &dave, &invite_d),
        alice_sync.run_once(),
    );
    println!("[4] Dave joined (Alice epoch {})", alice_sync.epoch());

    // Tick 1: Bob sees the future commit, buffers it, and queues a catch-up.
    bob_sync.run_once().await?;
    println!(
        "[5] Bob saw a future commit -> buffered (Bob still epoch {}, {} pending)",
        bob_sync.epoch(),
        bob_sync.stats().pending_commits
    );

    // Tick 2: Bob fetches the missing commits in order and converges.
    let (bob_tick, alice_tick) = tokio::join!(bob_sync.run_once(), alice_sync.run_once());
    bob_tick?;
    alice_tick?;
    println!(
        "[6] Bob auto-recovered via commit catch-up -> epoch {}",
        bob_sync.epoch()
    );

    println!();
    if stats {
        print_stats("alice", &alice_sync.stats());
        print_stats("bob", &bob_sync.stats());
        println!();
    }
    if bob_sync.epoch() == alice_sync.epoch() {
        println!(
            "[OK] Bob healed a missed membership commit and converged to epoch {}",
            bob_sync.epoch()
        );
        Ok(())
    } else {
        Err("recovery failed: Bob did not converge".into())
    }
}

/// Concatenate a member's messages for an equality check.
fn field_join(sync: &ChannelSync<MemNetwork, OsCryptoRng>) -> String {
    let Some(enc) = sync.doc(DocType::Channel, GENERAL) else {
        return String::new();
    };
    let doc = enc.doc();
    let mut out = String::new();
    if let Ok(Some((Value::Object(ObjType::List), list))) = doc.get(ROOT, "messages") {
        for i in 0..doc.length(&list) {
            if let Ok(Some((Value::Object(ObjType::Map), msg))) = doc.get(&list, i) {
                out.push_str(&field(doc, &msg, "author"));
                out.push(':');
                out.push_str(&field(doc, &msg, "text"));
                out.push('\n');
            }
        }
    }
    out
}
