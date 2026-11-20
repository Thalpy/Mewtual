//! First founder-only one-Create Append DATA. Real Root operational source/index/CAS/cold
//! selection remains separate. No caller HeaderSpec signer exists.
//! A valid action/math/signature supplies neither accepted absence nor a selected prefix.
//! Root MUST durably reserve ONE exact unsigned child intent for the selected parent BEFORE
//! invoking this private-key operation. CAS after signing alone permits equivocation: an
//! entered/ambiguous intent cannot be replaced. Native Stop/lock admission orders at that
//! irreversible boundary and cannot later claim the action was unaccepted. This DATA API
//! does not manufacture or validate the missing actual Store signing-intent receipt.
use super::ordinary_create::{self, V2CreateRefusal};
use super::*;
use catcoms_replication::chat::acceptance_tree::{Frontier, TreeContext};
use catcoms_replication::chat::owner_bodies::{
    AcceptanceRecord, AcceptanceSpec, AppendBody, AppendSpec,
};
use catcoms_replication::chat::owner_header::MAX_HEADER_BYTES;
use catcoms_replication::chat::owner_state::transition::{
    ReplayMath, UnsignedAppendStepData, REPLAY_MATH_PRIVATE_INVOICE, UNSIGNED_APPEND_DATA_INVOICE,
};
use catcoms_replication::chat::{Body, ChatError, SignedAction, MAX_GROUP_ID_BYTES};

/// NEW original operation buffers, independently paid outside writers. Neither original
/// source/SDK/current-math fee nor the separate Create DATA fee can be reused as credit.
pub const APPEND_DATA_BUFFER_BYTES: usize = 2 * 1024 * 1024;
mod materialized;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Planned,
    Sign,
    RawSigned,
    Complete,
    Failed,
}
struct AppendDataBody {
    original: OriginalV2GenesisSeedObservation,
    scope: OriginalInitializationObservation,
    context: BootstrapSourceContextData,
    parent: SignedOwnerHeader,
    parent_image: Vec<u8>,
    parent_frontier: Frontier,
    action: SignedAction,
    record: AcceptanceRecord,
    body: AppendBody,
    unsigned: UnsignedAppendStepData,
    raw: Option<[u8; 64]>,
    header: Option<SignedOwnerHeader>,
    stage: Stage,
    // LAST: the prepared plan's original identity; weak observations grant no authority.
    identity: Arc<()>,
}
pub struct PreparedV2AppendData {
    body: Box<AppendDataBody>,
}
pub struct V2AppendDataView<'a> {
    pub action: &'a SignedAction,
    pub record: &'a AcceptanceRecord,
    pub body: &'a AppendBody,
    pub header: &'a SignedOwnerHeader,
    pub image: &'a [u8],
}
/// Tiny exact-plan observation for the actual Root signing guard. It neither owns the
/// Prepared value nor borrows it, so the guard can coexist with its mutable raw-sign loan.
/// The host pays the weak tail; equal signed/intent labels never renew this identity.
pub struct V2AppendPlanObservation {
    identity: Weak<()>,
}
impl std::fmt::Debug for V2AppendPlanObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2AppendPlanObservation")
            .finish_non_exhaustive()
    }
}
impl V2AppendPlanObservation {
    pub fn matches_original_data(&self, prepared: &PreparedV2AppendData) -> bool {
        self.identity
            .upgrade()
            .is_some_and(|id| Arc::ptr_eq(&id, &prepared.body.identity))
    }
}

