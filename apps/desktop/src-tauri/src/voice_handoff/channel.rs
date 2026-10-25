//! Encrypted framing and prepaid native/physical positions for the application handoff.
//! Framing alone supplies no stock-IPC copy proof, browser-close proof or RTC grant. Its
//! bootstrap owns the exact ECDH channel used by the authenticated service and browser broker.
//! A MAC identifies this local channel only; every Root effect still needs its actual owner.
use super::custody::{Buffer, Phase};
use super::http;
#[cfg(test)]
use catcoms_rt::CryptoRngCore;
#[cfg(test)]
use catcoms_sync::voice_v2::resources::ResourceKind;
use catcoms_sync::voice_v2::resources::{Credit, Resources, WorkLane};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use zeroize::Zeroize;
#[cfg(test)]
use zeroize::Zeroizing;

pub(super) mod bootstrap;
mod crypto;
pub(in crate::voice_handoff) mod sdp;
pub(super) use crypto::{BodyKind, Operation};
use crypto::{Direction, Header, Keys};

pub(super) const CONTROL_BYTES: usize = 12 * 1024;
// Browser-owned fixed control metadata, SDK input/output views, one request and one BYOB
// response are separate from the native header/body/reply envelope. Both share the original
// serial Crypto lane, but these bytes coexist across the local HTTP boundary.
// Existing8KiB encrypted control plus16KiB metadata read/cleanup custody. Both are prepaid
// before setup ACK can enable independent producers; End/full-pool pressure needs no grant.
pub(super) const BROWSER_CONTROL_BYTES: usize = 24 * 1024;
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
    // Process shutdown closes admission permanently. An old Route cannot reopen this Session.
    closed: bool,
    busy: bool,
    // One independently funded native producer may coexist with the browser position.
    // This is drain custody only, never a browser Header or action/engine permission.
    native_busy: bool,
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
    // A separate, fixed paid allocation identifies this exact Session without retaining its
    // keys, counters or credits in final action history. Equal transcript bytes alone never
    // establish native provenance. Every Session receives its own fresh allocation.
    identity: Arc<SessionIdentity>,
    counters: Mutex<Counters>,
    idle: tokio::sync::Notify,
    _control: Credit,
    _bootstrap: Credit,
    _browser_control: Credit,
    // Prepaid from existing ordinary Crypto limits, independently of setup/data work.
    // Terminal browser control cannot depend on a fresh slot when other work fills the pool.
    control_lane: Arc<WorkLane>,
    // Negative-only interoperability seam. It cannot authenticate/admit anything and
    // is absent from shipping Sessions; the actual adapter must produce the refusal.
    #[cfg(test)]
    input_pressure: AtomicBool,
    // Observation only: an exact physical Result reached its actual post-preflight UI wait.
    // Monotonic request sequences prevent a previous attempt from satisfying a new barrier.
    #[cfg(test)]
    result_preflight_sequence: std::sync::atomic::AtomicU64,
}
pub(super) struct SessionIdentity(pub(super) [u8; 32]);
impl Session {
    #[cfg(test)]
    pub(super) fn fixture_arm_input_pressure(&self) {
        assert!(
            !self.input_pressure.swap(true, Ordering::AcqRel),
            "one original Input pressure observation at a time"
        );
    }
    #[cfg(test)]
    pub(super) fn fixture_take_input_pressure(&self) -> bool {
        self.input_pressure.swap(false, Ordering::AcqRel)
    }
    #[cfg(test)]
    pub(super) fn fixture_input_pressure_pending(&self) -> bool {
        self.input_pressure.load(Ordering::Acquire)
    }
    #[cfg(test)]
    pub(super) fn fixture_result_preflight_sequence(&self) -> u64 {
        self.result_preflight_sequence.load(Ordering::Acquire)
    }
    pub(super) fn identity(&self) -> &Arc<SessionIdentity> {
        &self.identity
    }
    #[cfg(test)]
    pub(super) fn fixture_counts(&self) -> (bool, u64, u64) {
        let counters = self
            .counters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (
            counters.busy,
            counters.request_floor,
            counters.response_sequence,
        )
    }
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
        let browser_control = resources
            .charge(group, BROWSER_CONTROL_BYTES, ResourceKind::Retained)
            .map_err(|_| Refused::Capacity)?;
        let control_lane = resources
            .reserve_crypto_lane(group)
            .map_err(|_| Refused::Capacity)?;
        let mut seed = Zeroizing::new([0; 32]);
        rng.fill_bytes(seed.as_mut());
        let keys = Keys::new(&seed);
        Ok(Arc::new(Self {
            identity: Arc::new(SessionIdentity({
                use sha2::{Digest, Sha256};
                let mut hash = Sha256::new();
                hash.update(b"native fixture session identity");
                hash.update(seed.as_ref());
                hash.finalize().into()
            })),
            resources,
            group,
            slot,
            keys,
            counters: Mutex::new(Counters {
                closed: false,
                busy: false,
                native_busy: false,
                request_floor: 0,
                response_sequence: 0,
            }),
            idle: tokio::sync::Notify::new(),
            _control: control,
            _bootstrap: bootstrap,
            _browser_control: browser_control,
            control_lane,
            input_pressure: AtomicBool::new(false),
            result_preflight_sequence: std::sync::atomic::AtomicU64::new(0),
        }))
    }
    /// Own the paid scratch BEFORE reading unauthenticated HTTP. A single position continues
    /// across header/body/reply and completed output collection; releasing a caller is irrelevant.
    pub(super) fn header(self: &Arc<Self>) -> Result<HeaderWorker, Refused> {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if counters.closed {
            return Err(Refused::Binding);
        }
        if counters.busy {
            return Err(Refused::Capacity);
        }
        counters.busy = true;
        Ok(HeaderWorker {
            scratch: Box::new([0; http::HEADER_BYTES]),
            filled: 0,
            position: Arc::new(PhysicalPosition {
                session: self.clone(),
                held: AtomicBool::new(false),
            }),
        })
    }

    /// Own one native producer position on this actual Session. The caller must pay its
    /// complete native task/preparation/output BEFORE constructing or awaiting that work;
    /// the browser's Control envelope may already be occupied. This private affine guard
    /// neither authenticates a frame nor creates a physical request/response nonce.
    pub(super) fn native_hold(self: &Arc<Self>) -> Result<NativeHold, Refused> {
        let mut counters = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        if counters.closed {
            return Err(Refused::Binding);
        }
        if counters.native_busy {
            return Err(Refused::Capacity);
        }
        counters.native_busy = true;
        Ok(NativeHold {
            session: self.clone(),
        })
    }

    /// Irreversible process-shutdown fence, independent of call lease revocation. Hold the
    /// counter mutex so an old Route's physical/native acquisition either precedes this
    /// fence (and must be collected), or refuses without debiting a sequence.
    pub(super) fn close_admission(&self) {
        self.counters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed = true;
    }

    /// The service has ONE shutdown collector. Register its wake before inspecting busy,
    /// avoiding a lost last-drop notification. This waits for actual header/reply AND native
    /// producer/worker/completion ownership; timeout, socket loss and End are not substitutes.
    /// The original control payment includes this Notify and its one in-place waiter.
    pub(super) async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let counters = self
                    .counters
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                debug_assert!(counters.closed);
                if !counters.busy && !counters.native_busy {
                    return;
                }
            }
            notified.await;
        }
    }
    /// Test-only observation of one original position while preserving this Session for a
    /// later exact history query. Production shutdown still requires close_admission first;
    /// this fixture observer grants neither admission nor authority and never clears busy.
    #[cfg(test)]
    pub(in crate::voice_handoff) async fn fixture_wait_physical_idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.counters.lock().unwrap_or_else(|e| e.into_inner()).busy {
                return;
            }
            notified.await;
        }
    }
}

