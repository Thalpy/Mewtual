//! The async **event-stream actor** around a [`Server`] (slice 8b-1).
//!
//! A GUI can't poll `sync_once` by hand — it needs a live thing it sends *commands* to
//! and gets *events* from. [`spawn`] moves a `Server` into a background task that owns
//! it, drives the network, and translates between a command channel and an event
//! channel. The Tauri command bridge (8b-2) is a thin shell over this; tests drive it
//! directly over the in-memory transport.
//!
//! The task `select!`s between the command channel and `Server::sync_once`. When a
//! command arrives mid-`sync_once`, the in-flight `sync_once` is cancelled — safe at its
//! only real suspension point (`next_event`, which leaves the event queued); a cancel
//! during the brief pre-event recovery work may at worst drop an in-flight catch-up,
//! which the recovery machinery re-detects on the next inbound event (self-healing).

use std::collections::HashMap;

use catcoms_rt::{CryptoRngCore, MeshTransport, PeerId};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::{ChatMessage, MemberView, Profile, Server};

/// A command from the UI to a running server actor.
#[derive(Debug)]
pub enum AppCommand {
    /// Open a channel (subscribe + create locally). Acked once subscribed, so a caller
    /// can avoid racing a subsequent publish ahead of the subscription.
    OpenChannel {
        channel: u128,
        ack: oneshot::Sender<()>,
    },
    /// Send a chat message to a channel.
    SendMessage { channel: u128, text: String },
    /// Pull a channel's history from `peer` (e.g. right after joining).
    CatchUp { peer: PeerId, channel: u128 },
    /// Pull a channel's history from the best known peer (no peer named).
    CatchUpAny { channel: u128 },
    /// Query a channel's current materialized messages.
    Messages {
        channel: u128,
        reply: oneshot::Sender<Vec<ChatMessage>>,
    },
    /// Query the current member count.
    MemberCount { reply: oneshot::Sender<usize> },
    /// Query the roster (member fingerprints + which one is self).
    Members {
        reply: oneshot::Sender<Vec<MemberView>>,
    },
    /// Set this member's own profile (name + styling).
    SetProfile { profile: Profile },
    /// Pull the profile document from `peer` (e.g. right after joining).
    CatchUpProfiles { peer: PeerId },
    /// Query all known member profiles, keyed by fingerprint.
    Profiles {
        reply: oneshot::Sender<HashMap<String, Profile>>,
    },
    /// Stop the actor.
    Shutdown,
}

/// An event from a running server actor to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// A channel's message list changed — the UI should re-fetch it (`messages`). Using
    /// a re-fetch signal (rather than diffed deltas) keeps ordering robust under CRDT
    /// merges of concurrent messages.
    ChannelUpdated { channel: u128 },
    /// The roster size changed (a member joined or was removed).
    MembersChanged { count: usize },
    /// A member profile changed — the UI should re-fetch profiles (`profiles`).
    ProfilesUpdated,
    /// The actor has stopped (transport closed or shutdown requested).
    Closed,
}

/// A handle to a running server actor: send commands, run queries.
#[derive(Debug, Clone)]
pub struct ServerActor {
    cmd_tx: mpsc::Sender<AppCommand>,
}

