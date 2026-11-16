//! Bounded native traversal of the current MLS roster. A connected-peer snapshot or evictable
//! descriptor cache is not a complete source catalog. These views expose only public member
//! keys; they grant no file, route, native-session or local-membership authority.
//!
//! OpenMLS `member_at` clones both keys and the credential. Before using it on a request path,
//! an existing native epoch observation attests that ALL key shapes fit the closed bound, and
//! each lookup checks its credential through the borrowed accessor. Native loading and that
//! existing O(roster) observation are outside the bounded-page claim.
use super::ServerGroup;
use catcoms_crypto::DeviceId;
use openmls::prelude::LeafNodeIndex;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

/// Raw leaf positions examined in one page, including holes and excluded members.
pub const MAX_MEMBER_CATALOG_PROBES: usize = 64;
/// Credential bytes accepted by one catalog lookup. Larger native shapes refuse before
/// OpenMLS copies them; this is a catalog limit, not a new MLS membership/admission rule.
pub const MAX_MEMBER_CATALOG_CREDENTIAL_BYTES: usize = 4 * 1024;
const KEY_BYTES: usize = 32;
const PAGE_MEMBERS: usize = 4;

#[derive(Default)]
pub(super) struct CatalogCache {
    instance: Arc<()>,
    observed: Mutex<Option<Arc<Summary>>>,
}
struct Summary {
    instance: Weak<()>,
    epoch: u64,
    members: usize,
    last_leaf: Option<u32>,
    keys_supported: bool,
}

/// Immutable native observation of one exact group object and committed epoch. It cannot be
/// constructed or deserialized by a renderer/peer. Refreshing it is native roster work, not a
/// page operation. An unsupported observation still reports its actual member count.
#[derive(Clone)]
pub struct MemberCatalogView(Arc<Summary>);
impl fmt::Debug for MemberCatalogView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemberCatalogView")
            .field("epoch", &self.0.epoch)
            .field("members", &self.0.members)
            .field("keys_supported", &self.0.keys_supported)
            .finish_non_exhaustive()
    }
}

/// A position within one immutable observation. Clone it to propose a page; callers retain the
/// original cursor until local preparation is admitted, so Busy cannot consume source coverage.
#[derive(Clone, Debug)]
pub struct MemberCatalogCursor {
    view: MemberCatalogView,
    next: u64,
    occupied: usize,
}

/// One indexed public signing key, bound to its original native group observation. A requester
/// must revalidate it at preparation; a source needs no direct connection to that requester.
#[derive(Clone)]
pub struct MemberCatalogEntry {
    view: MemberCatalogView,
    leaf: u32,
    device: DeviceId,
    key: [u8; KEY_BYTES],
}
impl fmt::Debug for MemberCatalogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemberCatalogEntry")
            .field("leaf", &self.leaf)
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}
impl MemberCatalogEntry {
    pub fn device_id(&self) -> DeviceId {
        self.device
    }
    pub fn signature_key(&self) -> &[u8; KEY_BYTES] {
        &self.key
    }
    /// Recheck through the same bounded indexed accessor, without scanning the roster or
    /// treating a credential's contents as the member's actual signing key.
    pub fn validate(&self, group: &ServerGroup) -> Result<(), MemberCatalogError> {
        let current = self.view.entry(group, self.leaf, &mut || {})?;
        if current.device != self.device || current.key != self.key {
            return Err(MemberCatalogError::Stale);
        }
        Ok(())
    }
}

/// At most four sorted distinct sources, and a proposed next cursor. Empty plus `!complete`
/// means Yield: holes or excluded members consumed this bounded turn, not evidence of absence.
#[derive(Debug)]
pub struct MemberCatalogPage {
    pub entries: Vec<MemberCatalogEntry>,
    pub next: MemberCatalogCursor,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MemberCatalogError {
    #[error("member catalog belongs to an earlier group or epoch")]
    Stale,
    #[error("member catalog contains an unsupported native member shape")]
    Unsupported,
    #[error("member catalog does not match its native observation")]
    Inconsistent,
}

impl ServerGroup {
    /// Observe the complete native roster once per epoch, retaining count, last occupied leaf
    /// and a key-copy bound in fixed metadata. This reuses `member_count`'s existing O(roster)
    /// traversal. Call during native construction/membership reconciliation, never to repair a
    /// missing view on an untrusted request/page path. This does not bound MLS load/observation.
    pub fn observe_member_catalog(&self) -> MemberCatalogView {
        if let Some(current) = self.current_member_catalog() {
            return current;
        }
        let mut summary = Summary {
            instance: Arc::downgrade(&self.catalog.instance),
            epoch: self.epoch(),
            members: 0,
            last_leaf: None,
            keys_supported: true,
        };
        for member in self.group.members() {
            summary.members += 1;
            summary.last_leaf = Some(summary.last_leaf.unwrap_or(0).max(member.index.u32()));
            summary.keys_supported &=
                member.signature_key.len() == KEY_BYTES && member.encryption_key.len() == KEY_BYTES;
        }
        let view = MemberCatalogView(Arc::new(summary));
        if let Ok(mut observed) = self.catalog.observed.lock() {
            *observed = Some(view.0.clone());
        }
        view
    }

