//! Exact low-side start history. A lost local response must recover its original action;
//! an explicit next candidate names its predecessor instead of asking for an ambiguous retry.
//! These three records own metadata only, sharing the original fixed confirmation row budget.
use super::*;
use crate::voice_v2::runtime::{EffectAuthority, LocalOrigin};

pub(in crate::voice_v2) struct StartAdmission {
    binding: Arc<PlanBinding>,
    chain: CallLease,
    predecessor: Option<Nonce>,
    pub(in crate::voice_v2) message: Nonce,
    pub(in crate::voice_v2) action: u64,
    candidate: u8,
    expires: u64,
    consent: ConfirmationConsent,
    _credit: Credit,
}

pub(in crate::voice_v2) enum StartCandidate {
    Cached(Arc<StartAdmission>),
    New(u8),
}

// As for incoming Hello, include both allocations and their Arc counters plus conservative
// map/bookkeeping structure. No Workflow, input, engine lender or action is retained here.
pub(super) const START_RECORD_BYTES: usize = (std::mem::size_of::<StartAdmission>()
    + 2 * std::mem::size_of::<usize>()
    + std::mem::size_of::<EffectAuthority>()
    + 2 * std::mem::size_of::<usize>()
    + std::mem::size_of::<Nonce>()
    + std::mem::size_of::<Arc<StartAdmission>>()
    + 3 * std::mem::size_of::<usize>()
    + 64
    + 31)
    & !31;

impl StartAdmission {
    pub(in crate::voice_v2) fn same_room(&self, room: &CallLease) -> bool {
        self.consent.room.is_same(room)
    }

    pub(in crate::voice_v2) fn authorized_at(&self, epoch: u64, wall: u64, mono: u64) -> bool {
        epoch == self.binding.epoch
            && self.binding.live()
            && self.chain.is_live()
            && self.consent.room.is_live()
            && self.consent.effect.authorized_at(wall, mono)
            && wall <= self.expires.saturating_add(crate::voice_v2::CLOCK_SKEW_MS)
            && mono < self.binding.deadline
    }
}

impl Candidates {
    pub(super) fn low(&self) -> Result<&[Option<Arc<StartAdmission>>; 3], VoiceError> {
        match self {
            Self::Low(rows) => Ok(rows),
            Self::High(_) => Err(VoiceError::Unauthorized),
        }
    }
    fn low_mut(&mut self) -> Result<&mut [Option<Arc<StartAdmission>>; 3], VoiceError> {
        match self {
            Self::Low(rows) => Ok(rows),
            Self::High(_) => Err(VoiceError::Unauthorized),
        }
    }
}

impl PreparedChain {
    /// Recover an immutable response identity before consulting current nonce/attempt state.
    /// It never grants fresh execution; the retained action has its original irreversible gates.
    pub(in crate::voice_v2) fn cached_response(
        &self,
        trigger: &ConfirmationAdmission,
    ) -> Result<Option<Arc<StartAdmission>>, VoiceError> {
        response_column(self.binding.low, trigger.kind())?;
        let state = self.history.state.lock().unwrap_or_else(|e| e.into_inner());
        let lane = usize::from(trigger.candidate());
        if state.closed
            || !self.live.is_live()
            || !self.binding.live()
            || lane >= 3
            || !Arc::ptr_eq(&trigger.binding, &self.binding)
            || !state
                .incoming_at(self.binding.low, trigger.kind(), lane)?
                .is_some_and(|original| std::ptr::eq(original.as_ref(), trigger))
        {
            return Err(VoiceError::Unauthorized);
        }
        let receipt = trigger.response.get().cloned();
        if receipt
            .as_ref()
            .is_some_and(|receipt| receipt.predecessor != Some(trigger.message))
        {
            return Err(VoiceError::RetryMismatch);
        }
        Ok(receipt)
    }

    pub(in crate::voice_v2) fn prepare_response(
        &self,
        trigger: &ConfirmationAdmission,
        message: Nonce,
        action: u64,
        expires: u64,
        consent: ConfirmationConsent,
    ) -> Result<(PreparedHistory, Arc<StartAdmission>, Arc<Workflow>), VoiceError> {
        let column = response_column(self.binding.low, trigger.kind())?;
        if !trigger.same_room(&consent.room) {
            return Err(VoiceError::Unauthorized);
        }
        if self.cached_response(trigger)?.is_some() {
            return Err(VoiceError::RetryMismatch);
        }
        let workflow = self.workflow()?;
        let mut history = self.history.prepare_row(
            column + usize::from(trigger.candidate()) * 3,
            Key::Encrypt(self.binding.handshake, action, message),
            &workflow,
        )?;
        let credit = history.record.split_retained(START_RECORD_BYTES)?;
        let receipt = Arc::new(StartAdmission {
            binding: self.binding.clone(),
            chain: self.live.clone(),
            predecessor: Some(trigger.message),
            message,
            action,
            candidate: trigger.candidate(),
            expires,
            consent,
            _credit: credit,
        });
        Ok((history, receipt, workflow))
    }

