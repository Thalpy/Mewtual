//! Native-only TCP key-task lifetime. Neither a drain nor opaque custody is
//! authentication. The checked Root owner must supply its original paid tail.
//!
//! Legacy constructors keep their existing TCP/QUIC/WebSocket behavior. This
//! constructor deliberately uses TCP/Noise/yamux: Quinn's endpoint driver is
//! spawned outside Swarm's Executor and cannot be covered by this observation.
use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use tokio::sync::Notify;

const CLOSED: usize = 1 << (usize::BITS - 1);
const MAX_TASKS: usize = 1024;
/// Named extra wrapper/control allowance, not the connection futures' backing
/// memory or process RSS. The actual producer must retain this paid allowance.
pub const NATIVE_TCP_LIFETIME_BYTES: usize = MAX_TASKS * 1024 + 16 * 1024;

type KeyFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type Custody = Arc<dyn Send + Sync>;

struct State {
    // Actor construction starts counted, before any key-bearing transport is
    // built. An idle OPEN zero is never completion. A closed state never reopens.
    state: AtomicUsize,
    stop: Arc<Notify>,
    changed: Notify,
    // Fee-only: never put task/Actor/Source/SDK/physical custody in this graph.
    _accounting: Custody,
}
impl State {
    fn close(&self) {
        self.state.fetch_or(CLOSED, Ordering::SeqCst);
        self.stop.notify_waiters();
        self.changed.notify_waiters();
    }
    fn enter(self: &Arc<Self>) -> Option<End> {
        self.state
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                (value & CLOSED == 0 && value < MAX_TASKS).then_some(value + 1)
            })
            .ok()?;
        Some(End(self.clone()))
    }
    fn drained(&self) -> bool {
        self.state.load(Ordering::SeqCst) == CLOSED
    }
}
struct End(Arc<State>);
impl Drop for End {
    fn drop(&mut self) {
        let before = self.0.state.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(before & !CLOSED != 0);
        self.0.changed.notify_waiters();
    }
}

/// Metadata-only original observation. It cannot stop, dial, construct an SDK,
/// retain a physical vault or establish that an unrelated runtime was destroyed.
#[derive(Clone)]
pub struct NativeTcpDrain(Arc<State>);
impl NativeTcpDrain {
    pub(super) fn is_closed(&self) -> bool {
        self.0.state.load(Ordering::SeqCst) & CLOSED != 0
    }
    pub fn is_drained(&self) -> bool {
        self.0.drained()
    }
    pub async fn wait(&self) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.is_drained() {
                return;
            }
            changed.await;
        }
    }
}
impl std::fmt::Debug for NativeTcpDrain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTcpDrain { .. }")
    }
}

// Passive task custody is SEPARATE from State/observers. In Root it pins the
// same original physical session and its fee, with no runtime/issuer backref.
// A surviving Executor may conservatively retain custody after count zero;
// zero establishes tracked-task drainage, never complete physical release.
#[derive(Clone)]
struct Executor {
    custody: Custody,
    state: Arc<State>,
}
impl libp2p::swarm::Executor for Executor {
    fn exec(&self, future: KeyFuture) {
        if let Some(end) = self.state.enter() {
            drop(tokio::spawn(Tracked::new(future, self, end)));
        } else {
            // Swarm's pending-event channel is SHARED by its Pool. Dropping
            // just this future does not report failure for its pending slot.
            // Terminate this original instead of silently stranding that slot.
            // Its Actor/parent remains counted through Pool destruction.
            self.state.close();
            drop(future);
        }
    }
}

// An explicit envelope preserves destruction order even when Ready is observed
// before Tokio drops its task allocation, or the task was never polled at all.
struct Tracked {
    inner: Option<KeyFuture>,
    custody: Option<Custody>,
    end: Option<End>,
    stop: KeyFuture,
    state: Arc<State>,
}
impl Tracked {
    fn new(inner: KeyFuture, executor: &Executor, end: End) -> Self {
        Self {
            inner: Some(inner),
            custody: Some(executor.custody.clone()),
            end: Some(end),
            stop: Box::pin(executor.state.stop.clone().notified_owned()),
            state: executor.state.clone(),
        }
    }
    fn finish(&mut self) {
        // Do not reorder: real key-bearing future -> original passive custody
        // -> count acknowledgement. Destructors run outside Root/native guards.
        drop(self.inner.take());
        drop(self.custody.take());
        drop(self.end.take());
    }
}
impl Future for Tracked {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        // Register the stop wake BEFORE sampling the SC negative. A close
        // between checks or before a completely unpolled task cannot be lost.
        let stopped = this.stop.as_mut().poll(cx).is_ready();
        if stopped || this.state.state.load(Ordering::SeqCst) & CLOSED != 0 {
            this.finish();
            return Poll::Ready(());
        }
        let ready = this
            .inner
            .as_mut()
            .is_none_or(|inner| inner.as_mut().poll(cx).is_ready());
        if ready {
            this.finish();
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
impl Drop for Tracked {
    fn drop(&mut self) {
        self.finish();
    }
}

pub(super) struct Owner(Executor);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.state.close();
    }
}
impl std::fmt::Debug for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTcpOwner { .. }")
    }
}
pub(super) struct Spawn {
    owner: Owner,
    actor: End,
    executor: Executor,
}
impl Spawn {
    pub(super) fn observe_drain(&self) -> NativeTcpDrain {
        NativeTcpDrain(self.executor.state.clone())
    }
    pub(super) fn run(self, actor: impl Future<Output = ()> + Send + 'static) -> Owner {
        // The prearmed construction token becomes the Actor token, without an
        // uncounted gap. External command handles cannot keep this task alive
        // after Owner closes the independent negative.
        drop(tokio::spawn(Tracked::new(
            Box::pin(actor),
            &self.executor,
            self.actor,
        )));
        self.owner
    }
}

