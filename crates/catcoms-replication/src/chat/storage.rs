use super::*;
use catcoms_wire::{Decoder, Encoder};
use std::collections::BTreeMap;

const SNAPSHOT_DOMAIN: &[u8] = b"mewtual/chat/snapshot/v1\0";
const PAGE_DOMAIN: &[u8] = b"mewtual/chat/page/v1\0";
pub const MAX_PAGE_RECORDS: usize = 64;
pub const MAX_PAGE_BYTES: usize = 256 * 1024;

/// Immutable evidence snapshot. It contains no verified authority tokens/status bits.
/// This is plaintext and MUST be sealed by any future durable native storage adapter.
/// Capture/decode scans the configured retained evidence, not a bounded network turn.
pub struct ChatSnapshot {
    realm: Realm,
    actions: Vec<SignedAction>,
    id: [u8; 32],
}
impl std::fmt::Debug for ChatSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatSnapshot")
            .field("actions", &self.actions.len())
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageCursor {
    snapshot: [u8; 32],
    after: ActionId,
}
pub struct Page {
    realm: RealmId,
    snapshot: [u8; 32],
    actions: Vec<SignedAction>,
    next: Option<PageCursor>,
}
impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("actions", &self.actions.len())
            .field("continued", &self.next.is_some())
            .finish_non_exhaustive()
    }
}
impl Page {
    pub fn actions(&self) -> &[SignedAction] {
        &self.actions
    }
    pub fn next(&self) -> Option<PageCursor> {
        self.next
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(PAGE_DOMAIN).expect("fixed domain");
        e.put_bytes(self.realm.as_bytes()).expect("fixed realm");
        e.put_bytes(&self.snapshot).expect("fixed snapshot");
        e.put_u16(self.actions.len() as u16);
        for action in &self.actions {
            e.put_bytes(&action.encode()).expect("bounded action");
        }
        e.put_u8(u8::from(self.next.is_some()));
        if let Some(cursor) = self.next {
            e.put_bytes(cursor.after.as_bytes()).expect("fixed cursor");
        }
        e.finish()
    }
    /// Decoding is syntax only, not signature/authentication. Import verifies each original
    /// action. A bad tail must not advance the caller's page continuation on partial import.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_PAGE_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != PAGE_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let realm = RealmId(
            d.get_bytes()?
                .try_into()
                .map_err(|_| ChatError::Malformed)?,
        );
        let snapshot = d
            .get_bytes()?
            .try_into()
            .map_err(|_| ChatError::Malformed)?;
        let count = usize::from(d.get_u16()?);
        if count > MAX_PAGE_RECORDS {
            return Err(ChatError::Bound);
        }
        let mut actions = Vec::with_capacity(count);
        for _ in 0..count {
            actions.push(SignedAction::decode(d.get_bytes()?)?);
        }
        check_order(&actions, Some(realm))?;
        let next = match d.get_u8()? {
            0 => None,
            1 => Some(PageCursor {
                snapshot,
                after: ActionId(
                    d.get_bytes()?
                        .try_into()
                        .map_err(|_| ChatError::Malformed)?,
                ),
            }),
            _ => return Err(ChatError::Malformed),
        };
        d.finish()?;
        if next.is_some_and(|cursor| actions.last().is_none_or(|last| cursor.after != last.id())) {
            return Err(ChatError::Malformed);
        }
        Ok(Self {
            realm,
            snapshot,
            actions,
            next,
        })
    }
}
impl ChatSnapshot {
    pub fn realm(&self) -> &Realm {
        &self.realm
    }
    pub fn id(&self) -> [u8; 32] {
        self.id
    }
    pub fn encode(&self) -> Vec<u8> {
        encode_snapshot(&self.realm, &self.actions)
    }
    pub fn decode(bytes: &[u8], limits: LedgerLimits) -> Result<Self, ChatError> {
        // Bound framing overhead separately from the canonical action-byte allowance.
        let maximum = limits
            .max_actions
            .checked_mul(4)
            .and_then(|n| n.checked_add(limits.max_action_bytes))
            .and_then(|n| n.checked_add(512))
            .ok_or(ChatError::Bound)?;
        if bytes.len() > maximum {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != SNAPSHOT_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let realm = Realm::decode_from(&mut d)?;
        let count = usize::try_from(d.get_u32()?).map_err(|_| ChatError::Bound)?;
        if count > limits.max_actions {
            return Err(ChatError::Capacity);
        }
        // Each action needs at least its four-byte outer length prefix. An
        // advertised count cannot reserve memory before its frame even exists.
        if count > d.remaining() / 4 {
            return Err(ChatError::Malformed);
        }
        // Grow only after a complete bounded action has decoded. A large trusted
        // count allowance must not turn a short malformed frame into a huge allocation.
        let mut actions = Vec::new();
        let mut total = 0usize;
        for _ in 0..count {
            let bytes = d.get_bytes()?;
            total = total.checked_add(bytes.len()).ok_or(ChatError::Capacity)?;
            if total > limits.max_action_bytes {
                return Err(ChatError::Capacity);
            }
            actions.push(SignedAction::decode(bytes)?);
        }
        d.finish()?;
        check_order(&actions, Some(realm.id()))?;
        Ok(Self {
            realm,
            actions,
            id: *blake3::hash(bytes).as_bytes(),
        })
    }
    /// CID-order pages from an immutable inventory. Cursors bind this exact snapshot hash,
    /// so import/restart/provider changes cannot silently move a cursor across inventories.
    /// Page construction examines at most the requested record/byte allowance.
    pub fn page(
        &self,
        cursor: Option<PageCursor>,
        records: usize,
        bytes: usize,
    ) -> Result<Page, ChatError> {
        if records == 0 || records > MAX_PAGE_RECORDS || bytes > MAX_PAGE_BYTES {
            return Err(ChatError::Bound);
        }
        if cursor.is_some_and(|cursor| cursor.snapshot != self.id) {
            return Err(ChatError::Scope);
        }
        let start = cursor.map_or(0, |cursor| {
            self.actions
                .partition_point(|action| action.id() <= cursor.after)
        });
        let mut actions = Vec::new();
        let mut used = 256usize;
        let mut position = start;
        while position < self.actions.len() && actions.len() < records {
            let action = &self.actions[position];
            let cost = action
                .encode()
                .len()
                .checked_add(4)
                .ok_or(ChatError::Bound)?;
            if used.checked_add(cost).is_none_or(|total| total > bytes) {
                break;
            }
            used += cost;
            actions.push(action.clone());
            position += 1;
        }
        if position < self.actions.len() && actions.is_empty() {
            return Err(ChatError::Capacity);
        }
        let next = (position < self.actions.len()).then(|| PageCursor {
            snapshot: self.id,
            after: actions.last().expect("nonempty continuation").id(),
        });
        let page = Page {
            realm: self.realm.id(),
            snapshot: self.id,
            actions,
            next,
        };
        if page.encode().len() > bytes {
            return Err(ChatError::Bound);
        }
        Ok(page)
    }
}
fn check_order(actions: &[SignedAction], realm: Option<RealmId>) -> Result<(), ChatError> {
    if actions.windows(2).any(|pair| pair[0].id() >= pair[1].id())
        || actions
            .iter()
            .any(|action| realm.is_some_and(|realm| action.spec().realm.id() != realm))
    {
        return Err(ChatError::Malformed);
    }
    Ok(())
}
fn encode_snapshot(realm: &Realm, actions: &[SignedAction]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(SNAPSHOT_DOMAIN).expect("fixed domain");
    realm.encode_into(&mut e);
    e.put_u32(u32::try_from(actions.len()).expect("validated snapshot action count"));
    for action in actions {
        e.put_bytes(&action.encode()).expect("bounded action");
    }
    e.finish()
}
impl ChatLedger {
    pub fn snapshot(&self) -> Result<ChatSnapshot, ChatError> {
        if self.entries.len() > u32::MAX as usize {
            return Err(ChatError::Bound);
        }
        let actions: Vec<_> = self
            .entries
            .values()
            .map(|entry| entry.action.clone())
            .collect();
        let bytes = encode_snapshot(&self.realm, &actions);
        Ok(ChatSnapshot {
            realm: self.realm.clone(),
            actions,
            id: *blake3::hash(&bytes).as_bytes(),
        })
    }
    /// No stored state or flag can mint authority. Without matching external witnesses ALL
    /// restored evidence is pending until explicitly authorized/resolved again.
    /// Selected coverage stays bound to each action, including still-missing parents.
    /// Unrelated actions in one basis must not share one capped coverage witness.
    pub fn restore(
        bytes: &[u8],
        limits: LedgerLimits,
        witnesses: &[(ActionId, VerifiedAuthorityBasis)],
    ) -> Result<Self, ChatError> {
        let snapshot = ChatSnapshot::decode(bytes, limits)?;
        if witnesses.len() > limits.max_actions {
            return Err(ChatError::Bound);
        }
        let mut ledger = Self::new(snapshot.realm, limits)?;
        for action in snapshot.actions {
            ledger.import(action)?;
        }
        let mut rights_by_grant = BTreeMap::new();
        for (id, witness) in witnesses {
            if witness.realm != ledger.realm.id() {
                return Err(ChatError::Authority);
            }
            let key = (witness.basis, witness.grant, witness.subject);
            // Selected coverage may differ; contradictory rights for the SAME
            // exact grant may not choose a winner through witness arrival order.
            if rights_by_grant
                .insert(key, witness.rights)
                .is_some_and(|original| original != witness.rights)
            {
                return Err(ChatError::AuthorityConflict);
            }
            ledger.authorize(*id, witness.clone())?;
        }
        Ok(ledger)
    }
    /// A verified prefix remains retained if a later action fails; commit the caller's
    /// continuation only after success. Retrying the SAME page deduplicates that prefix.
    pub fn import_page(&mut self, page: &Page) -> Result<usize, ChatError> {
        if page.realm != self.realm.id() {
            return Err(ChatError::Scope);
        }
        let mut retained = 0;
        for action in &page.actions {
            if matches!(self.import(action.clone())?, ImportOutcome::Retained(_)) {
                retained += 1;
            }
        }
        Ok(retained)
    }
}
