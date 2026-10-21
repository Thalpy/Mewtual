//! Internal status-only local handoff IO. Nothing starts this service in the application.
//! A confirmed channel is not a call grant, room proof, action admission or browser-drain proof.
use super::bootstrap::{Registry, Route};
use super::channel::{BodyKind, CompletedReply, Incoming};
use super::http;
use catcoms_rt::Clock;
use catcoms_sync::voice_v2::resources::ProcessRetainedCredit;
use catcoms_sync::voice_v2::runtime::Runtime;
use std::fmt::{self, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

// Current concrete task/owner peak is 20,528 bytes in the native debug tests. Charge a fixed
// 24 KiB envelope before bind; this spends more of the SAME process ceiling, never group work.
// Both runtime admission and the regression below measure the actual nested future.
const INFRASTRUCTURE_BYTES: usize = 24 * 1024;
// Pinned Tokio task/oneshot/Arc overhead and one boxed production Clock sleep. This is not a
// claim about arbitrary Clock providers, OS socket buffers, thread stacks or allocator RSS.
const TASK_AND_HANDLE_BYTES: usize = 2048;
const CONNECTION_MS: u64 = 5_000;
const RESPONSE_HEAD_BYTES: usize = 512;

/// Only fixed application origins can receive a response. The dev origin is absent in release
/// builds; the packaged variants are not evidence that either platform's WebView was tested.
#[derive(Clone, Copy)]
pub(super) enum Origin {
    PackagedHttp,
    PackagedScheme,
    #[cfg(debug_assertions)]
    Development,
}
impl Origin {
    fn value(self) -> &'static str {
        match self {
            Self::PackagedHttp => "http://tauri.localhost",
            Self::PackagedScheme => "tauri://localhost",
            #[cfg(debug_assertions)]
            Self::Development => "http://localhost:1420",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refused {
    Capacity,
    Binding,
    Io,
    Header,
    Body,
    Expired,
}
/// Bind synchronously only after paying all fixed router/task overhead, and call `start` only
/// inside the native IO-enabled runtime. No per-connection task, handler map or retry queue
/// is allocated. The Runtime supplies BOTH original Clock and exact shared capacity pool.
pub(super) struct BoundServer {
    listener: std::net::TcpListener,
    registry: Arc<Registry>,
    clock: Arc<dyn Clock>,
    origin: Origin,
    port: u16,
    credit: Arc<ProcessRetainedCredit>,
}
/// Even a finished, uncollected JoinHandle retains the original process payment. Group/input
/// credits remain owned by their own rows/Sessions; this credit cannot pay for those bytes.
pub(super) struct Completion {
    _credit: Arc<ProcessRetainedCredit>,
    pub(super) io_failed: bool,
}
impl BoundServer {
    pub(super) fn bind(
        registry: Arc<Registry>,
        runtime: Arc<Runtime>,
        origin: Origin,
    ) -> Result<Self, Refused> {
        let resources = runtime.resources();
        let clock = runtime.clock();
        registry
            .bind_pool(&resources)
            .map_err(|_| Refused::Binding)?;
        let credit = resources
            .charge_process_retained(INFRASTRUCTURE_BYTES)
            .map_err(|_| Refused::Capacity)?;
        let listener = std::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| Refused::Io)?;
        listener.set_nonblocking(true).map_err(|_| Refused::Io)?;
        let port = listener.local_addr().map_err(|_| Refused::Io)?.port();
        Ok(Self {
            listener,
            registry,
            clock,
            origin,
            port,
            credit: Arc::new(credit),
        })
    }
    pub(super) fn port(&self) -> u16 {
        self.port
    }

    pub(super) fn start(self) -> Result<RunningServer, Refused> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| Refused::Io)?;
        let listener = TcpListener::from_std(self.listener).map_err(|_| Refused::Io)?;
        let (stop, cancelled) = oneshot::channel();
        let credit = self.credit.clone();
        #[cfg(test)]
        let phase = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let state = ServerState {
            listener,
            registry: self.registry,
            clock: self.clock,
            origin: self.origin,
            port: self.port,
            credit: self.credit,
            #[cfg(test)]
            phase: phase.clone(),
        };
        let future = state.run(cancelled);
        if worker_layout(&future) > INFRASTRUCTURE_BYTES {
            // No future was polled/spawned; dropping its ordered state closes the listener
            // before the original process payment. Code growth cannot underpay silently.
            return Err(Refused::Capacity);
        }
        let task = runtime.spawn(future);
        Ok(RunningServer {
            task: Some(task),
            stop: Some(stop),
            _credit: credit,
            #[cfg(test)]
            phase,
        })
    }
}

fn worker_layout<F>(future: &F) -> usize {
    std::mem::size_of_val(future)
        + std::mem::size_of::<BoundServer>()
        + std::mem::size_of::<RunningServer>()
        + std::mem::size_of::<Completion>()
        + TASK_AND_HANDLE_BYTES
}

