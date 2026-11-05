//! Current effect filtering. Historical semantic APIs deliberately remain separate and intact.
use super::*;
use std::collections::BTreeSet;

/// Only the chosen current body escapes Visible. Suppressed/deleted variants contain no edit
/// alternatives or plaintext. Proven Delete support and resulting display certainty differ:
/// a Proven Delete of a currently Provisional Create still displays Provisional certainty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardedProjection {
    Visible {
        create: ActionId,
        author: [u8; 32],
        presented_ms: u64,
        reply_to: Option<ActionId>,
        selected: ActionId,
        text: String,
        certainty: ProjectionCertainty,
    },
    Deleted {
        create: ActionId,
        delete: ActionId,
        certainty: ProjectionCertainty,
    },
    Held {
        create: ActionId,
    },
    IncompleteSuppressed {
        create: ActionId,
    },
}

/// An exact result plus the original pre-body view/revision/scope lease. The future native
/// adapter must resolve its reply INSIDE with_publication after UI/registry guards, with no
/// await afterward. Sampling a newer view after constructing this object cannot renew it.
pub struct GuardedMessageProjection {
    lease: Option<ProjectionLease>,
    result: GuardedProjection,
}
impl std::fmt::Debug for GuardedMessageProjection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedMessageProjection")
            .finish_non_exhaustive()
    }
}
impl GuardedMessageProjection {
    pub fn with_publication<T>(
        &self,
        publish: impl FnOnce(&GuardedProjection) -> T,
    ) -> Result<T, ChatError> {
        if let Some(lease) = &self.lease {
            lease.publish(|| publish(&self.result))
        } else {
            Ok(publish(&self.result))
        } // default-closed Held contains no body or certainty
    }
}

/// Admitted causal heads only, distinct from complete retained semantic heads. Metadata still
/// crosses its original view/scope lease before publication; held Join ancestors can be
/// traversed as knowledge without turning their bodies/effects into admitted evidence.
pub struct GuardedCausalHeads {
    lease: Option<ProjectionLease>,
    heads: Vec<ActionId>,
}
impl std::fmt::Debug for GuardedCausalHeads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardedCausalHeads").finish_non_exhaustive()
    }
}
impl GuardedCausalHeads {
    pub fn with_publication<T>(
        &self,
        publish: impl FnOnce(&[ActionId]) -> T,
    ) -> Result<T, ChatError> {
        if let Some(lease) = &self.lease {
            lease.publish(|| publish(&self.heads))
        } else {
            Ok(publish(&self.heads))
        }
    }
}

struct Work(usize);
impl Work {
    fn spend(&mut self) -> Result<(), ChatError> {
        self.0 = self.0.checked_sub(1).ok_or(ChatError::WorkLimit)?;
        Ok(())
    }
}

impl ChatLedger {
    fn current_token(
        &self,
        id: ActionId,
        lease: &ProjectionLease,
    ) -> Option<&ProjectionAdmissionToken> {
        let entry = self.entries.get(&id)?;
        if entry.state != ActionState::Valid {
            return None;
        }
        entry
            .projection
            .as_ref()
            .filter(|token| token.live_for(&lease.view, id, &entry.action))
    }