/// Inert existing DATA after the actual raw-signature cut, including failed/late-negative
/// preparations. No source, signer, Clock/RNG, SDK graph or positive permission escapes.
/// The real host keeps the SAME shared append payment through this and all residual tails.
pub struct DetachedV2AppendData {
    parent: SignedOwnerHeader,
    parent_image: Vec<u8>,
    action: SignedAction,
    record: AcceptanceRecord,
    body: AppendBody,
    unsigned: UnsignedAppendStepData,
    raw: [u8; 64],
    header: Option<SignedOwnerHeader>,
}
impl std::fmt::Debug for DetachedV2AppendData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetachedV2AppendData")
            .finish_non_exhaustive()
    }
}
/// Exact pre-sign DATA for Root's durable intent. Neither this loan/commitment nor an
/// unsigned body grants permission to sign. Root retains and checks its actual Saved token.
pub struct V2UnsignedAppendDataView<'a> {
    pub action: &'a SignedAction,
    pub record: &'a AcceptanceRecord,
    pub body: &'a AppendBody,
    pub signing_bytes: &'a [u8],
    pub image: &'a [u8],
    pub intent: [u8; 32],
}
impl std::fmt::Debug for V2UnsignedAppendDataView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2UnsignedAppendDataView")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PreparedV2AppendData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2AppendData")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for V2AppendDataView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2AppendDataView").finish_non_exhaustive()
    }
}
const LAYOUT: usize = std::mem::size_of::<AppendDataBody>()
    + std::mem::size_of::<V2AppendDataView<'static>>()
    + std::mem::size_of::<V2UnsignedAppendDataView<'static>>()
    + std::mem::size_of::<DetachedV2AppendData>()
    + std::mem::size_of::<V2AppendPlanObservation>()
    + 32
    + REPLAY_MATH_PRIVATE_INVOICE
    + UNSIGNED_APPEND_DATA_INVOICE
    + 4 * ordinary_create::MAX_CREATE_ACTION_BYTES
    + 4 * MAX_STATE_BYTES
    + 8 * MAX_HEADER_BYTES
    + 8 * catcoms_replication::chat::owner_bodies::MAX_APPEND_BODY_BYTES
    + 8 * catcoms_replication::chat::owner_bodies::MAX_ACCEPTANCE_RECORD_BYTES
    + 16 * MAX_GROUP_ID_BYTES
    + 16 * 1024;
const _: () = assert!(LAYOUT <= APPEND_DATA_BUFFER_BYTES);

