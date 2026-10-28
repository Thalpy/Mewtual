//! Prepaid capacity for the first, deliberately narrow SDP exchange. This owner grants no
//! membership, pair, nonce, signature, replay or RTC authority. Native factories must install it
//! on a current confirmed LocalJoin and repeat their actual protocol checks at every commit.
//!
//! One local SDP, one remote SDP and at most two actual source flights fit together. A crossed
//! Offer must be refused before positive admission; this contract does not solve glare. The
//! phase quotas count retained terminal records and actual lower/engine/result owners too.

use super::resources::{
    ByteBudget, Credit, GroupForwardLanes, GroupProgressClaim, ProgressFunding, Reservation,
    ResourceKind, Resources, WorkLane,
};
use super::{CallLease, EncryptedKind, Scope, VoiceError, MAX_SDP_PLAINTEXT, ROUTE_METADATA_BYTES};
use catcoms_crypto::DeviceId;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

pub const EXCHANGE_BYTES: usize = 192 * 1024;
pub(super) const METADATA_BYTES: usize = 4096;
const STAGES: usize = 8;
// Maximum policy- and OfferId-bound native encoding sizes. Composition regressions must compare
// these with actual encoded packets; changing the wire encoding requires revisiting this peak.
const CIPHERTEXT: usize = MAX_SDP_PLAINTEXT + 16;
pub const MAX_AAD: usize = 579;
const PACKET_BYTES: usize = 2 * CIPHERTEXT + 1414;
// Canonical signed object plus Source's existing512-byte wire/identity allowance. Source
// metadata adds1024; Lower borrows that original object rather than cloning its ciphertext.
const SOURCE_WIRE_BYTES: usize = CIPHERTEXT + 1167;
const SOURCE_BYTES: usize = SOURCE_WIRE_BYTES + 1024;
const LOWER_BYTES: usize = 2 * SOURCE_WIRE_BYTES + 4096 + ROUTE_METADATA_BYTES;
const ACTION_BYTES: usize = CIPHERTEXT + MAX_AAD + 1536;
const ENGINE_BYTES: usize = 4 * (CIPHERTEXT + MAX_AAD) + 3 * MAX_SDP_PLAINTEXT + 2048;
const RESULT_BYTES: usize = MAX_SDP_PLAINTEXT + 1040;
pub(crate) mod admission;
pub(in crate::voice_v2) mod answer;
pub(in crate::voice_v2) mod initial_offer;
mod low_ready;
pub(crate) mod namespace;
mod packet_capacity;
mod ready_capacity;
pub(in crate::voice_v2) mod receive;
pub(in crate::voice_v2) use ready_capacity::PaidReadyCache;

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
    // One original Ready producer. Actual occupancy may drain, but issuance never rearms.
    ready_producer_issued: bool,
    // The original parents may initially have no unused positions. Once obsolete cache
    // rows return them, carve exactly1+1 children, then close surplus originals outside State.
    packet_parents: Option<super::packets::ConfirmationCapacity>,
    packet_claims: Option<super::packets::SdpPacketCapacity>,
    // Retired outside the phase fence; actual Lower borrowers keep original group custody.
    lower_lanes: Option<Arc<GroupForwardLanes>>,
    used: [usize; STAGES],
    // Issuance is irreversible at publication. Actual input occupancy can drain without
    // permitting a second seal/open identity to steal the opposite direction's reserved row.
    action_preparing: [bool; 2],
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
    state: Mutex<State>,
    // Permanent per-direction issuance. Publication must be pure under the eventual
    // Group/Policy tail; the affine issue itself remains owned outside those guards.
    action_issued: AtomicU8,
    binding: Option<Binding>,
    live: CallLease,
    action_slots: Arc<super::crypto_actions::slots::ReservedSlots>,
    // Pure observation: charge/action paths already hold State and must not lock it again.
    // Protected HIGH activates only after exact first-Offer token/event/history publication.
    activated: AtomicBool,
    // First admission fixes a bounded native deadline, capped by original key custody.
    active_until: std::sync::atomic::AtomicU64,
    successor: Option<super::chain::SuccessorProof>,
    // Present only on the protected affine path. Ordinary legacy SDP does not promise
    // causal Ready. Never store a Ready producer here: that owner retains Exchange one way.
    ready: Option<Arc<super::ready::capacity::ReadyCapacity>>,
    // Affine protected-capacity close right survives constructor unwinding/ticket loss.
    // No Workflow backedge: genuine old loans alone retain their old retirement witness.
    _capacity_closing: Option<super::chain::SdpCapacityClosing>,
    // Observers only. Actual inbox/canonical bytes remain outside Exchange to avoid cycles.
    ready_receive: std::sync::OnceLock<CallLease>,
    ready_publication: receive::PublicationCell,
    // One-way fixed receipt observation. Receipt owns NO Exchange/Workflow/progress credit.
    // It selects only exact Ready ACK recovery while generic SDP lenders remain inactive.
    low_ready: Option<Arc<super::ready::admission::ReadyAdmission>>,
    // Last: every enclosing owner/reservation/slot remains paid through field destruction.
    // All actual successor fields/credits precede this whole-plan drainage publisher;
    // the independent group progress claim follows it. Ready Source/Lower/ACK owners retain this Exchange
    // (one way, no cycle) or a separately counted lifetime owner through their real Drop.
    _successor_custody: Option<super::chain::SuccessorCustody>,
    // Last: a new same-group progression cannot enter until every preceding actual
    // capacity field and publisher has returned, including independently held Ready work.
    progress: GroupProgressClaim,
    // LAST: this Exchange's own4KiB fee stays paid through its potentially blocking
    // final claim tail. Protected and standalone paths use the same plain anchor type.
    _metadata: Arc<ProgressFunding>,
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
    binding_issued: bool,
}
impl ActionIssue {
    /// Affine fixed-record issuance, independent of eventual action publication. Even
    /// another internal caller cannot allocate multiple paid records from a borrowed
    /// direction issue. An abandoned whole preparation may retry the original capacity.
    pub(in crate::voice_v2) fn claim_sdp_binding(&mut self) -> Result<(), VoiceError> {
        if !self.unpublished || self.binding_issued {
            return Err(VoiceError::Unauthorized);
        }
        self.binding_issued = true;
        Ok(())
    }
    pub(in crate::voice_v2) fn covers_sdp_intent(
        &self,
        intent: &super::sdp::InitialSdpIntent,
        sealing: bool,
    ) -> bool {
        self.unpublished
            && self.direction == usize::from(!sealing)
            && self.exchange.live.is_live()
            && self.exchange.sdp_round() == Some(intent.round())
            && self
                .exchange
                .successor
                .as_ref()
                .is_some_and(|proof| intent.matches_successor(proof))
    }
    /// Pure issuance tail, not round/effect authorization. The caller MUST validate
    /// original authority immediately before this step and retain this SAME affine issue
    /// outside all final guards; its unpublished Drop may lock accounting State.
    pub(super) fn publish(&mut self) {
        assert!(
            self.unpublished,
            "one publication of this affine direction issue"
        );
        let bit = 1 << self.direction;
        let previous = self.exchange.action_issued.fetch_or(bit, Ordering::AcqRel);
        assert_eq!(previous & bit, 0, "one issued action per direction");
        // Preparing remains latched conservatively once published. The irreversible mask
        // rejects new issues even after payload occupancy and fixed history both drain.
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
    #[cfg(test)]
    pub(in crate::voice_v2) fn park_progress_release_for_test(
        &mut self,
        entered: Arc<std::sync::Barrier>,
        resume: Arc<std::sync::Barrier>,
    ) {
        self.progress.park_release_for_test(entered, resume);
    }
    /// Pure under the process registry lock; no Exchange/Ready/Workflow lock or callback.
    pub(in crate::voice_v2) fn progress_matches(
        &self,
        resources: &Resources,
        group: [u8; 32],
        key: usize,
    ) -> bool {
        self.progress.matches(resources, group, key)
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn ready_prepaid_for_test(&self) -> bool {
        self.ready.is_some()
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn capacity_identities_for_test(&self) -> [usize; 4] {
        let state = self.state.lock().unwrap();
        [
            Arc::as_ptr(&self.reserve) as usize,
            Arc::as_ptr(&self.lane) as usize,
            Arc::as_ptr(&self.action_slots) as usize,
            Arc::as_ptr(state.lower_lanes.as_ref().unwrap()) as usize,
        ]
    }
    /// Consume only the original affine ticket. No replacement lane, reservation or action
    /// position is acquired, even briefly. The high-side native holder retains this inert
    /// owner before any fallible registration/final fence, and never installs it in sdp yet.
    pub(in crate::voice_v2) fn from_transferred(
        ticket: super::chain::TransferredSdpCapacity,
        helpers: Vec<DeviceId>,
    ) -> Arc<Self> {
        Self::from_transferred_inner(ticket, helpers, None)
    }
    pub(in crate::voice_v2) fn from_low_transferred(
        ticket: super::chain::TransferredSdpCapacity,
        receipt: Arc<super::ready::admission::ReadyAdmission>,
    ) -> Arc<Self> {
        let helpers = receipt.helpers().collect();
        Self::from_transferred_inner(ticket, helpers, Some(receipt))
    }
    fn from_transferred_inner(
        ticket: super::chain::TransferredSdpCapacity,
        helpers: Vec<DeviceId>,
        low_ready: Option<Arc<super::ready::admission::ReadyAdmission>>,
    ) -> Arc<Self> {
        let super::chain::TransferredParts {
            // Metadata FIRST for reverse local-drop order during constructor unwinding:
            // the enclosing successor graph stays paid through the last claim tail.
            metadata,
            progress,
            custody,
            resources,
            reserve,
            lowers,
            slots,
            ready,
            packets,
            proof,
            lane,
            // Last binding drops FIRST on constructor unwind: close unused backing
            // before returning its metadata loan and before the last custody publisher.
            closing,
        } = ticket.into_parts();
        let original = proof.binding();
        let binding = Binding {
            scope: original.scope,
            local: original.local,
            join: original.local_join,
            key_ref: original.local_key,
            peer: super::packets::PairBinding {
                peer: original.peer,
                join: original.peer_join,
                key_ref: original.peer_key,
                handshake: original.handshake,
            },
            call: original.call.clone(),
            pair: original.pair.clone(),
            helpers,
        };
        Arc::new(Self {
            group: binding.scope.group,
            resources,
            reserve,
            lane,
            state: Mutex::new(State {
                closed: false,
                ready_producer_issued: false,
                packet_parents: Some(packets),
                packet_claims: None,
                lower_lanes: Some(lowers),
                used: [0; STAGES],
                action_preparing: [false; 2],
            }),
            action_issued: AtomicU8::new(0),
            binding: Some(binding),
            live: CallLease::default(),
            action_slots: slots,
            activated: AtomicBool::new(false),
            active_until: std::sync::atomic::AtomicU64::new(0),
            successor: Some(proof),
            ready: Some(ready),
            _capacity_closing: Some(closing),
            ready_receive: std::sync::OnceLock::new(),
            ready_publication: std::sync::OnceLock::new(),
            low_ready,
            _metadata: metadata,
            _successor_custody: Some(custody),
            progress,
        })
    }
    pub(in crate::voice_v2) fn successor_current_at(
        &self,
        binding: &super::chain::Binding,
        resources: &Arc<Resources>,
        mono: u64,
    ) -> bool {
        self.live.is_live()
            && self
                .successor
                .as_ref()
                .is_some_and(|proof| proof.matches(binding, resources) && proof.current_at(mono))
    }
    /// Quiet Pair housekeeping must call this even if registry publication was refused.
    /// There is deliberately no route from this observation to activation or SDP authority.
    pub(in crate::voice_v2) fn collect_pending_successor(&self, mono: u64) -> bool {
        let active = self.activated.load(Ordering::Acquire);
        let expired = self.successor.as_ref().is_some_and(|proof| {
            if active {
                !proof.live() || mono >= self.active_until.load(Ordering::Acquire)
            } else {
                !proof.current_at(mono)
                    || self
                        .low_ready
                        .as_ref()
                        .is_some_and(|receipt| mono >= receipt.deadline())
            }
        });
        if expired || !self.live.is_live() {
            self.close();
            return true;
        }
        false
    }
    pub(crate) fn uses_registry(&self, resources: &Resources, group: [u8; 32]) -> bool {
        std::ptr::eq(Arc::as_ptr(&self.resources), resources) && self.group == group
    }
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
    pub(in crate::voice_v2) fn helpers(&self) -> &[DeviceId] {
        self.binding
            .as_ref()
            .map_or(&[], |binding| &binding.helpers)
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
        resources.register_exchange(group, &exchange)?;
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
        let lower_lanes = resources.group_forward_lanes(group)?;
        let metadata = ByteBudget::Reserved(reserve.clone()).charge(
            &resources,
            group,
            METADATA_BYTES,
            ResourceKind::Retained,
        )?;
        // Standalone SDP must exclude an inert/active/protected chain too. A separate
        // weak Exchange index would allow the two paths to occupy one progress cell.
        let metadata = ProgressFunding::new(metadata);
        let progress = GroupProgressClaim::reserve(&resources, group, metadata.clone())?;
        Ok(Arc::new(Self {
            resources,
            group,
            reserve,
            lane,
            state: Mutex::new(State {
                closed: false,
                ready_producer_issued: false,
                packet_parents: None,
                packet_claims: None,
                lower_lanes: Some(lower_lanes),
                used: [0; STAGES],
                action_preparing: [false; 2],
            }),
            action_issued: AtomicU8::new(0),
            binding,
            live: CallLease::default(),
            _metadata: metadata,
            action_slots,
            activated: AtomicBool::new(true),
            active_until: std::sync::atomic::AtomicU64::new(0),
            successor: None,
            ready: None,
            _capacity_closing: None,
            ready_receive: std::sync::OnceLock::new(),
            ready_publication: std::sync::OnceLock::new(),
            low_ready: None,
            _successor_custody: None,
            progress,
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
        self.activated.load(Ordering::Acquire)
            && self.live.is_live()
            && self.successor.as_ref().is_none_or(|proof| proof.live())
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
            && header.kind.is_sdp()
            && self.accepts_sdp_kind(header.kind)
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
            state
                .lower_lanes
                .as_ref()
                .ok_or(VoiceError::Unauthorized)?
                .charge(&self.resources, self.group, &self.reserve, bytes)?
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
            || self.action_issued.load(Ordering::Acquire) & (1 << direction) != 0
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
                binding_issued: false,
            },
        ))
    }
    /// Retire effects separately through their actual protocol leases. This releases unused
    /// capacity only. Actual stage credits keep both this owner and their retired lane alive.
    pub(crate) fn close(&self) {
        let (lower_lanes, packet_parents, packet_claims) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return;
            }
            state.closed = true;
            (
                state.lower_lanes.take(),
                state.packet_parents.take(),
                state.packet_claims.take(),
            )
        };
        self.live.revoke();
        self.reserve.close();
        if let Some(ready) = &self.ready {
            ready.close();
        }
        self.action_slots.close();
        self.lane.close();
        if let Some(parents) = packet_parents {
            parents.close();
        }
        if let Some(claims) = packet_claims {
            claims.close();
        }
        // Never close another phase's capacity. Dropping this handle outside State releases
        // idle slots only when no live phase or actual Lower retains the original owner.
        drop(lower_lanes);
    }
}

/// Observation of one ORIGINAL inert native SDP successor. This exports no Exchange, budget,
/// activation method, packet factory, key or RTC authority. Retention still owns real capacity.
pub struct SdpSuccessor {
    pub(in crate::voice_v2) owner: Arc<Exchange>,
}
redacted_debug!(SdpSuccessor);
impl SdpSuccessor {
    pub fn confirmation_drained(&self) -> bool {
        self.owner
            .successor
            .as_ref()
            .is_some_and(|proof| proof.drained())
    }
    /// First immutable native Finish window; observing it never grants current authority.
    pub fn transition_until(&self) -> u64 {
        self.owner
            .successor
            .as_ref()
            .and_then(|proof| proof.until())
            .expect("native successor follows original Finish")
    }
}
impl Drop for Exchange {
    fn drop(&mut self) {
        let original = self as *const Self;
        self.close();
        self.resources.unregister_exchange(self.group, original);
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
    mod registry;
    #[test]
    fn action_issue_final_publication_does_not_lock_state_or_drop_its_owner() {
        let resources = Arc::new(Resources::default());
        let exchange = Exchange::new(resources.clone(), [1; 32]).unwrap();
        let (record, input, mut issue) = exchange
            .prepare_action_credits(Stage::EncryptAction, 1536, 128)
            .unwrap();
        fn publish_while_state_is_held(exchange: &Arc<Exchange>, issue: &mut ActionIssue) {
            let owners = Arc::strong_count(exchange);
            let state = exchange.state.lock().unwrap();
            std::thread::scope(|scope| {
                let (sent, received) = std::sync::mpsc::channel();
                let worker = scope.spawn(move || {
                    issue.publish();
                    sent.send(()).unwrap();
                });
                // This timeout only bounds a broken blocking implementation. The real
                // mutex, not elapsed time, controls the tested publication ordering.
                let result = received.recv_timeout(std::time::Duration::from_secs(5));
                assert_eq!(Arc::strong_count(exchange), owners);
                drop(state); // Release before join/panic, so regression cannot hang the suite.
                worker.join().unwrap();
                result.expect("final publication waited for the accounting mutex");
            });
        }
        let owners = Arc::strong_count(&exchange);
        publish_while_state_is_held(&exchange, &mut issue);
        assert_eq!(exchange.action_issued.load(Ordering::Acquire), 1);
        drop(issue);
        assert_eq!(Arc::strong_count(&exchange), owners - 1);
        drop((record, input));
        assert!(exchange
            .prepare_action_credits(Stage::EncryptAction, 1536, 128)
            .is_err());
        // The other direction retains its original independent opportunity.
        let (record, input, issue) = exchange
            .prepare_action_credits(Stage::DecryptAction, 1536, 128)
            .unwrap();
        drop((issue, input, record));
        let (record, input, mut issue) = exchange
            .prepare_action_credits(Stage::DecryptAction, 1536, 128)
            .unwrap();
        publish_while_state_is_held(&exchange, &mut issue);
        assert_eq!(exchange.action_issued.load(Ordering::Acquire), 3);
        drop((issue, input, record));
        assert!(exchange
            .prepare_action_credits(Stage::DecryptAction, 1536, 128)
            .is_err());
        drop(exchange);
        assert_eq!(resources.counts(), (0, 0));
    }

    #[test]
    fn successor_retained_rust_graph_fits_original_metadata_allowance() {
        use std::mem::size_of;
        // Structural growth guard for our owners, not allocator slack/Tokio watch heap/RSS.
        // Original PlanBinding and shared GroupForwardLanes have separate retained payments.
        let bytes = size_of::<Exchange>()
            + size_of::<Reservation>()
            + size_of::<WorkLane>()
            + size_of::<super::super::crypto_actions::slots::ReservedSlots>()
            + 2 * size_of::<super::super::crypto_actions::slots::ReservedSlots>()
            // Packet child reservations retain their two closed ORIGINAL parents even
            // after old Workflow metadata drains. Count those bodies/Arc headers here too.
            + 2 * size_of::<super::super::crypto_actions::slots::ReservedSlots>()
            + 2 * 2 * size_of::<usize>()
            + size_of::<super::super::LeaseState>()
            + size_of::<SdpSuccessor>()
            // The external receive graph consumes this ORIGINAL SDP metadata payment,
            // not an unaccounted addition to Ready's2KiB cache. Canonical input buffers
            // retain their full separate RemotePacket credit. Only one wrapper is live.
            + size_of::<receive::HighReceive>()
            + size_of::<Credit>()
            + size_of::<PhaseLoan>()
            + size_of::<super::super::LeaseState>()
            + super::super::packets::admission_metadata_bytes()
                .max(size_of::<receive::RetainedOffer>())
            + 4 * 2 * size_of::<usize>()
            + 2 * size_of::<DeviceId>()
            // Standalone SDP uses its ORIGINAL4KiB payment for the same progression
            // claim, without a Ready reserve. Conservative for protected successors too.
            + GroupProgressClaim::metadata_bytes()
            + 7 * 2 * size_of::<usize>();
        assert!(
            bytes < METADATA_BYTES,
            "successor native owner graph grew to {bytes}"
        );
    }
    #[test]
    fn four_closed_exchanges_release_idle_slots_while_the_last_real_lower_stays_charged() {
        let pool = Arc::new(Resources::default());
        let phases: Vec<_> = (1..=4)
            .map(|group| Exchange::new(pool.clone(), [group; 32]).unwrap())
            .collect();
        assert_eq!(
            pool.charge([5; 32], 1, ResourceKind::Forward).err(),
            Some(VoiceError::Capacity)
        );
        let late = Arc::new(phases[0].charge(Stage::Lower, 1).unwrap());
        for phase in &phases {
            phase.close();
        }
        let mut work: Vec<_> = (0..4)
            .map(|_| pool.charge([5; 32], 1, ResourceKind::Forward).unwrap())
            .collect();
        for _ in 0..2 {
            work.push(pool.charge([6; 32], 1, ResourceKind::Forward).unwrap());
        }
        assert_eq!(
            pool.charge([6; 32], 1, ResourceKind::Forward).err(),
            Some(VoiceError::Capacity)
        );
        let last = late.clone();
        drop(late);
        assert!(pool.charge([6; 32], 1, ResourceKind::Forward).is_err());
        drop(last);
        for _ in 0..2 {
            work.push(pool.charge([6; 32], 1, ResourceKind::Forward).unwrap());
        }
        drop((work, phases));
        assert_eq!(pool.counts(), (0, 0));
    }
    #[test]
    fn declared_maximum_exchange_fits_with_two_actual_lowers_and_retained_action_inputs() {
        let declared = METADATA_BYTES
            + 2 * PACKET_BYTES
            + SOURCE_BYTES
            + 2 * LOWER_BYTES
            + 2 * ACTION_BYTES
            + ENGINE_BYTES
            + RESULT_BYTES;
        assert_eq!(declared, 188_521);
        assert!(declared <= EXCHANGE_BYTES);
        let resources = Arc::new(Resources::default());
        let exchange = Exchange::new(resources.clone(), [1; 32]).unwrap();
        let ordinary = resources
            .charge(
                [1; 32],
                crate::voice_v2::resources::GROUP_BYTES
                    - EXCHANGE_BYTES
                    - super::super::resources::GROUP_FORWARD_METADATA_BYTES,
                ResourceKind::Retained,
            )
            .unwrap();
        let outgoing = exchange.charge(Stage::LocalPacket, PACKET_BYTES).unwrap();
        let (encrypt_history, encrypt_input, mut encrypt_issue) = exchange
            .prepare_action_credits(Stage::EncryptAction, 1536, ACTION_BYTES - 1536)
            .unwrap();
        encrypt_issue.publish();
        drop(encrypt_issue);
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
        let (decrypt_history, decrypt_input, mut decrypt_issue) = exchange
            .prepare_action_credits(Stage::DecryptAction, 1536, ACTION_BYTES - 1536)
            .unwrap();
        decrypt_issue.publish();
        drop(decrypt_issue);
        let engine = Arc::new(exchange.charge(Stage::Crypto, ENGINE_BYTES).unwrap());
        let result = SdpResult::new(&exchange, &vec![b'a'; MAX_SDP_PLAINTEXT]).unwrap();
        assert_eq!(result.bytes().len(), MAX_SDP_PLAINTEXT);
        assert_eq!(
            resources.counts(),
            (crate::voice_v2::resources::GROUP_BYTES, 1)
        );
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
        // The last actual Lower also retires the independently funded shared group owner;
        // closed Exchange handles intentionally retain neither idle slots nor that payment.
        assert_eq!(
            resources.counts().0,
            bytes - LOWER_BYTES - super::super::resources::GROUP_FORWARD_METADATA_BYTES
        );
        drop((exchange, ordinary));
        assert_eq!(resources.counts(), (0, 0));
    }
}
