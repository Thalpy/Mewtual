//! Ephemeral native voice ownership and process-wide WebCrypto charges. This is independent of
//! optional router mappings: losing mapping capacity must never select a protocol or own a call.
//! No keys, decrypted SDP/ICE or call state are persisted. Actual actor membership/room/policy
//! checks surround these synchronous transitions in the bridge, then repeat at protocol effects.

// The V2 action handoff has no production IPC consumer yet. Compile its registry only for
// contract tests until endpoint result recovery and the browser adapter are integrated.
#[cfg(test)]
use catcoms_sync::voice_v2::crypto_actions::{
    CryptoAction, CryptoResultReceipt, CryptoResultStatus,
};
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind, Resources};
use catcoms_sync::voice_v2::runtime::Runtime;
use catcoms_sync::voice_v2::{CallGrant, CallLease, Nonce, Scope};
use std::collections::HashMap;
use std::sync::Arc;

mod join_key;
pub(crate) use join_key::SelectedHandoffOwner;
pub(crate) use join_key::{
    AdmittedKeyCompletion, AdmittedKeyRetirement, AdmittedPublicationPlan, AdmittedPublicationWork,
    AdmittedPublishedKey,
};
pub(crate) use join_key::{JoinKeyAdmission, JoinKeyPublication, PublicationPlan};

const CANDIDATES: usize = 8;
const CANDIDATE_TTL: u64 = 30_000;
#[cfg(test)]
const REGISTERED_ACTIONS: usize = 32;

/// Combined pending/terminal history across native owners. The original prepaid action owns
/// its small Rust receipt; no serialized IPC/browser copy is covered by this row. Production
/// exposure still needs independently prepaid, finite reply custody before removing cfg(test).
#[cfg(test)]
struct RegisteredAction {
    action: Arc<CryptoAction>,
    receipt_acknowledged: bool,
}

/// Parsed closed action identity. The bridge must decode canonical decimal u64 strings and a
/// fixed hex fingerprint before constructing this; no JS payload can register a native action.
#[derive(Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct ActionIdentity {
    pub owner: String,
    pub action: u64,
    pub version: u64,
    pub input_tag: [u8; 32],
}
#[cfg(test)]
impl ActionIdentity {
    fn key(&self) -> Result<(String, u64), String> {
        if self.owner.is_empty() || self.owner.len() > 128 || self.action == 0 || self.version != 1
        {
            return Err("invalid voice action identity".into());
        }
        Ok((self.owner.clone(), self.action))
    }
}

