//! Fixed SDP history from the original authenticated Row. No input, SDK, grant or new
//! packet is recovered. The HTTP Control producer pays this synchronous snapshot graph.
use super::sdp_rows;
use super::transition::try_lock;
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use catcoms_sync::voice_v2::sdp::{SdpDelivery, SdpOpenResult};

/// Historical observations only. In particular Applied never grants a repeated RTC call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) enum DeliveryState {
    None,
    Prepared,
    Admitted,
    Applied,
}

/// Immutable metadata plus the SAME Root delivery observer. Keep the actual paid Row
/// through serialization; a copied RowId cannot keep a detached record funded. This holds
/// only that Row and its original parent, not the process Ledger and unrelated histories.
pub(in crate::voice_handoff) struct Snapshot {
    parent: RowId,
    purpose: NativeRowPurpose,
    status: Status,
    delivery: Option<Arc<SdpDelivery>>,
    _row: Arc<Row>,
}
impl Snapshot {
    pub(in crate::voice_handoff) fn row(&self) -> RowId {
        self._row.id
    }
    pub(in crate::voice_handoff) fn parent(&self) -> RowId {
        self.parent
    }
    pub(in crate::voice_handoff) fn purpose(&self) -> NativeRowPurpose {
        self.purpose
    }
    pub(in crate::voice_handoff) fn status(&self) -> &Status {
        &self.status
    }
    pub(in crate::voice_handoff) fn delivery(&self) -> Option<&Arc<SdpDelivery>> {
        self.delivery.as_ref()
    }
    pub(in crate::voice_handoff) fn delivery_state(&self) -> DeliveryState {
        match &self.delivery {
            Some(value) if value.was_applied() => DeliveryState::Applied,
            Some(value) if value.was_admitted() => DeliveryState::Admitted,
            Some(_) => DeliveryState::Prepared,
            None => DeliveryState::None,
        }
    }
}
fn action(state: &State) -> Result<Option<&Arc<CryptoAction>>, VoiceError> {
    match &state.body {
        Body::SdpPending(value) => Ok(value.published.as_ref()),
        Body::SdpLive(value) => Ok(Some(&value.action)),
        Body::SdpDrained(value) => Ok(value.action.as_ref()),
        _ => Err(VoiceError::Unauthorized),
    }
}
impl ResolvedRow {
    /// Caller has already resolved the exact native purpose/parent. Repeat actual Session
    /// provenance before any recovery and again after Root observation. History stays
    /// available after End; it is not a current SDK/engine/factory or delivery admission.
    pub(in crate::voice_handoff) fn sdp_history(
        &self,
        session: &Arc<SessionIdentity>,
    ) -> Result<Snapshot, VoiceError> {
        let (parent, purpose) = {
            let _index = self.transition_index()?;
            let state = try_lock(&self.row.state)?;
            let Purpose::Sdp { parent, purpose } = self.row.key.purpose else {
                return Err(VoiceError::Unauthorized);
            };
            if !self
                .row
                .key
                .parent
                .as_ref()
                .is_some_and(|row| row.id == parent)
                || !sdp_rows::session(&state).is_some_and(|original| Arc::ptr_eq(original, session))
            {
                return Err(VoiceError::Unauthorized);
            }
            (parent, purpose)
        };
        self.recover_sdp_publication()?;
        let original = {
            let state = try_lock(&self.row.state)?;
            action(&state)?.cloned()
        };
        // Original Clock and Engine may invoke/contend with native callers. No native
        // Index/State/UI/Coordinator guard may cross these historical Root observations.
        let opened = if purpose == NativeRowPurpose::Open {
            original
                .as_ref()
                .map(|action| action.recover_sdp_open_history())
                .transpose()?
                .flatten()
        } else {
            None
        };
        let status = self.reconcile()?;
        {
            let _index = self.transition_index()?;
            let state = try_lock(&self.row.state)?;
            if !sdp_rows::session(&state).is_some_and(|value| Arc::ptr_eq(value, session)) {
                return Err(VoiceError::Unauthorized);
            }
            let current = action(&state)?;
            if !match (&original, current) {
                (Some(first), Some(now)) => Arc::ptr_eq(first, now),
                (None, None) => true,
                _ => false,
            } {
                return Err(VoiceError::Capacity);
            } // publication raced the snapshot; retry history
            if status.action
                != original
                    .as_ref()
                    .map(|value| (value.serial(), value.version(), value.input_tag()))
            {
                return Err(VoiceError::Capacity);
            }
            if purpose == NativeRowPurpose::Open {
                match (&opened, &status.engine.receipt) {
                    (Some(opened), Some(receipt)) if opened.receipt.same_transition(receipt) => {}
                    (None, None) => {}
                    // A terminal appeared between the two observations. No absence or
                    // invented result may replace it; a repeat reads the SAME terminal.
                    (None, Some(_)) => return Err(VoiceError::Capacity),
                    _ => return Err(VoiceError::RetryMismatch),
                }
            }
            if status.drained && sdp_rows::locally_drained(&state) != Some(true) {
                return Err(VoiceError::Capacity);
            }
        }
        // Actual Result/Action owners, including last delivery references, stay outside
        // native guards on every success/refusal. No allocation or fresh observer is minted.
        Ok(Snapshot {
            parent,
            purpose,
            status,
            delivery: opened.and_then(|value| value.delivery),
            _row: self.row.clone(),
        })
    }
}
/// Synchronous history call-frame budget in addition to the enclosing HTTP future and
/// its220-byte reply. Persistent Action/Row/delivery allocations retain their ORIGINAL
/// fixed fees and are not charged again. No variable plaintext or engine appears here.
pub(in crate::voice_handoff) fn layout_bytes() -> usize {
    // Outer fixed locals can coexist with either nested Root observation or reconciliation.
    // Reuse the canonical reconciliation inventory: its receipt-merge and Observation
    // owners must not disappear from this proof when their implementation changes.
    let outer = std::mem::size_of::<Snapshot>()
        + std::mem::size_of::<Option<SdpOpenResult>>()
        + std::mem::size_of::<Option<Arc<CryptoAction>>>()
        + std::mem::size_of::<Status>()
        + 192;
    let root_history = std::mem::size_of::<Option<SdpOpenResult>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 128; // original Clock pair/borrows and terminal observation
    outer + reconciliation_layout_bytes().max(root_history)
}
