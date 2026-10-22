//! Product-defined bounded room visibility with irreversible native directory incarnations.
//! Schema stays in catcoms-app. Sync invokes this owner immediately after EACH mutation attempt,
//! including prefix-applying errors, before publication/await. Observation is not participation.

use super::helper::RoomAuthority;
use super::resources::{Credit, ResourceKind, Resources};
use super::runtime::ActorOwner;
use super::{CallLease, Scope, VoiceError, WatchLease};
use automerge::AutoCommit;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

pub const MAX_DIRECTORY_ROOMS: usize = 256;
pub const MAX_DIRECTORY_KEYS: usize = 512;
const ROOM_CHILDREN: usize = 64;

/// The reader must bound traversal/allocation before returning and explicitly state missing-doc
/// and permanent compatibility-room semantics. Overflow/read failure is a refusal, not truncation.
pub type DirectoryReader = fn(Option<&AutoCommit>) -> Result<BTreeSet<u128>, VoiceError>;
pub type DeletionReader = fn(&automerge::Change) -> Result<BTreeSet<u128>, VoiceError>;
struct Room {
    live: CallLease,
    children: Mutex<Vec<CallLease>>,
    _credit: Credit,
}
impl Room {
    fn revoke(&self) {
        self.live.revoke();
        let mut children = self.children.lock().unwrap_or_else(|e| e.into_inner());
        for child in children.iter() {
            child.revoke();
        }
        children.clear();
    }
}
impl Drop for Room {
    fn drop(&mut self) {
        self.revoke();
    }
}
pub struct Directory {
    group: [u8; 32],
    resources: Arc<Resources>,
    reader: DirectoryReader,
    deletions: DeletionReader,
    rooms: BTreeMap<u128, Arc<Room>>,
    faulted: bool,
}
redacted_debug!(Room, Directory);
impl Drop for Directory {
    fn drop(&mut self) {
        self.close();
    }
}
impl Directory {
    pub fn new(
        group: [u8; 32],
        resources: Arc<Resources>,
        reader: DirectoryReader,
        deletions: DeletionReader,
    ) -> Self {
        Self {
            group,
            resources,
            reader,
            deletions,
            rooms: BTreeMap::new(),
            faulted: true,
        }
    }
    fn close(&mut self) {
        for room in self.rooms.values() {
            room.revoke();
        }
        self.rooms.clear();
        self.faulted = true;
    }
    /// Only the replication owner's newly accepted authenticated changes may supply `Some`.
    /// Duplicate/rejected historical deletion must never retire a replacement room. Buffered
    /// changes and deletion/recreation inside one accepted change still retire its exact owner.
    pub fn observe_change(
        &mut self,
        doc: Option<&AutoCommit>,
        change: Option<&automerge::Change>,
    ) -> Result<(), VoiceError> {
        if let Some(change) = change {
            let deleted = match (self.deletions)(change) {
                Ok(deleted) if deleted.len() <= MAX_DIRECTORY_ROOMS => deleted,
                Ok(_) => {
                    self.close();
                    return Err(VoiceError::Capacity);
                }
                Err(error) => {
                    self.close();
                    return Err(error);
                }
            };
            for id in deleted {
                if let Some(room) = self.rooms.remove(&id) {
                    room.revoke();
                }
            }
        }
        self.observe(doc)
    }
    /// Finally-style observation. Call even when the surrounding operation reports Err: legacy
    /// edits/imports can have changed the document before that error. A valid later snapshot can
    /// reopen fresh owners; old clones/child leases remain permanently dead.
    pub fn observe(&mut self, doc: Option<&AutoCommit>) -> Result<(), VoiceError> {
        let snapshot = match (self.reader)(doc) {
            Ok(snapshot) if snapshot.len() <= MAX_DIRECTORY_ROOMS => snapshot,
            Ok(_) => {
                self.close();
                return Err(VoiceError::Capacity);
            }
            Err(error) => {
                self.close();
                return Err(error);
            }
        };
        self.rooms.retain(|id, room| {
            if snapshot.contains(id) {
                true
            } else {
                room.revoke();
                false
            }
        });
        for id in snapshot {
            if !self.rooms.contains_key(&id) {
                let credit = match self
                    .resources
                    .charge(self.group, 1024, ResourceKind::Retained)
                {
                    Ok(credit) => credit,
                    Err(error) => {
                        self.close();
                        return Err(error);
                    }
                };
                self.rooms.insert(
                    id,
                    Arc::new(Room {
                        live: CallLease::default(),
                        children: Mutex::new(Vec::new()),
                        _credit: credit,
                    }),
                );
            }
        }
        self.faulted = false;
        Ok(())
    }
    pub fn authority(&self, scope: Scope) -> Result<RoomAuthority, VoiceError> {
        if self.faulted || scope.group != self.group {
            return Err(VoiceError::Unauthorized);
        }
        let room = self
            .rooms
            .get(&scope.room)
            .ok_or(VoiceError::Unauthorized)?;
        if !room.live.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(RoomAuthority {
            scope,
            lease: room.live.clone(),
            custody: Some(room.clone()),
        })
    }
    /// Attach participant/watch revocation to both the current actor/policy AND exact room. The
    /// caller still binds controller, consent and join identity. Room presence alone grants none.
    pub fn call_lease(&self, actor: &ActorOwner, scope: Scope) -> Result<CallLease, VoiceError> {
        if actor.group_hash() != self.group {
            return Err(VoiceError::Scope);
        }
        let authority = self.authority(scope)?;
        let child = actor.child_lease()?;
        let room = &self.rooms[&scope.room];
        let mut children = room.children.lock().unwrap_or_else(|e| e.into_inner());
        children.retain(CallLease::is_live);
        if children.len() >= ROOM_CHILDREN {
            child.revoke();
            return Err(VoiceError::Capacity);
        }
        if !authority.lease.is_live() {
            child.revoke();
            return Err(VoiceError::Unauthorized);
        }
        children.push(child.clone());
        Ok(child)
    }
    pub fn watch_lease(&self, actor: &ActorOwner, scope: Scope) -> Result<WatchLease, VoiceError> {
        self.call_lease(actor, scope).map(WatchLease::new)
    }
    /// Check both issuer registries, including exact room incarnation. CallGrant::from_lease is
    /// only a custody adapter; accepting its scope alone would admit foreign or orphan grants.
    pub fn validate_call_grant(
        &self,
        actor: &ActorOwner,
        grant: &super::CallGrant,
    ) -> Result<(), VoiceError> {
        let scope = grant.scope();
        if actor.group_hash() != self.group {
            return Err(VoiceError::Scope);
        }
        actor.source_context(scope)?;
        self.authority(scope)?;
        let child = grant.lease();
        actor.recognizes_child(&child)?;
        let room = self
            .rooms
            .get(&scope.room)
            .ok_or(VoiceError::Unauthorized)?;
        let children = room.children.lock().unwrap_or_else(|e| e.into_inner());
        if !room.live.is_live()
            || !child.is_live()
            || !children.iter().any(|issued| issued.is_same(&child))
        {
            return Err(VoiceError::Unauthorized);
        }
        actor.recognizes_child(&child)
    }
    /// Actual asynchronous reply custody retains the room allocation and its own declared bytes
    /// through revocation/deletion. Room holds only lease handles, never this grant, so there is
    /// no ownership cycle. Charge first: a refusal must not issue an orphan live child.
    pub fn call_grant(
        &self,
        actor: &ActorOwner,
        scope: Scope,
    ) -> Result<super::CallGrant, VoiceError> {
        let credit = self
            .resources
            .charge(self.group, 256, ResourceKind::Retained)?;
        let room = self
            .rooms
            .get(&scope.room)
            .ok_or(VoiceError::Unauthorized)?
            .clone();
        let lease = self.call_lease(actor, scope)?;
        Ok(super::CallGrant::from_lease(lease, scope).with_custody(room, credit))
    }
}

#[cfg(test)]
mod tests;