/// Private non-Clone native drain position. Retain the canonical Route separately too:
/// its actual Row must remain discoverable by shutdown/GC while this guard owns its Session.
/// Payment, work, grants and commit must drop BEFORE this last field publishes native idle.
pub(super) struct NativeHold {
    session: Arc<Session>,
}
impl Drop for NativeHold {
    fn drop(&mut self) {
        let mut counters = self
            .session
            .counters
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        debug_assert!(counters.native_busy);
        counters.native_busy = false;
        drop(counters);
        self.session.idle.notify_waiters();
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        // Header/reply owners hold this actual Arc, so they have drained before final Drop.
        // Browser/provider retirement must separately precede registry collection; End alone
        // cannot remove the row. This closes its one original lane, never a replacement.
        self.control_lane.close();
    }
}
pub(super) struct HeaderWorker {
    // ONE fixed allocation, prepaid by the original Session control before header admission.
    // Moving this affine handle through async frames/output must not reserve another inline
    //4KiB scratch in each generator state. The codec peak counts the FULL backing allocation.
    // Keep this field first: zeroize/free before Position's last drop can publish idle.
    scratch: Box<[u8; http::HEADER_BYTES]>,
    filled: usize,
    position: Arc<PhysicalPosition>,
}
/// The actual physical position can outlive a cancelled socket/header when admitted actor
/// work is detached. Its separately paid Session remains busy until BOTH original owners
/// drain. This is not an engine/result authority and does not reset physical counters.
struct PhysicalPosition {
    session: Arc<Session>,
    held: AtomicBool,
}
impl Drop for PhysicalPosition {
    fn drop(&mut self) {
        let mut counters = self
            .session
            .counters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        debug_assert!(counters.busy);
        counters.busy = false;
        // Drop counters before waking a collector or the actual Session/credits destruct.
        drop(counters);
        self.session.idle.notify_waiters();
    }
}
/// At most ONE extra owner of an admitted frame; retain in the actual worker AND its completed
/// output until collection. No public Clone or constructor from a Session identifier exists.
pub(super) struct PhysicalHold {
    position: Arc<PhysicalPosition>,
}
impl PhysicalHold {
    pub(super) fn belongs_to(&self, session: &Arc<Session>) -> bool {
        Arc::ptr_eq(&self.position.session, session)
    }
}
impl Drop for PhysicalHold {
    fn drop(&mut self) {
        self.position.held.store(false, Ordering::Release);
    }
}
impl HeaderWorker {
    fn hold(&self) -> Result<PhysicalHold, Refused> {
        self.position
            .held
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Refused::Capacity)?;
        Ok(PhysicalHold {
            position: self.position.clone(),
        })
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_hold(&self) -> Result<PhysicalHold, Refused> {
        self.hold()
    }
    pub(super) fn scratch_mut(&mut self) -> &mut [u8; http::HEADER_BYTES] {
        self.scratch.as_mut()
    }
    pub(super) fn set_filled(&mut self, bytes: usize) -> Result<(), Refused> {
        if bytes > self.scratch.len() {
            return Err(Refused::Bounds);
        }
        self.filled = bytes;
        Ok(())
    }
    /// Low-level codec seam for a caller already holding its original Phase. A dispatcher
    /// first uses authenticate + the opaque proof when it needs to look up that capacity.
    /// Group/pool checks are capacity binding only, never original-action authority.
    /// Capacity refusal before admission spends neither RX nor TX sequence. Once admitted,
    /// truncation, bad AEAD, cancellation and lost replies never refund either counter.
    pub(super) fn admit(
        self,
        port: u16,
        origin: &str,
        phase: Option<&Arc<Phase>>,
    ) -> Result<Incoming, Refused> {
        self.authenticate(port, origin)?.admit(phase)
    }
    /// Closed status-only service seam: operation/class/exact length/logical session are
    /// authenticated BEFORE counter debit. No callback or renderer-selected predicate runs
    /// under the counter fence. An admitted malformed body still permanently spends both.
    pub(super) fn admit_status(
        self,
        port: u16,
        origin: &str,
        session: [u8; 16],
    ) -> Result<Incoming, Refused> {
        self.admit_service_control(port, origin, session, false)
    }
    /// Native-configured read-only service whitelist. Authentication and exact header shape
    /// precede counter debit. Historical Begin can only LOOK UP an original row after AEAD;
    /// no flag here enables factories, data input/results, engine admission or cleanup.
    pub(super) fn admit_service_control(
        self,
        port: u16,
        origin: &str,
        session: [u8; 16],
        history_available: bool,
    ) -> Result<Incoming, Refused> {
        let authenticated = self.authenticate(port, origin)?;
        let request = &authenticated.request;
        let permitted = request.kind() == BodyKind::Control
            && match request.operation() {
                Operation::Status => request.length() == 49 + 16 && request.logical() == session,
                Operation::BeginDerive if history_available => {
                    request.length() == 106 + 16 && request.logical() == [0; 16]
                }
                _ => false,
            };
        if !permitted {
            return Err(Refused::Binding);
        }
        authenticated.admit(None)
    }
    /// Authenticate immutable header metadata BEFORE any action-Phase lookup. This owner
    /// retains the original paid HeaderWorker and exact parsed bytes through body admission;
    /// a caller cannot supply another RowId or reparse a changed header after authentication.
    /// No body, counters, variable capacity, Action or Root authority is admitted here.
    pub(super) fn authenticate(
        self,
        port: u16,
        origin: &str,
    ) -> Result<AuthenticatedRequest, Refused> {
        let head = http::parse(&self.scratch[..self.filled], port, origin)
            .map_err(|_| Refused::Header)?
            .ok_or(Refused::Header)?;
        if head.method != http::Method::Post {
            return Err(Refused::Header);
        }
        let request = self
            .position
            .session
            .keys
            .authenticate_request(head.admission.ok_or(Refused::Header)?)?;
        if request.slot() != self.position.session.slot || request.length() != head.length {
            return Err(Refused::Binding);
        }
        let prefix_bytes = head.body_prefix.len();
        let prefix_start = self.filled - prefix_bytes;
        Ok(AuthenticatedRequest {
            request,
            prefix_start,
            prefix_bytes,
            worker: self,
        })
    }
}

