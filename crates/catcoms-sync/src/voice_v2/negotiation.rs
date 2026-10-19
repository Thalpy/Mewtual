//! Prepaid capacity for the first, deliberately narrow SDP exchange. This owner grants no
//! membership, pair, nonce, signature, replay or RTC authority. Native factories must install it
//! on a current confirmed LocalJoin and repeat their actual protocol checks at every commit.
//!
//! One local SDP, one remote SDP and at most two actual source flights fit together. A crossed
//! Offer must be refused before positive admission; this contract does not solve glare. The
//! phase quotas count retained terminal records and actual lower/engine/result owners too.

use super::resources::{ByteBudget, Credit, Reservation, ResourceKind, Resources, WorkLane};
use super::{CallLease, EncryptedKind, Scope, VoiceError, MAX_SDP_PLAINTEXT, ROUTE_METADATA_BYTES};
use catcoms_crypto::DeviceId;
use std::sync::{Arc, Mutex};

pub const EXCHANGE_BYTES: usize = 224 * 1024;
const METADATA_BYTES: usize = 4096;
const STAGES: usize = 8;
// Maximum policy- and OfferId-bound native encoding sizes. Composition regressions must compare
// these with actual encoded packets; changing the wire encoding requires revisiting this peak.
const CIPHERTEXT: usize = MAX_SDP_PLAINTEXT + 16;
pub const MAX_AAD: usize = 542;
const PACKET_BYTES: usize = 2 * CIPHERTEXT + 1340;
const SOURCE_BYTES: usize = CIPHERTEXT + 2154;
const LOWER_BYTES: usize = 4 * CIPHERTEXT + 8616 + ROUTE_METADATA_BYTES;
const ACTION_BYTES: usize = CIPHERTEXT + MAX_AAD + 1536;
const ENGINE_BYTES: usize = 4 * (CIPHERTEXT + MAX_AAD) + 3 * MAX_SDP_PLAINTEXT + 2048;
const RESULT_BYTES: usize = MAX_SDP_PLAINTEXT + 1040;