fn parent<T, R>(
    node: &UnboundV2GenesisNode<T, R>,
    model: &ReplayMath,
) -> Result<(), V2CreateRefusal> {
    ordinary_create::source(node)?;
    let s = &node.seed.body;
    if model.profile() != &s.genesis.profile || model.slots().len() != 1 {
        return Err(V2CreateRefusal::Source);
    }
    let frame = StateFrame::decode(model.image()).map_err(|_| V2CreateRefusal::Source)?;
    frame
        .verify_header_binding(model.header())
        .map_err(|_| V2CreateRefusal::Source)?;
    let slot = model
        .slot(s.genesis.admission.id())
        .ok_or(V2CreateRefusal::Source)?;
    if frame.meta().mls_epoch != 0 || slot.image.subject != s.context.spec_data().signer {
        return Err(V2CreateRefusal::Source);
    }
    Ok(())
}
impl AppendDataBody {
    fn require<T, R>(
        &self,
        node: &UnboundV2GenesisNode<T, R>,
        model: &ReplayMath,
    ) -> Result<(), V2CreateRefusal> {
        ordinary_create::require_original(node, &self.original, &self.scope, &self.context)?;
        parent(node, model)?;
        let own = model
            .slot(node.seed.body.genesis.admission.id())
            .ok_or(V2CreateRefusal::Source)?;
        if model.header() != &self.parent
            || model.image() != self.parent_image.as_slice()
            || own.frontier != &self.parent_frontier
        {
            return Err(V2CreateRefusal::RequestMismatch);
        }
        Ok(())
    }
}
impl<T: MeshTransport, R: CryptoRngCore> UnboundV2GenesisNode<T, R> {
    /// Root MUST supply its actual complete-index result and selected mathematical parent
    /// before this call, and later exact CAS/covering save. Neither is inferred here.
    /// A changed parent needs a NEW explicitly paid append attempt; original A stays exact.
    pub fn prepare_create_append_data(
        &self,
        model: &ReplayMath,
        action: &SignedAction,
        slot: &mut Option<PreparedV2AppendData>,
    ) -> Result<(), V2CreateRefusal> {
        // Verify every supplied representation before cached retry handling, including an
        // invalid same-ActionId signature. There is no semantic-ID validity cache.
        if !action.verify() {
            return Err(V2CreateRefusal::Source);
        }
        if let Some(original) = slot {
            if &original.body.action != action {
                return Err(V2CreateRefusal::RequestMismatch);
            }
            original.body.require(self, model)?;
        } else {
            parent(self, model)?;
            // Core already equates Create parents to its optional reply. Keep this
            // initial profile explicit if that broader action grammar ever changes.
            if !action.spec().parents.is_empty()
                || !matches!(&action.spec().body, Body::Create { reply_to: None, .. })
            {
                return Err(V2CreateRefusal::Bound);
            }
            let s = &self.seed.body;
            s.genesis
                .admission
                .matches_action_delegation(action)
                .map_err(|_| V2CreateRefusal::Source)?;
            let context = TreeContext {
                profile: s.genesis.profile.id(),
                admission: s.genesis.admission.admission_id(),
            };
            let record = AcceptanceRecord::new(AcceptanceSpec {
                group: s.group.group_id_ref().to_vec(),
                context,
                channel: action.spec().realm.channel(),
                action: action.id(),
                subject: s.context.spec_data().signer,
                grant: action.spec().grant,
                basis: action.spec().basis,
                create: None,
            })
            .map_err(|_| V2CreateRefusal::Bound)?;
            let leaf = record.leaf();
            let own = model
                .slot(s.genesis.admission.id())
                .ok_or(V2CreateRefusal::Source)?;
            let new_tip = own
                .frontier
                .prepare_append(&[leaf])
                .map_err(|_| V2CreateRefusal::Bound)?
                .new_tip();
            let body = AppendBody::new(AppendSpec {
                group: s.group.group_id_ref().to_vec(),
                previous_tip: own.tip_header,
                old_tip: own.frontier.tip(),
                new_tip,
                leaves: vec![leaf],
            })
            .map_err(|_| V2CreateRefusal::Bound)?;
            // The plan derives EVERY result field from this model's own slots/frontier.
            // No supplied image/HeaderSpec chooses a field that the key custodian signs.
            let unsigned = model
                .prepare_append_data(&body)
                .map_err(|_| V2CreateRefusal::Source)?;
            let scope = s
                .metadata
                .scope
                .observe_private_initialization()
                .map_err(|_| V2CreateRefusal::Source)?;
            let mut image = Vec::with_capacity(MAX_STATE_BYTES);
            image.extend_from_slice(model.image());
            *slot = Some(PreparedV2AppendData {
                body: Box::new(AppendDataBody {
                    original: self.seed.observe_original_seed_data(),
                    scope,
                    context: s.context.clone(),
                    parent: model.header().clone(),
                    parent_image: image,
                    parent_frontier: own.frontier.clone(),
                    action: action.clone(),
                    record,
                    body,
                    unsigned,
                    raw: None,
                    header: None,
                    stage: Stage::Planned,
                    identity: Arc::new(()),
                }),
            });
        }
        let p = &slot.as_ref().ok_or(V2CreateRefusal::Terminal)?.body;
        p.require(self, model)?;
        if !matches!(p.stage, Stage::Planned | Stage::RawSigned | Stage::Complete) {
            return Err(V2CreateRefusal::Terminal);
        }
        if p.stage == Stage::Complete && (p.header.is_none() || p.raw.is_none()) {
            return Err(V2CreateRefusal::Terminal);
        }
        // Deliberately STOP before the private key. Root persists this exact unsigned
        // child intent and actual source/index/parent binding before the separate call.
        Ok(())
    }

    /// ONLY the bounded raw signature cut. Root must hold the actual Current/exact selected
    /// SavedI/native Stop/close admission, using an observation captured outside its writers.
    /// No Header encoding/verification, SDK snapshot, replay or candidate allocation occurs
    /// here. Only the signer's already-invoiced bounded signature/error scratch is allowed. The
    /// immutable original plan already passed full checks in preparation; this call compares
    /// its original seed/scope identity and reaches only that seed's actual private signer.
    pub fn sign_reserved_append_bytes_data(
        &self,
        original_plan: &V2AppendPlanObservation,
        prepared: &mut PreparedV2AppendData,
    ) -> Result<(), V2CreateRefusal> {
        if !original_plan.matches_original_data(prepared) {
            return Err(V2CreateRefusal::RequestMismatch);
        }
        let p = &mut prepared.body;
        p.require_sign_cut(self)?;
        if matches!(p.stage, Stage::RawSigned | Stage::Complete) {
            return p.raw.as_ref().map(|_| ()).ok_or(V2CreateRefusal::Terminal);
        }
        if p.stage != Stage::Planned || p.raw.is_some() {
            return Err(V2CreateRefusal::Terminal);
        }
        p.stage = Stage::Sign;
        #[cfg(test)]
        SIGNS.with(|count| count.set(count.get() + 1));
        p.raw = match self.seed.body.device.sign(p.unsigned.signing_bytes_data()) {
            Ok(raw) => Some(raw),
            Err(_) => {
                p.stage = Stage::Failed;
                return Err(V2CreateRefusal::Terminal);
            }
        };
        // Cache the actual raw64 IMMEDIATELY. No hook, late source observation, allocation
        // or signature verification may run before this owning slot. Failed after this cut
        // still permits consuming inert DATA handoff; no caller may ask for a new child.
        p.stage = Stage::RawSigned;
        #[cfg(test)]
        after_sign();
        if let Err(error) = p.require_sign_cut(self) {
            p.stage = Stage::Failed;
            return Err(error);
        }
        Ok(())
    }

