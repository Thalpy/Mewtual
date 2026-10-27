//! Process-owned router mapping custody. IPC waiters and UI counters do not own router effects.
//!
//! A controller identifies a webview mount, an owner identifies one call join, and an attempt
//! identifies one physical UDP port operation. Slots survive cancellation through real cleanup.
//! Uncertain retirement is deliberately charged until process exit: the legacy PCP client has
//! no public drain barrier, and a timed-out IGD request may still have affected its router.
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

const MAX_CANDIDATES: usize = 8;
const MAX_PORT_SLOTS: usize = 64;
const CANDIDATE_TTL_MS: u64 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub generation: u64,
    pub server: u64,
    pub instance: u64,
    pub group: Vec<u8>,
    pub room: String,
}

#[derive(Clone, Debug)]
pub(crate) enum CandidateKind {
    Controller {
        generation: u64,
    },
    Call {
        controller: String,
        binding: Binding,
    },
}

#[derive(Clone)]
struct Candidate {
    revision: u64,
    expires: u64,
    kind: CandidateKind,
}

#[derive(Clone, Debug)]
struct Owner {
    id: String,
    binding: Binding,
}

enum PortState<G> {
    Acquiring,
    Granted(G),
    Cleaning,
    Uncertain,
}

struct Slot<G> {
    id: String,
    owner: String,
    state: PortState<G>,
}

/// Work returned by authority revocation. The caller starts a native-owned cleanup task, then
/// reports the exact result with `finish_cleanup`; moving a grant out does not refund its slot.
pub(crate) struct Cleanup<G> {
    pub id: String,
    pub grant: G,
}

pub(crate) enum Completion<G> {
    Installed,
    Cleanup(Cleanup<G>),
    Failed,
}

pub(crate) struct Coordinator<G> {
    serial: u64,
    revision: u64,
    controller: Option<(String, u64)>,
    owner: Option<Owner>,
    candidates: HashMap<String, Candidate>,
    ports: HashMap<u16, Slot<G>>,
}

/// Execute one admitted router acquisition independently of the command waiter. The authorize
/// callback returns a guard retained through installation (the production UI commit guard), not
/// just an earlier boolean. Cancelled/retired work retains custody through awaited cleanup.
pub(crate) fn spawn_acquisition<G, R, Guard, Acquire, Authorize, Auth, Project, Release, Retire>(
    coordinator: Arc<Mutex<Coordinator<G>>>,
    id: String,
    acquire: Acquire,
    authorize: Authorize,
    project: Project,
    release: Release,
) -> tokio::task::JoinHandle<Result<R, String>>
where
    G: Send + 'static,
    R: Send + 'static,
    Guard: Send + 'static,
    Acquire: Future<Output = Result<G, (bool, String)>> + Send + 'static,
    Authorize: FnOnce() -> Auth + Send + 'static,
    Auth: Future<Output = Option<Guard>> + Send + 'static,
    Project: FnOnce(&G, &str) -> R + Send + 'static,
    Release: FnOnce(G) -> Retire + Send + 'static,
    Retire: Future<Output = bool> + Send + 'static,
{
    tokio::spawn(async move {
        let acquired = acquire.await;
        let response = acquired
            .as_ref()
            .map(|grant| project(grant, &id))
            .map_err(|(_, error)| error.clone());
        let authorization = authorize().await;
        let completion = coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .complete(
                &id,
                acquired.map_err(|(uncertain, _)| uncertain),
                authorization.is_some(),
            );
        drop(authorization);
        match completion {
            Completion::Installed | Completion::Failed => response,
            Completion::Cleanup(cleanup) => {
                let reusable = release(cleanup.grant).await;
                coordinator
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .finish_cleanup(&cleanup.id, reusable);
                Err("call mapping owner changed while the router was responding".into())
            }
        }
    })
}

impl<G> Default for Coordinator<G> {
    fn default() -> Self {
        Self {
            serial: 0,
            revision: 0,
            controller: None,
            owner: None,
            candidates: HashMap::new(),
            ports: HashMap::new(),
        }
    }
}

impl<G> Coordinator<G> {
    fn next_id(&mut self) -> Result<String, String> {
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or("mapping identity exhausted")?;
        Ok(format!("{:016x}", self.serial))
    }

    pub fn prepare(&mut self, kind: CandidateKind, now: u64) -> Result<String, String> {
        self.candidates.retain(|_, c| c.expires > now);
        if let CandidateKind::Call {
            controller,
            binding,
        } = &kind
        {
            if self.controller.as_ref() != Some(&(controller.clone(), binding.generation)) {
                return Err("call mapping controller changed".into());
            }
        }
        if self.candidates.len() >= MAX_CANDIDATES {
            return Err("too many pending call mapping joins".into());
        }
        let id = self.next_id()?;
        self.candidates.insert(
            id.clone(),
            Candidate {
                revision: self.revision,
                expires: now.saturating_add(CANDIDATE_TTL_MS),
                kind,
            },
        );
        Ok(id)
    }

    pub fn candidate(&self, id: &str, now: u64) -> Option<CandidateKind> {
        self.candidates
            .get(id)
            .filter(|c| c.expires > now)
            .map(|c| c.kind.clone())
    }

