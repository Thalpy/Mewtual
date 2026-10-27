//! Fixed admission history for three protected identities per incoming confirmation role.
//! Hello candidates and follow-up inboxes are independent; causal echoes are visible only
//! after opening. This is custody evidence,
//! independent of token consumption and successful confirmation. It owns no action, engine,
//! payload or Workflow; its conserved part of a Hello row can outlive all actual progress work.
use super::*;
use crate::voice_v2::runtime::EffectAuthority;
use crate::voice_v2::runtime::EndpointOrigin;
use catcoms_rt::{Clock, RequestAuthority};
use futures::FutureExt;

pub(in crate::voice_v2) struct ConfirmationConsent {
    pub room: CallLease,
    // Ownership, rather than an observed flag: keep the original directory allocation paid.
    pub _room_custody: Option<catcoms_rt::SharedRequestKeepalive>,
    pub effect: Arc<EffectAuthority>,
    pub clock: Arc<dyn Clock>,
}
use crate::voice_v2::{Body, EncryptedKind, SignedObject, CLOCK_SKEW_MS};

// Keep confirmation history independent of payload-bearing SDP variants. This receipt can
// contain only these three roles; retaining the complete encrypted-kind enum would charge
// every confirmation row for an unrelated SDP round and exceed its original fixed payment.
#[derive(Clone, Copy)]
enum ConfirmationRole {
    Hello,
    Proof,
    Finish,
}
impl ConfirmationRole {
    fn from_kind(kind: EncryptedKind) -> Result<Self, VoiceError> {
        match kind {
            EncryptedKind::ConfirmHello => Ok(Self::Hello),
            EncryptedKind::ConfirmProof => Ok(Self::Proof),
            EncryptedKind::ConfirmFinish => Ok(Self::Finish),
            _ => Err(VoiceError::Unauthorized),
        }
    }
    fn kind(self) -> EncryptedKind {
        match self {
            Self::Hello => EncryptedKind::ConfirmHello,
            Self::Proof => EncryptedKind::ConfirmProof,
            Self::Finish => EncryptedKind::ConfirmFinish,
        }
    }
}

/// Opaque identity of one admitted candidate under the original native call/pair/chain.
/// Repeating its packet recovers admission history, never another decrypt or confirmation grant.
pub(crate) struct ConfirmationAdmission {
    pub(super) binding: Arc<PlanBinding>,
    chain: CallLease,
    pub(crate) issuer: DeviceId,
    pub(crate) message: Nonce,
    pub(crate) hash: [u8; 32],
    pub(crate) sequence: u64,
    pub(crate) token: u64,
    kind: ConfirmationRole,
    candidate: u8,
    expires: u64,
    deadline: u64,
    consent: ConfirmationConsent,
    // At most three fixed identities per follow-up role. Immutable backward links avoid an
    // uncharged per-role container. No node owns an action, input, engine lender or Workflow.
    previous: Option<Arc<ConfirmationAdmission>>,
    pub(super) response: std::sync::OnceLock<Arc<StartAdmission>>,
    _credit: Credit,
}
redacted_debug!(ConfirmationAdmission);

// Include the actual fixed receipt AND its newly allocated effect owner, with both Arc
// counters, rounded to 32 bytes. Other consent handles retain already charged native owners.
// This is a structural payload/layout allowance, excluding allocator slack.
pub(super) const ADMISSION_RECORD_BYTES: usize = (std::mem::size_of::<ConfirmationAdmission>()
    + 2 * std::mem::size_of::<usize>()
    + std::mem::size_of::<EffectAuthority>()
    + 2 * std::mem::size_of::<usize>()
    + std::mem::size_of::<DeviceId>()
    + std::mem::size_of::<Nonce>()
    + std::mem::size_of::<Arc<ConfirmationAdmission>>()
    + 3 * std::mem::size_of::<usize>()
    + 64
    + 31)
    & !31;

