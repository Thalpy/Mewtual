use super::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Explicit caller storage allowance. Limits refuse new evidence; they NEVER evict an
/// accepted/pending action or required proof. Bytes count canonical action envelopes only,
/// not allocator/RSS or a complete production payer invoice (activation is absent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedgerLimits {
    pub max_actions: usize,
    pub max_action_bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingReason {
    Authority,
    Dependencies,
    CausalAuthority,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionState {
    Pending(PendingReason),
    Valid,
    Invalid(ChatError),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    Retained(ActionId),
    Duplicate(ActionId),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Advance {
    pub work: usize,
    pub finalized: usize,
    pub remaining: bool,
}

pub(super) struct Entry {
    pub(super) action: SignedAction,
    pub(super) state: ActionState,
    authority: Option<VerifiedAuthorityBasis>,
}
#[derive(Clone, Copy)]
enum Task {
    Evaluate(ActionId),
    Wake {
        parent: ActionId,
        after: Option<ActionId>,
    },
}

/// Additive semantic evidence. There is no Automerge actor/sequence slot to occupy first.
/// Unknown authority/dependencies stay pending; invalid semantic evidence stays retained.
pub struct ChatLedger {
    pub(super) realm: Realm,
    pub(super) limits: LedgerLimits,
    pub(super) entries: BTreeMap<ActionId, Entry>,
    bytes: usize,
    dependents: BTreeMap<ActionId, BTreeSet<ActionId>>,
    tasks: VecDeque<Task>,
    queued: BTreeSet<ActionId>,
}
impl std::fmt::Debug for ChatLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatLedger")
            .field("actions", &self.entries.len())
            .field("queued_work", &self.tasks.len())
            .finish_non_exhaustive()
    }
}
impl ChatLedger {
    pub fn new(realm: Realm, limits: LedgerLimits) -> Result<Self, ChatError> {
        if limits.max_actions == 0 || limits.max_action_bytes == 0 {
            return Err(ChatError::Bound);
        }
        Ok(Self {
            realm,
            limits,
            entries: BTreeMap::new(),
            bytes: 0,
            dependents: BTreeMap::new(),
            tasks: VecDeque::new(),
            queued: BTreeSet::new(),
        })
    }
    pub fn realm(&self) -> &Realm {
        &self.realm
    }
    pub fn action_ids(&self) -> impl Iterator<Item = ActionId> + '_ {
        self.entries.keys().copied()
    }
    pub fn action(&self, id: ActionId) -> Option<&SignedAction> {
        self.entries.get(&id).map(|entry| &entry.action)
    }
    pub fn state(&self, id: ActionId) -> Option<ActionState> {
        self.entries.get(&id).map(|entry| entry.state)
    }
    pub fn retained_action_bytes(&self) -> usize {
        self.bytes
    }
    pub fn has_pending_work(&self) -> bool {
        !self.tasks.is_empty()
    }
    pub fn import(&mut self, action: SignedAction) -> Result<ImportOutcome, ChatError> {
        // Verify EVERY variant before looking up an existing semantic CID or replacing its
        // signature representative. A bad variant cannot poison a valid stored witness.
        if action.spec().realm != self.realm {
            return Err(ChatError::Scope);
        }
        if !action.verify() {
            return Err(ChatError::Signature);
        }
        let id = action.id();
        if action.dependencies().contains(&id) {
            return Err(ChatError::Dependency);
        }
        if let Some(entry) = self.entries.get_mut(&id) {
            if entry.action.semantic_bytes() != action.semantic_bytes() {
                return Err(ChatError::Malformed);
            }
            if action.signature() < entry.action.signature() {
                entry.action = action;
            }
            return Ok(ImportOutcome::Duplicate(id));
        }
        let bytes = action.encode().len();
        let total = self.bytes.checked_add(bytes).ok_or(ChatError::Capacity)?;
        if self.entries.len() >= self.limits.max_actions || total > self.limits.max_action_bytes {
            return Err(ChatError::Capacity);
        }
        for dependency in action.dependencies() {
            self.dependents.entry(dependency).or_default().insert(id);
        }
        self.entries.insert(
            id,
            Entry {
                action,
                state: ActionState::Pending(PendingReason::Authority),
                authority: None,
            },
        );
        self.bytes = total;
        self.enqueue(id);
        Ok(ImportOutcome::Retained(id))
    }
    /// A missing action never creates a record. An imported label is not an authority token.
    /// Coverage may be augmented only by another externally verified, identically scoped
    /// decision; rights/subject/grant cannot change through first-witness replacement.
    pub fn authorize(
        &mut self,
        id: ActionId,
        witness: VerifiedAuthorityBasis,
    ) -> Result<(), ChatError> {
        let entry = self.entries.get_mut(&id).ok_or(ChatError::Unknown)?;
        if !witness.matches(&entry.action) {
            return Err(ChatError::Authority);
        }
        if let Some(original) = &mut entry.authority {
            original.merge(&witness)?;
        } else {
            entry.authority = Some(witness);
        }
        if matches!(entry.state, ActionState::Pending(_)) {
            self.enqueue(id);
        }
        Ok(())
    }
    fn enqueue(&mut self, id: ActionId) {
        if self.queued.insert(id) {
            self.tasks.push_back(Task::Evaluate(id));
        }
    }
    /// Every charged unit evaluates at most65 fixed dependency labels, or wakes ONE
    /// dependent. A wide fanout is continued fairly rather than hidden in a single unit.
    /// There is no recursive dependency walk or retry-on-every-import archive scan.
    pub fn advance(&mut self, budget: usize) -> Advance {
        use std::ops::Bound::{Excluded, Unbounded};
        let mut progress = Advance::default();
        while progress.work < budget {
            let Some(task) = self.tasks.pop_front() else {
                break;
            };
            progress.work += 1;
            match task {
                Task::Evaluate(id) => {
                    self.queued.remove(&id);
                    if self
                        .entries
                        .get(&id)
                        .is_none_or(|entry| !matches!(entry.state, ActionState::Pending(_)))
                    {
                        continue;
                    }
                    let next = self.evaluate(id);
                    self.entries
                        .get_mut(&id)
                        .expect("queued original action")
                        .state = next;
                    if !matches!(next, ActionState::Pending(_)) {
                        progress.finalized += 1;
                        self.tasks.push_back(Task::Wake {
                            parent: id,
                            after: None,
                        });
                    }
                }
                Task::Wake { parent, after } => {
                    let child = self
                        .dependents
                        .get(&parent)
                        .and_then(|children| match after {
                            Some(after) => {
                                children.range((Excluded(after), Unbounded)).next().copied()
                            }
                            None => children.first().copied(),
                        });
                    if let Some(child) = child {
                        self.enqueue(child);
                        self.tasks.push_back(Task::Wake {
                            parent,
                            after: Some(child),
                        });
                    }
                }
            }
        }
        progress.remaining = !self.tasks.is_empty();
        progress
    }
    fn evaluate(&self, id: ActionId) -> ActionState {
        let entry = &self.entries[&id];
        let action = &entry.action;
        let spec = action.spec();
        let Some(authority) = &entry.authority else {
            return ActionState::Pending(PendingReason::Authority);
        };
        for dependency in action.dependencies() {
            let Some(parent) = self.entries.get(&dependency) else {
                return ActionState::Pending(PendingReason::Dependencies);
            };
            match parent.state {
                ActionState::Pending(_) => {
                    return ActionState::Pending(PendingReason::Dependencies)
                }
                ActionState::Invalid(_) => return ActionState::Invalid(ChatError::Dependency),
                ActionState::Valid => {}
            }
            if !authority.covers(parent.action.spec().basis) {
                return ActionState::Pending(PendingReason::CausalAuthority);
            }
            if let Some(create) = spec.body.create() {
                if dependency != create && parent.action.spec().body.create() != Some(create) {
                    return ActionState::Invalid(ChatError::Dependency);
                }
            }
        }
        let needed = match &spec.body {
            Body::Create { reply_to, .. } => {
                if reply_to.is_some_and(|id| {
                    !matches!(self.entries[&id].action.spec().body, Body::Create { .. })
                }) {
                    return ActionState::Invalid(ChatError::Dependency);
                }
                Rights::CREATE
            }
            Body::Join { .. } => Rights::JOIN,
            Body::Edit { .. } => Rights::SELF_EDIT,
            Body::Delete { .. } if authority.rights.contains(Rights::DELETE_ANY) => {
                Rights::DELETE_ANY
            }
            Body::Delete { .. } => Rights::SELF_DELETE,
        };
        if !authority.rights.contains(needed) {
            return ActionState::Invalid(ChatError::Permission);
        }
        if let Some(create) = spec.body.create() {
            let original = &self.entries[&create].action;
            if !matches!(original.spec().body, Body::Create { .. }) {
                return ActionState::Invalid(ChatError::Dependency);
            }
            // Stable full-key authorship is intentional across rejoin/incarnations. A fresh
            // ordinary grant does not inherit the old incarnation's DELETE_ANY permission.
            if (matches!(spec.body, Body::Edit { .. })
                || (matches!(spec.body, Body::Delete { .. }) && needed != Rights::DELETE_ANY))
                && action.signer() != original.signer()
            {
                return ActionState::Invalid(ChatError::Permission);
            }
        }
        ActionState::Valid
    }
}