    /// Classify BEFORE RNG, packet preparation, or row allocation. None always means the exact
    /// first start, even after later candidates exist. Some(id) always means the exact successor
    /// of that native-issued packet, so delayed duplicate requests cannot restart confirmation.
    pub(in crate::voice_v2) fn classify_start(
        &self,
        predecessor: Option<Nonce>,
    ) -> Result<StartCandidate, VoiceError> {
        let state = self
            .history
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed || !self.binding.low || !self.live.is_live() || !self.binding.live() {
            return Err(VoiceError::Unauthorized);
        }
        let rows = state.candidates.low()?;
        if let Some(receipt) = rows
            .iter()
            .flatten()
            .find(|row| row.predecessor == predecessor)
        {
            return Ok(StartCandidate::Cached(receipt.clone()));
        }
        let count = rows.iter().take_while(|row| row.is_some()).count();
        let expected = count
            .checked_sub(1)
            .map(|last| rows[last].as_ref().expect("prefix").message);
        if predecessor != expected {
            return Err(VoiceError::RetryMismatch);
        }
        if count == rows.len() {
            return Err(VoiceError::RateLimited);
        }
        if rows[count..].iter().any(Option::is_some) || state.issued & (1 << (1 + count * 3)) != 0 {
            return Err(VoiceError::Unauthorized);
        }
        Ok(StartCandidate::New(count as u8))
    }

    pub(in crate::voice_v2) fn prepare_start(
        &self,
        predecessor: Option<Nonce>,
        message: Nonce,
        action: u64,
        expires: u64,
        consent: ConfirmationConsent,
    ) -> Result<(PreparedHistory, Arc<StartAdmission>, Arc<Workflow>), VoiceError> {
        let StartCandidate::New(candidate) = self.classify_start(predecessor)? else {
            return Err(VoiceError::RetryMismatch);
        };
        let workflow = self.workflow()?;
        let key = Key::Encrypt(self.binding.handshake, action, message);
        let mut history =
            self.history
                .prepare_row(1 + usize::from(candidate) * 3, key, &workflow)?;
        let credit = history.record.split_retained(START_RECORD_BYTES)?;
        let receipt = Arc::new(StartAdmission {
            binding: self.binding.clone(),
            chain: self.live.clone(),
            predecessor,
            message,
            action,
            candidate,
            expires,
            consent,
            _credit: credit,
        });
        Ok((history, receipt, workflow))
    }
}

fn response_column(
    low: bool,
    trigger: crate::voice_v2::EncryptedKind,
) -> Result<usize, VoiceError> {
    use crate::voice_v2::EncryptedKind;
    match (low, trigger) {
        (false, EncryptedKind::ConfirmHello) => Ok(2),
        (true, EncryptedKind::ConfirmProof) => Ok(3),
        _ => Err(VoiceError::Unauthorized),
    }
}

impl RowIssue {
    pub(in crate::voice_v2) fn publish_response(
        &mut self,
        receipt: &Arc<StartAdmission>,
        trigger: &Arc<ConfirmationAdmission>,
        origin: LocalOrigin<'_>,
        time: (u64, u64),
        publish: impl FnOnce(),
    ) -> Result<(), VoiceError> {
        let mut state = match self.plan.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let lane = usize::from(trigger.candidate());
        let column = response_column(self.plan.binding.low, trigger.kind())?;
        if lane >= 3
            || receipt.candidate != trigger.candidate()
            || receipt.predecessor != Some(trigger.message)
            || self.row != column + lane * 3
            || self.key != Key::Encrypt(receipt.binding.handshake, receipt.action, receipt.message)
            || state.closed
            || state.active_row != self.row as u8
            || state.serial != self.ticket
            || trigger.response.get().is_some()
            || !Arc::ptr_eq(&receipt.binding, &self.plan.binding)
            || !Arc::ptr_eq(&trigger.binding, &self.plan.binding)
            || !trigger.same_room(&receipt.consent.room)
            || !state
                .incoming_at(self.plan.binding.low, trigger.kind(), lane)?
                .is_some_and(|original| Arc::ptr_eq(original, trigger))
            || !receipt.authorized_at(origin.epoch(), time.0, time.1)
        {
            return Err(VoiceError::Unauthorized);
        }
        origin.publish(|| {
            state.issued |= 1 << self.row;
            state.active_row = NO_ROW;
            // This is the sole writer, serialized by History. It never waits for another
            // initializer: no get_or_init or externally accessible setter exists.
            assert!(
                trigger.response.set(receipt.clone()).is_ok(),
                "one native response identity"
            );
            self.unpublished = false;
            publish();
        })
    }

    /// Borrow the outer issue: every refusal must destroy unpublished owners after Group and
    /// Policy are released. A moved RowIssue could otherwise reenter History during refusal.
    pub(in crate::voice_v2) fn publish_start(
        &mut self,
        receipt: &Arc<StartAdmission>,
        origin: LocalOrigin<'_>,
        wall: u64,
        mono: u64,
        publish: impl FnOnce(),
    ) -> Result<(), VoiceError> {
        let mut state = match self.plan.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let lane = usize::from(receipt.candidate);
        if lane >= 3
            || self.row != 1 + lane * 3
            || self.key != Key::Encrypt(receipt.binding.handshake, receipt.action, receipt.message)
            || state.closed
            || state.active_row != self.row as u8
            || state.serial != self.ticket
            || state.candidates.low()?[lane].is_some()
            || !Arc::ptr_eq(&receipt.binding, &self.plan.binding)
            || !receipt.authorized_at(origin.epoch(), wall, mono)
        {
            return Err(VoiceError::Unauthorized);
        }
        // The predecessor identity is rechecked while the actual History publication fence is
        // held. No result, candidate or nonce can be substituted between classify and apply.
        let expected = if lane == 0 {
            None
        } else {
            Some(
                state.candidates.low()?[lane - 1]
                    .as_ref()
                    .ok_or(VoiceError::Unauthorized)?
                    .message,
            )
        };
        if receipt.predecessor != expected {
            return Err(VoiceError::Unauthorized);
        }
        origin.publish(|| {
            state.issued |= 1 << self.row;
            state.active_row = NO_ROW;
            state.candidates.low_mut().expect("validated low role")[lane] = Some(receipt.clone());
            self.unpublished = false;
            publish();
        })
    }
}