/// Native allocation purpose. Callers cannot replace one phase with another to escape the
/// record limits: each consuming factory selects its own stage, independently of IPC input.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    LocalPacket,
    RemotePacket,
    Source,
    Lower,
    EncryptAction,
    DecryptAction,
    Result,
    Crypto,
}
impl Stage {
    pub(crate) fn resource_kind(self) -> ResourceKind {
        self.contract().3
    }
    fn contract(self) -> (usize, usize, usize, ResourceKind) {
        match self {
            Self::LocalPacket => (0, 1, PACKET_BYTES, ResourceKind::Retained),
            Self::RemotePacket => (1, 1, PACKET_BYTES, ResourceKind::Retained),
            Self::Source => (2, 1, SOURCE_BYTES, ResourceKind::Retained),
            Self::Lower => (3, 2, LOWER_BYTES, ResourceKind::Forward),
            Self::EncryptAction => (4, 1, ACTION_BYTES, ResourceKind::Retained),
            Self::DecryptAction => (5, 1, ACTION_BYTES, ResourceKind::Retained),
            Self::Result => (6, 1, RESULT_BYTES, ResourceKind::Retained),
            Self::Crypto => (7, 1, ENGINE_BYTES, ResourceKind::Crypto),
        }
    }
}
struct State {
    closed: bool,
    used: [usize; STAGES],
    // Issuance is irreversible at publication. Actual input occupancy can drain without
    // permitting a second seal/open identity to steal the opposite direction's reserved row.
    action_preparing: [bool; 2],
    action_issued: [bool; 2],
}
pub(super) struct Binding {
    pub scope: Scope,
    pub local: DeviceId,
    pub join: [u8; 32],
    pub key_ref: [u8; 32],
    pub peer: super::packets::PairBinding,
    pub call: CallLease,
    pub pair: CallLease,
    pub helpers: Vec<DeviceId>,
}
pub struct Exchange {
    resources: Arc<Resources>,
    group: [u8; 32],
    reserve: Arc<Reservation>,
    lane: Arc<WorkLane>,
    lower_lanes: [Arc<WorkLane>; 2],
    state: Mutex<State>,
    binding: Option<Binding>,
    live: CallLease,
    _metadata: Credit,
    action_slots: Arc<super::crypto_actions::slots::ReservedSlots>,
}
struct PhaseLoan {
    exchange: Arc<Exchange>,
    index: usize,
}
/// Affine direction reservation. Only unpublished abandonment returns this identity; neither
/// input compaction nor actual engine settlement makes a published action eligible again.
pub(super) struct ActionIssue {
    exchange: Arc<Exchange>,
    direction: usize,
    unpublished: bool,
}
impl ActionIssue {
    pub(super) fn publish(mut self) {
        let mut state = self
            .exchange
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            state.action_preparing[self.direction],
            "prepared action direction"
        );
        assert!(
            !state.action_issued[self.direction],
            "one issued action per direction"
        );
        state.action_preparing[self.direction] = false;
        state.action_issued[self.direction] = true;
        self.unpublished = false;
    }
}
impl Drop for ActionIssue {
    fn drop(&mut self) {
        if self.unpublished {
            self.exchange
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .action_preparing[self.direction] = false;
        }
    }
}
redacted_debug!(Exchange, Stage, PhaseLoan, SdpResult);
impl Drop for PhaseLoan {
    fn drop(&mut self) {
        let mut state = self
            .exchange
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(state.used[self.index] > 0, "actual native phase custody");
        state.used[self.index] -= 1;
    }
}
impl Exchange {
    pub(crate) fn allows_helper(&self, helper: DeviceId) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|binding| binding.helpers.contains(&helper))
    }
    pub(super) fn same_helpers(&self, helpers: &[DeviceId]) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|binding| binding.helpers == helpers)
    }
    pub(crate) fn uses_resources(&self, resources: &Arc<Resources>, group: [u8; 32]) -> bool {
        Arc::ptr_eq(&self.resources, resources) && self.group == group
    }
    /// Capacity for the registered native action, including its retained terminal input/receipt.
    /// It grants no cryptographic or protocol authority and cannot substitute for registration.
    pub fn encryption_budget(self: &Arc<Self>) -> ByteBudget {
        ByteBudget::Exchange {
            exchange: self.clone(),
            stage: Stage::EncryptAction,
        }
    }
    pub fn decryption_budget(self: &Arc<Self>) -> ByteBudget {
        ByteBudget::Exchange {
            exchange: self.clone(),
            stage: Stage::DecryptAction,
        }
    }
    /// Retain one bounded native result. Dispatch still needs an exact current native grant.
    pub fn retain_result(self: &Arc<Self>, plaintext: &[u8]) -> Result<SdpResult, VoiceError> {
        SdpResult::new(self, plaintext)
    }
    pub(super) fn new_bound(
        resources: Arc<Resources>,
        binding: Binding,
        slots: Arc<super::crypto_actions::slots::ReservedSlots>,
    ) -> Result<Arc<Self>, VoiceError> {
        let group = binding.scope.group;
        let exchange = Self::new_inner(resources.clone(), group, Some(binding), slots)?;
        resources.register_exchange(group, &exchange);
        Ok(exchange)
    }
    /// The caller must reach a native readiness boundary before requesting this capacity.
    /// Existing confirmation/presence/room/pair protection remains charged outside this owner.
    /// Refusal therefore precedes the first outgoing SDP nonce or incoming positive admission.
    #[cfg(test)]
    fn new(resources: Arc<Resources>, group: [u8; 32]) -> Result<Arc<Self>, VoiceError> {
        Self::new_for_test(
            resources,
            group,
            Arc::new(super::crypto_actions::slots::Slots::default()),
        )
    }
    #[cfg(test)]
    pub(super) fn new_for_test(
        resources: Arc<Resources>,
        group: [u8; 32],
        slots: Arc<super::crypto_actions::slots::Slots>,
    ) -> Result<Arc<Self>, VoiceError> {
        Self::new_inner(resources, group, None, slots.reserve(2)?)
    }
    fn new_inner(
        resources: Arc<Resources>,
        group: [u8; 32],
        binding: Option<Binding>,
        action_slots: Arc<super::crypto_actions::slots::ReservedSlots>,
    ) -> Result<Arc<Self>, VoiceError> {
        let reserve = resources.reserve(group, EXCHANGE_BYTES)?;
        let lane = resources.reserve_crypto_lane(group)?;
        let lower_lanes = [
            resources.reserve_forward_lane(group)?,
            resources.reserve_forward_lane(group)?,
        ];
        let metadata = ByteBudget::Reserved(reserve.clone()).charge(
            &resources,
            group,
            METADATA_BYTES,
            ResourceKind::Retained,
        )?;
        Ok(Arc::new(Self {
            resources,
            group,
            reserve,
            lane,
            lower_lanes,
            state: Mutex::new(State {
                closed: false,
                used: [0; STAGES],
                action_preparing: [false; 2],
                action_issued: [false; 2],
            }),
            binding,
            live: CallLease::default(),
            _metadata: metadata,
            action_slots,
        }))
    }
    pub(crate) fn action_slot(
        &self,
        expected: &Arc<super::crypto_actions::slots::Slots>,
    ) -> Result<super::crypto_actions::slots::Slot, VoiceError> {
        if !self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        self.action_slots.loan(expected)
    }
    pub(crate) fn is_live(&self) -> bool {
        self.live.is_live()
            && self
                .binding
                .as_ref()
                .is_none_or(|binding| binding.call.is_live() && binding.pair.is_live())
    }
    pub(crate) async fn revoked(&self) {
        use futures::FutureExt;
        if !self.is_live() {
            return;
        }
        let mut waits = vec![self.live.wait_revoked().boxed()];
        if let Some(binding) = &self.binding {
            waits.push(binding.call.wait_revoked().boxed());
            waits.push(binding.pair.wait_revoked().boxed());
        }
        futures::future::select_all(waits).await;
    }
    /// A bounded untrusted peek may select an EXISTING loan, never create an owner or validate
    /// a packet. Requester/process debits, signatures and current endpoint checks still run.
    pub(crate) fn lends_verification(&self, requester: DeviceId, frame: &[u8]) -> bool {
        let Some(binding) = &self.binding else {
            return false;
        };
        if !self.is_live() {
            return false;
        }
        let Ok(peek) = super::Envelope::peek_encrypted(frame) else {
            return false;
        };
        let header = &peek.header;
        peek.scope == binding.scope
            && peek.issuer == binding.peer.peer
            && header.target == binding.local
            && header.source_join == binding.peer.join
            && header.target_join == binding.join
            && header.source_key_ref == binding.peer.key_ref
            && header.target_key_ref == binding.key_ref
            && header.handshake == binding.peer.handshake
            && matches!(header.kind, EncryptedKind::Offer | EncryptedKind::Answer)
            && ((peek.envelope_kind == 1 && requester == peek.issuer)
                || (peek.envelope_kind == 3 && binding.helpers.contains(&requester)))
    }
    /// No actor may await a busy phase. Its bounded completion/input keeps actual custody and
    /// can retry fairly, with the original deadline and no second nonce/origin/attempt debit.
    pub(crate) fn charge(
        self: &Arc<Self>,
        stage: Stage,
        bytes: usize,
    ) -> Result<Credit, VoiceError> {
        // Registered actions require the affine publication permit below. A generic phase
        // loan would re-arm after input retirement and bypass the one-action direction limit.
        if matches!(stage, Stage::EncryptAction | Stage::DecryptAction) {
            return Err(VoiceError::Scope);
        }
        let (index, maximum_count, maximum_bytes, kind) = stage.contract();
        if bytes == 0 || bytes > maximum_bytes {
            return Err(VoiceError::Bounds);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !self.is_live() || state.used[index] >= maximum_count {
            return Err(VoiceError::Capacity);
        }
        // Source scratch exists only before its first source record is published. Its lower
        // flights therefore cannot coexist with that scratch; exact retries need no scratch.
        if stage == Stage::Source && state.used[3] != 0 {
            return Err(VoiceError::Capacity);
        }
        let credit = if kind == ResourceKind::Forward {
            let mut found = Err(VoiceError::Capacity);
            for lane in &self.lower_lanes {
                found = ByteBudget::ReservedLane {
                    reservation: self.reserve.clone(),
                    lane: lane.clone(),
                }
                .charge(&self.resources, self.group, bytes, kind);
                if !matches!(found, Err(VoiceError::Capacity)) {
                    break;
                }
            }
            found?
        } else {
            ByteBudget::ReservedLane {
                reservation: self.reserve.clone(),
                lane: self.lane.clone(),
            }
            .charge(&self.resources, self.group, bytes, kind)?
        };
        state.used[index] += 1;
        Ok(credit.with_custody(Arc::new(PhaseLoan {
            exchange: self.clone(),
            index,
        })))
    }
    /// Prepare a fixed history row and variable input under one bounded action identity.
    /// History retains only its exact reservation bytes; input alone owns live phase custody.
    /// Every fallible charge precedes attaching that custody, so refusal cannot run its
    /// destructor while holding the exchange accounting fence.
    pub(super) fn prepare_action_credits(
        self: &Arc<Self>,
        stage: Stage,
        history_bytes: usize,
        input_bytes: usize,
    ) -> Result<(Credit, Credit, ActionIssue), VoiceError> {
        let direction = match stage {
            Stage::EncryptAction => 0,
            Stage::DecryptAction => 1,
            _ => return Err(VoiceError::Scope),
        };
        let total = history_bytes
            .checked_add(input_bytes)
            .ok_or(VoiceError::Bounds)?;
        let (index, _, maximum_bytes, _) = stage.contract();
        if history_bytes == 0 || input_bytes == 0 || total > maximum_bytes {
            return Err(VoiceError::Bounds);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed
            || !self.is_live()
            || state.action_preparing[direction]
            || state.action_issued[direction]
            || state.used[index] != 0
        {
            return Err(VoiceError::Capacity);
        }
        let budget = ByteBudget::Reserved(self.reserve.clone());
        let history = budget.charge(
            &self.resources,
            self.group,
            history_bytes,
            ResourceKind::Retained,
        )?;
        let input = budget.charge(
            &self.resources,
            self.group,
            input_bytes,
            ResourceKind::Retained,
        )?;
        state.action_preparing[direction] = true;
        state.used[index] = 1;
        drop(state);
        let input = input.with_custody(Arc::new(PhaseLoan {
            exchange: self.clone(),
            index,
        }));
        Ok((
            history,
            input,
            ActionIssue {
                exchange: self.clone(),
                direction,
                unpublished: true,
            },
        ))
    }
    /// Retire effects separately through their actual protocol leases. This releases unused
    /// capacity only. Actual stage credits keep both this owner and their retired lane alive.
    pub(crate) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return;
        }
        state.closed = true;
        self.live.revoke();
        self.reserve.close();
        self.action_slots.close();
        self.lane.close();
        for lane in &self.lower_lanes {
            lane.close();
        }
    }
}
impl Drop for Exchange {
    fn drop(&mut self) {
        self.close();
    }
}

