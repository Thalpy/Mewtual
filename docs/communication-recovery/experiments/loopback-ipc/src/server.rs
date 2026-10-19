//! Tiny closed loopback fixture for packaged-origin/CSP/CORS checks. No file/URL/proxy API,
//! application keys, media or production authorization. Encrypted frames are generated fixture
//! patterns only; framing does not authorize any application action or measure engine heaps.
use crate::framing::{self, Header, Keys, Sequences};
use catcoms_rt::{Clock, SystemClock};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{watch, Semaphore},
    task::JoinHandle,
};
use zeroize::Zeroizing;

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
    frame_headers: AtomicU64,
    frame_bodies: AtomicU64,
    frame_replies: AtomicU64,
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
    frame_headers: u64,
    frame_bodies: u64,
    frame_replies: u64,
}
pub(super) struct Service {
    port: u16,
    blocked_port: u16,
    stopped: watch::Sender<bool>,
    pub metrics: Metrics,
    keys: [Keys; 2],
    sequences: Mutex<Sequences>,
    frames: Arc<Semaphore>,
    clock: Arc<dyn Clock>,
    closed: watch::Sender<u8>,
}
impl Service {
    pub async fn start(seeds: &[[u8; 32]; 2]) -> std::io::Result<(Arc<Self>, Vec<JoinHandle<()>>)> {
        let permitted = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let blocked = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let service = Arc::new(Self {
            port: permitted.local_addr()?.port(),
            blocked_port: blocked.local_addr()?.port(),
            stopped: watch::channel(false).0,
            metrics: Metrics::default(),
            keys: [Keys::new(&seeds[0]), Keys::new(&seeds[1])],
            sequences: Mutex::new(Sequences::default()),
            frames: Arc::new(Semaphore::new(2)),
            clock: Arc::new(SystemClock),
            closed: watch::channel(0).0,
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
    /// Fixture-only takeover gate. Both listeners and every original worker must really drop
    /// before the independent keyless runner may try binding the old permitted port.
    pub async fn wait_closed(&self) {
        let mut closed = self.closed.subscribe();
        while *closed.borrow() != 2 {
            if closed.changed().await.is_err() {
                break;
            }
        }
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
            frame_headers: m.frame_headers.load(Ordering::SeqCst),
            frame_bodies: m.frame_bodies.load(Ordering::SeqCst),
            frame_replies: m.frame_replies.load(Ordering::SeqCst),
        }
    }
    async fn worker(
        self: Arc<Self>,
        socket: TcpStream,
        permitted: bool,
        permit: tokio::sync::OwnedSemaphorePermit,
        clock: Arc<dyn Clock>,
        deadline: u64,
    ) -> (
        tokio::sync::OwnedSemaphorePermit,
        Option<tokio::sync::OwnedSemaphorePermit>,
    ) {
        let current = self.metrics.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.metrics
            .peak_active
            .fetch_max(current, Ordering::SeqCst);
        let _active = Active(self.clone());
        let mut frame_permit = None;
        if clock.monotonic_ms() < deadline {
            tokio::select! {
                biased;
                _ = clock.sleep(Duration::from_millis(deadline.saturating_sub(clock.monotonic_ms()))) => {},
                _ = self.serve(socket, permitted, &mut frame_permit) => {},
            }
        }
        // A completed task still has a retained JoinSet result/header. Its actual permit
        // follows the OUTPUT until collection, rather than returning merely on I/O completion.
        (permit, frame_permit)
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
                    let clock = self.clock.clone();
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
        self.closed.send_modify(|count| *count += 1);
    }
    async fn serve(
        &self,
        mut socket: TcpStream,
        permitted: bool,
        frame_permit: &mut Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> std::io::Result<()> {
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
        if request.method == "POST" {
            if !permitted || request.origin != Some(PACKAGED_ORIGIN) {
                return Ok(());
            }
            let Some(admission) = request.admission else {
                return Ok(());
            };
            let Ok(grant) = framing::claimed_grant(admission) else {
                return Ok(());
            };
            let Ok(header) = self.keys[grant].authenticate_request(admission) else {
                return Ok(());
            };
            if header.cipher_bytes() != request.length {
                return Ok(());
            }
            let Ok(permit) = self.frames.clone().try_acquire_owned() else {
                return Ok(());
            };
            // One synchronous transaction after authentication and capacity, before allocation.
            // Original replay/capacity spend persists through EOF, bad AEAD or worker timeout.
            if self
                .sequences
                .lock()
                .expect("fixture sequence lock")
                .admit(&header)
                .is_err()
            {
                return Ok(());
            }
            *frame_permit = Some(permit);
            self.metrics.frame_headers.fetch_add(1, Ordering::SeqCst);
            return self.frame(&mut socket, header, request.prefix).await;
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
            let response = format!("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {PACKAGED_ORIGIN}\r\nAccess-Control-Allow-Methods: GET, POST\r\nAccess-Control-Allow-Headers: x-probe, x-probe-admission\r\nAccess-Control-Allow-Private-Network: true\r\n\r\n");
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
    async fn frame(
        &self,
        socket: &mut TcpStream,
        request: Header,
        prefix: &[u8],
    ) -> std::io::Result<()> {
        // The authenticated, bounded length is known before this first body allocation. The
        // HTTP scratch may already hold ciphertext; preserve it rather than losing coalesced bytes.
        let mut bytes = Zeroizing::new(vec![0; request.cipher_bytes()]);
        bytes[..prefix.len()].copy_from_slice(prefix);
        socket.read_exact(&mut bytes[prefix.len()..]).await?;
        if self.keys[request.grant()]
            .open_request(&request, &mut bytes)
            .is_err()
        {
            return Ok(());
        }
        if !bytes
            .iter()
            .enumerate()
            .all(|(i, byte)| *byte == (i.wrapping_mul(17).wrapping_add(3) & 255) as u8)
        {
            return Ok(());
        }
        self.metrics.frame_bodies.fetch_add(1, Ordering::SeqCst);
        let response = self
            .sequences
            .lock()
            .expect("fixture sequence lock")
            .response(&request, bytes.len())
            .expect("finite fixture response count");
        if self.keys[request.grant()]
            .seal_response(&response, &request, &mut bytes)
            .is_err()
        {
            return Ok(());
        }
        let admission = self.keys[request.grant()].response_admission(&response, &request);
        let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {PACKAGED_ORIGIN}\r\nAccess-Control-Expose-Headers: x-probe-admission\r\nx-probe-admission: {admission}\r\n\r\n", bytes.len());
        socket.write_all(reply.as_bytes()).await?;
        socket.write_all(&bytes).await?;
        socket.shutdown().await?;
        self.metrics.frame_replies.fetch_add(1, Ordering::SeqCst);
        Ok(())
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
    admission: Option<&'a str>,
    length: usize,
    prefix: &'a [u8],
}
/// Closed HTTP subset. The fixed scratch may contain a ciphertext prefix; only the header
/// prefix is parsed. Body allocation requires separate authentication. No chunking or pipeline.
fn parse(bytes: &[u8]) -> Option<Request<'_>> {
    let end = bytes.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    if bytes.len() > HEADER_BYTES {
        return None;
    }
    let text = std::str::from_utf8(&bytes[..end]).ok()?;
    let mut lines = text[..text.len() - 4].split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?;
    let path = first.next()?;
    if !matches!(method, "GET" | "OPTIONS" | "POST")
        || !matches!(path, "/ok" | "/wrong-cors" | "/remote" | "/frame")
        || (method == "POST") != (path == "/frame" && method != "OPTIONS")
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
    if headers.contains_key("transfer-encoding") {
        return None;
    }
    let length = match headers.get("content-length") {
        Some(length) if !length.is_empty() && length.bytes().all(|b| b.is_ascii_digit()) => {
            let parsed: usize = length.parse().ok()?;
            if parsed.to_string() != *length {
                return None;
            }
            parsed
        }
        None if method != "POST" => 0,
        _ => return None,
    };
    if (method == "POST" && !(16..=framing::MAX_CIPHER).contains(&length))
        || (method != "POST" && length != 0)
        || bytes.len() - end > length
    {
        return None;
    }
    Some(Request {
        method,
        path,
        host: headers.get("host")?,
        origin: headers.get("origin").copied(),
        private_network: headers.get("access-control-request-private-network") == Some(&"true"),
        admission: headers.get("x-probe-admission").copied(),
        length,
        prefix: &bytes[end..],
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
            keys: [Keys::new(&[1; 32]), Keys::new(&[2; 32])],
            sequences: Mutex::new(Sequences::default()),
            frames: Arc::new(Semaphore::new(2)),
            clock: Arc::new(catcoms_rt::ManualClock::new(1000)),
            closed: watch::channel(0).0,
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

    #[test]
    fn frame_parser_preserves_every_coalesced_ciphertext_prefix_and_refuses_ambiguity() {
        let head = b"POST /frame HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: 32\r\nx-probe-admission: fixture\r\n\r\n";
        for split in 0..=head.len() {
            assert_eq!(parse(&head[..split]).is_some(), split == head.len());
        }
        for count in 0..=32 {
            let mut packet = head.to_vec();
            packet.extend(vec![255; count]);
            let parsed = parse(&packet).unwrap();
            assert_eq!(parsed.length, 32);
            assert_eq!(parsed.prefix, vec![255; count]);
        }
        let mut extra = head.to_vec();
        extra.extend([255; 33]);
        assert!(parse(&extra).is_none());
        for bad in [
            "POST /frame HTTP/1.1\r\nHost: x\r\nContent-Length: 16\r\nX-Probe-Admission: a\r\nx-probe-admission: b\r\n\r\n",
            "POST /frame HTTP/1.1\r\nHost: x\r\nContent-Length: 016\r\n\r\n",
            "POST /frame HTTP/1.1\r\nHost: x\r\nContent-Length: 8209\r\n\r\n",
            "POST /frame HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nContent-Length: 16\r\n\r\n",
        ] { assert!(parse(bad.as_bytes()).is_none()); }
    }

    async fn fixture() -> (TcpListener, Arc<Service>, Arc<catcoms_rt::ManualClock>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let clock = Arc::new(catcoms_rt::ManualClock::new(1000));
        let service = Arc::new(Service {
            port: listener.local_addr().unwrap().port(),
            blocked_port: 1,
            stopped: watch::channel(false).0,
            metrics: Metrics::default(),
            keys: [Keys::new(&[1; 32]), Keys::new(&[2; 32])],
            sequences: Mutex::new(Sequences::default()),
            frames: Arc::new(Semaphore::new(2)),
            clock: clock.clone(),
            closed: watch::channel(0).0,
        });
        (listener, service, clock)
    }
    fn request_bytes(service: &Service, sequence: u64) -> Vec<u8> {
        let mut body: Vec<u8> = (0..128).map(|i| (i * 17 + 3) as u8).collect();
        let admission = service.keys[0].test_request(sequence, &mut body);
        let mut request = format!("POST /frame HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: {PACKAGED_ORIGIN}\r\nContent-Length: {}\r\nx-probe-admission: {admission}\r\n\r\n", service.port, body.len()).into_bytes();
        request.extend(body);
        request
    }
    #[tokio::test]
    async fn completed_frames_keep_both_actual_permits_until_result_collection() {
        let (listener, service, clock) = fixture().await;
        let slots = Arc::new(Semaphore::new(4));
        let mut workers = tokio::task::JoinSet::new();
        let mut finished = Vec::new();
        for sequence in 1..=2 {
            let mut client = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (socket, _) = listener.accept().await.unwrap();
            client
                .write_all(&request_bytes(&service, sequence))
                .await
                .unwrap();
            let service = service.clone();
            let clock = clock.clone();
            let permit = slots.clone().try_acquire_owned().unwrap();
            let (sent, received) = tokio::sync::oneshot::channel();
            finished.push(received);
            workers.spawn(async move {
                let result = service.worker(socket, true, permit, clock, 3000).await;
                let _ = sent.send(());
                result
            });
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            assert!(response.starts_with(b"HTTP/1.1 200"));
        }
        for finished in finished {
            finished.await.unwrap();
        }
        assert_eq!(service.metrics.frame_replies.load(Ordering::SeqCst), 2);
        assert_eq!(service.metrics.active.load(Ordering::SeqCst), 0);
        assert_eq!(
            service.frames.available_permits(),
            0,
            "finished records retain actual authenticated-frame positions"
        );
        assert_eq!(slots.available_permits(), 2);
        drop(workers.join_next().await.unwrap().unwrap());
        assert_eq!(service.frames.available_permits(), 1);
        assert_eq!(slots.available_permits(), 3);
        drop(workers.join_next().await.unwrap().unwrap());
        assert_eq!(service.frames.available_permits(), 2);
        assert_eq!(slots.available_permits(), 4);
    }
    #[tokio::test]
    async fn shutdown_waits_for_original_partial_body_deadline_without_refunding_admission() {
        let (listener, service, clock) = fixture().await;
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let request = request_bytes(&service, 1);
        client
            .write_all(&request[..request.len() - 100])
            .await
            .unwrap();
        let task = tokio::spawn(service.clone().listen(
            listener,
            true,
            Arc::new(Semaphore::new(4)),
        ));
        for _ in 0..1000 {
            if service.metrics.frame_headers.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(service.metrics.frame_headers.load(Ordering::SeqCst), 1);
        service.stop();
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "stop cannot refund a still-owned partial body"
        );
        assert_eq!(service.frames.available_permits(), 1);
        clock.advance_ms(2000);
        task.await.unwrap();
        assert_eq!(service.frames.available_permits(), 2);
        assert_eq!(service.metrics.frame_bodies.load(Ordering::SeqCst), 0);
        let header = service.keys[0]
            .authenticate_request(
                parse(&request[..request.len() - 100])
                    .unwrap()
                    .admission
                    .unwrap(),
            )
            .unwrap();
        assert!(
            service.sequences.lock().unwrap().admit(&header).is_err(),
            "timeout never refunds the spent sequence"
        );
    }
}
