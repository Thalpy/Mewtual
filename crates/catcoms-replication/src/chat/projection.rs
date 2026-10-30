use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditAlternative {
    pub action: ActionId,
    pub text: String,
    pub presented_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageProjection {
    pub create: ActionId,
    pub author: [u8; 32],
    pub presented_ms: u64,
    pub reply_to: Option<ActionId>,
    /// None means a verified persistent tombstone. Historical alternatives remain below.
    pub visible_text: Option<String>,
    /// Causally maximal authorized edits, in CID order. The greatest CID is the display
    /// choice; clocks and arrival order do not select a semantic winner.
    pub alternatives: Vec<EditAlternative>,
    pub deleted_by: Vec<ActionId>,
}
struct Work(usize);
impl Work {
    fn spend(&mut self) -> Result<(), ChatError> {
        self.0 = self.0.checked_sub(1).ok_or(ChatError::WorkLimit)?;
        Ok(())
    }
}
impl ChatLedger {
    /// Complete projection or explicit work refusal; never a partial view presented as
    /// complete. Missing authority/dependencies yield None. The caller owns this scan's
    /// explicit work/storage allowance; no network hot-path boundedness is claimed here.
    pub fn projection(
        &self,
        create: ActionId,
        budget: usize,
    ) -> Result<Option<MessageProjection>, ChatError> {
        let original = self.entries.get(&create).ok_or(ChatError::Unknown)?;
        match original.state {
            ActionState::Pending(_) => return Ok(None),
            ActionState::Invalid(error) => return Err(error),
            ActionState::Valid => {}
        }
        let Body::Create { text, reply_to } = &original.action.spec().body else {
            return Err(ChatError::Dependency);
        };
        let mut work = Work(budget);
        let mut edits = BTreeSet::new();
        let mut deleted_by = Vec::new();
        for (id, entry) in &self.entries {
            work.spend()?;
            if entry.state != ActionState::Valid
                || entry.action.spec().body.create() != Some(create)
            {
                continue;
            }
            match entry.action.spec().body {
                Body::Edit { .. } => {
                    edits.insert(*id);
                }
                Body::Delete { .. } => deleted_by.push(*id),
                _ => {}
            }
        }
        let mut superseded = BTreeSet::new();
        for edit in &edits {
            // Iterative bounded traversal. A Join carries knowledge but is not itself an
            // edit: only a later authorized Edit supersedes its ancestral edit alternatives.
            let mut stack = self.entries[edit].action.spec().parents.clone();
            let mut visited = BTreeSet::new();
            while let Some(parent) = stack.pop() {
                work.spend()?;
                if !visited.insert(parent) {
                    continue;
                }
                if edits.contains(&parent) {
                    superseded.insert(parent);
                }
                let ancestor = &self.entries[&parent].action;
                // This Create is the boundary of the message's edit history. Its
                // optional reply names another message, whose history cannot contain
                // an edit of this not-yet-existing content-addressed Create. Following
                // that edge would charge every old reply against an unrelated local
                // edit and eventually make otherwise small projections fail.
                if !matches!(ancestor.spec().body, Body::Create { .. }) {
                    stack.extend(ancestor.spec().parents.iter().copied());
                }
            }
        }
        let alternatives: Vec<_> = edits
            .difference(&superseded)
            .map(|id| {
                let action = &self.entries[id].action;
                let Body::Edit { text, .. } = &action.spec().body else {
                    unreachable!("edit index");
                };
                EditAlternative {
                    action: *id,
                    text: text.clone(),
                    presented_ms: action.spec().presented_ms,
                }
            })
            .collect();
        let visible_text = deleted_by.is_empty().then(|| {
            alternatives
                .last()
                .map_or_else(|| text.clone(), |edit| edit.text.clone())
        });
        Ok(Some(MessageProjection {
            create,
            author: *original.action.signer(),
            presented_ms: original.action.spec().presented_ms,
            reply_to: *reply_to,
            visible_text,
            alternatives,
            deleted_by,
        }))
    }
    /// ALL valid per-message causal heads. More than MAX_PARENTS is legal: compose bounded
    /// Join actions over these heads. Do not silently truncate this set to fit a new edit.
    pub fn causal_heads(
        &self,
        create: ActionId,
        budget: usize,
    ) -> Result<Vec<ActionId>, ChatError> {
        let mut work = Work(budget);
        let mut heads = BTreeSet::new();
        let mut parents = BTreeSet::new();
        for (id, entry) in &self.entries {
            work.spend()?;
            if entry.state == ActionState::Valid
                && (*id == create || entry.action.spec().body.create() == Some(create))
            {
                heads.insert(*id);
                for parent in &entry.action.spec().parents {
                    work.spend()?;
                    parents.insert(*parent);
                }
            }
        }
        Ok(heads.difference(&parents).copied().collect())
    }
}