impl ConfirmationAdmission {
    pub(in crate::voice_v2) fn kind(&self) -> EncryptedKind {
        self.kind.kind()
    }
    pub(crate) fn same_call(&self, call: &CallLease) -> bool {
        self.binding.call.is_same(call)
    }
    pub(crate) fn same_room(&self, room: &CallLease) -> bool {
        self.consent.room.is_same(room)
    }
    pub(crate) fn authorized_at(&self, epoch: u64, wall: u64, mono: u64) -> bool {
        epoch == self.binding.epoch
            && self.binding.live()
            && self.chain.is_live()
            && wall <= self.expires
            && mono < self.deadline
            && self.consent.room.is_live()
            && self.consent.effect.authorized_at(wall, mono)
    }
    pub(crate) fn candidate(&self) -> u8 {
        self.candidate
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.binding.epoch
    }
    pub(crate) fn authorized(&self) -> bool {
        let wall = self.consent.clock.now_ms();
        let mono = self.consent.clock.monotonic_ms();
        self.authorized_at(self.binding.epoch, wall, mono)
    }
    pub(crate) fn retains_identity(&self) -> bool {
        self.binding.call.is_live()
    }
    pub(crate) async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let waits = vec![
            self.binding.call.wait_revoked().boxed(),
            self.binding.pair.wait_revoked().boxed(),
            self.chain.wait_revoked().boxed(),
            self.consent.room.wait_revoked().boxed(),
            self.consent.effect.revoked().boxed(),
            self.consent
                .clock
                .sleep(std::time::Duration::from_millis(
                    self.deadline
                        .saturating_sub(self.consent.clock.monotonic_ms()),
                ))
                .boxed(),
        ];
        futures::future::select_all(waits).await;
    }
}

/// Fixed role columns, selected from the original native role rather than a packet descriptor.
fn column(low: bool, kind: EncryptedKind) -> Result<usize, VoiceError> {
    match (low, kind) {
        (false, EncryptedKind::ConfirmHello) => Ok(1),
        (true, EncryptedKind::ConfirmProof) => Ok(2),
        (false, EncryptedKind::ConfirmFinish) => Ok(3),
        _ => Err(VoiceError::Unauthorized),
    }
}
impl HistoryState {
    pub(super) fn incoming_at(
        &self,
        low: bool,
        kind: EncryptedKind,
        lane: usize,
    ) -> Result<Option<&Arc<ConfirmationAdmission>>, VoiceError> {
        match column(low, kind)? {
            1 => Ok(self.candidates.high()?.get(lane).and_then(Option::as_ref)),
            _ => Ok(self
                .followup_receipts()
                .find(|receipt| usize::from(receipt.candidate()) == lane)),
        }
    }
    fn followup_receipts(&self) -> impl Iterator<Item = &Arc<ConfirmationAdmission>> {
        std::iter::successors(self.followups.as_ref(), |receipt| receipt.previous.as_ref())
    }
    fn incoming_receipts(&self) -> impl Iterator<Item = &Arc<ConfirmationAdmission>> {
        let hello: &[Option<Arc<ConfirmationAdmission>>] = match &self.candidates {
            Candidates::High(rows) => rows,
            Candidates::Low(_) => &[],
        };
        hello.iter().flatten().chain(self.followup_receipts())
    }
}

impl PreparedChain {
    pub(in crate::voice_v2) fn workflow(&self) -> Result<Arc<Workflow>, VoiceError> {
        let control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if control.closed || !self.live.is_live() || !self.binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        control.workflow.clone().ok_or(VoiceError::Unauthorized)
    }

    /// Classify exact identity before replay/token preparation. A terminal rejected candidate
    /// stays spent, while a lost positive admission reply can recover without a live token.
    pub(in crate::voice_v2) fn find_confirmation(
        &self,
        object: &SignedObject,
    ) -> Result<Option<Arc<ConfirmationAdmission>>, VoiceError> {
        let state = self.history.state.lock().unwrap_or_else(|e| e.into_inner());
        for receipt in state.incoming_receipts() {
            if receipt.issuer == object.issuer() && receipt.message == object.id() {
                return if receipt.hash == object.hash()
                    && matches!(object.body(), Body::Encrypted { header, .. } if header.sequence == receipt.sequence)
                {
                    Ok(Some(receipt.clone()))
                } else {
                    Err(VoiceError::RetryMismatch)
                };
            }
        }
        Ok(None)
    }

    pub(in crate::voice_v2) fn confirmation_token(
        &self,
        token: u64,
    ) -> Result<Arc<ConfirmationAdmission>, VoiceError> {
        let state = self
            .history
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let receipt = state
            .incoming_receipts()
            .find(|receipt| receipt.token == token)
            .cloned();
        receipt.ok_or(VoiceError::Unauthorized)
    }