/// One native plaintext result plus its eventual dispatch payload. An adapter may borrow it
/// while holding this owner; exporting further owned/string copies needs separately charged
/// custody. The eventual RTC dispatcher must also revalidate the actual pair and OfferId.
pub struct SdpResult {
    bytes: Vec<u8>,
    _credit: Credit,
}
impl SdpResult {
    pub(crate) fn new(exchange: &Arc<Exchange>, plaintext: &[u8]) -> Result<Self, VoiceError> {
        if plaintext.len() > MAX_SDP_PLAINTEXT || std::str::from_utf8(plaintext).is_err() {
            return Err(VoiceError::Bounds);
        }
        let credit = exchange.charge(Stage::Result, plaintext.len() + 1040)?;
        Ok(Self {
            bytes: plaintext.to_vec(),
            _credit: credit,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declared_maximum_exchange_fits_with_two_actual_lowers_and_retained_action_inputs() {
        let resources = Arc::new(Resources::default());
        let exchange = Exchange::new(resources.clone(), [1; 32]).unwrap();
        let ordinary = resources
            .charge([1; 32], 32 * 1024, ResourceKind::Retained)
            .unwrap();
        let outgoing = exchange.charge(Stage::LocalPacket, PACKET_BYTES).unwrap();
        let (encrypt_history, encrypt_input, encrypt_issue) = exchange
            .prepare_action_credits(Stage::EncryptAction, 1536, ACTION_BYTES - 1536)
            .unwrap();
        encrypt_issue.publish();
        let engine = exchange.charge(Stage::Crypto, ENGINE_BYTES).unwrap();
        assert!(exchange.charge(Stage::Crypto, 1).is_err());
        drop(engine);
        let source_scratch = exchange
            .charge(Stage::Crypto, 2 * (CIPHERTEXT + 1024) + 4096)
            .unwrap();
        let source = exchange.charge(Stage::Source, SOURCE_BYTES).unwrap();
        drop(source_scratch);
        let lower1 = Arc::new(exchange.charge(Stage::Lower, LOWER_BYTES).unwrap());
        let lower2 = exchange.charge(Stage::Lower, LOWER_BYTES).unwrap();
        assert!(exchange.charge(Stage::Lower, 1).is_err());
        let verification = exchange
            .charge(Stage::Crypto, 5 * (CIPHERTEXT + 1020) + 4096)
            .unwrap();
        let incoming = exchange.charge(Stage::RemotePacket, PACKET_BYTES).unwrap();
        drop(verification);
        let (decrypt_history, decrypt_input, decrypt_issue) = exchange
            .prepare_action_credits(Stage::DecryptAction, 1536, ACTION_BYTES - 1536)
            .unwrap();
        decrypt_issue.publish();
        let engine = Arc::new(exchange.charge(Stage::Crypto, ENGINE_BYTES).unwrap());
        let result = SdpResult::new(&exchange, &vec![b'a'; MAX_SDP_PLAINTEXT]).unwrap();
        assert_eq!(result.bytes().len(), MAX_SDP_PLAINTEXT);
        assert_eq!(resources.counts(), (256 * 1024, 1));
        exchange.close();
        assert!(exchange.charge(Stage::Crypto, 1).is_err());
        assert_eq!(
            resources.counts().1,
            1,
            "actual engine still owns the retired lane"
        );
        drop(engine);
        assert_eq!(resources.counts().1, 0);
        let held = lower1.clone();
        drop((
            lower1,
            lower2,
            outgoing,
            source,
            incoming,
            encrypt_history,
            encrypt_input,
            decrypt_history,
            decrypt_input,
            result,
        ));
        let bytes = resources.counts().0;
        drop(held);
        assert_eq!(resources.counts().0, bytes - LOWER_BYTES);
        drop((exchange, ordinary));
        assert_eq!(resources.counts(), (0, 0));
    }
}