/// Cache this WHOLE original preparation before building transport or moving it
/// into a consuming founder factory. Even constructor failure/unwind retains
/// its exact drain. No second build is allowed on an attempted preparation.
///
/// `task_custody` is passive original resource custody; `accounting` is ONLY
/// its fee tail. An arbitrary caller-supplied object grants no native authority.
pub struct NativeTcpPreparation {
    owner: Option<Owner>,
    executor: Executor,
    actor: Option<End>,
    attempted: bool,
}
impl NativeTcpPreparation {
    pub fn new(task_custody: impl Send + Sync + 'static, accounting: Custody) -> Self {
        let state = Arc::new(State {
            state: AtomicUsize::new(1),
            stop: Arc::new(Notify::new()),
            changed: Notify::new(),
            _accounting: accounting,
        });
        let executor = Executor {
            custody: Arc::new(task_custody),
            state: state.clone(),
        };
        Self {
            owner: Some(Owner(executor.clone())),
            executor,
            actor: Some(End(state)),
            attempted: false,
        }
    }
    pub fn observe_drain(&self) -> NativeTcpDrain {
        NativeTcpDrain(self.executor.state.clone())
    }

    /// Build direct TCP/Noise/yamux plus the member-relay client. QUIC,
    /// WebSocket and DNS are deliberately absent from this initial constructor.
    /// Mapper cleanup is best effort and public-address-only; this observation
    /// promises neither mapper task drainage nor immediate router lease removal.
    pub fn build(
        &mut self,
        key: libp2p::identity::Keypair,
        listen: &[Multiaddr],
        dial: &[Multiaddr],
    ) -> Result<(MeshService, libp2p::PeerId, Vec<Multiaddr>), NetError> {
        self.build_original(key, listen, dial, false)
    }

    /// Product opt-in to public-address mapper helpers. Ordinary library/test
    /// construction must never alter a real router merely by binding loopback.
    pub fn build_with_port_mapping(
        &mut self,
        key: libp2p::identity::Keypair,
        listen: &[Multiaddr],
        dial: &[Multiaddr],
    ) -> Result<(MeshService, libp2p::PeerId, Vec<Multiaddr>), NetError> {
        self.build_original(key, listen, dial, true)
    }

    fn build_original(
        &mut self,
        key: libp2p::identity::Keypair,
        listen: &[Multiaddr],
        dial: &[Multiaddr],
        enable_port_mapping: bool,
    ) -> Result<(MeshService, libp2p::PeerId, Vec<Multiaddr>), NetError> {
        if self.attempted {
            return Err(NetError::Build(
                "native TCP construction already attempted".into(),
            ));
        }
        self.attempted = true;
        struct CloseOnFailure {
            state: Arc<State>,
            complete: bool,
        }
        impl Drop for CloseOnFailure {
            fn drop(&mut self) {
                if !self.complete {
                    self.state.close();
                }
            }
        }
        let mut failure = CloseOnFailure {
            state: self.executor.state.clone(),
            complete: false,
        };
        let actor = self.actor.take().expect("one prearmed original Actor");
        let mut swarm = SwarmBuilder::with_existing_identity(key)
            .with_tokio()
            .with_other_transport(|key| {
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(
                    libp2p::tcp::tokio::Transport::new(libp2p::tcp::Config::default())
                        .upgrade(Version::V1Lazy)
                        .authenticate(noise::Config::new(key)?)
                        .multiplex(yamux::Config::default()),
                )
            })
            .map_err(|e| NetError::Build(e.to_string()))?
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .map_err(|e| NetError::Build(e.to_string()))?
            .with_behaviour(move |key, relay| {
                MeshBehaviour::new_with_port_mapping(key, relay, enable_port_mapping)
            })
            .map_err(|e| NetError::Build(e.to_string()))?
            .with_swarm_config(|_| libp2p::swarm::Config::with_executor(self.executor.clone()))
            .build();
        let peer = *swarm.local_peer_id();
        let mut bound = Vec::with_capacity(listen.len());
        for address in listen {
            swarm
                .listen_on(address.clone())
                .map_err(|e| NetError::Build(e.to_string()))?;
            bound.push(address.clone());
        }
        let protected =
            dial_bootstraps(&mut swarm, dial).map_err(|e| NetError::Build(e.to_string()))?;
        if self.executor.state.state.load(Ordering::SeqCst) & CLOSED != 0 {
            return Err(NetError::Build("native TCP construction was closed".into()));
        }
        let spawn = Spawn {
            owner: self.owner.take().expect("one original transport owner"),
            actor,
            executor: self.executor.clone(),
        };
        let service = MeshService::spawn_protecting_original(
            swarm,
            protected,
            enable_port_mapping,
            Some(spawn),
        );
        failure.complete = true;
        Ok((service, peer, bound))
    }
}
impl std::fmt::Debug for NativeTcpPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeTcpPreparation { .. }")
    }
}

const _: () = assert!(2 * std::mem::size_of::<Tracked>() + 512 <= 1024);

#[cfg(test)]
mod tests;
