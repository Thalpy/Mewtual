//! One protocol-support namespace per peer in the SAME admitted presence.
//! Explicit refresh closes q1 before installing q2, including when q2's source
//! queue later refuses capacity. Missing network replies never create owners.
use super::*;
use paid::{Payment, Retained};
use record::{Query, Role};
const QUERIES: usize = 32;
const TABLE_BYTES: usize = 4096;

/// Exact immutable local query identity. It cannot be decoded or constructed by
/// callers. Refresh accepts only the actual current original, never copied IDs.
#[derive(Clone)]
pub struct SupportQueryIdentity(pub(super) Owner);
#[cfg(test)]
impl SupportQueryIdentity {
    pub(crate) fn owner_for_test(&self) -> &Owner {
        &self.0
    }
}
#[derive(Clone)]
pub struct EndpointSupportQuery(SupportQueryIdentity);
#[derive(Clone)]
pub struct DirectHelperSupportProbe(SupportQueryIdentity);
impl EndpointSupportQuery {
    pub fn identity(&self) -> &SupportQueryIdentity {
        &self.0
    }
}
impl DirectHelperSupportProbe {
    pub fn identity(&self) -> &SupportQueryIdentity {
        &self.0
    }
}
/// Historical observation only. Zero/partial bits grant no reservation, key,
/// consent, Availability tuple or authority to start a native operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointSupportObservation {
    Unknown,
    Observed { features: u16 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectHelperSupportObservation {
    Unknown,
    Observed { features: u16 },
}

struct Table {
    rows: Mutex<Vec<Owner>>,
}
pub(crate) struct Queries(Retained<Table>);
impl Queries {
    pub(super) fn inventory_bytes() -> usize {
        2 * std::mem::size_of::<Table>() + 2 * QUERIES * std::mem::size_of::<Owner>() + 256
    }
    pub(crate) fn new(resources: &Arc<Resources>, group: Nonce) -> Result<Self, VoiceError> {
        if Self::inventory_bytes() > TABLE_BYTES {
            return Err(VoiceError::Bounds);
        }
        let payment = Payment::reserve(resources, group, TABLE_BYTES, ResourceKind::Retained)?;
        Ok(Self(payment.retain(Table {
            rows: Mutex::new(Vec::with_capacity(QUERIES)),
        })))
    }
    pub(crate) fn existing(
        &self,
        peer: DeviceId,
        helper: bool,
    ) -> Result<Option<Owner>, VoiceError> {
        let rows = self.0.get().rows.lock().unwrap_or_else(|e| e.into_inner());
        let Some(owner) = rows
            .iter()
            .find(|row| row.object().metadata().target() == peer)
        else {
            return Ok(None);
        };
        let Role::Query(query) = &owner.get().role else {
            return Err(VoiceError::Unauthorized);
        };
        if matches!(query.binding, Binding::DirectHelper(_)) != helper {
            return Err(VoiceError::RetryMismatch);
        }
        Ok(Some(owner.clone()))
    }
    pub(crate) fn next_ordinal(
        &self,
        peer: DeviceId,
        previous: Option<&SupportQueryIdentity>,
    ) -> Result<u64, VoiceError> {
        let rows = self.0.get().rows.lock().unwrap_or_else(|e| e.into_inner());
        match rows
            .iter()
            .find(|row| row.object().metadata().target() == peer)
        {
            Some(owner) => {
                if !previous.is_some_and(|old| owner.same(&old.0)) {
                    return Err(VoiceError::RetryMismatch);
                }
                let Role::Query(query) = &owner.get().role else {
                    return Err(VoiceError::Unauthorized);
                };
                query.ordinal.checked_add(1).ok_or(VoiceError::Capacity)
            }
            None if previous.is_none() && rows.len() < QUERIES => Ok(1),
            None if previous.is_none() => Err(VoiceError::Capacity),
            None => Err(VoiceError::RetryMismatch),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        actor: &ActorOwner,
        original: &super::super::call_activation::ActivationObserver,
        key: &JoinKey,
        context: &Context,
        room: RoomAuthority,
        binding: Binding,
        ordinal: u64,
        signer: &dyn VoiceSigner,
        rng: &mut dyn CryptoRngCore,
        resources: &Arc<Resources>,
        clock: Arc<dyn Clock>,
        peer: DeviceId,
    ) -> Result<Owner, VoiceError> {
        let payment = Owner::reserve(resources, context.scope.group)?;
        actor.admit_control_service()?;
        let issued = clock.now_ms();
        let mono = clock.monotonic_ms();
        let expires = issued
            .checked_add(record::LIFETIME_MS)
            .ok_or(VoiceError::Freshness)?;
        let deadline = mono
            .checked_add(record::LIFETIME_MS)
            .ok_or(VoiceError::Freshness)?;
        let effect = actor.effect(context.local, peer, expires)?;
        let epoch = actor.observe_epoch(context)?;
        let check = |wall, now| -> Result<(), VoiceError> {
            if !original.is_active()
                || !epoch.is_live()
                || !room.lease.is_live()
                || !effect.authorized_at(wall, now)
                || now >= deadline
            {
                return Err(VoiceError::Unauthorized);
            }
            match &binding {
                Binding::Endpoint(fresh) => fresh.authorize_at(context, wall, now),
                Binding::DirectHelper(route) if route.authorized() => Ok(()),
                Binding::DirectHelper(_) => Err(VoiceError::Unauthorized),
            }
        };
        check(clock.now_ms(), clock.monotonic_ms())?;
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        // A retained but expired watcher candidate must not cause even a paid
        // stale signature. RNG/Clock callbacks can close the original, so repeat
        // every pure binding after them and before the actual native signer.
        check(clock.now_ms(), clock.monotonic_ms())?;
        let object = Object::query(
            context.scope,
            context.epoch,
            peer,
            signer,
            id,
            issued,
            expires,
        )?;
        if object.metadata().issuer() != context.local {
            return Err(VoiceError::Unauthorized);
        }
        let owner = Owner::new(
            payment,
            object,
            Role::Query(Query {
                original: original.clone(),
                local_key: key.clone(),
                binding,
                ordinal,
            }),
            room,
            epoch,
            effect,
            clock,
            deadline,
        );
        if !owner.authorized() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(owner)
    }
    /// All preparation/signing is complete. Returns detached q1; the caller must
    /// release its native guards, notify q1, and only then publish q2 to Source.
    pub(crate) fn install_at(
        &self,
        prepared: &Owner,
        previous: Option<&SupportQueryIdentity>,
        wall: u64,
        mono: u64,
    ) -> Result<Option<Owner>, VoiceError> {
        if !prepared.authorized_at(wall, mono) {
            return Err(VoiceError::Unauthorized);
        }
        let mut rows = self.0.get().rows.lock().unwrap_or_else(|e| e.into_inner());
        let peer = prepared.object().metadata().target();
        if rows
            .iter()
            .any(|row| row.object().metadata().id() == prepared.object().metadata().id())
        {
            return Err(VoiceError::RetryMismatch);
        }
        if let Some(index) = rows
            .iter()
            .position(|row| row.object().metadata().target() == peer)
        {
            if !previous.is_some_and(|old| rows[index].same(&old.0)) {
                return Err(VoiceError::RetryMismatch);
            }
            let old = std::mem::replace(&mut rows[index], prepared.clone());
            old.invalidate();
            Ok(Some(old))
        } else if previous.is_none() && rows.len() < QUERIES {
            rows.push(prepared.clone());
            Ok(None)
        } else {
            Err(VoiceError::Capacity)
        }
    }
    pub(crate) fn current(&self, id: &SupportQueryIdentity) -> Result<Owner, VoiceError> {
        self.0
            .get()
            .rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|row| row.same(&id.0))
            .cloned()
            .ok_or(VoiceError::RetryMismatch)
    }
    pub(crate) fn pending(&self) -> Vec<Owner> {
        self.0
            .get()
            .rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(crate) fn response_target(&self, metadata: &Metadata) -> Option<Owner> {
        let ProtocolOperation::Support { query, .. } = metadata.operation() else {
            return None;
        };
        self.0
            .get()
            .rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|row| {
                let m = row.object().metadata();
                m.id() == query
                    && m.target() == metadata.issuer()
                    && m.scope() == metadata.scope()
                    && m.epoch() == metadata.epoch()
            })
            .cloned()
    }
    pub(crate) fn compatible_helpers(
        &self,
        wall: u64,
        mono: u64,
        skip: DeviceId,
        after: Option<DeviceId>,
    ) -> [Option<Owner>; 2] {
        // Scan at most32 paid current queries. Rotating after the original
        // source's cursor prevents the first two busy helpers pinning it forever.
        let rows = self.0.get().rows.lock().unwrap_or_else(|e| e.into_inner());
        let mut candidates: [Option<Owner>; 2] = [None, None];
        for pass in 0..2 {
            for row in rows.iter() {
                let peer = row.object().metadata().target();
                let Role::Query(query) = &row.get().role else {
                    continue;
                };
                if !matches!(query.binding, Binding::DirectHelper(_))
                    || peer == skip
                    || candidates
                        .iter()
                        .flatten()
                        .any(|owner| owner.object().metadata().target() == peer)
                    || (pass == 0 && after.is_some_and(|last| peer <= last))
                    || (pass == 1 && after.is_none_or(|last| peer > last))
                    || row
                        .features_at(wall, mono)
                        .is_none_or(|features| features & 1 == 0)
                {
                    continue;
                }
                if let Some(slot) = candidates.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some(row.clone());
                }
            }
        }
        candidates
    }
    pub(crate) fn endpoint(owner: Owner) -> EndpointSupportQuery {
        EndpointSupportQuery(SupportQueryIdentity(owner))
    }
    pub(crate) fn helper(owner: Owner) -> DirectHelperSupportProbe {
        DirectHelperSupportProbe(SupportQueryIdentity(owner))
    }
}
impl Drop for Queries {
    fn drop(&mut self) {
        // Queries is owned by ONE presence. Clone-free table ownership prevents
        // an external observation from keeping a removed native call current.
        for owner in self
            .0
            .get()
            .rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            owner.invalidate();
        }
        // Notification is outside the table/native retirement path: parent End
        // already wakes actual lower work, which also observes the pure q gate.
    }
}
redacted_debug!(
    SupportQueryIdentity,
    EndpointSupportQuery,
    DirectHelperSupportProbe
);