impl ServerActor {
    /// Open a channel and wait until it is subscribed.
    pub async fn open_channel(&self, channel: u128) {
        let (ack, done) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::OpenChannel { channel, ack })
            .await
            .is_ok()
        {
            let _ = done.await;
        }
    }

    /// Send a chat message to a channel (fire-and-forget; a `ChannelUpdated` event follows).
    pub async fn send_message(&self, channel: u128, text: impl Into<String>) {
        let _ = self
            .cmd_tx
            .send(AppCommand::SendMessage {
                channel,
                text: text.into(),
            })
            .await;
    }

    /// Pull a channel's history from `peer`.
    pub async fn catch_up(&self, peer: PeerId, channel: u128) {
        let _ = self
            .cmd_tx
            .send(AppCommand::CatchUp { peer, channel })
            .await;
    }

    /// Pull a channel's history from the best known peer (no peer named).
    pub async fn catch_up_any(&self, channel: u128) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpAny { channel }).await;
    }

    /// Fetch a channel's current messages.
    pub async fn messages(&self, channel: u128) -> Vec<ChatMessage> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Messages { channel, reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Fetch the current member count.
    pub async fn member_count(&self) -> usize {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::MemberCount { reply })
            .await
            .is_err()
        {
            return 0;
        }
        rx.await.unwrap_or(0)
    }

    /// Fetch the roster (member fingerprints).
    pub async fn members(&self) -> Vec<MemberView> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Members { reply })
            .await
            .is_err()
        {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Set this member's own profile (a `ProfilesUpdated` event follows).
    pub async fn set_profile(&self, profile: Profile) {
        let _ = self.cmd_tx.send(AppCommand::SetProfile { profile }).await;
    }

    /// Pull the profile document from `peer`.
    pub async fn catch_up_profiles(&self, peer: PeerId) {
        let _ = self.cmd_tx.send(AppCommand::CatchUpProfiles { peer }).await;
    }

    /// Fetch all known member profiles, keyed by fingerprint.
    pub async fn profiles(&self) -> HashMap<String, Profile> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(AppCommand::Profiles { reply })
            .await
            .is_err()
        {
            return HashMap::new();
        }
        rx.await.unwrap_or_default()
    }

    /// Stop the actor.
    pub async fn shutdown(&self) {
        let _ = self.cmd_tx.send(AppCommand::Shutdown).await;
    }
}

/// Move `server` into a background task. Returns a [`ServerActor`] handle, a receiver of
/// [`AppEvent`]s, and the task's [`JoinHandle`].
pub fn spawn<T, R>(
    mut server: Server<T, R>,
) -> (ServerActor, mpsc::Receiver<AppEvent>, JoinHandle<()>)
where
    T: MeshTransport + Send + 'static,
    R: CryptoRngCore + Send + 'static,
{
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<AppCommand>(64);
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>(256);
    let handle = tokio::spawn(async move {
        let mut counts: HashMap<u128, usize> = HashMap::new();
        let mut members = server.member_count();
        // Open the per-server profile document and seed this member's name from the
        // display name, so the roster/messages show a name immediately (the user can
        // customize color/font/effect later via SetProfile).
        if let Err(e) = server.open_profiles().await {
            tracing::warn!(error = %e, "open_profiles failed");
        }
        let seed = Profile {
            name: server.display_name().to_string(),
            ..Profile::default()
        };
        if let Err(e) = server.set_profile(seed).await {
            tracing::warn!(error = %e, "seed profile failed");
        }
        let mut last_profiles = server.profiles();
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => match cmd {
                    Some(AppCommand::OpenChannel { channel, ack }) => {
                        if let Err(e) = server.open_channel(channel).await {
                            tracing::warn!(error = %e, channel, "open_channel failed");
                        }
                        counts.entry(channel).or_insert(0);
                        let _ = ack.send(());
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::SendMessage { channel, text }) => {
                        if let Err(e) = server.send_message(channel, &text).await {
                            tracing::warn!(error = %e, channel, "send_message failed");
                        }
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::CatchUp { peer, channel }) => {
                        if let Err(e) = server.request_channel_catchup(peer, channel).await {
                            tracing::warn!(error = %e, channel, "catch-up failed");
                        }
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::CatchUpAny { channel }) => {
                        if let Err(e) = server.request_channel_catchup_any(channel).await {
                            tracing::warn!(error = %e, channel, "any-peer catch-up failed");
                        }
                        if channel_changed(&server, channel, &mut counts) {
                            let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                        }
                    }
                    Some(AppCommand::Messages { channel, reply }) => {
                        let _ = reply.send(server.messages(channel));
                    }
                    Some(AppCommand::MemberCount { reply }) => {
                        let _ = reply.send(server.member_count());
                    }
                    Some(AppCommand::Members { reply }) => {
                        let _ = reply.send(server.members_view());
                    }
                    Some(AppCommand::SetProfile { profile }) => {
                        if let Err(e) = server.set_profile(profile).await {
                            tracing::warn!(error = %e, "set_profile failed");
                        }
                        if profiles_changed(&server, &mut last_profiles) {
                            let _ = event_tx.send(AppEvent::ProfilesUpdated).await;
                        }
                    }
                    Some(AppCommand::CatchUpProfiles { peer }) => {
                        if let Err(e) = server.request_profiles_catchup(peer).await {
                            tracing::warn!(error = %e, "profiles catch-up failed");
                        }
                        if profiles_changed(&server, &mut last_profiles) {
                            let _ = event_tx.send(AppEvent::ProfilesUpdated).await;
                        }
                    }
                    Some(AppCommand::Profiles { reply }) => {
                        let _ = reply.send(server.profiles());
                    }
                    Some(AppCommand::Shutdown) | None => {
                        let _ = event_tx.send(AppEvent::Closed).await;
                        break;
                    }
                },
                cont = server.sync_once() => match cont {
                    Ok(true) => {
                        for channel in counts.keys().copied().collect::<Vec<_>>() {
                            if channel_changed(&server, channel, &mut counts) {
                                let _ = event_tx.send(AppEvent::ChannelUpdated { channel }).await;
                            }
                        }
                        let mc = server.member_count();
                        if mc != members {
                            members = mc;
                            let _ = event_tx.send(AppEvent::MembersChanged { count: mc }).await;
                        }
                        if profiles_changed(&server, &mut last_profiles) {
                            let _ = event_tx.send(AppEvent::ProfilesUpdated).await;
                        }
                    }
                    _ => {
                        let _ = event_tx.send(AppEvent::Closed).await;
                        break;
                    }
                },
            }
        }
    });
    (ServerActor { cmd_tx }, event_rx, handle)
}