    /// Prepare one new incoming role identity. The caller supplies only an actual unpublished
    /// ledger event/token. Publication is separate and consumes both row and candidate once.
    pub(in crate::voice_v2) fn prepare_confirmation(
        &self,
        object: &SignedObject,
        token: u64,
        consent: ConfirmationConsent,
    ) -> Result<(PreparedHistory, Arc<ConfirmationAdmission>, Arc<Workflow>), VoiceError> {
        let Body::Encrypted { header, .. } = object.body() else {
            return Err(VoiceError::Malformed);
        };
        let column = column(self.binding.low, header.kind)?;
        if object.scope() != self.binding.scope
            || object.issuer() != self.binding.peer
            || header.handshake != self.binding.handshake
            || header.target != self.binding.local
            || header.target_join != self.binding.local_join
            || header.source_join != self.binding.peer_join
            || header.target_key_ref != self.binding.local_key
            || header.source_key_ref != self.binding.peer_key
        {
            return Err(VoiceError::Unauthorized);
        }
        let workflow = self.workflow()?;
        let (lane, previous) = {
            let state = self.history.state.lock().unwrap_or_else(|e| e.into_inner());
            let lane = (0..3)
                .find(|lane| {
                    state.issued & (1 << (column + lane * 3)) == 0
                        && state
                            .incoming_at(self.binding.low, header.kind, *lane)
                            .expect("validated role")
                            .is_none()
                })
                .ok_or(VoiceError::RateLimited)?;
            (
                lane,
                if column == 1 {
                    None
                } else {
                    state.followups.clone()
                },
            )
        };
        let mut history =
            self.history
                .prepare_row(column + lane * 3, Key::Decrypt(token), &workflow)?;
        // One split of this previously unsplit fixed-history row. Payload is separately paid;
        // neither half can acquire Workflow custody or be partitioned again.
        let credit = history.record.split_retained(ADMISSION_RECORD_BYTES)?;
        let receipt = Arc::new(ConfirmationAdmission {
            binding: self.binding.clone(),
            chain: self.live.clone(),
            issuer: object.issuer(),
            message: object.id(),
            hash: object.hash(),
            sequence: header.sequence,
            token,
            kind: ConfirmationRole::from_kind(header.kind)?,
            candidate: lane as u8,
            expires: object.expires().saturating_add(CLOCK_SKEW_MS),
            deadline: self.binding.deadline,
            consent,
            previous,
            response: std::sync::OnceLock::new(),
            _credit: credit,
        });
        Ok((history, receipt, workflow))
    }
}

impl Workflow {
    pub(in crate::voice_v2) fn incoming_confirmation_slot(
        &self,
        resources: &Arc<Resources>,
        scope: Scope,
        call: &CallLease,
        slots: &crate::voice_v2::packets::PacketSlots,
        object: &SignedObject,
    ) -> Result<slots::Slot, VoiceError> {
        let Body::Encrypted { header, .. } = object.body() else {
            return Err(VoiceError::Malformed);
        };
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed
            || !self.live.is_live()
            || !self.binding.live()
            || !Arc::ptr_eq(resources, self.binding.resources())
            || scope != self.binding.scope
            || !call.is_same(&self.binding.call)
            || column(self.binding.low, header.kind).is_err()
            || object.issuer() != self.binding.peer
            || object.scope() != scope
            || header.handshake != self.binding.handshake
            || header.target != self.binding.local
            || header.target_join != self.binding.local_join
            || header.source_join != self.binding.peer_join
            || header.target_key_ref != self.binding.local_key
            || header.source_key_ref != self.binding.peer_key
        {
            return Err(VoiceError::Unauthorized);
        }
        state
            .packets
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?
            .confirmation_incoming_slot(slots)
    }
}

impl RowIssue {
    /// Borrow the outer unpublished owner. Refusal must drop it only AFTER Group/Policy return:
    /// RowIssue::drop reacquires History, so moving it into a refused origin closure deadlocks.
    pub(in crate::voice_v2) fn publish_confirmation(
        &mut self,
        receipt: &Arc<ConfirmationAdmission>,
        origin: EndpointOrigin<'_>,
        wall: u64,
        mono: u64,
        publish: impl FnOnce(),
    ) -> Result<(), VoiceError> {
        let mut state = match self.plan.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let lane = usize::from(receipt.candidate());
        let column = column(self.plan.binding.low, receipt.kind.kind())?;
        if lane >= 3
            || self.row != column + lane * 3
            || self.key != Key::Decrypt(receipt.token)
            || state.closed
            || (state.active_row != self.row as u8 || state.serial != self.ticket)
            || state
                .incoming_at(self.plan.binding.low, receipt.kind.kind(), lane)?
                .is_some()
            || !Arc::ptr_eq(&receipt.binding, &self.plan.binding)
            || !receipt.authorized_at(origin.epoch(), wall, mono)
        {
            return Err(VoiceError::Unauthorized);
        }
        origin.publish(|| {
            state.issued |= 1 << self.row;
            state.active_row = NO_ROW;
            if column == 1 {
                state.candidates.high_mut().expect("validated high role")[lane] =
                    Some(receipt.clone());
            } else {
                state.followups = Some(receipt.clone());
            }
            self.unpublished = false;
            publish();
        })
    }
}