/// One process task, owned by Tauri rather than an unlocked UI or server actor. It keeps only a
/// weak runtime reference and is cancelled when the native host drops. Runtime construction is
/// deliberately synchronous, including in tests and pre-runtime application setup.
pub(crate) struct CollectionTask {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}
impl CollectionTask {
    pub fn start(runtime: &Arc<Runtime>) -> Self {
        let (stop, cancelled) = tokio::sync::oneshot::channel();
        super::supervise_detached(
            "voice_collection",
            None,
            None,
            collect_until_stopped(Arc::downgrade(runtime), runtime.clock(), cancelled),
        );
        Self { stop: Some(stop) }
    }
}
impl Drop for CollectionTask {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
async fn collect_until_stopped(
    runtime: std::sync::Weak<Runtime>,
    clock: Arc<dyn catcoms_rt::Clock>,
    mut cancelled: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        {
            let Some(runtime) = runtime.upgrade() else {
                return;
            };
            runtime.collect_expired();
        }
        // No strong runtime, native coordinator mutex, or resource credit crosses this wait.
        // Uncertain WebCrypto charges live in Coordinator.operations and are never TTL swept.
        tokio::select! {
            _ = &mut cancelled => return,
            _ = clock.sleep(std::time::Duration::from_millis(1000)) => {}
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub generation: u64,
    pub server: u64,
    pub instance: u64,
    pub group: Vec<u8>,
    pub scope: Scope,
}
#[derive(Clone)]
pub(crate) enum CandidateKind {
    Controller(u64),
    Call {
        controller: String,
        binding: Binding,
    },
    // Selected only by the inactive trusted native entry. Ordinary IPC cannot
    // choose a profile or reinterpret this candidate through activate().
    #[allow(dead_code)]
    // Private admitted activation is staged; no production command selects it yet.
    AdmittedCall {
        controller: String,
        binding: Binding,
    },
}
struct Candidate {
    revision: u64,
    expires: u64,
    kind: CandidateKind,
}
struct Controller {
    id: String,
    generation: u64,
    // Explicit room observers borrow this parent. Replacing/ending the WebView controller must
    // close them synchronously, before any actor stop command or stale callback is processed.
    lease: CallLease,
}
#[derive(Clone)]
pub(crate) struct Owner {
    pub id: String,
    pub binding: Binding,
    pub join: Nonce,
    pub lease: CallLease,
    _grant: Arc<CallGrant>,
}
/// Actual emptied custody-index backing, followed by its still-paid original.
/// Drop this AFTER Coordinator. A per-row refund must not leave the final Vec
/// allocation alive in an empty process registry with no remaining payer.
pub(crate) struct DetachedActivation {
    _allocation: Vec<crate::voice_call_activation::Handle>,
    _original: crate::voice_call_activation::Handle,
}
impl DetachedActivation {
    fn destroy_backing(&mut self, after_deallocation: impl FnOnce()) {
        // The original Handle is deliberately still a field here. Vec drops its
        // elements BEFORE freeing its backing; a payer inside those elements alone
        // would therefore return too early when the last index owner disappears.
        drop(std::mem::take(&mut self._allocation));
        after_deallocation(); // production is empty; tests observe the actual return boundary
    }
}
impl Drop for DetachedActivation {
    fn drop(&mut self) {
        self.destroy_backing(|| {});
        // Empty Vec, then SAME original Handle/funding, in declaration order.
    }
}
impl Owner {
    /// Cleanup only: the original affine allocation remains usable after synchronous End
    /// revokes its lease. Never use this snapshot as a current grant for a new first effect.
    pub(crate) fn cleanup_grant(&self) -> Arc<CallGrant> {
        self._grant.clone()
    }
}

/// A metadata-only snapshot of one accepted activation. Keeping the original parent and actor
/// grant distinguishes a retry from an equal-scope replacement. The bridge rechecks this exact
/// snapshot after its actor query and UI commit wait; it cannot reactivate a historical owner.
#[derive(Clone)]
pub(crate) struct AcceptedActivation {
    id: String,
    controller: String,
    generation: u64,
    parent: CallLease,
    owner: Option<Owner>,
}
impl AcceptedActivation {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn owner(&self) -> Option<&Owner> {
        self.owner.as_ref()
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Operation {
    Generate,
    Derive,
    Encrypt,
    Decrypt,
}
impl Operation {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "generate" => Ok(Self::Generate),
            "derive" => Ok(Self::Derive),
            "encrypt" => Ok(Self::Encrypt),
            "decrypt" => Ok(Self::Decrypt),
            _ => Err("unknown voice crypto operation".into()),
        }
    }
    fn max_bytes(self) -> usize {
        match self {
            Self::Generate => 128,
            Self::Derive => 4096 + 256,
            Self::Encrypt => 4096 + 32 * 1024,
            Self::Decrypt => 4096 + 32 * 1024 + 16,
        }
    }
}
pub(crate) struct Coordinator {
    serial: u64,
    revision: u64,
    controller: Option<Controller>,
    owner: Option<Owner>,
    candidates: HashMap<String, Candidate>,
    // Bounded original pending/terminal custody, not another call-winner cell.
    // The SAME activation gate and actual owner remain the positive authority.
    // End/lock do not acknowledge lost replies or erase unfinished cleanup.
    activations: Vec<crate::voice_call_activation::Handle>,
    // These charges deliberately outlive every controller/owner and UI authorization epoch.
    // Dropping a WebView or IPC waiter is not evidence that WebCrypto settled.
    operations: HashMap<String, Credit>,
    // A selected join has one generation, unlike the compatibility fresh-token operations.
    // Uncertain generation/publication survives owner retirement until actual work drains.
    join_generations: HashMap<String, join_key::Generation>,
    // The SAME existing table's actual backing becomes separately process-paid
    // before its first row of EITHER profile. LAST relative to the table: empty retained
    // capacity and final table deallocation remain paid through Coordinator Drop.
    join_index_funding: Option<Arc<join_key::JoinIndexFunding>>,
    // Registered only from opaque Root-issued owners, before pull/admission can race. These
    // survive End/lock/replacement while an engine is admitted but actual settlement is unknown.
    // Lock order is Coordinator -> action fence. Root never calls Coordinator under its fence.
    #[cfg(test)]
    actions: HashMap<(String, u64), RegisteredAction>,
    resources: Arc<Resources>,
    runtime: Arc<Runtime>,
}
impl Default for Coordinator {
    fn default() -> Self {
        Self::with_runtime(Runtime::new(Arc::new(catcoms_rt::SystemClock)))
    }
}
impl Drop for Coordinator {
    fn drop(&mut self) {
        self.destroy_activation_index(Self::revoke_all, || {});
    }
}
impl Coordinator {
    /// Sponsor the entire actual index allocation through normal teardown and
    /// inherited revocation callback unwind. No new Handle, payment or allocation
    /// is created: move the SAME final original out before revocation can panic.
    pub(super) fn destroy_activation_index(
        &mut self,
        revoke: impl FnOnce(&mut Self),
        after_deallocation: impl FnOnce(),
    ) {
        struct Retirement<F: FnOnce()> {
            detached: Option<DetachedActivation>,
            after: Option<F>,
        }
        impl<F: FnOnce()> Drop for Retirement<F> {
            fn drop(&mut self) {
                if let Some(detached) = &mut self.detached {
                    detached.destroy_backing(self.after.take().expect("one backing return"));
                }
                // DetachedActivation drops its empty backing before its original,
                // including if the test callback unwinds. Production callback is ZST.
            }
        }
        let retirement = Retirement {
            detached: self.activations.pop().map(|original| DetachedActivation {
                _allocation: std::mem::take(&mut self.activations),
                _original: original,
            }),
            after: Some(after_deallocation),
        };
        revoke(self); // production is exactly revoke_all; tests pin its unwind boundary
        drop(retirement);
    }
    pub fn with_runtime(runtime: Arc<Runtime>) -> Self {
        Self {
            serial: 0,
            revision: 0,
            controller: None,
            owner: None,
            candidates: HashMap::new(),
            activations: Vec::new(),
            operations: HashMap::new(),
            join_generations: HashMap::new(),
            join_index_funding: None,
            #[cfg(test)]
            actions: HashMap::new(),
            resources: runtime.resources(),
            runtime,
        }
    }
    pub fn runtime(&self) -> Arc<Runtime> {
        self.runtime.clone()
    }
    pub fn controller_parent(&self, id: &str, generation: u64) -> Result<CallLease, String> {
        self.controller
            .as_ref()
            .filter(|current| {
                current.id == id && current.generation == generation && current.lease.is_live()
            })
            .map(|current| current.lease.clone())
            .ok_or_else(|| "voice controller changed".into())
    }
    fn retire_controller(&mut self) {
        if let Some(current) = self.controller.take() {
            current.lease.revoke();
        }
    }
    fn id(&mut self) -> Result<String, String> {
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or("voice identity exhausted")?;
        Ok(format!("voice-{:016x}", self.serial))
    }
    pub fn prepare(&mut self, kind: CandidateKind, now: u64) -> Result<String, String> {
        self.candidates
            .retain(|_, candidate| candidate.expires > now);
        if let CandidateKind::Call {
            controller,
            binding,
        }
        | CandidateKind::AdmittedCall {
            controller,
            binding,
        } = &kind
        {
            self.controller_parent(controller, binding.generation)?;
        }
        if self.candidates.len() >= CANDIDATES {
            return Err("too many pending voice joins".into());
        }
        let id = self.id()?;
        self.candidates.insert(
            id.clone(),
            Candidate {
                revision: self.revision,
                expires: now.saturating_add(CANDIDATE_TTL),
                kind,
            },
        );
        Ok(id)
    }
    pub fn candidate(&self, id: &str, now: u64) -> Option<CandidateKind> {
        self.candidates
            .get(id)
            .filter(|candidate| candidate.expires > now)
            .map(|candidate| candidate.kind.clone())
    }
    /// Trusted negative/result recovery. History is bound to this original native
    /// candidate, independent of current UI authorization or replacement calls.
    #[allow(dead_code)] // Inactive trusted result-recovery entry; never a renderer command.
    pub(crate) fn admitted_activation(
        &self,
        id: &str,
    ) -> Option<crate::voice_call_activation::Handle> {
        self.activations
            .iter()
            .find(|handle| handle.id() == id)
            .cloned()
    }

    /// Borrow before claiming an original paid physical serialization slot; do
    /// not clone another pending response owner before its bounded admission.
    pub(crate) fn admitted_activation_ref(
        &self,
        id: &str,
    ) -> Option<&crate::voice_call_activation::Handle> {
        self.activations.iter().find(|handle| handle.id() == id)
    }
    /// Stage the separately paid canonical owner BEFORE native selection. The map
    /// is bounded custody/history only; inserting it grants no Root permission.
    #[allow(dead_code)] // Staged canonical admission; production integration remains gated.
    pub(crate) fn retain_admitted_activation(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
    ) -> Result<(), String> {
        if let Some(original) = self
            .activations
            .iter()
            .find(|original| original.id() == handle.id())
        {
            return if original.same(handle) {
                Ok(())
            } else {
                Err("voice activation changed".into())
            };
        }
        if self.activations.len() >= crate::voice_call_activation::MAX_ACTIVATIONS {
            return Err("too many unfinished voice activations".into());
        }
        if self.activations.capacity() == 0 {
            // Every admitted Cell pays the ENTIRE fixed maximum backing, not an
            // optimistic load factor or one slice of a previously larger map.
            self.activations
                .reserve_exact(crate::voice_call_activation::MAX_ACTIVATIONS);
        }
        self.activations.push(handle.clone());
        Ok(())
    }
    /// Native CAS for the inactive admitted entry, while holding its ORIGINAL UI
    /// guard. Prepared grants stay inert before this turn; Root selection follows
    /// on the canonical task. Return the old owner for destruction AFTER guards.
    #[allow(dead_code)] // Staged native CAS; production integration remains gated.
    pub(crate) fn activate_admitted(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
        join: Nonce,
        now: u64,
    ) -> Result<Option<Owner>, String> {
        if !self
            .activations
            .iter()
            .any(|original| original.same(handle))
        {
            return Err("voice activation is not retained".into());
        }
        let candidate = self
            .candidates
            .get(handle.id())
            .ok_or("voice candidate expired")?;
        if candidate.expires <= now || candidate.revision != self.revision {
            return Err("voice join was superseded".into());
        }
        let CandidateKind::AdmittedCall {
            controller,
            binding,
        } = &candidate.kind
        else {
            return Err("this voice candidate has a different protocol".into());
        };
        if controller != handle.controller() || binding != handle.binding() {
            return Err("voice activation binding changed".into());
        }
        self.controller_parent(controller, binding.generation)?;
        let grant = handle.grant().ok_or("voice actor permission changed")?;
        if !handle.original().matches_grant(&grant)
            || !grant.lease().is_admitted()
            || grant.scope() != binding.scope
            || handle.original().is_closed()
        {
            return Err("voice actor permission changed".into());
        }
        // Fresh activation, never an in-place upgrade or replacement of a healthy
        // endpoint. A losing candidate cannot revoke any current healthy call.
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| owner.lease.is_live())
        {
            return Err("end the current call before starting this call".into());
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or("voice revision exhausted")?;
        let owner = Owner {
            id: handle.id().into(),
            binding: binding.clone(),
            join,
            lease: grant.lease(),
            _grant: grant,
        };
        self.candidates.remove(handle.id());
        self.revision = revision;
        Ok(self.owner.replace(owner))
    }
    /// Final synchronous native fence after actual original Actor argument
    /// retirement. The caller has already checked its STILL-HELD original UI guard
    /// identity/generation; this checks exact controller/owner/grant/original gate.
    pub(crate) fn finish_admitted_activation(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
        selected: catcoms_sync::voice_v2::call_activation::RootSelectedCall,
    ) -> Result<(), catcoms_sync::voice_v2::VoiceError> {
        use catcoms_sync::voice_v2::VoiceError;
        let owner = self
            .owner
            .as_ref()
            .filter(|owner| {
                owner.id == handle.id()
                    && owner.binding == *handle.binding()
                    && owner.lease.is_admitted()
                    && owner.lease.is_live()
            })
            .ok_or(VoiceError::Unauthorized)?;
        if !self
            .activations
            .iter()
            .any(|original| original.same(handle))
            || self
                .controller_parent(handle.controller(), handle.binding().generation)
                .is_err()
            || !handle.original().matches_grant(&owner._grant)
            || !selected.original().is_same(handle.original())
        {
            return Err(VoiceError::Unauthorized);
        }
        handle.admit(selected, &owner._grant, owner.join)
    }
    /// Detach ONLY this original admitted owner for actual heavy destruction after
    /// Coordinator/UI release. A late failed task may never End its replacement.
    pub(crate) fn retire_admitted_activation(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
    ) -> Option<Owner> {
        let same = self
            .activations
            .iter()
            .any(|original| original.same(handle))
            && self.owner.as_ref().is_some_and(|owner| {
                owner.id == handle.id()
                    && owner.binding == *handle.binding()
                    && handle.original().matches_grant(&owner._grant)
            });
        same.then(|| self.owner.take()).flatten()
    }
    /// Explicit trusted local Forget, never End/TTL/absence. The returned paid
    /// handle must be dropped after Coordinator; live reply copies retain their
    /// SAME payer independently. Unknown or foreign originals create no history.
    #[allow(dead_code)] // Staged explicit result ACK/Forget; End is not an ACK.
    pub(crate) fn forget_admitted_activation(
        &mut self,
        handle: &crate::voice_call_activation::Handle,
    ) -> Option<DetachedActivation> {
        if !handle.collectible()
            || self
                .join_generations
                .get(handle.id())
                .is_some_and(|generation| {
                    generation.blocks_admitted_activation_forget(handle.original())
                })
        {
            return None;
        }
        let index = self
            .activations
            .iter()
            .position(|original| original.same(handle))?;
        let original = self.activations.swap_remove(index);
        let allocation = if self.activations.is_empty() {
            std::mem::take(&mut self.activations)
        } else {
            Vec::new()
        };
        Some(DetachedActivation {
            _allocation: allocation,
            _original: original,
        })
    }

    /// Collect at most one acknowledged original after actual call/Work/storage
    /// and task return. End alone is never result acknowledgement. The existing
    /// key-debt fence still prevents a parent from losing its only recovery
    /// handle while SDK settlement or publication history remains outstanding.
    ///
    /// The bounded native caller may repeat this up to its existing 32-row cap,
    /// dropping each returned owner AFTER Coordinator. Only activation Cell locks
    /// are used here; those paths never acquire Coordinator while holding Cell.
    /// Publication Cell -> Coordinator workers use a DIFFERENT Cell, and their
    /// reverse Pending checks are pure atomics/immutable metadata only.
    pub(crate) fn collect_one_acknowledged_admitted_activation(
        &mut self,
    ) -> Option<DetachedActivation> {
        let original = self
            .activations
            .iter()
            .find(|handle| {
                handle.collectible()
                    && self
                        .join_generations
                        .get(handle.id())
                        .is_none_or(|generation| {
                            !generation.blocks_admitted_activation_forget(handle.original())
                        })
            })?
            .clone();
        self.forget_admitted_activation(&original)
    }
    /// Recover only a currently installed activation, before looking for its consumed candidate.
    /// Candidate expiry does not expire an accepted call, and a lost reply must not allocate a
    /// second join nonce or actor grant. This creates no registry, serial, revision or new lease.
    pub fn accepted_activation(&self, id: &str) -> Option<AcceptedActivation> {
        let controller = self.controller.as_ref().filter(|c| c.lease.is_live())?;
        let owner = if id == controller.id {
            None
        } else {
            let owner = self.owner(id)?;
            if owner.binding.generation != controller.generation {
                return None;
            }
            Some(owner)
        };
        Some(AcceptedActivation {
            id: id.into(),
            controller: controller.id.clone(),
            generation: controller.generation,
            parent: controller.lease.clone(),
            owner,
        })
    }
    /// Final native identity fence, after every asynchronous snapshot/commit wait. Equal scopes
    /// and live-looking renderer IDs cannot substitute for the original controller/actor owners.
    /// A receipt is metadata only; later protocol work still validates the actual CallGrant.
    pub fn validate_accepted_activation(
        &self,
        accepted: &AcceptedActivation,
    ) -> Result<(), String> {
        let controller = self
            .controller
            .as_ref()
            .filter(|current| {
                current.id == accepted.controller
                    && current.generation == accepted.generation
                    && current.lease.is_live()
                    && current.lease.is_same(&accepted.parent)
            })
            .ok_or("voice controller changed")?;
        match &accepted.owner {
            None if accepted.id == controller.id => Ok(()),
            Some(original) => {
                let current = self.owner(&accepted.id).ok_or("voice owner changed")?;
                if current.binding == original.binding
                    && current.join == original.join
                    && current.lease.is_same(&original.lease)
                    && original.lease.is_live()
                    && Arc::ptr_eq(&current._grant, &original._grant)
                {
                    Ok(())
                } else {
                    Err("voice owner changed".into())
                }
            }
            None => Err("voice controller changed".into()),
        }
    }
    /// CAS activation after the bridge revalidates the exact UI/actor/group/room/policy. Native
    /// supplies a fresh injected-RNG nonce; a failed/stale candidate can never revoke a new call.
    /// Borrow the pending grant until admission succeeds: refusal must leave its affine cleanup
    /// to the bridge, after the Coordinator and UI commit guards have both been released.
    pub fn activate(
        &mut self,
        id: &str,
        join: Nonce,
        now: u64,
        grant: &mut Option<CallGrant>,
    ) -> Result<(), String> {
        // A second physical/pilot channel cannot turn an admitted issuer child
        // into Existing permission, even before RootSelected/NativeActive.
        if self
            .candidates
            .get(id)
            .is_some_and(|candidate| matches!(candidate.kind, CandidateKind::AdmittedCall { .. }))
        {
            return Err("this voice candidate needs native admitted activation".into());
        }
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| owner.lease.is_admitted() && owner.lease.is_live())
        {
            return Err("end the current call before replacing its protocol".into());
        }
        let candidate = self
            .candidates
            .remove(id)
            .ok_or("voice candidate expired")?;
        if candidate.expires <= now || candidate.revision != self.revision {
            return Err("voice join was superseded".into());
        }
        if let CandidateKind::Call {
            controller,
            binding,
        } = &candidate.kind
        {
            self.controller_parent(controller, binding.generation)?;
            if grant.as_ref().is_none_or(|grant| {
                grant.scope() != binding.scope
                    || !grant.lease().is_live()
                    || grant.lease().is_admitted()
            }) {
                return Err("voice actor permission changed".into());
            }
        } else if grant.is_some() {
            return Err("unexpected voice actor permission".into());
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("voice revision exhausted")?;
        self.retire_owner();
        match candidate.kind {
            CandidateKind::Controller(generation) => {
                self.retire_controller();
                self.controller = Some(Controller {
                    id: id.into(),
                    generation,
                    lease: CallLease::default(),
                })
            }
            CandidateKind::Call { binding, .. } => {
                let grant = Arc::new(
                    grant
                        .take()
                        .expect("call grant checked before retiring owner"),
                );
                self.owner = Some(Owner {
                    id: id.into(),
                    binding,
                    join,
                    lease: grant.lease(),
                    _grant: grant,
                })
            }
            CandidateKind::AdmittedCall { .. } => {
                unreachable!("admitted candidate refused before consumption")
            }
        }
        Ok(())
    }
    fn retire_owner(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.lease.revoke();
            #[cfg(test)]
            {
                for ((id, _), registered) in &self.actions {
                    if id == &owner.id {
                        registered.action.retire_if_unstarted();
                    }
                }
                self.prune_actions();
            }
        }
        self.prune_join_generations();
    }
    #[cfg(test)]
    fn prune_actions(&mut self) {
        let current = self.owner.as_ref().map(|owner| owner.id.as_str());
        self.actions.retain(|(owner, _), registered| {
            // End is not an acknowledgement of a result lost in transit. Read terminal state
            // and settlement under one action fence: an admitted first effect cannot publish
            // a result between two independent observations and then be silently collected.
            let status = registered.action.terminal_status();
            if status.receipt.is_some() {
                !(registered.receipt_acknowledged && status.settled)
            } else {
                current == Some(owner.as_str()) || !status.settled
            }
        });
    }

    /// Trusted Root-to-native registration, not a command callable by the browser. The exact
    /// scope, call lease and shared process pool must all match. ID equality cannot substitute
    /// another Root action/fence; repeated registration of that exact Arc is idempotent.
    #[cfg(test)]
    pub fn register_action(
        &mut self,
        owner_id: &str,
        action: Arc<CryptoAction>,
    ) -> Result<ActionIdentity, String> {
        let owner = self.owner(owner_id).ok_or("voice owner changed")?;
        if !action.belongs_to(owner.binding.scope, &owner.lease)
            || !action.uses_resources(&self.resources)
        {
            return Err("voice action owner changed".into());
        }
        let identity = ActionIdentity {
            owner: owner_id.into(),
            action: action.serial(),
            version: action.version(),
            input_tag: action.input_tag(),
        };
        let key = (identity.owner.clone(), identity.action);
        if let Some(previous) = self.actions.get(&key) {
            return if Arc::ptr_eq(&previous.action, &action) {
                Ok(identity)
            } else {
                Err("voice action identity differs".into())
            };
        }
        self.prune_actions();
        if self.actions.len() >= REGISTERED_ACTIONS || !action.is_pending() {
            return Err("voice action is unavailable".into());
        }
        self.actions.insert(
            key,
            RegisteredAction {
                action,
                receipt_acknowledged: false,
            },
        );
        Ok(identity)
    }

    #[cfg(test)]
    fn action(&self, identity: &ActionIdentity) -> Result<&Arc<CryptoAction>, String> {
        let registered = self
            .actions
            .get(&identity.key()?)
            .ok_or("voice action is unavailable")?;
        let action = &registered.action;
        if identity.version != action.version() || identity.input_tag != action.input_tag() {
            return Err("voice action identity differs".into());
        }
        Ok(action)
    }

    /// History-only lookup does not require the original Root, LocalJoin or current call owner.
    /// IPC must still authorize account access; no caller bytes are validated and no effect or
    /// dispatch capability is returned. Clone before any future await outside Coordinator.
    #[cfg(test)]
    pub fn action_status(&self, identity: &ActionIdentity) -> Result<CryptoResultStatus, String> {
        Ok(self.action(identity)?.terminal_status())
    }

    /// Exact immutable receipt ACK may precede engine settlement. Latch it in the existing row
    /// and remove only after actual settlement. Unknown/duplicate ACK creates no tombstone, and
    /// dynamic retirement is deliberately excluded from receipt identity. No RPC-copy custody
    /// is implied: production collection must additionally await every prepaid reply owner.
    #[cfg(test)]
    pub fn acknowledge_action(
        &mut self,
        identity: &ActionIdentity,
        receipt: &CryptoResultReceipt,
    ) -> Result<bool, String> {
        let key = identity.key()?;
        if !self.actions.contains_key(&key) {
            return Ok(false);
        }
        let status = self.action(identity)?.terminal_status();
        if !status
            .receipt
            .as_ref()
            .is_some_and(|past| past.same_transition(receipt))
        {
            return Err("voice receipt identity differs".into());
        }
        self.action(identity)?
            .acknowledge_result(receipt)
            .map_err(|error| error.to_string())?;
        self.actions
            .get_mut(&key)
            .expect("checked registered action")
            .receipt_acknowledged = true;
        self.prune_actions();
        Ok(true)
    }

    /// Idempotent exact-action engine admission. Capacity refusal is temporary: it neither
    /// consumes the action nor creates a terminal rejected receipt. Never delegate this to
    /// ordinary fresh-token admit, which cannot recover a lost success reply without duplication.
    #[cfg(test)]
    pub fn admit_action(&self, identity: &ActionIdentity, bytes: usize) -> Result<u64, String> {
        let owner = self.owner(&identity.owner).ok_or("voice owner changed")?;
        let action = self.action(identity)?;
        if !action.belongs_to(owner.binding.scope, &owner.lease) {
            return Err("voice action owner changed".into());
        }
        action
            .admit_engine(bytes)
            .map_err(|error| error.to_string())
    }

    /// Latch settlement even if admission has not arrived yet. Unknown IDs create no tombstone,
    /// charge or authority. A removed retired identity can never register/admit under a new owner.
    #[cfg(test)]
    pub fn settle_action(&mut self, identity: &ActionIdentity) -> Result<bool, String> {
        if !self.actions.contains_key(&identity.key()?) {
            return Ok(false);
        }
        self.action(identity)?.settle_engine();
        self.prune_actions();
        Ok(true)
    }
    pub fn owner(&self, id: &str) -> Option<Owner> {
        self.owner
            .as_ref()
            // This is the Existing positive-key/crypto/pilot entry. The staged
            // admitted activation deliberately has no key or endpoint API yet.
            .filter(|owner| owner.id == id && owner.lease.is_live() && !owner.lease.is_admitted())
            .cloned()
    }
    pub fn end(&mut self, id: &str) {
        self.candidates.remove(id);
        if self
            .controller
            .as_ref()
            .is_some_and(|current| current.id == id)
        {
            self.retire_controller();
            self.candidates.clear();
        } else if self.owner.as_ref().is_none_or(|owner| owner.id != id) {
            return;
        }
        self.revision = self.revision.saturating_add(1);
        self.retire_owner();
    }
    /// Retain the SAME actual call owner before End can remove the last grant upgrade path.
    /// The caller drops this snapshot outside Coordinator and orders cleanup on its ORIGINAL
    /// actor. A stale id cannot capture the replacement; controller End includes its child call.
    pub(crate) fn end_with_cleanup(&mut self, id: &str) -> Option<Owner> {
        let matches = self
            .controller
            .as_ref()
            .is_some_and(|controller| controller.id == id)
            || self.owner.as_ref().is_some_and(|owner| owner.id == id);
        // Admitted negative cleanup belongs to its original paid canonical task,
        // not the legacy Stop/key producer. Its Cell already retains the actual
        // Grant until original arguments return, then drops it outside guards.
        let original = matches
            .then(|| {
                self.owner
                    .as_ref()
                    .filter(|owner| !owner.lease.is_admitted())
                    .cloned()
            })
            .flatten();
        self.end(id);
        original
    }
    pub fn revoke_all(&mut self) {
        self.runtime.revoke_session();
        self.retire_controller();
        self.candidates.clear();
        self.revision = self.revision.saturating_add(1);
        self.retire_owner();
    }
    pub fn retire_server(&mut self, server: u64, instance: u64) {
        self.candidates.retain(|_, c| !matches!(&c.kind, CandidateKind::Call { binding, .. } | CandidateKind::AdmittedCall { binding, .. } if binding.server == server && binding.instance == instance));
        if let Some(id) = self
            .owner
            .as_ref()
            .filter(|owner| owner.binding.server == server && owner.binding.instance == instance)
            .map(|owner| owner.id.clone())
        {
            self.end(&id);
        }
    }
    /// Charge before engine work begins. The trusted window reports actual settlement. An old
    /// exact token can settle while locked but cannot inspect a call or refund another operation.
    pub fn admit(
        &mut self,
        owner: &str,
        operation: Operation,
        bytes: usize,
    ) -> Result<String, String> {
        let owner = self.owner(owner).ok_or("voice owner changed")?;
        if bytes > operation.max_bytes() {
            return Err("voice crypto byte bound exceeded".into());
        }
        let group = owner.binding.scope.group;
        let credit = self
            .resources
            .charge(group, bytes, ResourceKind::Crypto)
            .map_err(|error| error.to_string())?;
        let id = self.id()?;
        self.operations.insert(id.clone(), credit);
        Ok(id)
    }
    pub fn settle(&mut self, id: &str) -> bool {
        self.operations.remove(id).is_some()
    }
}

#[cfg(test)]
pub(crate) mod tests;
