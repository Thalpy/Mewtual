//! Separate Control7 source/helper custody; existing Source/Body enums stay closed.
//! Fixed backing and each original record are paid before allocation/publication.
use super::*;
use futures::FutureExt;
use paid::{Payment, Retained};
use record::Role;
const SOURCES: usize = 32;
const HELPERS: usize = 64;
const CACHE: usize = 32;
const LEDGER_BYTES: usize = 8192;
const LOWER_BYTES: usize = 3 * (2048 + 1024 + 16) + 4096 + ROUTE_METADATA_BYTES;

struct State {
    sources: Vec<Owner>,
    helpers: Vec<Owner>,
    cache: Vec<Owner>,
    source_cursor: usize,
    helper_cursor: usize,
}
struct Core {
    state: Mutex<State>,
    resources: Arc<Resources>,
    clock: Arc<dyn Clock>,
    group: Nonce,
}
#[derive(Clone)]
pub(crate) struct ControlLedger(Retained<Core>);
impl ControlLedger {
    pub(super) fn inventory_bytes() -> usize {
        2 * std::mem::size_of::<Core>()
            + (SOURCES + HELPERS + CACHE + HELPERS) * std::mem::size_of::<Owner>()
            + 512
    }
    pub(crate) fn new(
        resources: Arc<Resources>,
        clock: Arc<dyn Clock>,
        group: Nonce,
    ) -> Result<Self, VoiceError> {
        // Includes the largest scheduling snapshot (64 owners), all three fixed
        // Vec backings and constructor/Core Arc overlap. No uncharged map leaves.
        if Self::inventory_bytes() > LEDGER_BYTES {
            return Err(VoiceError::Bounds);
        }
        let payment = Payment::reserve(&resources, group, LEDGER_BYTES, ResourceKind::Retained)?;
        Ok(Self(payment.retain(Core {
            resources,
            clock,
            group,
            state: Mutex::new(State {
                sources: Vec::with_capacity(SOURCES),
                helpers: Vec::with_capacity(HELPERS),
                cache: Vec::with_capacity(CACHE),
                source_cursor: 0,
                helper_cursor: 0,
            }),
        })))
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.0.same(&other.0)
    }
    pub(crate) fn is_empty(&self) -> bool {
        let state = self.0.get().state.lock().unwrap_or_else(|e| e.into_inner());
        state.sources.is_empty() && state.helpers.is_empty()
    }
    #[cfg(test)]
    pub(crate) fn counts_for_test(&self) -> (usize, usize, usize) {
        let state = self.0.get().state.lock().unwrap();
        (state.sources.len(), state.helpers.len(), state.cache.len())
    }
    pub(crate) fn collect_expired(&self) {
        let core = self.0.get();
        let wall = core.clock.now_ms();
        let mono = core.clock.monotonic_ms();
        // Detach ONE original at a time, then destroy outside every ledger/native
        // guard. Never release a final driver payer from within Vec::retain.
        loop {
            let removed = {
                let mut state = core.state.lock().unwrap_or_else(|e| e.into_inner());
                let mut removed = None;
                for which in 0..3 {
                    let rows = match which {
                        0 => &mut state.sources,
                        1 => &mut state.helpers,
                        _ => &mut state.cache,
                    };
                    if let Some(index) = rows
                        .iter()
                        .position(|owner| !owner.authorized_at(wall, mono))
                    {
                        removed = Some(rows.swap_remove(index));
                        break;
                    }
                }
                removed
            };
            match removed {
                Some(owner) => {
                    owner.invalidate();
                    owner.notify();
                    drop(owner);
                }
                None => break,
            }
        }
    }
    pub(crate) fn queue_query(
        &self,
        actor: &ActorOwner,
        owner: &Owner,
        context: &Context,
    ) -> Result<(), VoiceError> {
        if !self.same(&actor.control()?) {
            return Err(VoiceError::Unauthorized);
        }
        actor.with_control_source(
            owner.object().metadata(),
            context,
            |wall, mono, members, limits| {
                if !members(owner.object().metadata().target())
                    || !owner.authorized_at(wall, mono)
                    || !matches!(owner.get().role, Role::Query(_))
                {
                    return Err(VoiceError::Unauthorized);
                }
                let mut state = self
                    .0
                    .get()
                    .state
                    .try_lock()
                    .map_err(|_| VoiceError::Capacity)?;
                if state.sources.iter().any(|old| old.same(owner)) {
                    return Ok(());
                }
                if state.sources.len() == SOURCES {
                    return Err(VoiceError::Capacity);
                }
                let mut query_state = owner
                    .get()
                    .state
                    .try_lock()
                    .map_err(|_| VoiceError::Capacity)?;
                if !query_state.published {
                    limits.prepared_origin(
                        context.local,
                        super::super::limits::Traffic::Setup,
                        mono,
                    )?;
                    query_state.published = true;
                }
                state.sources.push(owner.clone());
                Ok(())
            },
        )
    }
    pub(crate) fn cached(
        &self,
        request: &Metadata,
        hash: Nonce,
        helper: bool,
    ) -> Result<Option<Owner>, VoiceError> {
        let state = self.0.get().state.lock().unwrap_or_else(|e| e.into_inner());
        let rows = if helper { &state.helpers } else { &state.cache };
        for row in rows {
            let original = match &row.get().role {
                Role::Response { request } => request,
                Role::Forward { .. } => row.object(),
                Role::Query(_) => continue,
            };
            if original.metadata().issuer() == request.issuer()
                && original.metadata().id() == request.id()
            {
                return if original.hash() == hash {
                    Ok(Some(row.clone()))
                } else {
                    Err(VoiceError::RetryMismatch)
                };
            }
        }
        Ok(None)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_response(
        &self,
        actor: &ActorOwner,
        request: &Verified<'_>,
        context: &Context,
        room: RoomAuthority,
        signer: &dyn VoiceSigner,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Owner, VoiceError> {
        if request.metadata().operation() != ProtocolOperation::Query {
            return Err(VoiceError::Malformed);
        }
        let core = self.0.get();
        let payment = Owner::reserve(&core.resources, core.group)?;
        let issued = core.clock.now_ms();
        let mono = core.clock.monotonic_ms();
        let expires = request.metadata().expires().min(
            issued
                .checked_add(record::LIFETIME_MS)
                .ok_or(VoiceError::Freshness)?,
        );
        let deadline = mono
            .checked_add(expires.saturating_sub(issued))
            .ok_or(VoiceError::Freshness)?;
        let mut id = [0; 32];
        rng.fill_bytes(&mut id);
        // Only the implemented targeted Control7 forwarding path is advertised.
        // This contains no endpoint state or inferred FreshPeer capability.
        let features = u16::from(context.indirect && context.scope.policy.is_some());
        let object = Object::support(
            context.scope,
            context.epoch,
            request.metadata().issuer(),
            request.metadata().id(),
            features,
            signer,
            id,
            issued,
            expires,
        )?;
        if object.metadata().issuer() != context.local {
            return Err(VoiceError::Unauthorized);
        }
        let effect = actor.effect(context.local, request.metadata().issuer(), expires)?;
        let epoch = actor.observe_epoch(context)?;
        Ok(Owner::new(
            payment,
            object,
            Role::Response {
                request: Object::copy(request),
            },
            room,
            epoch,
            effect,
            core.clock.clone(),
            deadline,
        ))
    }
    pub(crate) fn prepare_forward(
        &self,
        actor: &ActorOwner,
        envelope: &wire::Envelope<'_>,
        context: &Context,
        room: RoomAuthority,
    ) -> Result<Owner, VoiceError> {
        let core = self.0.get();
        let payment = Owner::reserve(&core.resources, core.group)?;
        let m = envelope.object().metadata();
        let wall = core.clock.now_ms();
        let mono = core.clock.monotonic_ms();
        let remaining = m
            .expires()
            .saturating_add(CLOCK_SKEW_MS)
            .saturating_sub(wall)
            .min(record::LIFETIME_MS);
        let deadline = mono.checked_add(remaining).ok_or(VoiceError::Freshness)?;
        let effect = actor.effect(m.issuer(), m.target(), m.expires())?;
        let epoch = actor.observe_epoch(context)?;
        let permit = envelope
            .helper_permit()
            .ok_or(VoiceError::Unauthorized)?
            .clone();
        Ok(Owner::new(
            payment,
            Object::copy(envelope.object()),
            Role::Forward { permit },
            room,
            epoch,
            effect,
            core.clock.clone(),
            deadline,
        ))
    }
    pub(crate) fn admit(
        &self,
        actor: &ActorOwner,
        ticket: &IngressTicket,
        envelope: &wire::Envelope<'_>,
        context: &Context,
        owner: &Owner,
        cached: bool,
    ) -> Result<Receipt, VoiceError> {
        if !self.same(&actor.control()?) {
            return Err(VoiceError::Unauthorized);
        }
        actor.with_control_origin(ticket, envelope, context, |wall, mono, origin| {
            if !owner.authorized_at(wall, mono) {
                return Err(VoiceError::Unauthorized);
            }
            let mut state = self
                .0
                .get()
                .state
                .try_lock()
                .map_err(|_| VoiceError::Capacity)?;
            if cached {
                return Ok(Receipt(owner.clone()));
            }
            let forward = matches!(owner.get().role, Role::Forward { .. });
            if (forward && state.helpers.len() == HELPERS)
                || (!forward && (state.cache.len() == CACHE || state.sources.len() == SOURCES))
            {
                return Err(VoiceError::Capacity);
            }
            // Every check/payment is complete before spending the original
            // issuer's shared Setup allowance. Duplicate custody spends none.
            origin.publish(|| {
                if forward {
                    state.helpers.push(owner.clone());
                } else {
                    state.cache.push(owner.clone());
                    state.sources.push(owner.clone());
                }
            })?;
            Ok(Receipt(owner.clone()))
        })
    }
    pub(crate) fn candidates(&self, helper: bool) -> Vec<Owner> {
        let mut state = self.0.get().state.lock().unwrap_or_else(|e| e.into_inner());
        let (rows, cursor) = if helper {
            (&state.helpers, state.helper_cursor)
        } else {
            (&state.sources, state.source_cursor)
        };
        let mut result = Vec::with_capacity(rows.len());
        for offset in 0..rows.len() {
            result.push(rows[(cursor + offset) % rows.len()].clone());
        }
        let next = if rows.is_empty() {
            0
        } else {
            (cursor + 1) % rows.len()
        };
        if helper {
            state.helper_cursor = next;
        } else {
            state.source_cursor = next;
        }
        result
    }
    /// Reserve the actual lower memory + independent512-byte metadata and shared
    /// Forward slot BEFORE spending an attempt. All later preparation failures
    /// conservatively burn it; neither busy nor lost ACK refunds the original.
    pub(crate) fn job(
        &self,
        owner: &Owner,
        route: BoundRoute,
        helper: Option<Owner>,
    ) -> Result<Job, VoiceError> {
        let core = self.0.get();
        let (wall, mono) = owner.samples();
        if !owner.authorized_at(wall, mono) || !owner.get().sending.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let payment = Payment::reserve(
            &core.resources,
            core.group,
            LOWER_BYTES,
            ResourceKind::Forward,
        )?;
        let mut state = owner.get().state.lock().unwrap_or_else(|e| e.into_inner());
        let max = 3; // Same agreed original allowance for Source and delegated edge.
        if state.in_flight || state.settled || state.attempts >= max || mono < state.next {
            return Err(VoiceError::Capacity);
        }
        state.attempts += 1;
        state.in_flight = true;
        state.next = mono.saturating_add(record::GAP_MS);
        if let Some(helper) = &helper {
            state.helper_cursor = Some(helper.object().metadata().target());
        }
        let attempt = state.attempts;
        drop(state);
        Ok(Job {
            lower: payment.retain(Lower {
                owner: owner.clone(),
                route,
                helper,
                attempt,
            }),
            ledger: self.clone(),
        })
    }
}
struct Lower {
    owner: Owner,
    route: BoundRoute,
    helper: Option<Owner>,
    attempt: u8,
}
impl Drop for Lower {
    fn drop(&mut self) {
        let mut state = self
            .owner
            .get()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.attempts == self.attempt {
            state.in_flight = false;
        }
    }
}
pub(crate) struct Job {
    lower: Retained<Lower>,
    ledger: ControlLedger,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Refused,
    Queued,
    Delivered,
}
impl Job {
    pub(crate) fn object(&self) -> &Object {
        self.lower.get().owner.object()
    }
    pub(crate) fn owner(&self) -> &Owner {
        &self.lower.get().owner
    }
    pub(crate) fn peer(&self) -> PeerId {
        self.lower.get().route.peer
    }
    pub(crate) fn target(&self) -> DeviceId {
        self.lower.get().route.device
    }
    pub(crate) fn selected_helper(&self) -> Option<DeviceId> {
        self.lower
            .get()
            .helper
            .as_ref()
            .map(|owner| owner.object().metadata().target())
    }
    pub(crate) fn helper_permit(&self) -> Option<&wire::Permit> {
        if let Role::Forward { permit } = &self.owner().get().role {
            Some(permit)
        } else {
            None
        }
    }
    pub(crate) fn keepalive(&self) -> SharedRequestKeepalive {
        Arc::new(self.lower.clone())
    }
    pub(crate) fn current(&self, actor: &ActorOwner) -> Result<(), VoiceError> {
        if self.ledger.same(&actor.control()?) && self.authorized() {
            Ok(())
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
    pub(crate) fn complete(&self, outcome: Option<Outcome>) {
        if outcome == Some(Outcome::Delivered) {
            self.owner()
                .get()
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .settled = true;
        }
    }
}
#[async_trait::async_trait]
impl RequestAuthority for Job {
    fn authorized(&self) -> bool {
        // Sample both ORIGINAL clocks before checking either irreversible owner.
        let own = self.owner().samples();
        let helper = self
            .lower
            .get()
            .helper
            .as_ref()
            .map(|owner| (owner, owner.samples()));
        self.owner().authorized_at(own.0, own.1)
            && self.owner().get().sending.is_live()
            && helper.is_none_or(|(owner, (wall, mono))| {
                owner
                    .features_at(wall, mono)
                    .is_some_and(|bits| bits & 1 != 0)
            })
    }
    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let original = futures::future::select(
            self.owner().revoked().boxed(),
            self.owner().get().sending.wait_revoked().boxed(),
        )
        .boxed();
        if let Some(helper) = &self.lower.get().helper {
            futures::future::select(original, helper.revoked().boxed()).await;
        } else {
            original.await;
        }
    }
}
redacted_debug!(ControlLedger, Job);