/// Opaque proof of canonical authenticated header metadata only. It is not body admission,
/// current effect authority or a reusable token; consuming admission spends counters exactly
/// once on the original Session. No public fields, Clone or renderer constructor exists.
pub(super) struct AuthenticatedRequest {
    request: Header,
    prefix_start: usize,
    prefix_bytes: usize,
    worker: HeaderWorker,
}
impl AuthenticatedRequest {
    /// Native allocation identity of the channel which verified this immutable Header.
    /// Equal seeds/transcripts/slots do not make another Session its original owner.
    pub(in crate::voice_handoff) fn belongs_to(&self, identity: &Arc<SessionIdentity>) -> bool {
        Arc::ptr_eq(self.worker.position.session.identity(), identity)
    }
    /// Authenticated ciphertext length, including the AEAD tag. A dispatcher may reject an
    /// impossible fixed grammar before Phase lookup/allocation or irreversible counter debit.
    pub(super) fn length(&self) -> usize {
        self.request.length()
    }
    pub(super) fn logical(&self) -> [u8; 16] {
        self.request.logical()
    }
    pub(super) fn operation(&self) -> Operation {
        self.request.operation()
    }
    pub(super) fn kind(&self) -> BodyKind {
        self.request.kind()
    }
    pub(super) fn admit(self, phase: Option<&Arc<Phase>>) -> Result<Incoming, Refused> {
        let request = self.request;
        let body = Body::new(
            request.kind(),
            request.length(),
            phase,
            &self.worker.position.session.resources,
            self.worker.position.session.group,
        )?;
        self.admit_body(body)
    }
    /// Separate SDP-only custody. The generic optional confirmation Phase path always
    /// rejects class2, including with an otherwise genuine paid confirmation owner.
    pub(in crate::voice_handoff) fn admit_sdp(
        self,
        permit: sdp::Permit,
    ) -> Result<Incoming, Refused> {
        let buffer = permit.allocate(&self)?;
        let body = Body {
            storage: Storage::Sdp(buffer),
            cipher_bytes: self.request.length(),
            plain_bytes: None,
        };
        self.admit_body(body)
    }
    fn admit_body(self, mut body: Body) -> Result<Incoming, Refused> {
        let request = self.request;
        body.cipher_mut()[..self.prefix_bytes].copy_from_slice(
            &self.worker.scratch[self.prefix_start..self.prefix_start + self.prefix_bytes],
        );
        let filled = self.prefix_bytes;
        let response_sequence = {
            let mut counters = self
                .worker
                .position
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
            worker: self.worker,
        })
    }
}
impl Drop for HeaderWorker {
    fn drop(&mut self) {
        self.scratch.as_mut().zeroize();
        // Body/reply objects drop first. The separate physical guard may still belong to an
        // actual detached actor worker/output; only its LAST drop can reopen this Session.
    }
}
// Control is deliberately inline in the prepaid physical worker. Boxing its fixed bytes would
// introduce another allocation/custody path precisely when the data Phase may already be closed.
#[allow(clippy::large_enum_variant)]
enum Storage {
    Control([u8; CONTROL_CIPHER]),
    Data(Buffer),
    Sdp(catcoms_sync::voice_v2::chain::sdp_handoff::Buffer),
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
            Storage::Sdp(buffer) => buffer.bytes_mut(),
        }
    }
    fn cipher(&self) -> &[u8] {
        match &self.storage {
            Storage::Control(bytes) => &bytes[..self.cipher_bytes],
            Storage::Data(buffer) => buffer.bytes(),
            Storage::Sdp(buffer) => buffer.bytes(),
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
    /// Read-only test observation from the real prepared worker. This changes no authority,
    /// counters, capacity or scheduling and cannot be selected by HTTP or ordinary IPC.
    #[cfg(test)]
    pub(super) fn fixture_mark_result_ui_wait(&self) {
        assert!(self.body.plain_bytes.is_some() && self.operation() == Operation::Result);
        self.worker
            .position
            .session
            .result_preflight_sequence
            .store(self.request.sequence(), Ordering::Release);
    }
    /// Native allocation binding, not a copied slot/session label. A successfully opened
    /// request from another Session cannot authorize work through this Route.
    pub(super) fn belongs_to(&self, session: &Arc<Session>) -> bool {
        Arc::ptr_eq(&self.worker.position.session, session)
    }
    /// Read only the already AEAD-opened native body. Calling this never decrypts twice or
    /// creates a separately owned plaintext copy; parsed borrows must end before extraction.
    pub(super) fn plain(&self) -> Result<&[u8], Refused> {
        self.body.plain()
    }
    /// Obtain the unique detached-worker hold only AFTER this original request was AEAD-opened.
    /// The dispatcher still must bind full purpose/RowId/current authority before effects.
    pub(super) fn hold_position(&self) -> Result<PhysicalHold, Refused> {
        if self.body.plain_bytes.is_none() {
            return Err(Refused::Body);
        }
        self.worker.hold()
    }
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
            .position
            .session
            .keys
            .open_request(&self.request, self.body.cipher_mut())?;
        self.body.plain_bytes = Some(self.body.cipher_bytes - 16);
        self.body.plain()
    }
    pub(super) fn operation(&self) -> Operation {
        self.request.operation()
    }
    pub(super) fn kind(&self) -> BodyKind {
        self.request.kind()
    }
    pub(super) fn logical(&self) -> [u8; 16] {
        self.request.logical()
    }

    /// Move the actual AEAD-opened Data buffer into the original Phase's actor body owner.
    /// The native dispatcher must already have resolved full purpose/action against the SAME
    /// authenticated header RowId, and selected the exact decoded output range. No bytes or
    /// Vec backing allocation are duplicated. The remaining affine request can still seal its
    /// one response; detached actor work additionally needs this position's unique hold.
    ///
    /// All local refusals preserve the exact request buffer for retry/diagnostics. Metadata
    /// comes from the original loan before ownership moves into the actor bridge. A successful
    /// move is irreversible: neither a second extraction nor another open is possible.
    pub(super) fn result_body(
        &mut self,
        phase: &Arc<Phase>,
        range: Range<usize>,
    ) -> Result<catcoms_app::VoiceConfirmationBody, Refused> {
        self.result_body_with_publication(phase, range, None)
    }
    /// Publication custody moves with the actual buffer, rather than staying solely in a
    /// cancellable worker. All local failures still restore the original opened body.
    pub(super) fn result_body_with_publication(
        &mut self,
        phase: &Arc<Phase>,
        range: Range<usize>,
        publication: Option<Arc<super::custody::PublicationCustody>>,
    ) -> Result<catcoms_app::VoiceConfirmationBody, Refused> {
        let plain = self.body.plain_bytes.ok_or(Refused::Body)?;
        if self.request.operation() != Operation::Result
            || !matches!(self.body.storage, Storage::Data(_))
            || range.start > range.end
            || range.end > plain
            || range.len() > 528
        {
            return Err(Refused::Bounds);
        }
        let Storage::Data(buffer) = std::mem::replace(
            &mut self.body.storage,
            Storage::Control([0; CONTROL_CIPHER]),
        ) else {
            unreachable!("checked Data storage");
        };
        let original_cipher = self.body.cipher_bytes;
        let original_filled = self.filled;
        // An empty opened placeholder keeps Body's Drop and the affine reply path valid.
        // Its bytes contain no extracted plaintext and never become a second request body.
        self.body.cipher_bytes = 16;
        self.body.plain_bytes = Some(0);
        self.filled = 16;
        match phase.result_body_with_publication(buffer, range, publication) {
            Ok(body) => Ok(body),
            Err((error, buffer)) => {
                self.body.storage = Storage::Data(buffer);
                self.body.cipher_bytes = original_cipher;
                self.body.plain_bytes = Some(plain);
                self.filled = original_filled;
                Err(match error {
                    catcoms_sync::voice_v2::VoiceError::Capacity => Refused::Capacity,
                    _ => Refused::Bounds,
                })
            }
        }
    }
    /// Regression seam: replace the actual opened SDP backing with a separately prepaid
    /// same-counter allocation which deliberately lacks Actor RPC metadata. This changes no
    /// Header, nonce, content or authority; production cannot select this helper. It permits
    /// exercising exact-buffer restoration on the real constructor's Capacity refusal.
    #[cfg(test)]
    pub(super) fn fixture_underpay_sdp_rpc(
        &mut self,
        phase: &Arc<catcoms_sync::voice_v2::chain::sdp_handoff::Phase>,
        funding: &Arc<catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowFunding>,
    ) {
        assert!(matches!(self.body.storage, Storage::Sdp(_)) && self.body.plain_bytes.is_some());
        let mut copy = phase.copy(self.body.cipher_bytes).unwrap();
        copy.bind_native_history(funding.clone()).unwrap();
        let replacement = copy.adopt(self.body.cipher().to_vec()).unwrap();
        let old = std::mem::replace(&mut self.body.storage, Storage::Sdp(replacement));
        drop(old);
    }
    /// Move exactly the SDP payload selected by the closed version8 decoder. This is a
    /// distinct Storage::Sdp path; confirmation Data/528 limits never widen. Native must
    /// first validate encrypted parent/PC/SDK/action bindings against this SAME row.
    /// Constructor refusal restores the identical opened allocation and affine reply.
    pub(super) fn sdp_body(
        &mut self,
        intent: Arc<catcoms_sync::voice_v2::sdp::InitialSdpIntent>,
        funding: &Arc<catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowFunding>,
    ) -> Result<catcoms_app::VoiceSdpBody, Refused> {
        use super::requests::sdp_action::{Kind, Request};
        use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
        let plain = self.body.plain_bytes.ok_or(Refused::Body)?;
        let bytes = self.body.plain()?;
        let request = Request::decode(self.operation(), self.kind(), self.logical(), bytes)
            .map_err(|_| Refused::Body)?;
        let (payload, purpose) = match request {
            Request::Begin(request) => (
                request.plaintext.ok_or(Refused::Body)?,
                NativeRowPurpose::Seal,
            ),
            Request::Existing(request) => match request.kind {
                Kind::Sealed { bytes, .. } => (bytes, NativeRowPurpose::Seal),
                Kind::Opened { bytes, .. } => (bytes, NativeRowPurpose::Open),
                _ => return Err(Refused::Body),
            },
            _ => return Err(Refused::Body),
        };
        // The closed decoder requires payload to be the final field and rejects suffixes.
        // Derive the range here; a caller cannot smuggle header bytes as original SDP.
        let range = plain.checked_sub(payload.len()).ok_or(Refused::Bounds)?..plain;
        let Storage::Sdp(buffer) = &self.body.storage else {
            return Err(Refused::Binding);
        };
        if funding.purpose() != purpose
            || !buffer.uses_intent(&intent)
            || !buffer.uses_native_history(funding)
        {
            return Err(Refused::Binding);
        }
        let Storage::Sdp(buffer) = std::mem::replace(
            &mut self.body.storage,
            Storage::Control([0; CONTROL_CIPHER]),
        ) else {
            unreachable!("checked SDP storage");
        };
        let original_cipher = self.body.cipher_bytes;
        let original_filled = self.filled;
        self.body.cipher_bytes = 16;
        self.body.plain_bytes = Some(0);
        self.filled = 16;
        match catcoms_app::VoiceSdpBody::from_buffer(buffer, range, intent) {
            Ok(body) => Ok(body),
            Err(refusal) => {
                let (error, buffer) = refusal.into_parts();
                self.body.storage = Storage::Sdp(buffer);
                self.body.cipher_bytes = original_cipher;
                self.body.plain_bytes = Some(plain);
                self.filled = original_filled;
                Err(if error == catcoms_sync::voice_v2::VoiceError::Capacity {
                    Refused::Capacity
                } else {
                    Refused::Binding
                })
            }
        }
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
            &worker.position.session.resources,
            worker.position.session.group,
        )?;
        Ok(PreparedReply {
            body,
            input: None,
            request,
            response,
            worker,
        })
    }
}
pub(super) struct PreparedReply {
    body: Body,
    input: Option<ReplyInput>,
    request: Header,
    response: Header,
    worker: HeaderWorker,
}
impl PreparedReply {
    /// Transfer actual native input/row custody, not a descriptor. Keep it through sealing,
    /// socket writing and completed-output collection; a lost browser reply cannot free it.
    pub(super) fn retain_input(
        mut self,
        input: super::custody::InputCustody,
        work: super::ledger::Work,
    ) -> Self {
        self.input = Some(ReplyInput {
            _input: input,
            _work: work,
        });
        self
    }
    pub(super) fn plain_mut(&mut self) -> &mut [u8] {
        let bytes = self.body.cipher_bytes - 16;
        &mut self.body.cipher_mut()[..bytes]
    }
    /// Consuming this owner makes encryption nonrepeatable under the issued physical nonce.
    pub(super) fn seal(mut self) -> Result<CompletedReply, Refused> {
        self.worker.position.session.keys.seal_response(
            &self.response,
            &self.request,
            self.body.cipher_mut(),
        )?;
        let admission = self
            .worker
            .position
            .session
            .keys
            .response_admission(&self.response, &self.request);
        Ok(CompletedReply {
            body: self.body,
            _input: self.input,
            admission,
            worker: self.worker,
        })
    }
}
/// Retain through the actual socket writer and completed-task output collection. Returning
/// readiness or dropping the caller's wait is not collection and cannot free this position.
pub(super) struct CompletedReply {
    body: Body,
    // Actual buffers/input/Work drain BEFORE the last physical HeaderWorker clears busy.
    _input: Option<ReplyInput>,
    admission: [u8; crypto::ADMISSION],
    worker: HeaderWorker,
}
struct ReplyInput {
    _input: super::custody::InputCustody,
    _work: super::ledger::Work,
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
pub(super) mod tests;
