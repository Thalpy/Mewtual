//! One native-chosen first PC identity, nested in the original Session/key/storage.
//! Permanent intent history and the actual native worker flag are distinct. In-flight
//! Actor commands/completions also own their original phase Copy, even after waiter Drop.
use super::super::channel::{PhysicalHold, Session};
use super::super::requests::pc::Request;
use super::transition::try_lock;
use super::*;
use catcoms_sync::voice_v2::chain::{sdp_handoff, EstablishedKeyCustody};
use catcoms_sync::voice_v2::sdp::InitialSdpIntent;
use catcoms_sync::voice_v2::SdpRound;
use sha2::{Digest, Sha256};

pub(super) struct Claim {
    proof: [u8; 32],
    // Exclusion only. The permanent Claim survives a refused/abandoned local attempt.
    worker: bool,
    intent: Option<Arc<InitialSdpIntent>>,
}
#[derive(Clone, Copy)]
pub(in crate::voice_handoff) enum State {
    Preparing,
    Admitted {
        round: SdpRound,
        low: bool,
        remaining: u64,
    },
    Closed,
}
pub(in crate::voice_handoff) struct Receipt {
    pub proof: Option<[u8; 32]>,
    pub state: State,
}
fn storage<'a>(
    row: &ResolvedRow,
    state: &'a super::State,
    session: &Arc<SessionIdentity>,
    request: &Request,
) -> Result<&'a super::sdp::Claim, VoiceError> {
    let key = super::sdp::claim(row, state, session, &request.base)?;
    let storage = key.sdp.as_ref().ok_or(VoiceError::Unauthorized)?;
    if storage.proof != request.storage
        || storage
            .sdk
            .as_ref()
            .is_none_or(|sdk| !sdk.matches(request.sdk_generation, request.sdk))
    {
        return Err(VoiceError::Unauthorized);
    }
    Ok(storage)
}
fn mutable_storage(state: &mut super::State) -> &mut super::sdp::Claim {
    state
        .plan_mut()
        .unwrap()
        .transition
        .as_mut()
        .unwrap()
        .key
        .as_mut()
        .unwrap()
        .sdp
        .as_mut()
        .unwrap()
}
fn is_closed(storage: &super::sdp::Claim) -> bool {
    storage.closed
        || storage.sdk.as_ref().is_none_or(|sdk| sdk.released())
        || storage
            .phase
            .as_ref()
            .is_none_or(|phase| !phase.effect().is_live())
}
/// Original row owns the exclusion marker; phase owns each actual queued RPC contribution.
/// No root intent/Pair retains this Row or Phase, avoiding a whole-drain custody cycle.
pub(in crate::voice_handoff) struct Preparation {
    row: Arc<Row>,
    pub custody: EstablishedKeyCustody,
    pub phase: Arc<sdp_handoff::Phase>,
    candidate: Option<Arc<InitialSdpIntent>>,
}
impl Preparation {
    pub(in crate::voice_handoff) fn observe(&mut self, intent: &Arc<InitialSdpIntent>) {
        self.candidate = Some(intent.clone());
    }
}
impl Drop for Preparation {
    fn drop(&mut self) {
        drop(self.candidate.take());
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        mutable_storage(&mut state)
            .pc
            .as_mut()
            .expect("original PC intent")
            .worker = false;
        drop(state);
        self.row.changed.notify_waiters();
    }
}
impl Claim {
    pub(super) fn worker(&self) -> bool {
        self.worker
    }
}
impl ResolvedRow {
    /// Closed recovery is cleanup history only and remains available after lock/End.
    pub(in crate::voice_handoff) fn pc_early_receipt(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
    ) -> Result<Option<Receipt>, VoiceError> {
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        let storage = storage(self, &state, session, request)?;
        Ok(if is_closed(storage) {
            Some(Receipt {
                proof: storage.pc.as_ref().map(|pc| pc.proof),
                state: State::Closed,
            })
        } else {
            storage
                .pc
                .as_ref()
                .filter(|pc| pc.worker)
                .map(|pc| Receipt {
                    proof: Some(pc.proof),
                    state: State::Preparing,
                })
        })
    }
    pub(in crate::voice_handoff) fn begin_pc(
        &self,
        admission: &super::super::transition::Admission<'_>,
        physical: &PhysicalHold,
        session: &Arc<Session>,
        request: &Request,
    ) -> Result<Option<Preparation>, VoiceError> {
        if !physical.belongs_to(session)
            || !Arc::ptr_eq(session.identity(), admission.session())
            || !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let _index = self.transition_index()?;
        let mut state = try_lock(&self.row.state)?;
        let key = super::sdp::claim(self, &state, admission.session(), &request.base)?;
        let storage = storage(self, &state, admission.session(), request)?;
        if key.released
            || !key.custody.is_current()
            || is_closed(storage)
            || state.final_phase.is_some()
            || state.final_delivery().is_some()
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(pc) = &storage.pc {
            if pc.intent.is_some() {
                return Ok(None);
            }
            if pc.worker {
                return Err(VoiceError::Capacity);
            }
        }
        let custody = key.custody.clone();
        let phase = storage.phase.as_ref().ok_or(VoiceError::Capacity)?.clone();
        // Fixed proof depends only on original obligations; Ready becoming known after a
        // lost Actor reply never changes this identity or creates another generation.
        let mut hash = Sha256::new();
        hash.update(b"Mewtual native initial PC intent v1\0");
        hash.update(admission.session().0);
        hash.update(self.row.id.logical());
        hash.update(request.base.action.serial.to_be_bytes());
        hash.update(request.base.action.version.to_be_bytes());
        hash.update(request.base.action.input_tag);
        hash.update(request.base.handshake);
        hash.update(request.base.key);
        hash.update(request.base.sequence.to_be_bytes());
        hash.update(request.storage);
        hash.update(request.sdk_generation.to_be_bytes());
        hash.update(request.sdk);
        hash.update(1u64.to_be_bytes());
        let proof = hash.finalize().into();
        admission.current()?;
        let storage = mutable_storage(&mut state);
        if storage.pc.is_none() {
            storage.pc = Some(Claim {
                proof,
                worker: false,
                intent: None,
            });
        }
        storage.pc.as_mut().unwrap().worker = true;
        Ok(Some(Preparation {
            row: self.row.clone(),
            custody,
            phase,
            candidate: None,
        }))
    }
    /// Native clocks are sampled before the final lock chain. Admitted is a new bounded
    /// observation under current UI/selection/SDK/effect authority, never historical permission.
    pub(in crate::voice_handoff) fn pc_receipt(
        &self,
        admission: &super::super::transition::Admission<'_>,
        request: &Request,
        preparation: Option<&mut Preparation>,
        wall: u64,
        mono: u64,
    ) -> Result<Receipt, VoiceError> {
        if !Arc::ptr_eq(&self.ledger, admission.ledger())
            || !self
                .row
                .key
                .matches(admission.selected(), self.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let _index = self.transition_index()?;
        let mut state = try_lock(&self.row.state)?;
        let key = super::sdp::claim(self, &state, admission.session(), &request.base)?;
        let storage = storage(self, &state, admission.session(), request)?;
        let pc = storage.pc.as_ref().ok_or(VoiceError::Unauthorized)?;
        if key.released || !key.custody.is_current() || is_closed(storage) {
            return Ok(Receipt {
                proof: Some(pc.proof),
                state: State::Closed,
            });
        }
        if state.final_phase.is_some() || state.final_delivery().is_some() {
            return Err(VoiceError::Unauthorized);
        }
        let candidate = if let Some(preparation) = preparation.as_ref() {
            if !Arc::ptr_eq(&preparation.row, &self.row) || !pc.worker {
                return Err(VoiceError::Unauthorized);
            }
            preparation.candidate.as_ref().ok_or(VoiceError::Capacity)?
        } else {
            pc.intent.as_ref().ok_or(VoiceError::Capacity)?
        };
        let phase = storage.phase.as_ref().ok_or(VoiceError::Unauthorized)?;
        if candidate.generation() != 1 || !candidate.uses_phase(phase) {
            return Err(VoiceError::Unauthorized);
        }
        let remaining = candidate.remaining_at(wall, mono)?;
        let result = Receipt {
            proof: Some(pc.proof),
            state: State::Admitted {
                round: candidate.round(),
                low: candidate.local_is_low(),
                remaining,
            },
        };
        admission.current()?;
        if let Some(preparation) = preparation {
            mutable_storage(&mut state).pc.as_mut().unwrap().intent = preparation.candidate.take();
        }
        Ok(result)
    }
    /// Original authenticated obligations suffice to close even when the first reply and
    /// Root intent are unknown. Persisting Phase closure fences every late queued mint.
    pub(in crate::voice_handoff) fn close_pc(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
    ) -> Result<Receipt, VoiceError> {
        let (proof, close) = {
            let _index = self.transition_index()?;
            let mut state = try_lock(&self.row.state)?;
            let storage = storage(self, &state, session, request)?;
            let proof = storage.pc.as_ref().map(|pc| pc.proof);
            let close = mutable_storage(&mut state).request_close();
            (proof, close)
        };
        drop(close); // Actual credit/callback cleanup belongs outside native guards.
                     // Close is valid before PC admission too. No native PC identity existed in that
                     // ordering; the reply uses a distinct absent-identity variant, never a zero Ready.
        Ok(Receipt {
            proof,
            state: State::Closed,
        })
    }
}
#[cfg(test)]
impl ResolvedRow {
    /// Observe only the actual stored Root result after HTTP dispatch. This test seam
    /// creates no Claim/worker/effect and cannot make a browser Promise its own oracle.
    pub(in crate::voice_handoff) fn pc_test_intent(&self) -> Option<Arc<InitialSdpIntent>> {
        let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .plan()?
            .transition
            .as_ref()?
            .key
            .as_ref()?
            .sdp
            .as_ref()?
            .pc
            .as_ref()?
            .intent
            .clone()
    }
    /// Read-only original-key observation for a real Actor exact-recovery test; this
    /// creates neither a native claim/worker nor another Root key domain.
    pub(in crate::voice_handoff) fn pc_test_keys(
        &self,
        session: &Arc<SessionIdentity>,
        request: &Request,
    ) -> Result<EstablishedKeyCustody, VoiceError> {
        let _index = self.transition_index()?;
        let state = try_lock(&self.row.state)?;
        storage(self, &state, session, request)?;
        Ok(super::sdp::claim(self, &state, session, &request.base)?
            .custody
            .clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pc_fixed_graph_fits_its_share_of_the_original_phase_metadata() {
        let bytes = std::mem::size_of::<Claim>()
            + std::mem::size_of::<Preparation>()
            + std::mem::size_of::<Sha256>();
        assert!(
            bytes <= sdp_handoff::NATIVE_PC_ADAPTER_METADATA_BYTES,
            "native initial PC fixed graph {bytes}"
        );
    }
}
