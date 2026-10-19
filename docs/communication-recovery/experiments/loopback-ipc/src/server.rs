//! Tiny closed loopback fixture for packaged-origin/CSP/CORS checks. No file/URL/proxy API,
//! application keys, media or production authorization. Binary authenticated framing is a later
//! experiment slice, not something established by these harmless fixed GET responses.
use catcoms_rt::{Clock, SystemClock};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{watch, Semaphore},
    task::JoinHandle,
};

const HEADER_BYTES: usize = 4096;
const PACKAGED_ORIGIN: &str = "http://tauri.localhost";
const REMOTE_PAGE: &str = "<!doctype html><title>Remote ACL probe</title><script>window.__probe=(async()=>{try{await window.__TAURI_INTERNALS__.invoke('bootstrap');return {surface:'remote',denied:false}}catch{return {surface:'remote',denied:true,bridge:!!window.__TAURI_INTERNALS__}}})();</script>";

#[derive(Default)]
pub(super) struct Metrics {
    pub bootstrap_entries: AtomicU64,
    pub bootstrap_grants: AtomicU64,
    options: AtomicU64,
    private_network_preflights: AtomicU64,
    good_requests: AtomicU64,
    wrong_cors_requests: AtomicU64,
    blocked_requests: AtomicU64,
    active: AtomicU64,
    peak_active: AtomicU64,
}
#[derive(Serialize)]
pub(super) struct Snapshot {
    bootstrap_entries: u64,
    bootstrap_grants: u64,
    options: u64,
    private_network_preflights: u64,
    good_requests: u64,
    wrong_cors_requests: u64,
    blocked_requests: u64,
    active: u64,
    peak_active: u64,
}
pub(super) struct Service {
    port: u16,
    blocked_port: u16,
    stopped: watch::Sender<bool>,
    pub metrics: Metrics,
}
impl Service {
    pub async fn start() -> std::io::Result<(Arc<Self>, Vec<JoinHandle<()>>)> {
        let permitted = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let blocked = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let service = Arc::new(Self {
            port: permitted.local_addr()?.port(),
            blocked_port: blocked.local_addr()?.port(),
            stopped: watch::channel(false).0,
            metrics: Metrics::default(),
        });
        let slots = Arc::new(Semaphore::new(4));
        let mut tasks = Vec::new();
        for (listener, permitted) in [(permitted, true), (blocked, false)] {
            let service = service.clone();
            let slots = slots.clone();
            tasks.push(tokio::spawn(async move {
                service.listen(listener, permitted, slots).await;
            }));
        }
        Ok((service, tasks))
    }
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    pub fn blocked_origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.blocked_port)
    }
    pub fn stop(&self) {
        self.stopped.send_replace(true);
    }
    pub fn drained(&self) -> bool {
        self.metrics.active.load(Ordering::SeqCst) == 0
    }
    pub fn snapshot(&self) -> Snapshot {
        let m = &self.metrics;
        Snapshot {
            bootstrap_entries: m.bootstrap_entries.load(Ordering::SeqCst),
            bootstrap_grants: m.bootstrap_grants.load(Ordering::SeqCst),
            options: m.options.load(Ordering::SeqCst),
            private_network_preflights: m.private_network_preflights.load(Ordering::SeqCst),
            good_requests: m.good_requests.load(Ordering::SeqCst),
            wrong_cors_requests: m.wrong_cors_requests.load(Ordering::SeqCst),
            blocked_requests: m.blocked_requests.load(Ordering::SeqCst),
            active: m.active.load(Ordering::SeqCst),
            peak_active: m.peak_active.load(Ordering::SeqCst),
        }
    }
    async fn worker(
        self: Arc<Self>,
        socket: TcpStream,
        permitted: bool,
        permit: tokio::sync::OwnedSemaphorePermit,
        clock: Arc<dyn Clock>,
        deadline: u64,
    ) -> tokio::sync::OwnedSemaphorePermit {
        let current = self.metrics.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.metrics
            .peak_active
            .fetch_max(current, Ordering::SeqCst);
        let _active = Active(self.clone());
        if clock.monotonic_ms() < deadline {
            tokio::select! {
                biased;
                _ = clock.sleep(Duration::from_millis(deadline.saturating_sub(clock.monotonic_ms()))) => {},
                _ = self.serve(socket, permitted) => {},
            }
        }
        // A completed task still has a retained JoinSet result/header. Its actual permit
        // follows the OUTPUT until collection, rather than returning merely on I/O completion.
        permit
    }
    async fn listen(
        self: Arc<Self>,
        listener: TcpListener,
        permitted: bool,
        slots: Arc<Semaphore>,
    ) {
        let mut stop = self.stopped.subscribe();
        let mut workers = tokio::task::JoinSet::new();
        while !*stop.borrow() {
            tokio::select! {
                biased;
                changed = stop.changed() => { if changed.is_err() { break; } },
                Some(_) = workers.join_next(), if !workers.is_empty() => {},
                result = listener.accept(), if workers.len() < 4 => {
                    let Ok((socket, peer)) = result else { break; };
                    if !peer.ip().is_loopback() { continue; }
                    let Ok(permit) = slots.clone().try_acquire_owned() else { continue; };
                    let service = self.clone();
                    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
                    let deadline = clock.monotonic_ms().saturating_add(2000);
                    // The explicit per-listener record cap also covers a future worker panic,
                    // whose stack permit would drop before its JoinError is reaped. Normal
                    // completions retain the shared four-position cap across BOTH listeners.
                    workers.spawn(service.worker(socket, permitted, permit, clock, deadline));
                }
            }
        }
        drop(listener);
        // Existing workers have their original two-second Clock deadline. Stop does not
        // refund their socket/permit by merely abandoning the listener or a browser request.
        while workers.join_next().await.is_some() {}
    }
    async fn serve(&self, mut socket: TcpStream, permitted: bool) -> std::io::Result<()> {
        let mut bytes = [0; HEADER_BYTES];
        let mut used = 0;
        loop {
            if used == bytes.len() {
                return Ok(());
            }
            let read = socket.read(&mut bytes[used..]).await?;
            if read == 0 {
                return Ok(());
            }
            used += read;
            if bytes[..used].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let Some(request) = parse(&bytes[..used]) else {
            return Ok(());
        };
        let port = if permitted {
            self.port
        } else {
            self.blocked_port
        };
        if request.host != format!("127.0.0.1:{port}") {
            return Ok(());
        }
        if !permitted {
            self.metrics.blocked_requests.fetch_add(1, Ordering::SeqCst);
        }
        if request.method == "OPTIONS" {
            if request.origin != Some(PACKAGED_ORIGIN) {
                return Ok(());
            }
            self.metrics.options.fetch_add(1, Ordering::SeqCst);
            if request.private_network {
                self.metrics
                    .private_network_preflights
                    .fetch_add(1, Ordering::SeqCst);
            }
            let response = format!("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {PACKAGED_ORIGIN}\r\nAccess-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: x-probe\r\nAccess-Control-Allow-Private-Network: true\r\n\r\n");
            socket.write_all(response.as_bytes()).await?;
        } else {
            let (body, mime, origin) = match request.path {
                "/ok" if request.origin == Some(PACKAGED_ORIGIN) => {
                    self.metrics.good_requests.fetch_add(1, Ordering::SeqCst);
                    ("fixed probe bytes", "text/plain", PACKAGED_ORIGIN)
                }
                "/wrong-cors" if request.origin == Some(PACKAGED_ORIGIN) => {
                    self.metrics
                        .wrong_cors_requests
                        .fetch_add(1, Ordering::SeqCst);
                    ("fixed probe bytes", "text/plain", "http://wrong.localhost")
                }
                "/remote" => (REMOTE_PAGE, "text/html", PACKAGED_ORIGIN),
                _ => return Ok(()),
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {mime}\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {origin}\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await?;
        }
        socket.shutdown().await
    }
}
struct Active(Arc<Service>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.metrics.active.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Request<'a> {
    method: &'a str,
    path: &'a str,
    host: &'a str,
    origin: Option<&'a str>,
    private_network: bool,
}
/// Closed body-free HTTP subset. One bounded request per socket; reject bodies, duplicate
/// headers, pipelining, chunking and ambiguous syntax before any test response is constructed.
fn parse(bytes: &[u8]) -> Option<Request<'_>> {
    if bytes.len() > HEADER_BYTES || !bytes.ends_with(b"\r\n\r\n") {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text[..text.len() - 4].split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?;
    let path = first.next()?;
    if !matches!(method, "GET" | "OPTIONS")
        || !matches!(path, "/ok" | "/wrong-cors" | "/remote")
        || first.next()? != "HTTP/1.1"
        || first.next().is_some()
    {
        return None;
    }
    let mut headers = std::collections::BTreeMap::new();
    for line in lines {
        if headers.len() == 48 {
            return None;
        }
        let (name, value) = line.split_once(':')?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return None;
        }
        if headers
            .insert(name.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return None;
        }
    }
    if headers.contains_key("transfer-encoding")
        || headers
            .get("content-length")
            .is_some_and(|length| *length != "0")
    {
        return None;
    }
    Some(Request {
        method,
        path,
        host: headers.get("host")?,
        origin: headers.get("origin").copied(),
        private_network: headers.get("access-control-request-private-network") == Some(&"true"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_http_subset_rejects_ambiguous_or_body_bearing_requests() {
        assert!(parse(
            b"GET /ok HTTP/1.1\r\nHost: 127.0.0.1:1\r\nOrigin: http://tauri.localhost\r\n\r\n"
        )
        .is_some());
        for bad in [
            "GET /ok HTTP/1.1\r\nHost: x\r\nhost: y\r\n\r\n",
            "GET /ok HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\n\r\n",
            "GET /ok HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n",
            "POST /ok HTTP/1.1\r\nHost: x\r\n\r\n",
            "GET /other HTTP/1.1\r\nHost: x\r\n\r\n",
            "GET /ok HTTP/1.1\r\nHost: x\r\n\r\nGET /ok HTTP/1.1\r\n\r\n",
        ] {
            assert!(parse(bad.as_bytes()).is_none());
        }
        assert!(parse(&vec![b'a'; HEADER_BYTES + 1]).is_none());
    }

    #[tokio::test]
    async fn completed_workers_keep_admission_until_their_results_are_collected() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let service = Arc::new(Service {
            port: listener.local_addr().unwrap().port(),
            blocked_port: 1,
            stopped: watch::channel(false).0,
            metrics: Metrics::default(),
        });
        let slots = Arc::new(Semaphore::new(4));
        let clock = Arc::new(catcoms_rt::ManualClock::new(1000));
        let mut finished = Vec::new();
        let mut workers = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let client = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (socket, _) = listener.accept().await.unwrap();
            drop(client);
            let permit = slots.clone().try_acquire_owned().unwrap();
            let service = service.clone();
            let clock = clock.clone();
            let (sent, observed) = tokio::sync::oneshot::channel();
            finished.push(observed);
            workers.spawn(async move {
                let permit = service.worker(socket, true, permit, clock, 3000).await;
                let _ = sent.send(());
                permit
            });
        }
        for finished in finished {
            finished.await.unwrap();
        }
        assert_eq!(workers.len(), 4);
        assert_eq!(
            service.metrics.active.load(Ordering::SeqCst),
            0,
            "all actual I/O has ended"
        );
        assert_eq!(
            slots.available_permits(),
            0,
            "completed records still retain their positions"
        );
        assert!(slots.clone().try_acquire_owned().is_err());
        drop(workers.join_next().await.unwrap().unwrap());
        assert_eq!(slots.available_permits(), 1);
        while let Some(result) = workers.join_next().await {
            drop(result.unwrap());
        }
        assert_eq!(slots.available_permits(), 4);
    }
}
