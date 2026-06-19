//! `catcomsctl` — a dev CLI that drives the CatComs stack end to end.
//!
//! The `demo` command runs the whole pipeline in one process: Alice founds a
//! server, mints a single-use invite, Bob redeems it and joins the MLS group,
//! both open a channel over the (in-memory) mesh transport, exchange end-to-end
//! encrypted chat messages, and converge. It composes every layer — identity,
//! MLS groups, invites, encrypted CRDT replication, and channel sync — and is
//! fully scriptable for testing. `--debug` additionally writes a verbose
//! `debug_log_<timestamp>.txt`.

use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, AutomergeError, ObjType, ReadDoc, Value, ROOT};
use catcoms_mls::{InviteLedger, InviteToken, MlsDevice, ServerGroup};
use catcoms_net::MeshService;
use catcoms_rt::{
    Clock, Hub, MemNetwork, MeshTransport, OsCryptoRng, PeerId, RngCore, SystemClock,
    TransportEvent,
};
use catcoms_sync::{ChannelSync, SyncStats};
use catcoms_wire::DocType;
use clap::{Parser, Subcommand};
use libp2p::Multiaddr;
use tokio::time::timeout;

/// The channel all demo messages go to.
const GENERAL: u128 = 1;

#[derive(Parser)]
#[command(
    name = "catcomsctl",
    about = "Drive the CatComs stack from the command line"
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
        } => run_serve(port, invite_file, host).await?,
        Command::Join { invite_file } => run_join(invite_file).await?,
    }
    Ok(())
}

/// Found a server over real libp2p TCP, write a join invite, and serve forever.
async fn run_serve(port: u16, invite_file: PathBuf, host: String) -> Result<(), Box<dyn Error>> {
    let listen: Multiaddr = format!("/ip4/0.0.0.0/tcp/{port}").parse()?;
    let (mesh, libp2p_id) = MeshService::new_tcp(Some(listen), &[])?;
    let bootstrap = format!("/ip4/{host}/tcp/{port}/p2p/{libp2p_id}");

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
            "Welcome! You joined a CatComs server over libp2p.",
        )
    })
    .await?;

    let now = SystemClock.now_ms();
    let invite = sync.mint_invite(random_nonce(), now + 3_600_000, vec![bootstrap.clone()])?;
    let blob = hex::encode(invite.encode());
    tokio::fs::write(&invite_file, &blob).await?;

    println!("== CatComs server ==");
    println!("[serve] listening on tcp/{port} (peer {libp2p_id})");
    println!("[serve] bootstrap: {bootstrap}");
    println!("[serve] invite written to {}", invite_file.display());
    println!("[serve] serving — run `catcomsctl join` elsewhere; Ctrl-C to stop\n");

    // Serve indefinitely: admit joiners, answer catch-up, apply membership commits.
    while sync.run_once().await? {}
    Ok(())
}

/// Join a server from an invite file over real libp2p, catch up, and print the chat.
async fn run_join(invite_file: PathBuf) -> Result<(), Box<dyn Error>> {
    let blob = tokio::fs::read_to_string(&invite_file).await?;
    let invite = InviteToken::decode(&hex::decode(blob.trim())?)?;
    let boot = invite
        .bootstrap
        .first()
        .ok_or("invite carries no bootstrap address")?;
    let addr: Multiaddr = boot.parse()?;
    println!("== CatComs join ==");
    println!("[join] dialing {addr}");

    let (mesh, _) = MeshService::new_tcp(None, std::slice::from_ref(&addr))?;

    // Wait for the libp2p connection to the server before requesting.
    let inviter = timeout(Duration::from_secs(20), async {
        loop {
            if let Some(TransportEvent::PeerConnected(p)) = mesh.next_event().await {
                break p;
            }
        }
    })
    .await
    .map_err(|_| "timed out connecting to the server")?;
    println!("[join] connected; requesting to join…");

    let device = MlsDevice::generate()?;
    let group = timeout(
        Duration::from_secs(20),
        catcoms_sync::request_join(&mesh, inviter, &device, &invite),
    )
    .await
    .map_err(|_| "join timed out")??;
    println!("[join] joined the server (epoch {})", group.epoch());

    let mut sync = ChannelSync::new(mesh, group, device, OsCryptoRng, Box::new(SystemClock));
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
    println!("== CatComs end-to-end demo ==\n");
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
/// the gap, fetches the missing commits in order, and converges — without an
/// explicit catch-up call.
async fn run_recover(stats: bool) -> Result<(), Box<dyn Error>> {
    println!("== CatComs membership-recovery demo (6d-1b) ==\n");
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
    let mut bob_sync =
        ChannelSync::new(bob_net, bob_group?, bob, OsCryptoRng, Box::new(SystemClock));
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