/// Whether a channel's message count changed since last seen (updating the record).
/// Synchronous — the `&Server` borrow ends before the caller awaits the event send, so
/// the actor future stays `Send` (a `&Server` held across an await would require
/// `Server: Sync`, which it is not).
fn channel_changed<T, R>(
    server: &Server<T, R>,
    channel: u128,
    counts: &mut HashMap<u128, usize>,
) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let n = server.messages(channel).len();
    if counts.get(&channel).copied() != Some(n) {
        counts.insert(channel, n);
        true
    } else {
        false
    }
}

/// Whether the profile document changed since last seen (updating the record). Compares
/// the materialized map; profiles are small, so this is cheap to run per tick.
fn profiles_changed<T, R>(server: &Server<T, R>, last: &mut HashMap<String, Profile>) -> bool
where
    T: MeshTransport,
    R: CryptoRngCore,
{
    let now = server.profiles();
    if now != *last {
        *last = now;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Server;
    use catcoms_mls::MlsDevice;
    use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::time::Duration;
    use tokio::time::timeout;

    const GENERAL: u128 = 1;

    fn founder(
        hub: &std::sync::Arc<Hub>,
        peer: PeerId,
        name: &str,
        seed: u64,
    ) -> Server<MemNetwork, ChaCha20Rng> {
        Server::found(
            hub.join(peer),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(seed),
            Box::new(ManualClock::new(1_000)),
            name,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn the_actor_signals_a_channel_update_on_send_and_serves_queries() {
        let hub = Hub::new();
        let (actor, mut events, handle) = spawn(founder(&hub, PeerId::from_u64(1), "alice", 1));

        actor.open_channel(GENERAL).await;
        actor.send_message(GENERAL, "hi there").await;

        let ev = timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("event timeout")
            .expect("actor closed");
        assert_eq!(ev, AppEvent::ChannelUpdated { channel: GENERAL });

        let msgs = actor.messages(GENERAL).await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "hi there");
        assert_eq!(actor.member_count().await, 1);

        actor.shutdown().await;
        let _ = handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_actors_converge_on_a_channel() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        alice_srv.open_channel(GENERAL).await.unwrap();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, _alice_events, alice_handle) = spawn(alice_srv);

        // Bob joins — Alice's actor serves the join via its own sync loop.
        let bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        )
        .await
        .unwrap();
        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);
        bob.open_channel(GENERAL).await; // subscribes before Alice publishes

        alice.send_message(GENERAL, "hello bob").await;

        // Bob's actor should signal the channel changed; then re-fetch shows the message.
        timeout(Duration::from_secs(10), async {
            loop {
                match bob_events.recv().await {
                    Some(AppEvent::ChannelUpdated { channel }) if channel == GENERAL => break,
                    Some(_) => continue,
                    None => panic!("bob actor closed"),
                }
            }
        })
        .await
        .expect("bob did not observe the channel update");

        let msgs = bob.messages(GENERAL).await;
        assert!(
            msgs.iter().any(|m| m.text == "hello bob"),
            "bob converged on Alice's message: {msgs:?}"
        );

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_actors_converge_on_profiles() {
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, _alice_events, alice_handle) = spawn(alice_srv);

        let bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        )
        .await
        .unwrap();
        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);

        // Alice customizes her profile; Bob catches the profile document up and converges
        // (the distinctive capitalized "Alice" + effect proves it is her *custom* profile,
        // not just the seeded lowercase display name).
        alice
            .set_profile(Profile {
                name: "Alice".into(),
                color: "#ffcc00".into(),
                font: "serif".into(),
                effect: "wave".into(),
                ..Default::default()
            })
            .await;
        bob.catch_up_profiles(alice_peer).await;

        timeout(Duration::from_secs(10), async {
            loop {
                if bob.profiles().await.values().any(|p| p.name == "Alice") {
                    break;
                }
                match bob_events.recv().await {
                    Some(_) => continue,
                    None => panic!("bob actor closed"),
                }
            }
        })
        .await
        .expect("bob did not converge on Alice's profile");

        let alice_profile = bob
            .profiles()
            .await
            .into_values()
            .find(|p| p.name == "Alice")
            .expect("Alice's profile present");
        assert_eq!(alice_profile.effect, "wave");
        assert_eq!(alice_profile.font, "serif");

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_founder_catches_up_a_channel_the_joiner_created() {
        const SECRET: u128 = 0xBEEF;
        let hub = Hub::new();
        let alice_peer = PeerId::from_u64(1);
        let mut alice_srv = founder(&hub, alice_peer, "alice", 1);
        alice_srv.subscribe_control().await.unwrap();
        let invite = alice_srv.mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let (alice, mut alice_events, alice_handle) = spawn(alice_srv);

        let bob_srv = Server::join(
            hub.join(PeerId::from_u64(2)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            "bob",
            alice_peer,
            &invite,
        )
        .await
        .unwrap();
        let (bob, mut bob_events, bob_handle) = spawn(bob_srv);

        // Bob creates a channel Alice has never opened and posts to it.
        bob.open_channel(SECRET).await;
        bob.send_message(SECRET, "from bob").await;
        timeout(Duration::from_secs(10), async {
            loop {
                if bob
                    .messages(SECRET)
                    .await
                    .iter()
                    .any(|m| m.text == "from bob")
                {
                    break;
                }
                match bob_events.recv().await {
                    Some(_) => continue,
                    None => panic!("bob actor closed"),
                }
            }
        })
        .await
        .expect("bob has his own message");

        // Alice opens the same channel and pulls the backlog with no named peer — the
        // founder catching up a joiner-created channel (the symmetric case 8i could not do).
        alice.open_channel(SECRET).await;
        alice.catch_up_any(SECRET).await;
        timeout(Duration::from_secs(10), async {
            loop {
                if alice
                    .messages(SECRET)
                    .await
                    .iter()
                    .any(|m| m.text == "from bob")
                {
                    break;
                }
                match alice_events.recv().await {
                    Some(_) => continue,
                    None => panic!("alice actor closed"),
                }
            }
        })
        .await
        .expect("alice caught up the joiner-created channel from the best peer");

        alice.shutdown().await;
        bob.shutdown().await;
        let _ = alice_handle.await;
        let _ = bob_handle.await;
    }
}