    /// Compare-and-swap activation. Preparing A never revokes B; a delayed activation prepared
    /// before B became current cannot revoke it either. A refused candidate is single-use.
    pub fn activate(&mut self, id: &str, now: u64) -> Result<Vec<Cleanup<G>>, String> {
        let candidate = self
            .candidates
            .remove(id)
            .ok_or("call mapping candidate expired")?;
        if candidate.expires <= now || candidate.revision != self.revision {
            return Err("call mapping join was superseded".into());
        }
        if let CandidateKind::Call {
            controller,
            binding,
        } = &candidate.kind
        {
            if self.controller.as_ref() != Some(&(controller.clone(), binding.generation)) {
                return Err("call mapping controller changed".into());
            }
        }
        self.bump_revision()?;
        let cleanup = self.retire_owner();
        match candidate.kind {
            CandidateKind::Controller { generation } => {
                self.controller = Some((id.into(), generation))
            }
            CandidateKind::Call { binding, .. } => {
                self.owner = Some(Owner {
                    id: id.into(),
                    binding,
                })
            }
        }
        Ok(cleanup)
    }

    fn bump_revision(&mut self) -> Result<(), String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("mapping revision exhausted")?;
        Ok(())
    }

    fn retire_owner(&mut self) -> Vec<Cleanup<G>> {
        let Some(owner) = self.owner.take() else {
            return Vec::new();
        };
        let mut cleanup = Vec::new();
        for slot in self.ports.values_mut().filter(|s| s.owner == owner.id) {
            // Acquisitions stay in their slots until their native worker completes. Already
            // cleaning/uncertain slots keep their charge, even across repeated End/lock.
            if matches!(slot.state, PortState::Granted(_)) {
                if let PortState::Granted(grant) =
                    std::mem::replace(&mut slot.state, PortState::Cleaning)
                {
                    cleanup.push(Cleanup {
                        id: slot.id.clone(),
                        grant,
                    });
                }
            }
        }
        cleanup
    }

    pub fn end(&mut self, id: &str) -> Vec<Cleanup<G>> {
        self.candidates.remove(id);
        if self
            .controller
            .as_ref()
            .is_some_and(|(current, _)| current == id)
        {
            self.controller = None;
            self.candidates.clear();
        } else if self.owner.as_ref().is_none_or(|o| o.id != id) {
            return Vec::new();
        }
        // Saturation is fail-closed: no older candidate remains valid if the revision exhausts.
        if self.bump_revision().is_err() {
            self.candidates.clear();
        }
        self.retire_owner()
    }

    pub fn revoke_all(&mut self) -> Vec<Cleanup<G>> {
        self.controller = None;
        self.candidates.clear();
        let _ = self.bump_revision();
        self.retire_owner()
    }

    pub fn retire_server(&mut self, server: u64, instance: u64) -> Vec<Cleanup<G>> {
        self.candidates.retain(|_, c| !matches!(&c.kind, CandidateKind::Call { binding, .. } if binding.server == server && binding.instance == instance));
        let id = self
            .owner
            .as_ref()
            .filter(|o| o.binding.server == server && o.binding.instance == instance)
            .map(|o| o.id.clone());
        id.map(|id| self.end(&id)).unwrap_or_default()
    }

    pub fn binding(&self, owner: &str) -> Option<&Binding> {
        self.owner
            .as_ref()
            .filter(|o| o.id == owner)
            .map(|o| &o.binding)
    }

    pub fn reserve(&mut self, owner: &str, port: u16) -> Result<String, String> {
        if port == 0 || self.binding(owner).is_none() {
            return Err("call mapping owner is no longer active".into());
        }
        if let Some(slot) = self.ports.get(&port) {
            return Err(if matches!(slot.state, PortState::Uncertain) {
                "router mapping cleanup is uncertain for this port"
            } else {
                "router mapping port is busy"
            }
            .into());
        }
        if self.ports.len() >= MAX_PORT_SLOTS {
            return Err("router mapping capacity reached; STUN and TURN remain available".into());
        }
        let id = self.next_id()?;
        self.ports.insert(
            port,
            Slot {
                id: id.clone(),
                owner: owner.into(),
                state: PortState::Acquiring,
            },
        );
        Ok(id)
    }

    pub fn complete(
        &mut self,
        id: &str,
        result: Result<G, bool>,
        authorized: bool,
    ) -> Completion<G> {
        let current = self.owner.as_ref().map(|o| o.id.as_str());
        let Some((&port, slot)) = self.ports.iter_mut().find(|(_, s)| s.id == id) else {
            // Only the admitted worker can complete its slot; slots are never removed while
            // acquisition runs. Treat a violation as a programming failure, not a leaked grant.
            panic!("mapping acquisition lost its charged slot");
        };
        assert!(matches!(slot.state, PortState::Acquiring));
        match result {
            Ok(grant) if authorized && current == Some(slot.owner.as_str()) => {
                slot.state = PortState::Granted(grant);
                Completion::Installed
            }
            Ok(grant) => {
                slot.state = PortState::Cleaning;
                Completion::Cleanup(Cleanup {
                    id: id.into(),
                    grant,
                })
            }
            Err(true) => {
                slot.state = PortState::Uncertain;
                Completion::Failed
            }
            Err(false) => {
                self.ports.remove(&port);
                Completion::Failed
            }
        }
    }

    pub fn unmap(&mut self, owner: &str, id: &str) -> Option<Cleanup<G>> {
        let slot = self
            .ports
            .values_mut()
            .find(|s| s.id == id && s.owner == owner)?;
        if !matches!(slot.state, PortState::Granted(_)) {
            return None;
        }
        if let PortState::Granted(grant) = std::mem::replace(&mut slot.state, PortState::Cleaning) {
            Some(Cleanup {
                id: id.into(),
                grant,
            })
        } else {
            unreachable!()
        }
    }

    pub fn finish_cleanup(&mut self, id: &str, reusable: bool) {
        let Some((&port, slot)) = self.ports.iter_mut().find(|(_, s)| s.id == id) else {
            return;
        };
        if !matches!(slot.state, PortState::Cleaning) {
            return;
        }
        if reusable {
            self.ports.remove(&port);
        } else {
            slot.state = PortState::Uncertain;
        }
    }
}

#[cfg(test)]
mod tests;