/// Keep infrastructure in ONE state with known field order even when the future is aborted.
/// Exposing its raw JoinHandle would let cancellation lose completed-error custody.
struct ServerState {
    listener: TcpListener,
    registry: Arc<Registry>,
    clock: Arc<dyn Clock>,
    origin: Origin,
    port: u16,
    credit: Arc<ProcessRetainedCredit>,
    /// Passive test-only scheduling observation. It creates no production callback or API.
    #[cfg(test)]
    phase: Arc<std::sync::atomic::AtomicU8>,
}
pub(super) struct RunningServer {
    task: Option<tokio::task::JoinHandle<Completion>>,
    stop: Option<oneshot::Sender<()>>,
    _credit: Arc<ProcessRetainedCredit>,
    #[cfg(test)]
    phase: Arc<std::sync::atomic::AtomicU8>,
}
impl RunningServer {
    pub(super) fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
    pub(super) async fn finish(mut self) -> Result<Completion, Refused> {
        self.stop();
        self.task
            .take()
            .ok_or(Refused::Io)?
            .await
            .map_err(|_| Refused::Io)
    }
}
impl Drop for RunningServer {
    fn drop(&mut self) {
        self.stop();
        // Dropping JoinHandle detaches rather than aborts. The actual future/output retains
        // the shared payment until it really drains, including if finish's waiter disappears.
    }
}
impl ServerState {
    async fn run(self, mut stop: oneshot::Receiver<()>) -> Completion {
        loop {
            #[cfg(test)]
            self.phase.store(1, std::sync::atomic::Ordering::Release);
            let accepted = tokio::select! {
                biased;
                _ = &mut stop => break,
                accepted = self.listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else {
                return Completion {
                    _credit: self.credit.clone(),
                    io_failed: true,
                };
            };
            #[cfg(test)]
            self.phase.store(2, std::sync::atomic::Ordering::Release);
            let Some(deadline) = self.clock.monotonic_ms().checked_add(CONNECTION_MS) else {
                // Never wrap an elapsed-time deadline into a fresh allowance.
                drop(stream);
                continue;
            };
            // Construct one absolute timer per accepted socket. Neither a read nor a change
            // of phase can renew it. Stop/expiry win even against continuously-ready traffic.
            let mut connection = Connection::new(stream);
            tokio::select! {
                biased;
                _ = &mut stop => break,
                _ = expire_at(self.clock.as_ref(), deadline) => {},
                _ = connection.serve(&self.registry, self.port, self.origin, self.clock.as_ref(), deadline) => {},
            }
            // Connection fields destroy the stream before any admitted Session/reply owner.
        }
        Completion {
            _credit: self.credit.clone(),
            io_failed: false,
        }
    }
}

async fn expire_at(clock: &dyn Clock, deadline: u64) {
    loop {
        let now = clock.monotonic_ms();
        if now >= deadline {
            return;
        }
        clock.sleep(Duration::from_millis(deadline - now)).await;
    }
}
fn current(clock: &dyn Clock, deadline: u64) -> Result<(), Refused> {
    if clock.monotonic_ms() < deadline {
        Ok(())
    } else {
        Err(Refused::Expired)
    }
}

/// Declaration order matters on error, cancellation and deadline: close the socket before
/// retiring admitted cipher/reply custody. Router scratch is paid BEFORE unauthenticated IO;
/// the separately paid Session HeaderWorker scratch coexists and is counted as well.
struct Connection<S = TcpStream> {
    stream: S,
    reply: Option<CompletedReply>,
    incoming: Option<Incoming>,
    route: Option<Route>,
    scratch: [u8; http::HEADER_BYTES],
    filled: usize,
    output: Output,
}
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> Connection<S> {
    fn new(stream: S) -> Self {
        Self {
            stream,
            reply: None,
            incoming: None,
            route: None,
            scratch: [0; http::HEADER_BYTES],
            filled: 0,
            output: Output::new(),
        }
    }
    async fn serve(
        &mut self,
        registry: &Registry,
        port: u16,
        origin: Origin,
        clock: &dyn Clock,
        deadline: u64,
    ) -> Result<(), Refused> {
        let method = loop {
            current(clock, deadline)?;
            if let Some(head) = http::parse(&self.scratch[..self.filled], port, origin.value())
                .map_err(|_| Refused::Header)?
            {
                break head.method;
            }
            let count = self
                .stream
                .read(&mut self.scratch[self.filled..])
                .await
                .map_err(|_| Refused::Io)?;
            if count == 0 {
                return Err(Refused::Header);
            }
            self.filled += count;
        };
        if method == http::Method::Preflight {
            self.output.preflight(origin)?;
        } else {
            let head = http::parse(&self.scratch[..self.filled], port, origin.value())
                .map_err(|_| Refused::Header)?
                .ok_or(Refused::Header)?;
            // This unauthenticated fixed slot is an index hint ONLY. Existing immutable
            // native rows supply the actual Session and transcript; lookup creates no row.
            let admission = head.admission.ok_or(Refused::Header)?;
            let slot = u8::from_str_radix(&admission[14..16], 16).map_err(|_| Refused::Header)?;
            self.route = Some(registry.route(slot).map_err(|_| Refused::Binding)?);
            let route = self.route.as_ref().ok_or(Refused::Binding)?;
            let mut worker = route.session().header().map_err(|_| Refused::Capacity)?;
            worker.scratch_mut()[..self.filled].copy_from_slice(&self.scratch[..self.filled]);
            worker
                .set_filled(self.filled)
                .map_err(|_| Refused::Header)?;
            current(clock, deadline)?;
            self.incoming = Some(
                worker
                    .admit_status(port, origin.value(), route.session_id())
                    .map_err(|_| Refused::Header)?,
            );
            let incoming = self.incoming.as_mut().ok_or(Refused::Body)?;
            while !incoming.remaining_mut().is_empty() {
                current(clock, deadline)?;
                let count = self
                    .stream
                    .read(incoming.remaining_mut())
                    .await
                    .map_err(|_| Refused::Io)?;
                if count == 0 {
                    return Err(Refused::Body);
                }
                incoming.advance(count).map_err(|_| Refused::Body)?;
            }
            // Content-Length is the request boundary. Never wait for EOF: a real fetch waits
            // for the response first. Coalesced excess is refused by HTTP parsing; later bytes
            // can never become a second request because this connection is always closed.
            current(clock, deadline)?;
            let plain = incoming.open().map_err(|_| Refused::Body)?;
            if plain.len() != 49
                || plain[0] != 1
                || plain[1..17] != route.session_id()
                || plain[17..49] != route.hash()
            {
                return Err(Refused::Body);
            }
            // The retained browser constructs this only after ECDH/HKDF and outgoing seal/MAC
            // promises have settled. Response crypto uses its separately prepaid control lane.
            route.acknowledge_setup().map_err(|_| Refused::Body)?;
            let snapshot = route.snapshot();
            let incoming = self.incoming.take().ok_or(Refused::Body)?;
            let mut prepared = incoming
                .reply(BodyKind::Control, 3, None)
                .map_err(|_| Refused::Body)?;
            prepared.plain_mut().copy_from_slice(&snapshot);
            self.reply = Some(prepared.seal().map_err(|_| Refused::Body)?);
            let reply = self.reply.as_ref().ok_or(Refused::Body)?;
            self.output.reply(origin, reply.body().len())?;
        }
        current(clock, deadline)?;
        self.stream
            .write_all(self.output.bytes())
            .await
            .map_err(|_| Refused::Io)?;
        if let Some(reply) = &self.reply {
            // Fixed BYOB prefix: reading an HTTP header value would materialize an unbounded
            // browser string before its length could be checked after a local port takeover.
            current(clock, deadline)?;
            self.stream
                .write_all(reply.admission())
                .await
                .map_err(|_| Refused::Io)?;
            current(clock, deadline)?;
            self.stream
                .write_all(reply.body())
                .await
                .map_err(|_| Refused::Io)?;
        }
        current(clock, deadline)?;
        self.stream.shutdown().await.map_err(|_| Refused::Io)?;
        Ok(())
    }
}

/// Fixed response headers only; no attacker-controlled String or header map is constructed.
struct Output {
    scratch: [u8; RESPONSE_HEAD_BYTES],
    filled: usize,
}
impl Output {
    fn new() -> Self {
        Self {
            scratch: [0; RESPONSE_HEAD_BYTES],
            filled: 0,
        }
    }
    fn bytes(&self) -> &[u8] {
        &self.scratch[..self.filled]
    }
    fn preflight(&mut self, origin: Origin) -> Result<(), Refused> {
        // HTTP 204 has no payload and must not include Content-Length (RFC 9110 §8.6).
        write!(self, "HTTP/1.1 204 No Content\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type, x-voice-admission\r\nAccess-Control-Max-Age: 0\r\n\r\n", origin.value())
            .map_err(|_| Refused::Header)
    }
    fn reply(&mut self, origin: Origin, length: usize) -> Result<(), Refused> {
        let length = length.checked_add(144).ok_or(Refused::Header)?;
        write!(self, "HTTP/1.1 200 OK\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {}\r\nVary: Origin\r\nContent-Type: application/octet-stream\r\nContent-Length: {length}\r\n\r\n", origin.value())
            .map_err(|_| Refused::Header)
    }
}
impl fmt::Write for Output {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.filled.checked_add(value.len()).ok_or(fmt::Error)?;
        let target = self.scratch.get_mut(self.filled..end).ok_or(fmt::Error)?;
        target.copy_from_slice(value.as_bytes());
        self.filled = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
