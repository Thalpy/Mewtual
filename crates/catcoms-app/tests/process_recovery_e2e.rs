//! Recovery-code acceptance across real process and socket boundaries.
//!
//! `tcp_product_e2e` exercises more product behavior, but both peers live in one address space.
//! That cannot catch an application snapshot that accidentally depends on a live task, handle, or
//! other process-local state. This test launches this test binary as Alice, then as Bob twice:
//! admission and a new process restoring on a different listener. Bob's generated transport seed
//! also crosses an explicit local-state file (standing in for sealed `ServerNet`; vault sealing is
//! covered separately). The signed recovery code crosses another file, standing in for the human
//! copy/paste channel, before messaging resumes over TCP.

use std::ffi::OsString;
use std::fs::{self, File};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use catcoms_app::{channel_id, spawn, Server, ServerActor};
use catcoms_mls::{InviteToken, MlsDevice};
use catcoms_net::{phase0_peer_id, target_peer_in_multiaddr, MeshService};
use catcoms_rt::{MeshTransport, OsCryptoRng, RngCore, SystemClock, TransportEvent};
use libp2p::Multiaddr;
use tokio::time::{sleep, timeout};

const WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(20);
// The synchronous parent cannot use the runtime's deterministic timeout seam. Bound its polling
// by an explicit attempt budget instead of consulting ambient wall-clock time; the async children
// remain bounded by Tokio's timeout wrapper below.
const POLL_ATTEMPTS: usize = 3_000;
const ROLE_ENV: &str = "MEWTUAL_PROCESS_RECOVERY_ROLE";
const DIR_ENV: &str = "MEWTUAL_PROCESS_RECOVERY_DIR";
const ALICE_PORT_ENV: &str = "MEWTUAL_PROCESS_RECOVERY_ALICE_PORT";
const BOB_PORT_ENV: &str = "MEWTUAL_PROCESS_RECOVERY_BOB_PORT";

fn wait_for_file(path: &Path) {
    for _ in 0..POLL_ATTEMPTS {
        if path.is_file() {
            return;
        }
        std::thread::sleep(POLL);
    }
    panic!("timed out waiting for {}", path.display());
}

struct ChildGuard {
    role: &'static str,
    child: Child,
    log: PathBuf,
}