    /// Deterministic materialization OUTSIDE writers. Uses the existing raw signature,
    /// never the key, then verifies its exact canonical Header. The actual Core prefix
    /// verifier derives the owning successor ONCE later; Sync does not derive/drop a copy.
    pub fn materialize_signed_append_data(
        &self,
        model: &ReplayMath,
        prepared: &mut PreparedV2AppendData,
    ) -> Result<(), V2CreateRefusal> {
        let p = &mut prepared.body;
        p.require(self, model)?;
        if p.stage == Stage::Complete {
            return p
                .header
                .as_ref()
                .map(|_| ())
                .ok_or(V2CreateRefusal::Terminal);
        }
        if p.stage != Stage::RawSigned {
            return Err(V2CreateRefusal::Terminal);
        }
        let raw = p.raw.ok_or(V2CreateRefusal::Terminal)?;
        let header =
            SignedOwnerHeader::sign_using(p.unsigned.header_spec_data().clone(), |bytes| {
                if bytes != p.unsigned.signing_bytes_data() {
                    return Err(ChatError::Dependency);
                }
                Ok(raw)
            })
            .map_err(|_| V2CreateRefusal::Source)?;
        p.header = Some(header);
        p.stage = Stage::Complete;
        #[cfg(test)]
        after_materialize();
        if let Err(error) = p.require(self, model) {
            p.stage = Stage::Failed;
            return Err(error);
        }
        Ok(())
    }
}
#[cfg(test)]
std::thread_local! {
    static SIGNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static AFTER_SIGN: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_MATERIALIZE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) fn reset_for_test() {
    SIGNS.with(|count| count.set(0));
    AFTER_SIGN.with(|h| {
        h.borrow_mut().take();
    });
    AFTER_MATERIALIZE.with(|h| {
        h.borrow_mut().take();
    });
}
#[cfg(test)]
pub(super) fn signs_for_test() -> usize {
    SIGNS.with(std::cell::Cell::get)
}
#[cfg(test)]
fn after_sign() {
    let h = AFTER_SIGN.with(|h| h.borrow_mut().take());
    if let Some(h) = h {
        h();
    }
}
#[cfg(test)]
fn after_materialize() {
    let hook = AFTER_MATERIALIZE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
pub(super) fn after_materialize_for_test(hook: Box<dyn FnOnce()>) {
    AFTER_MATERIALIZE.with(|slot| *slot.borrow_mut() = Some(hook));
}
#[cfg(test)]
pub(super) fn after_sign_for_test(h: Box<dyn FnOnce()>) {
    AFTER_SIGN.with(|slot| *slot.borrow_mut() = Some(h));
}
#[cfg(test)]
impl PreparedV2AppendData {
    pub(super) fn payload_presence_for_test(&self) -> (bool, bool) {
        (self.body.raw.is_some(), self.body.header.is_some())
    }
    pub(super) fn drop_header_for_test(&mut self) {
        self.body.header = None;
    }
}
impl PreparedV2AppendData {
    /// Borrow only an already cached raw signature from this EXACT original plan.
    /// This negative DATA loan works after a late failure/cancellation and performs no
    /// source, node, signing, materialization or allocation work. It grants no durable
    /// S selection or completed Header authority; the host must separately admit those
    /// effects. Keeping the whole Prepared value also keeps its original identity alive.
    pub fn borrow_existing_raw_signature_data(
        &self,
        original_plan: &V2AppendPlanObservation,
    ) -> Result<&[u8; 64], V2CreateRefusal> {
        if !original_plan.matches_original_data(self) {
            return Err(V2CreateRefusal::RequestMismatch);
        }
        self.body.raw.as_ref().ok_or(V2CreateRefusal::Terminal)
    }

    pub fn borrow_unsigned_data<'a, T, R>(
        &'a self,
        node: &'a UnboundV2GenesisNode<T, R>,
        model: &ReplayMath,
    ) -> Result<V2UnsignedAppendDataView<'a>, V2CreateRefusal> {
        self.body.require(node, model)?;
        if !matches!(
            self.body.stage,
            Stage::Planned | Stage::RawSigned | Stage::Complete
        ) {
            return Err(V2CreateRefusal::Terminal);
        }
        Ok(V2UnsignedAppendDataView {
            action: &self.body.action,
            record: &self.body.record,
            body: &self.body.body,
            signing_bytes: self.body.unsigned.signing_bytes_data(),
            image: self.body.unsigned.image_data(),
            intent: self.body.unsigned.intent_commitment_data(),
        })
    }
    pub fn borrow_data<'a, T, R>(
        &'a self,
        node: &'a UnboundV2GenesisNode<T, R>,
        model: &ReplayMath,
    ) -> Result<V2AppendDataView<'a>, V2CreateRefusal> {
        self.body.require(node, model)?;
        if self.body.stage != Stage::Complete {
            return Err(V2CreateRefusal::Terminal);
        }
        Ok(V2AppendDataView {
            action: &self.body.action,
            record: &self.body.record,
            body: &self.body.body,
            header: self.body.header.as_ref().ok_or(V2CreateRefusal::Terminal)?,
            image: self.body.unsigned.image_data(),
        })
    }

    pub fn observe_plan_data<T, R>(
        &self,
        node: &UnboundV2GenesisNode<T, R>,
        model: &ReplayMath,
    ) -> Result<V2AppendPlanObservation, V2CreateRefusal> {
        self.body.require(node, model)?;
        if !matches!(
            self.body.stage,
            Stage::Planned | Stage::RawSigned | Stage::Complete
        ) {
            return Err(V2CreateRefusal::Terminal);
        }
        Ok(V2AppendPlanObservation {
            identity: Arc::downgrade(&self.body.identity),
        })
    }

    /// Negative-only, one-time owning extraction. It invokes no source/key/export/time/RNG,
    /// signer or math callback, including after Failed/cancellation. A missing raw signature
    /// returns the SAME preparation unchanged; extraction never fabricates missing output.
    pub fn into_handoff_data(self) -> Result<DetachedV2AppendData, (Self, V2CreateRefusal)> {
        let Some(raw) = self.body.raw else {
            return Err((self, V2CreateRefusal::Terminal));
        };
        let p = *self.body;
        Ok(DetachedV2AppendData {
            parent: p.parent,
            parent_image: p.parent_image,
            action: p.action,
            record: p.record,
            body: p.body,
            unsigned: p.unsigned,
            raw,
            header: p.header,
        })
    }
}