    pub fn guarded_projection(
        &self,
        create: ActionId,
        budget: usize,
    ) -> Result<GuardedMessageProjection, ChatError> {
        // Capture before even borrowing a body field. No current owner/token is a closed
        // result, including after raw snapshot restore with valid historical witnesses.
        let Ok(mut lease) = self.projection.lease() else {
            return Ok(GuardedMessageProjection {
                lease: None,
                result: GuardedProjection::Held { create },
            });
        };
        let original = self.entries.get(&create).ok_or(ChatError::Unknown)?;
        if !matches!(original.action.spec().body, Body::Create { .. }) {
            return Err(ChatError::Dependency);
        }
        let Some(create_token) = self.current_token(create, &lease) else {
            return Ok(GuardedMessageProjection {
                lease: Some(lease),
                result: GuardedProjection::Held { create },
            });
        };
        let create_certainty = create_token.certainty();
        lease.observe(create_token);
        let mut work = Work(budget);
        let mut edits = BTreeSet::new();
        let mut proven_delete = None;
        let mut provisional_delete = None;
        let mut incomplete = false;
        for (id, entry) in &self.entries {
            work.spend()?;
            if entry.action.spec().body.create() != Some(create) {
                continue;
            }
            if entry.prior_delete.is_some() {
                incomplete = true;
            }
            let Some(token) = self.current_token(*id, &lease) else {
                continue;
            };
            match entry.action.spec().body {
                Body::Edit { .. } => {
                    edits.insert(*id);
                }
                Body::Delete { .. } if token.certainty() == ProjectionCertainty::Proven => {
                    proven_delete = Some(*id)
                }
                Body::Delete { .. } => provisional_delete = Some(*id),
                _ => {}
            }
        }
        let result = if let Some(delete) = proven_delete {
            let token = self.current_token(delete, &lease).ok_or(ChatError::Scope)?;
            lease.observe(token);
            GuardedProjection::Deleted {
                create,
                delete,
                certainty: create_certainty,
            }
        } else if incomplete {
            // An unavailable previously effective Proven Delete cannot resurrect text. This
            // is incomplete negative custody, NOT a new approved tombstone in this view.
            GuardedProjection::IncompleteSuppressed { create }
        } else if let Some(delete) = provisional_delete {
            lease.observe(self.current_token(delete, &lease).ok_or(ChatError::Scope)?);
            GuardedProjection::Deleted {
                create,
                delete,
                certainty: ProjectionCertainty::Provisional,
            }
        } else {
            let superseded = self.guarded_ancestors(&edits, &mut work)?;
            let selected = edits
                .difference(&superseded)
                .last()
                .copied()
                .unwrap_or(create);
            let token = self
                .current_token(selected, &lease)
                .ok_or(ChatError::Scope)?;
            lease.observe(token);
            let certainty = if create_certainty == ProjectionCertainty::Proven
                && token.certainty() == ProjectionCertainty::Proven
            {
                ProjectionCertainty::Proven
            } else {
                ProjectionCertainty::Provisional
            };
            let action = &self.entries[&selected].action;
            let text = match &action.spec().body {
                Body::Create { text, .. } | Body::Edit { text, .. } => text.clone(),
                _ => return Err(ChatError::Dependency),
            };
            let Body::Create { reply_to, .. } = original.action.spec().body else {
                return Err(ChatError::Dependency);
            };
            GuardedProjection::Visible {
                create,
                author: *original.action.signer(),
                presented_ms: original.action.spec().presented_ms,
                reply_to,
                selected,
                text,
                certainty,
            }
        };
        lease.validate()?;
        Ok(GuardedMessageProjection {
            lease: Some(lease),
            result,
        })
    }

    /// Projectable heads require an admitted Create and individual live effect tokens. This
    /// does not replace semantic causal knowledge: held ancestors remain traversable metadata.
    pub fn projectable_causal_heads(
        &self,
        create: ActionId,
        budget: usize,
    ) -> Result<GuardedCausalHeads, ChatError> {
        let Ok(mut lease) = self.projection.lease() else {
            return Ok(GuardedCausalHeads {
                lease: None,
                heads: Vec::new(),
            });
        };
        let original = self.entries.get(&create).ok_or(ChatError::Unknown)?;
        if !matches!(original.action.spec().body, Body::Create { .. }) {
            return Err(ChatError::Dependency);
        }
        let Some(create_token) = self.current_token(create, &lease) else {
            return Ok(GuardedCausalHeads {
                lease: Some(lease),
                heads: Vec::new(),
            });
        };
        // Keep the ORIGINAL Create scope observation even if it closes before this scan
        // reaches that row. A Proven Edit alone cannot admit metadata for a now-held Create.
        lease.observe(create_token);
        let mut work = Work(budget);
        let mut heads = BTreeSet::new();
        for (id, entry) in &self.entries {
            work.spend()?;
            if *id != create && entry.action.spec().body.create() != Some(create) {
                continue;
            }
            if let Some(token) = self.current_token(*id, &lease) {
                heads.insert(*id);
                lease.observe(token);
            }
        }
        let ancestors = self.guarded_ancestors(&heads, &mut work)?;
        let heads = heads.difference(&ancestors).copied().collect();
        lease.validate()?;
        Ok(GuardedCausalHeads {
            lease: Some(lease),
            heads,
        })
    }

    fn guarded_ancestors(
        &self,
        selected: &BTreeSet<ActionId>,
        work: &mut Work,
    ) -> Result<BTreeSet<ActionId>, ChatError> {
        let mut ancestors = BTreeSet::new();
        for id in selected {
            if matches!(self.entries[id].action.spec().body, Body::Create { .. }) {
                continue;
            }
            let mut stack = Vec::new();
            for parent in &self.entries[id].action.spec().parents {
                work.spend()?;
                stack.push(*parent);
            }
            let mut visited = BTreeSet::new();
            while let Some(parent) = stack.pop() {
                work.spend()?;
                if !visited.insert(parent) {
                    continue;
                }
                if selected.contains(&parent) {
                    ancestors.insert(parent);
                }
                let entry = self.entries.get(&parent).ok_or(ChatError::Dependency)?;
                // Valid semantic ancestry was checked independently. Its body text remains
                // untouched even if its current token is Held/absent or the ancestor is Join.
                if !matches!(entry.action.spec().body, Body::Create { .. }) {
                    for next in &entry.action.spec().parents {
                        work.spend()?;
                        stack.push(*next);
                    }
                }
            }
        }
        Ok(ancestors)
    }
}
