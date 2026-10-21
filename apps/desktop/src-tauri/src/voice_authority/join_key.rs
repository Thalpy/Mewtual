//! Exact native admission for one browser-held key per accepted join. These fixed records own
//! accounting, not private keys. A lost reply cannot create another operation or let another
//! WebView adopt a generation whose original engine may still be running.
use super::*;
use serde::Serialize;

const MAX_GENERATIONS: usize = 32;
const GENERATION_BYTES: usize = 1024;
const GENERATE_BYTES: usize = 128;

pub(super) struct Generation {
    executor: [u8; 32],
    permit: String,
    join: Nonce,
    grant: std::sync::Weak<CallGrant>,
    _metadata: Credit,
    engine: Option<Credit>,
    settlement_requested: bool,
    point: Option<[u8; 65]>,
    publishing: bool,
    published: bool,
    drained: tokio::sync::watch::Sender<bool>,
}

/// All fields are public metadata. `closed` is history only and never permission to generate
/// again, even if a remounted browser no longer has the private key.
#[derive(Clone, Serialize)]
pub(crate) struct JoinKeyAdmission {
    pub owner: String,
    pub executor: String,
    pub permit: String,
    pub join: String,
    pub kind: &'static str,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JoinKeyPublication {
    pub owner: String,
    pub executor: String,
    pub permit: String,
    pub join: String,
    pub key_ref: String,
}

/// Native-owned publication carries the exact affine grant outside Coordinator. Its actual
/// actor turn and the UI commit guard must outlive an abandoned command/reply waiter.
pub(crate) struct JoinKeyWork {
    owner: Owner,
    executor: [u8; 32],
    point: [u8; 65],
    finished: bool,
}
impl JoinKeyWork {
    pub fn grant(&self) -> Arc<CallGrant> {
        self.owner._grant.clone()
    }
    pub fn join(&self) -> Nonce {
        self.owner.join
    }
    pub fn point(&self) -> [u8; 65] {
        self.point
    }
}
pub(crate) enum PublicationPlan {
    Accepted(JoinKeyPublication),
    Start(JoinKeyWork),
}

impl Coordinator {
    pub(super) fn prune_join_generations(&mut self) {
        let current = self.owner.as_ref().map(|owner| owner.id.as_str());
        self.join_generations.retain(|id, generation| {
            current == Some(id.as_str()) || generation.engine.is_some() || generation.publishing
        });
    }

    fn generation_row(
        &mut self,
        owner: &Owner,
        executor: [u8; 32],
        generate: bool,
    ) -> Result<Generation, String> {
        self.prune_join_generations();
        if self.join_generations.len() >= MAX_GENERATIONS {
            return Err("voice key generation capacity reached".into());
        }
        let group = owner.binding.scope.group;
        // Pay before inserting any record. Capacity refusal leaves this join untouched; credit
        // cleanup has no actor/policy callbacks and creates no new retry or engine authority.
        let metadata = self
            .resources
            .charge(group, GENERATION_BYTES, ResourceKind::Retained)
            .map_err(|e| e.to_string())?;
        let engine = if generate {
            Some(
                self.resources
                    .charge(group, GENERATE_BYTES, ResourceKind::Crypto)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        Ok(Generation {
            executor,
            permit: self.id()?,
            join: owner.join,
            grant: Arc::downgrade(&owner._grant),
            _metadata: metadata,
            engine,
            settlement_requested: !generate,
            point: None,
            publishing: false,
            published: false,
            drained: tokio::sync::watch::channel(!generate).0,
        })
    }

    pub fn admit_join_key(
        &mut self,
        owner_id: &str,
        executor: [u8; 32],
    ) -> Result<JoinKeyAdmission, String> {
        let owner = self.owner(owner_id).ok_or("voice owner changed")?;
        if !self.join_generations.contains_key(owner_id) {
            let generation = self.generation_row(&owner, executor, true)?;
            self.join_generations.insert(owner_id.into(), generation);
        }
        let generation = &self.join_generations[owner_id];
        if !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant)) {
            return Err("voice key belongs to another browser generation".into());
        }
        if generation.executor != executor {
            // A definite no-admission result is distinct from an uncertain transport reply.
            // It discloses none of the original executor/permit/join and cannot settle them.
            return Ok(JoinKeyAdmission {
                owner: owner_id.into(),
                executor: hex::encode(executor),
                permit: String::new(),
                join: String::new(),
                kind: "refused",
            });
        }
        Ok(JoinKeyAdmission {
            owner: owner_id.into(),
            executor: hex::encode(executor),
            permit: generation.permit.clone(),
            join: hex::encode(generation.join),
            kind: if generation.settlement_requested {
                "closed"
            } else {
                "admitted"
            },
        })
    }