impl AppendDataBody {
    fn require_sign_cut<T, R>(
        &self,
        node: &UnboundV2GenesisNode<T, R>,
    ) -> Result<(), V2CreateRefusal> {
        self.original
            .require_same_seed_data(&node.seed)
            .map_err(|_| V2CreateRefusal::Source)?;
        if self.context != node.seed.body.context
            || !self.scope.matches(&node.seed.body.metadata.scope)
        {
            return Err(V2CreateRefusal::Source);
        }
        Ok(())
    }
}
impl DetachedV2AppendData {
    pub fn parent_header_data(&self) -> &SignedOwnerHeader {
        &self.parent
    }
    pub fn parent_image_data(&self) -> &[u8] {
        &self.parent_image
    }
    pub fn raw_signature_data(&self) -> &[u8; 64] {
        &self.raw
    }
    pub fn header_data(&self) -> Option<&SignedOwnerHeader> {
        self.header.as_ref()
    }
    pub fn unsigned_data(&self) -> V2UnsignedAppendDataView<'_> {
        V2UnsignedAppendDataView {
            action: &self.action,
            record: &self.record,
            body: &self.body,
            signing_bytes: self.unsigned.signing_bytes_data(),
            image: self.unsigned.image_data(),
            intent: self.unsigned.intent_commitment_data(),
        }
    }
}