    /// Read an already observed, exact-current native summary. None never triggers a scan and
    /// must mean defer/unavailable to a request path. Epoch changes fail closed even if a merge
    /// succeeded before a caller returned a later serialization error.
    pub fn current_member_catalog(&self) -> Option<MemberCatalogView> {
        let current = self.catalog.observed.lock().ok()?.clone()?;
        let view = MemberCatalogView(current);
        view.check(self).ok()?;
        Some(view)
    }
}

impl MemberCatalogView {
    /// Check only the exact native group instance and committed epoch, without a roster scan.
    /// This grants no active-membership, native-session, file or transport authority.
    pub fn validate(&self, group: &ServerGroup) -> Result<(), MemberCatalogError> {
        self.check(group)
    }

    pub fn epoch(&self) -> u64 {
        self.0.epoch
    }
    pub fn member_count(&self) -> usize {
        self.0.members
    }
    pub fn start(&self) -> MemberCatalogCursor {
        MemberCatalogCursor {
            view: self.clone(),
            next: 0,
            occupied: 0,
        }
    }
    fn check(&self, group: &ServerGroup) -> Result<(), MemberCatalogError> {
        if self.0.epoch != group.epoch()
            || !Weak::ptr_eq(&self.0.instance, &Arc::downgrade(&group.catalog.instance))
        {
            return Err(MemberCatalogError::Stale);
        }
        Ok(())
    }
    fn entry(
        &self,
        group: &ServerGroup,
        leaf: u32,
        before_copy: &mut impl FnMut(),
    ) -> Result<MemberCatalogEntry, MemberCatalogError> {
        self.check(group)?;
        if !self.0.keys_supported {
            return Err(MemberCatalogError::Unsupported);
        }
        let index = LeafNodeIndex::new(leaf);
        let credential = group
            .group
            .member(index)
            .ok_or(MemberCatalogError::Inconsistent)?;
        if credential.serialized_content().len() > MAX_MEMBER_CATALOG_CREDENTIAL_BYTES {
            return Err(MemberCatalogError::Unsupported);
        }
        // No await/mutation separates the borrowed size check from the indexed copy. The opaque
        // exact-epoch summary attests BOTH other copied vectors before OpenMLS allocates them.
        before_copy();
        let member = group
            .group
            .member_at(index)
            .ok_or(MemberCatalogError::Inconsistent)?;
        let key: [u8; KEY_BYTES] = member
            .signature_key
            .as_slice()
            .try_into()
            .map_err(|_| MemberCatalogError::Inconsistent)?;
        if member.encryption_key.len() != KEY_BYTES {
            return Err(MemberCatalogError::Inconsistent);
        }
        Ok(MemberCatalogEntry {
            view: self.clone(),
            leaf,
            device: DeviceId::from_public_key_bytes(&key),
            key,
        })
    }
    /// Examine at most64 raw leaf positions and return at most four distinct sources. Exclusion
    /// is fixed at the native requester and helper identities, not an unbounded caller list.
    /// Occupied leaves count BEFORE exclusion so C/B and holes cannot prevent completion. No
    /// cursor is mutated: keep the proposed page across proven local pre-handoff Busy.
    pub fn page(
        &self,
        group: &ServerGroup,
        cursor: &MemberCatalogCursor,
        requester: DeviceId,
        helper: DeviceId,
    ) -> Result<MemberCatalogPage, MemberCatalogError> {
        self.page_with(group, cursor, requester, helper, &mut || {})
    }
    fn page_with(
        &self,
        group: &ServerGroup,
        cursor: &MemberCatalogCursor,
        requester: DeviceId,
        helper: DeviceId,
        before_copy: &mut impl FnMut(),
    ) -> Result<MemberCatalogPage, MemberCatalogError> {
        self.check(group)?;
        if !Arc::ptr_eq(&self.0, &cursor.view.0) {
            return Err(MemberCatalogError::Stale);
        }
        if !self.0.keys_supported {
            return Err(MemberCatalogError::Unsupported);
        }
        let mut next = cursor.clone();
        let mut entries = Vec::with_capacity(PAGE_MEMBERS);
        let end = self.0.last_leaf.map_or(0, |last| u64::from(last) + 1);
        for _ in 0..MAX_MEMBER_CATALOG_PROBES {
            if next.next >= end || entries.len() == PAGE_MEMBERS {
                break;
            }
            let leaf = next.next as u32; // less than the attested u32 last leaf +1
            next.next += 1;
            if group.group.member(LeafNodeIndex::new(leaf)).is_none() {
                continue;
            }
            next.occupied += 1;
            if next.occupied > self.0.members {
                return Err(MemberCatalogError::Inconsistent);
            }
            let entry = self.entry(group, leaf, before_copy)?;
            if entry.device != requester
                && entry.device != helper
                && !entries
                    .iter()
                    .any(|old: &MemberCatalogEntry| old.device == entry.device)
            {
                entries.push(entry);
            }
        }
        let complete = next.next >= end;
        if complete && next.occupied != self.0.members {
            return Err(MemberCatalogError::Inconsistent);
        }
        entries.sort_unstable_by_key(|entry| entry.device);
        Ok(MemberCatalogPage {
            entries,
            next,
            complete,
        })
    }
}

#[cfg(test)]
mod tests;