impl ChildGuard {
    fn wait_success(&mut self) {
        for _ in 0..POLL_ATTEMPTS {
            match self.child.try_wait().expect("poll recovery child") {
                Some(status) if status.success() => return,
                Some(status) => {
                    let log = fs::read_to_string(&self.log).unwrap_or_default();
                    panic!("{} child exited {status}\n{log}", self.role);
                }
                None => std::thread::sleep(POLL),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let log = fs::read_to_string(&self.log).unwrap_or_default();
        panic!("{} child timed out\n{log}", self.role);
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn spawn_role(role: &'static str, dir: &Path, alice_port: u16, bob_port: u16) -> ChildGuard {
    let log = dir.join(format!("{role}.log"));
    let stdout = File::create(&log).expect("create child log");
    let stderr = stdout.try_clone().expect("clone child log handle");
    let child = Command::new(std::env::current_exe().expect("current integration test binary"))
        .args([
            "--exact",
            "process_recovery_child",
            "--ignored",
            "--nocapture",
        ])
        .env(ROLE_ENV, role)
        .env(DIR_ENV, dir)
        .env(ALICE_PORT_ENV, alice_port.to_string())
        .env(BOB_PORT_ENV, bob_port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn recovery child");
    ChildGuard { role, child, log }
}

#[test]
fn a_recovery_code_reconnects_two_native_processes_after_a_listener_change() {
    let dir = tempfile::tempdir().expect("process recovery directory");
    // Let each child ask the OS for an ephemeral listener. Once the first Bob exits, bind and hold
    // his old listener here; that makes address replacement an enforced precondition rather than
    // an assumption about whether an ephemeral allocator happens to reuse its last port.
    let mut alice = spawn_role("alice", dir.path(), 0, 0);
    wait_for_file(&dir.path().join("invite.txt"));

    let mut bob_initial = spawn_role("bob-initial", dir.path(), 0, 0);
    bob_initial.wait_success();
    wait_for_file(&dir.path().join("bob.snapshot"));
    let old_address = fs::read_to_string(dir.path().join("bob-initial-address.txt"))
        .expect("read Bob's first listener");
    let old_address: Multiaddr = old_address.parse().expect("parse Bob's first listener");
    let old_port = old_address
        .iter()
        .find_map(|protocol| match protocol {
            libp2p::multiaddr::Protocol::Tcp(port) => Some(port),
            _ => None,
        })
        .expect("Bob's first listener is TCP");
    let _old_listener = TcpListener::bind(("127.0.0.1", old_port))
        .expect("hold Bob's old listener while the replacement starts");

    let mut bob_restarted = spawn_role("bob-restarted", dir.path(), 0, 0);
    bob_restarted.wait_success();
    alice.wait_success();

    assert!(dir.path().join("alice-success").is_file());
    assert!(dir.path().join("bob-success").is_file());
}

#[test]
#[ignore = "spawned by the multi-process recovery parent"]
fn process_recovery_child() {
    let role = std::env::var(ROLE_ENV).expect("child role");
    let dir = PathBuf::from(std::env::var_os(DIR_ENV).expect("child exchange directory"));
    let alice_port = std::env::var(ALICE_PORT_ENV)
        .expect("Alice port")
        .parse::<u16>()
        .expect("numeric Alice port");
    let bob_port = std::env::var(BOB_PORT_ENV)
        .expect("Bob port")
        .parse::<u16>()
        .expect("numeric Bob port");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(3)
        .enable_all()
        .build()
        .expect("child Tokio runtime");
    runtime
        .block_on(async {
            match role.as_str() {
                "alice" => run_alice(&dir, alice_port).await,
                "bob-initial" => run_bob_initial(&dir, bob_port).await,
                "bob-restarted" => run_bob_restarted(&dir, bob_port).await,
                _ => Err(format!("unknown recovery child role {role}")),
            }
        })
        .unwrap_or_else(|error| panic!("{role}: {error}"));
}

async fn wait_path(path: &Path) -> Result<(), String> {
    timeout(WAIT, async {
        while !path.is_file() {
            sleep(POLL).await;
        }
    })
    .await
    .map_err(|_| format!("timed out waiting for {}", path.display()))
}

fn exchange_staging_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.file_name().expect("exchange path has a file name"));
    name.push(format!(".publishing-{}", std::process::id()));
    path.with_file_name(name)
}

/// Publish process-exchange data only after every byte has been written and its handle closed.
/// The filename itself is the readiness signal, so writing it directly would let another process
/// observe an empty/truncated invite or recovery code between create and close.
async fn publish_exchange(path: &Path, bytes: &[u8]) -> Result<(), String> {
    publish_exchange_with_hook(path, bytes, |_| {}).await
}

async fn publish_exchange_with_hook(
    path: &Path,
    bytes: &[u8],
    before_publish: impl FnOnce(&Path),
) -> Result<(), String> {
    let staging = exchange_staging_path(path);
    tokio::fs::write(&staging, bytes)
        .await
        .map_err(|error| error.to_string())?;
    before_publish(&staging);
    tokio::fs::rename(&staging, path)
        .await
        .map_err(|error| error.to_string())
}

#[tokio::test]
async fn exchange_filename_becomes_visible_only_with_complete_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery-code.txt");
    let staging = exchange_staging_path(&path);
    let contents = vec![0xA5; 512 * 1024];

    publish_exchange_with_hook(&path, &contents, |actual_staging| {
        assert_eq!(actual_staging, staging);
        assert!(
            !path.exists(),
            "the readiness filename is not published early"
        );
        assert_eq!(fs::read(actual_staging).unwrap(), contents);
    })
    .await
    .unwrap();
    assert_eq!(tokio::fs::read(path).await.unwrap(), contents);
}

async fn my_fp(actor: &ServerActor) -> Result<String, String> {
    actor
        .members()
        .await
        .into_iter()
        .find(|member| member.is_self)
        .map(|member| member.fingerprint)
        .ok_or_else(|| "self missing from roster".to_string())
}

async fn run_alice(dir: &Path, port: u16) -> Result<(), String> {
    catcoms_log::init_test();
    let key = libp2p::identity::Keypair::ed25519_from_bytes([0xA1; 32])
        .map_err(|error| error.to_string())?;
    let listen: Multiaddr = format!("/ip4/127.0.0.1/tcp/{port}")
        .parse()
        .map_err(|error: libp2p::multiaddr::Error| error.to_string())?;
    let (mesh, transport, _) =
        MeshService::new_tcp_with_key(key, &[listen], &[]).map_err(|error| error.to_string())?;
    let address = timeout(WAIT, mesh.next_listen_addr())
        .await
        .map_err(|_| "Alice listener timed out".to_string())?
        .ok_or_else(|| "Alice listener closed".to_string())?;
    let mut server = Server::found(
        mesh,
        MlsDevice::generate().map_err(|error| error.to_string())?,
        OsCryptoRng,
        Box::new(SystemClock),
        "alice",
    )
    .map_err(|error| error.to_string())?;
    server
        .subscribe_control()
        .await
        .map_err(|error| error.to_string())?;
    server
        .publish_self_record(vec![address.to_string()], 65_536)
        .map_err(|error| error.to_string())?;
    let invite = server
        .mint_invite(
            [7; 16],
            u64::MAX,
            vec![format!("{address}/p2p/{transport}")],
        )
        .map_err(|error| error.to_string())?;
    let (alice, _events, task) = spawn(server);
    let channel = channel_id("process-recovery");
    alice.open_channel(channel).await;
    let encoded_invite = hex::encode(invite.encode());
    publish_exchange(&dir.join("invite.txt"), encoded_invite.as_bytes()).await?;

    wait_path(&dir.join("bob-online")).await?;
    alice
        .drive_discovery()
        .await
        .map_err(|_| "Alice actor stopped during PEX".to_string())?;
    let _ = alice.member_count().await;
    tokio::fs::write(dir.join("alice-release-bob"), b"ok")
        .await
        .map_err(|error| error.to_string())?;

    wait_path(&dir.join("recovery-code.txt")).await?;
    let code = tokio::fs::read_to_string(dir.join("recovery-code.txt"))
        .await
        .map_err(|error| error.to_string())?;
    let applied = alice.apply_member_recovery(code).await?;
    if applied.submitted_routes != 1 {
        return Err(format!(
            "expected one recovery dial, got {}",
            applied.submitted_routes
        ));
    }
    timeout(WAIT, async {
        loop {
            if alice
                .messages(channel)
                .await
                .iter()
                .any(|message| message.text == "hello across process recovery")
            {
                break;
            }
            sleep(POLL).await;
        }
    })
    .await
    .map_err(|_| "Alice did not receive Bob's recovered message".to_string())?;
    alice
        .send_message(channel, "reply across process recovery")
        .await;
    tokio::fs::write(dir.join("alice-success"), b"ok")
        .await
        .map_err(|error| error.to_string())?;
    wait_path(&dir.join("bob-success")).await?;
    alice.shutdown().await;
    task.await.map_err(|error| error.to_string())?;
    Ok(())
}

async fn connect_from_invite(
    dir: &Path,
    port: u16,
    key_seed: [u8; 32],
) -> Result<(Server<MeshService, OsCryptoRng>, Multiaddr, libp2p::PeerId), String> {
    let encoded = tokio::fs::read_to_string(dir.join("invite.txt"))
        .await
        .map_err(|error| error.to_string())?;
    let invite =
        InviteToken::decode(&hex::decode(encoded.trim()).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let bootstrap: Multiaddr = invite.bootstrap[0]
        .parse()
        .map_err(|error: libp2p::multiaddr::Error| error.to_string())?;
    let inviter = target_peer_in_multiaddr(&bootstrap)
        .map(|peer| phase0_peer_id(&peer))
        .ok_or_else(|| "invite bootstrap has no peer".to_string())?;
    let key = libp2p::identity::Keypair::ed25519_from_bytes(key_seed)
        .map_err(|error| error.to_string())?;
    let listen: Multiaddr = format!("/ip4/127.0.0.1/tcp/{port}")
        .parse()
        .map_err(|error: libp2p::multiaddr::Error| error.to_string())?;
    let (mesh, transport, _) =
        MeshService::new_tcp_with_key(key, &[listen], std::slice::from_ref(&bootstrap))
            .map_err(|error| error.to_string())?;
    let local_address = timeout(WAIT, mesh.next_listen_addr())
        .await
        .map_err(|_| "Bob listener timed out".to_string())?
        .ok_or_else(|| "Bob listener closed".to_string())?;
    let mesh = Arc::new(mesh);
    timeout(WAIT, async {
        loop {
            match mesh.next_event().await {
                Some(TransportEvent::PeerConnected(peer)) if peer == inviter => break,
                Some(_) => continue,
                None => panic!("Bob transport closed before admission"),
            }
        }
    })
    .await
    .map_err(|_| "Bob did not connect to Alice".to_string())?;
    let mesh = Arc::try_unwrap(mesh).map_err(|_| "extra Bob mesh owner".to_string())?;
    let mut server = Server::join(
        mesh,
        MlsDevice::generate().map_err(|error| error.to_string())?,
        OsCryptoRng,
        Box::new(SystemClock),
        "bob",
        inviter,
        &invite,
    )
    .await
    .map_err(|error| error.to_string())?;
    let learned = server
        .request_pex(inviter)
        .await
        .map_err(|error| error.to_string())?;
    if learned == 0 {
        return Err("Bob did not learn Alice's signed transport record".into());
    }
    server
        .publish_self_record(Vec::new(), 65_536)
        .map_err(|error| error.to_string())?;
    Ok((server, local_address, transport))
}

async fn run_bob_initial(dir: &Path, port: u16) -> Result<(), String> {
    catcoms_log::init_test();
    let mut key_seed = [0_u8; 32];
    OsCryptoRng.fill_bytes(&mut key_seed);
    publish_exchange(&dir.join("bob-transport-seed.bin"), &key_seed).await?;
    let (server, address, transport) = connect_from_invite(dir, port, key_seed).await?;
    let (bob, _events, task) = spawn(server);
    bob.open_channel(channel_id("process-recovery")).await;
    tokio::fs::write(dir.join("bob-initial-address.txt"), address.to_string())
        .await
        .map_err(|error| error.to_string())?;
    publish_exchange(
        &dir.join("bob-initial-peer.txt"),
        transport.to_string().as_bytes(),
    )
    .await?;
    tokio::fs::write(dir.join("bob-online"), b"ok")
        .await
        .map_err(|error| error.to_string())?;
    wait_path(&dir.join("alice-release-bob")).await?;
    let snapshot = bob.snapshot().await?;
    tokio::fs::write(dir.join("bob.snapshot"), snapshot)
        .await
        .map_err(|error| error.to_string())?;
    bob.shutdown().await;
    task.await.map_err(|error| error.to_string())?;
    Ok(())
}

async fn run_bob_restarted(dir: &Path, port: u16) -> Result<(), String> {
    catcoms_log::init_test();
    let snapshot = tokio::fs::read(dir.join("bob.snapshot"))
        .await
        .map_err(|error| error.to_string())?;
    let persisted_seed = tokio::fs::read(dir.join("bob-transport-seed.bin"))
        .await
        .map_err(|error| error.to_string())?;
    let key_seed: [u8; 32] = persisted_seed
        .try_into()
        .map_err(|_| "Bob's persisted transport seed has the wrong length".to_string())?;
    let key = libp2p::identity::Keypair::ed25519_from_bytes(key_seed)
        .map_err(|error| error.to_string())?;
    let listen: Multiaddr = format!("/ip4/127.0.0.1/tcp/{port}")
        .parse()
        .map_err(|error: libp2p::multiaddr::Error| error.to_string())?;
    let (mesh, transport, _) =
        MeshService::new_tcp_with_key(key, &[listen], &[]).map_err(|error| error.to_string())?;
    let initial_transport = tokio::fs::read_to_string(dir.join("bob-initial-peer.txt"))
        .await
        .map_err(|error| error.to_string())?;
    if transport.to_string() != initial_transport {
        return Err("Bob's replacement did not reload the persisted transport identity".into());
    }
    let address = timeout(WAIT, mesh.next_listen_addr())
        .await
        .map_err(|_| "restarted Bob listener timed out".to_string())?
        .ok_or_else(|| "restarted Bob listener closed".to_string())?;
    let old_address = tokio::fs::read_to_string(dir.join("bob-initial-address.txt"))
        .await
        .map_err(|error| error.to_string())?;
    if address.to_string() == old_address {
        return Err("the replacement process reused Bob's old listener".into());
    }
    let mut server = Server::restore(&snapshot, mesh, OsCryptoRng, Box::new(SystemClock), "bob")
        .map_err(|error| error.to_string())?;
    server
        .subscribe_control()
        .await
        .map_err(|error| error.to_string())?;
    let code = server
        .mint_member_recovery_code(vec![format!("{address}/p2p/{transport}")])
        .map_err(|error| error.to_string())?
        .encode();
    let (bob, _events, task) = spawn(server);
    let channel = channel_id("process-recovery");
    bob.open_channel(channel).await;
    publish_exchange(&dir.join("recovery-code.txt"), code.as_bytes()).await?;

    timeout(WAIT, async {
        while bob.online_members().await.is_empty() {
            sleep(POLL).await;
        }
    })
    .await
    .map_err(|_| "restarted Bob never authenticated Alice".to_string())?;
    let self_fp = my_fp(&bob).await?;
    bob.send_message(channel, "hello across process recovery")
        .await;
    timeout(WAIT, async {
        loop {
            if bob.messages(channel).await.iter().any(|message| {
                message.author != self_fp && message.text == "reply across process recovery"
            }) {
                break;
            }
            sleep(POLL).await;
        }
    })
    .await
    .map_err(|_| "restarted Bob did not receive Alice's reply".to_string())?;
    tokio::fs::write(dir.join("bob-success"), b"ok")
        .await
        .map_err(|error| error.to_string())?;
    bob.shutdown().await;
    task.await.map_err(|error| error.to_string())?;
    Ok(())
}
