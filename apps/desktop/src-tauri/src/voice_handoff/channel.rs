//! Disabled encrypted framing and one prepaid physical position. This is not a listener,
//! stock-IPC copy proof, browser-close proof, native action adapter or live driver. Its child
//! bootstrap module is an internal ECDH candidate, without a renderer/IO consumer yet.
//! A MAC identifies this local channel only; every Root effect still needs its actual owner.
use super::custody::{Buffer, Phase};
use super::http;
#[cfg(test)]
use catcoms_rt::CryptoRngCore;
#[cfg(test)]
use catcoms_sync::voice_v2::resources::ResourceKind;
use catcoms_sync::voice_v2::resources::{Credit, Resources};
use std::sync::{Arc, Mutex};
use zeroize::Zeroize;
#[cfg(test)]
use zeroize::Zeroizing;

pub(super) mod bootstrap;
mod crypto;
pub(super) use crypto::{BodyKind, Operation};
use crypto::{Direction, Header, Keys};

pub(super) const CONTROL_BYTES: usize = 12 * 1024;
pub(super) const BOOTSTRAP_BYTES: usize = 2048;
const CONTROL_CIPHER: usize = 256 + 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    Bounds,
    Header,
    Authentication,
    Body,
    Capacity,
    Replay,
    Exhausted,
    Binding,
}
struct Counters {
    busy: bool,
    request_floor: u64,
    response_sequence: u64,
}
/// Keys/counters never reset or adopt an imported seed. The native candidate's immutable-slot
/// ECDH registry keeps this original Session through uncertain replies. The fresh-seed fixture
/// constructor is test-only; no renderer seed/descriptor can reinstall keys or counters.
pub(super) struct Session {
    resources: Arc<Resources>,
    group: [u8; 32],
    slot: u8,
    keys: Keys,
    counters: Mutex<Counters>,
    _control: Credit,
    _bootstrap: Credit,
}
impl Session {
    #[cfg(test)]
    pub(super) fn new(
        resources: Arc<Resources>,
        group: [u8; 32],
        slot: u8,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Arc<Self>, Refused> {
        if !(1..=64).contains(&slot) {
            return Err(Refused::Header);
        }
        // No fallible Root/actor work precedes this payment. The unused pilot exports no seed;
        // actual bootstrap reply copies need their own finite custody before one is exposed.
        let control = resources
            .charge(group, CONTROL_BYTES, ResourceKind::Retained)
            .map_err(|_| Refused::Capacity)?;
        let bootstrap = resources
            .charge(group, BOOTSTRAP_BYTES, ResourceKind::Retained)
            .map_err(|_| Refused::Capacity)?;
        let mut seed = Zeroizing::new([0; 32]);
        rng.fill_bytes(seed.as_mut());
        let keys = Keys::new(&seed);
        Ok(Arc::new(Self {
            resources,
            group,
            slot,
            keys,
            counters: Mutex::new(Counters {
                busy: false,
                request_floor: 0,
                response_sequence: 0,
            }),
            _control: control,
            _bootstrap: bootstrap,
        }))
    }
    /// Own the paid scratch BEFORE reading unauthenticated HTTP. A single position continues
    /// across header/body/reply and completed output collection; releasing a caller is irrelevant.
    pub(super) fn header(self: &Arc<Self>) -> Result<HeaderWorker, Refused> {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if counters.busy {
            return Err(Refused::Capacity);
        }
        counters.busy = true;
        Ok(HeaderWorker {
            scratch: [0; http::HEADER_BYTES],
            filled: 0,
            session: self.clone(),
        })
    }
}
pub(super) struct HeaderWorker {
    scratch: [u8; http::HEADER_BYTES],
    filled: usize,
    session: Arc<Session>,
}
impl HeaderWorker {
    pub(super) fn scratch_mut(&mut self) -> &mut [u8; http::HEADER_BYTES] {
        &mut self.scratch
    }
    pub(super) fn set_filled(&mut self, bytes: usize) -> Result<(), Refused> {
        if bytes > self.scratch.len() {
            return Err(Refused::Bounds);
        }
        self.filled = bytes;
        Ok(())
    }
    /// The actual adapter resolves the native row and selects its original Phase before this
    /// call. Group/pool checks below are capacity binding only, never original-action authority.
    /// Capacity refusal before admission spends neither RX nor TX sequence. Once admitted,
    /// truncation, bad AEAD, cancellation and lost replies never refund either counter.
    pub(super) fn admit(
        self,
        port: u16,
        origin: &str,
        phase: Option<&Arc<Phase>>,
    ) -> Result<Incoming, Refused> {
        let head = http::parse(&self.scratch[..self.filled], port, origin)
            .map_err(|_| Refused::Header)?
            .ok_or(Refused::Header)?;
        if head.method != http::Method::Post {
            return Err(Refused::Header);
        }
        let request = self
            .session
            .keys
            .authenticate_request(head.admission.ok_or(Refused::Header)?)?;
        if request.slot() != self.session.slot || request.length() != head.length {
            return Err(Refused::Binding);
        }
        let mut body = Body::new(
            request.kind(),
            request.length(),
            phase,
            &self.session.resources,
            self.session.group,
        )?;
        body.cipher_mut()[..head.body_prefix.len()].copy_from_slice(head.body_prefix);
        let filled = head.body_prefix.len();
        let response_sequence = {
            let mut counters = self
                .session
                .counters
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if request.sequence() <= counters.request_floor {
                return Err(Refused::Replay);
            }
            let next = counters
                .response_sequence
                .checked_add(1)
                .ok_or(Refused::Exhausted)?;
            counters.request_floor = request.sequence();
            counters.response_sequence = next;
            next
        };
        Ok(Incoming {
            body,
            filled,
            request,
            response_sequence,
            worker: self,
        })
    }
}
impl Drop for HeaderWorker {
    fn drop(&mut self) {
        self.scratch.zeroize();
        // All body/reply objects are declared before this final owner and drop first. No actual
        // input, crypto/Root callback or affine owner destructor runs under the counter fence.
        let mut counters = self
            .session
            .counters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        debug_assert!(counters.busy);
        counters.busy = false;
    }
}
// Control is deliberately inline in the prepaid physical worker. Boxing its fixed bytes would
// introduce another allocation/custody path precisely when the data Phase may already be closed.
#[allow(clippy::large_enum_variant)]
enum Storage {
    Control([u8; CONTROL_CIPHER]),
    Data(Buffer),
}
struct Body {
    storage: Storage,
    cipher_bytes: usize,
    plain_bytes: Option<usize>,
}
impl Body {
    fn new(
        kind: BodyKind,
        bytes: usize,
        phase: Option<&Arc<Phase>>,
        resources: &Arc<Resources>,
        group: [u8; 32],
    ) -> Result<Self, Refused> {
        let storage = match kind {
            BodyKind::Control if (16..=CONTROL_CIPHER).contains(&bytes) => {
                Storage::Control([0; CONTROL_CIPHER])
            }
            BodyKind::Data if (16..=http::BODY_BYTES).contains(&bytes) => {
                let phase = phase.ok_or(Refused::Capacity)?;
                if !phase.uses_pool(resources, group) {
                    return Err(Refused::Binding);
                }
                Storage::Data(phase.buffer(bytes).map_err(|_| Refused::Capacity)?)
            }
            _ => return Err(Refused::Bounds),
        };
        Ok(Self {
            storage,
            cipher_bytes: bytes,
            plain_bytes: None,
        })
    }
    fn cipher_mut(&mut self) -> &mut [u8] {
        match &mut self.storage {
            Storage::Control(bytes) => &mut bytes[..self.cipher_bytes],
            Storage::Data(buffer) => buffer.bytes_mut(),
        }
    }
    fn cipher(&self) -> &[u8] {
        match &self.storage {
            Storage::Control(bytes) => &bytes[..self.cipher_bytes],
            Storage::Data(buffer) => buffer.bytes(),
        }
    }
    fn plain(&self) -> Result<&[u8], Refused> {
        Ok(&self.cipher()[..self.plain_bytes.ok_or(Refused::Body)?])
    }
}
impl Drop for Body {
    fn drop(&mut self) {
        self.cipher_mut().zeroize();
    }
}
pub(super) struct Incoming {
    body: Body,
    filled: usize,
    request: Header,
    // Private affine outgoing nonce, spent before any request body/Root effect. No Clone or
    // constructor can give another reply preparation this key/sequence pair.
    response_sequence: u64,
    worker: HeaderWorker,
}
impl Incoming {
    pub(super) fn remaining_mut(&mut self) -> &mut [u8] {
        &mut self.body.cipher_mut()[self.filled..]
    }
    pub(super) fn advance(&mut self, bytes: usize) -> Result<(), Refused> {
        let total = self.filled.checked_add(bytes).ok_or(Refused::Bounds)?;
        if total > self.body.cipher_bytes {
            return Err(Refused::Bounds);
        }
        self.filled = total;
        Ok(())
    }
    pub(super) fn open(&mut self) -> Result<&[u8], Refused> {
        if self.filled != self.body.cipher_bytes || self.body.plain_bytes.is_some() {
            return Err(Refused::Body);
        }
        self.worker
            .session
            .keys
            .open_request(&self.request, self.body.cipher_mut())?;
        self.body.plain_bytes = Some(self.body.cipher_bytes - 16);
        self.body.plain()
    }
    pub(super) fn operation(&self) -> Operation {
        self.request.operation()
    }
    pub(super) fn logical(&self) -> [u8; 16] {
        self.request.logical()
    }
    /// Consume the one nonce/paid worker. The old request drops before allocating the reply;
    /// bytes may not escape their paid owner. This is not yet an Actor payload-transfer API.
    pub(super) fn reply(
        self,
        kind: BodyKind,
        plain_bytes: usize,
        phase: Option<&Arc<Phase>>,
    ) -> Result<PreparedReply, Refused> {
        if self.body.plain_bytes.is_none() {
            return Err(Refused::Body);
        }
        let Self {
            body,
            request,
            response_sequence,
            worker,
            ..
        } = self;
        drop(body);
        let cipher_bytes = plain_bytes.checked_add(16).ok_or(Refused::Bounds)?;
        let response = Header::new(
            Direction::Response,
            request.operation(),
            kind,
            request.slot(),
            response_sequence,
            request.logical(),
            cipher_bytes,
        )?;
        let body = Body::new(
            kind,
            cipher_bytes,
            phase,
            &worker.session.resources,
            worker.session.group,
        )?;
        Ok(PreparedReply {
            body,
            request,
            response,
            worker,
        })
    }
}
pub(super) struct PreparedReply {
    body: Body,
    request: Header,
    response: Header,
    worker: HeaderWorker,
}
impl PreparedReply {
    pub(super) fn plain_mut(&mut self) -> &mut [u8] {
        let bytes = self.body.cipher_bytes - 16;
        &mut self.body.cipher_mut()[..bytes]
    }
    /// Consuming this owner makes encryption nonrepeatable under the issued physical nonce.
    pub(super) fn seal(mut self) -> Result<CompletedReply, Refused> {
        self.worker.session.keys.seal_response(
            &self.response,
            &self.request,
            self.body.cipher_mut(),
        )?;
        let admission = self
            .worker
            .session
            .keys
            .response_admission(&self.response, &self.request);
        Ok(CompletedReply {
            body: self.body,
            admission,
            worker: self.worker,
        })
    }
}
/// Retain through the actual socket writer and completed-task output collection. Returning
/// readiness or dropping the caller's wait is not collection and cannot free this position.
pub(super) struct CompletedReply {
    body: Body,
    admission: [u8; crypto::ADMISSION],
    worker: HeaderWorker,
}
impl CompletedReply {
    pub(super) fn admission(&self) -> &[u8; crypto::ADMISSION] {
        &self.admission
    }
    pub(super) fn body(&self) -> &[u8] {
        self.body.cipher()
    }
}

#[cfg(test)]
mod tests;
