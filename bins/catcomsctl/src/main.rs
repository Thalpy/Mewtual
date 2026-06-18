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

use automerge::transaction::Transactable;
use automerge::{AutoCommit, AutomergeError, ObjType, ReadDoc, Value, ROOT};
use catcoms_mls::{InviteLedger, InviteToken, MlsDevice, ServerGroup};
use catcoms_rt::{Clock, Hub, MemNetwork, OsCryptoRng, PeerId, RngCore, SystemClock};
use catcoms_sync::ChannelSync;
use catcoms_wire::DocType;
use clap::{Parser, Subcommand};

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
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the full end-to-end demo (found server -> invite -> join -> chat).
    Demo,
    /// Print the version.
    Version,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let _log_guard = catcoms_log::init_debug(cli.debug, &cli.log_dir);

    match cli.command {
        Command::Version => println!("catcomsctl {}", env!("CARGO_PKG_VERSION")),
        Command::Demo => run_demo().await?,
    }
    Ok(())
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
fn print_transcript(label: &str, sync: &ChannelSync<MemNetwork, OsCryptoRng>) {
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

async fn run_demo() -> Result<(), Box<dyn Error>> {
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
    if alice_text == bob_text && !alice_text.is_empty() {
        println!("[OK] both members converged on an identical, encrypted transcript");
    } else {
        return Err("members did not converge".into());
    }
    Ok(())
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
