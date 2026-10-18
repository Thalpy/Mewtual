//! Ephemeral native voice ownership and process-wide WebCrypto charges. This is independent of
//! optional router mappings: losing mapping capacity must never select a protocol or own a call.
//! No keys, decrypted SDP/ICE or call state are persisted. Actual actor membership/room/policy
//! checks surround these synchronous transitions in the bridge, then repeat at protocol effects.

// The V2 action handoff has no production IPC consumer yet. Compile its registry only for
// contract tests until endpoint result recovery and the browser adapter are integrated.
#[cfg(test)]
use catcoms_sync::voice_v2::crypto_actions::CryptoAction;
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind, Resources};
use catcoms_sync::voice_v2::runtime::Runtime;
use catcoms_sync::voice_v2::{CallGrant, CallLease, Nonce, Scope};
use std::collections::HashMap;
use std::sync::Arc;

const CANDIDATES: usize = 8;
const CANDIDATE_TTL: u64 = 30_000;
#[cfg(test)]
const REGISTERED_ACTIONS: usize = 256;

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
    // These charges deliberately outlive every controller/owner and UI authorization epoch.
    // Dropping a WebView or IPC waiter is not evidence that WebCrypto settled.
    operations: HashMap<String, Credit>,
    // Registered only from opaque Root-issued owners, before pull/admission can race. These
    // survive End/lock/replacement while an engine is admitted but actual settlement is unknown.
    // Lock order is Coordinator -> action fence. Root never calls Coordinator under its fence.
    #[cfg(test)]
    actions: HashMap<(String, u64), Arc<CryptoAction>>,
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
        self.revoke_all();
    }
}
impl Coordinator {
    pub fn with_runtime(runtime: Arc<Runtime>) -> Self {
        Self {
            serial: 0,
            revision: 0,
            controller: None,
            owner: None,
            candidates: HashMap::new(),
            operations: HashMap::new(),
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
    /// CAS activation after the bridge revalidates the exact UI/actor/group/room/policy. Native
    /// supplies a fresh injected-RNG nonce; a failed/stale candidate can never revoke a new call.
    pub fn activate(
        &mut self,
        id: &str,
        join: Nonce,
        now: u64,
        grant: Option<CallGrant>,
    ) -> Result<(), String> {
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
            if grant
                .as_ref()
                .is_none_or(|grant| grant.scope() != binding.scope || !grant.lease().is_live())
            {
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
                let grant = Arc::new(grant.expect("call grant checked before retiring owner"));
                self.owner = Some(Owner {
                    id: id.into(),
                    binding,
                    join,
                    lease: grant.lease(),
                    _grant: grant,
                })
            }
        }
        Ok(())
    }
    fn retire_owner(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.lease.revoke();
            #[cfg(test)]
            {
                for ((id, _), action) in &self.actions {
                    if id == &owner.id {
                        action.retire_if_unstarted();
                    }
                }
                self.prune_actions();
            }
        }
    }
    #[cfg(test)]
    fn prune_actions(&mut self) {
        let current = self.owner.as_ref().map(|owner| owner.id.as_str());
        self.actions
            .retain(|(owner, _), action| current == Some(owner.as_str()) || !action.is_settled());
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
            return if Arc::ptr_eq(previous, &action) {
                Ok(identity)
            } else {
                Err("voice action identity differs".into())
            };
        }
        self.prune_actions();
        if self.actions.len() >= REGISTERED_ACTIONS || !action.is_pending() {
            return Err("voice action is unavailable".into());
        }
        self.actions.insert(key, action);
        Ok(identity)
    }

    #[cfg(test)]
    fn action(&self, identity: &ActionIdentity) -> Result<&Arc<CryptoAction>, String> {
        let action = self
            .actions
            .get(&identity.key()?)
            .ok_or("voice action is unavailable")?;
        if identity.version != action.version() || identity.input_tag != action.input_tag() {
            return Err("voice action identity differs".into());
        }
        Ok(action)
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
            .filter(|owner| owner.id == id && owner.lease.is_live())
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
    pub fn revoke_all(&mut self) {
        self.runtime.revoke_session();
        self.retire_controller();
        self.candidates.clear();
        self.revision = self.revision.saturating_add(1);
        self.retire_owner();
    }
    pub fn retire_server(&mut self, server: u64, instance: u64) {
        self.candidates.retain(|_, c| !matches!(&c.kind, CandidateKind::Call { binding, .. } if binding.server == server && binding.instance == instance));
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
mod tests;