    /// Exact settlement remains available after lock/End. Settlement before a delayed admission
    /// installs a closed current-owner row; a later admission cannot rearm it. Unknown retired
    /// owners need no tombstone: their native identity can never be activated again.
    pub fn settle_join_key(
        &mut self,
        owner_id: &str,
        executor: [u8; 32],
    ) -> Result<Option<tokio::sync::watch::Receiver<bool>>, String> {
        if !self.join_generations.contains_key(owner_id) {
            let Some(owner) = self.owner(owner_id) else {
                return Ok(None);
            };
            let generation = self.generation_row(&owner, executor, false)?;
            self.join_generations.insert(owner_id.into(), generation);
        }
        let generation = self
            .join_generations
            .get_mut(owner_id)
            .expect("generation inserted");
        if generation.executor != executor {
            return Err("voice key belongs to another browser generation".into());
        }
        generation.settlement_requested = true;
        // An already queued actor effect is real outstanding work too. Its native-owned worker
        // retires publishing before this exact execution charge can return.
        if generation.publishing {
            return Ok(Some(generation.drained.subscribe()));
        }
        generation.engine.take();
        generation.drained.send_replace(true);
        self.prune_join_generations();
        Ok(None)
    }

    pub fn prepare_join_key_publication(
        &mut self,
        owner_id: &str,
        executor: [u8; 32],
        permit: &str,
        point: [u8; 65],
    ) -> Result<PublicationPlan, String> {
        let owner = self.owner(owner_id).ok_or("voice owner changed")?;
        let generation = self
            .join_generations
            .get_mut(owner_id)
            .ok_or("voice key is unavailable")?;
        if generation.executor != executor
            || generation.permit != permit
            || !generation.grant.ptr_eq(&Arc::downgrade(&owner._grant))
            || point[0] != 4
            || generation.point.is_some_and(|original| original != point)
        {
            return Err("voice key publication changed".into());
        }
        if generation.published {
            return Ok(PublicationPlan::Accepted(generation.publication(owner_id)));
        }
        if generation.settlement_requested || generation.publishing {
            return Err("voice key publication is unavailable".into());
        }
        // Freeze the point BEFORE actor handoff: even an uncertain actor reply cannot substitute
        // another generated key under this join. Concurrent retries cannot start another worker.
        generation.point = Some(point);
        generation.publishing = true;
        Ok(PublicationPlan::Start(JoinKeyWork {
            owner,
            executor,
            point,
            finished: false,
        }))
    }

    /// Called by the native worker for both success and refusal. No IPC waiter owns this cleanup.
    pub fn finish_join_key_publication(
        &mut self,
        work: &mut JoinKeyWork,
        accepted: bool,
    ) -> Result<JoinKeyPublication, String> {
        // Consume this opaque work's one completion, but leave affine grant destruction with
        // its native worker after Coordinator unlock. Replaying an old completion cannot clear
        // a later retry's publishing flag or return that later worker's execution charge.
        if std::mem::replace(&mut work.finished, true) {
            return Err("voice key publication already finished".into());
        }
        let current = self
            .owner(&work.owner.id)
            .is_some_and(|owner| Arc::ptr_eq(&owner._grant, &work.owner._grant));
        let generation = self
            .join_generations
            .get_mut(&work.owner.id)
            .ok_or("voice key is unavailable")?;
        if generation.executor != work.executor || generation.point != Some(work.point) {
            return Err("voice key publication changed".into());
        }
        generation.publishing = false;
        generation.published |= accepted;
        if generation.settlement_requested {
            generation.engine.take();
            generation.drained.send_replace(true);
        }
        let reply = generation.publication(&work.owner.id);
        self.prune_join_generations();
        if accepted && current {
            Ok(reply)
        } else {
            Err("voice endpoint was not activated".into())
        }
    }
}
impl Generation {
    fn publication(&self, owner: &str) -> JoinKeyPublication {
        JoinKeyPublication {
            owner: owner.into(),
            executor: hex::encode(self.executor),
            permit: self.permit.clone(),
            join: hex::encode(self.join),
            key_ref: hex::encode(catcoms_sync::voice_v2::key_reference(
                &self.point.expect("published point"),
            )),
        }
    }
}
