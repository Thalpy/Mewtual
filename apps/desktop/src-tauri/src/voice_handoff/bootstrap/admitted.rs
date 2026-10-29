//! Inert physical bootstrap for an ORIGINAL admitted published browser key.
//! No operation31/ACK/worker or agreement/capacity permission exists in this leaf.
use super::*;
use crate::voice_authority::AdmittedPublishedKey;
use crate::voice_call_activation::Handle;

impl Registry {
    pub(in crate::voice_handoff) fn admitted_identity(
        &self,
        owner: &str,
        executor: [u8; 32],
    ) -> bool {
        self.index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|index| {
                index.rows.iter().flatten().any(|row| {
                    row.selected.id() == owner
                        && row.selected.executor() == executor
                        && row.selected.profile() == ChannelProfile::Admitted
                })
            })
    }
    #[allow(dead_code)] // Future operation31 must use this typed route, never route().
    pub(in crate::voice_handoff) fn admitted_route(
        &self,
        slot: u8,
    ) -> Result<AdmittedRoute, Refused> {
        let slot = slot
            .checked_sub(1)
            .filter(|slot| *slot < 64)
            .ok_or(Refused::Binding)?;
        let held = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopping.load(Ordering::Acquire) {
            return Err(Refused::Binding);
        }
        let row = held
            .as_ref()
            .and_then(|index| index.rows[usize::from(slot)].as_ref())
            .ok_or(Refused::Binding)?;
        Self::admitted_row_route(row.clone()).ok_or(Refused::Binding)
    }
    pub(super) fn admitted_row_route(row: Arc<Row>) -> Option<AdmittedRoute> {
        row.selected.admitted()?;
        let (session, public) = {
            let state = row.state.lock().unwrap_or_else(|e| e.into_inner());
            (state.session.clone()?, state.public.clone()?)
        };
        // Closed retained Rows still service authenticated31 negative cleanup;
        // Register independently requires current NativeActive and final fences.
        Some(AdmittedRoute {
            row,
            session,
            public,
        })
    }
    pub(in crate::voice_handoff) fn project_admitted(
        &self,
        owner: &str,
        executor: [u8; 32],
    ) -> Result<MetadataProjection, Refused> {
        let held = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let row = held
            .as_ref()
            .and_then(|index| {
                index.rows.iter().flatten().find(|row| {
                    row.selected.id() == owner
                        && row.selected.executor() == executor
                        && row.selected.profile() == ChannelProfile::Admitted
                })
            })
            .ok_or(Refused::Binding)?;
        row.projection_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Refused::Capacity)?;
        Ok(MetadataProjection { row: row.clone() })
    }
}

fn require_admitted_selected(
    state: &AppState,
    authority: &crate::voice_authority::Coordinator,
    handle: &Handle,
    selected: &AdmittedPublishedKey,
) -> Result<(), Refused> {
    check_voice_activation_generation(state, selected.binding().generation)
        .map_err(|_| Refused::Binding)?;
    if !authority.is_current_admitted_published_key(handle, selected) {
        return Err(Refused::Binding);
    }
    Ok(())
}

/// Fixed trusted-main bootstrap, using only the real canonical NativeActive and
/// published-key allocations. Holding UI commit orders BOTH fences against lock;
/// injected RNG/curve work runs outside Coordinator/Index/Row guards.
pub(crate) async fn ensure_admitted(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
    rng: &mut impl CryptoRngCore,
) -> Result<Status, Refused> {
    state.voice_channels.collect_retired(&state.voice_handoff);
    let handle = state
        .voice_authority
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admitted_activation(owner)
        .ok_or(Refused::Binding)?;
    check_voice_binding(state, handle.binding())
        .await
        .map_err(|_| Refused::Binding)?;
    let _commit = state.ui_session_commit.clone().lock_owned().await;
    require_unlocked_session(state)
        .await
        .map_err(|_| Refused::Binding)?;
    let device = {
        let servers = state.servers.lock().await;
        let binding = handle.binding();
        *servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .ok_or(Refused::Binding)?
            .device_id
            .as_bytes()
    };
    let (selected, begin) = {
        let mut authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        check_voice_activation_generation(state, handle.binding().generation)
            .map_err(|_| Refused::Binding)?;
        let selected = authority
            .admitted_published_key(&handle, executor)
            .map_err(|_| Refused::Binding)?;
        require_admitted_selected(state, &authority, &handle, &selected)?;
        let begin = state
            .voice_channels
            .begin_owner(ChannelOwner::Admitted(selected.clone()), device)?;
        (selected, begin)
    };
    let Begin::Work(mut ticket) = begin else {
        let Begin::Existing(status) = begin else {
            unreachable!()
        };
        return Ok(status);
    };
    let (session, public, setup) = ticket.compute(rng)?;
    let reply = Arc::new(SetupOutput {
        public,
        _credit: ticket.row._credit.clone(),
    });
    let mut output = Some((session, reply.clone(), setup));
    {
        let authority = state
            .voice_authority
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        require_admitted_selected(state, &authority, &handle, &selected)?;
        ticket.install(&mut output)?;
    }
    Ok(Status::Ready(reply))
}

/// Negative commands resolve only the retained SAME admitted row; they cannot
/// create a close-first row or turn missing historical work into a certificate.
async fn finish_admitted(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
    closure: Closure,
) -> Result<Status, Refused> {
    let commit = state.ui_session_commit.clone().lock_owned().await;
    let (row, ready) = {
        let held = state
            .voice_channels
            .index
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let row = held
            .as_ref()
            .and_then(|index| {
                index.rows.iter().flatten().find(|row| {
                    row.selected.id() == owner
                        && row.selected.executor() == executor
                        && row.selected.profile() == ChannelProfile::Admitted
                })
            })
            .ok_or(Refused::Binding)?
            .clone();
        {
            let mut current = row.state.lock().unwrap_or_else(|e| e.into_inner());
            current.effects_closed = true;
            current.closed = true;
            if let Some(stage) = current.admitted_stage.as_mut() {
                stage.close();
            }
        }
        let ready = row
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admitted_returned();
        (row, ready)
    };
    drop(commit);
    row.admission_changed.notify_waiters();
    if let Some(route) = Registry::admitted_row_route(row.clone()) {
        route.nudge_admitted();
    }
    if !ready {
        return Err(Refused::Capacity);
    }
    match closure {
        Closure::Setup => row.retire_setup()?,
        Closure::Channel => row.retire_channel()?,
    }
    let status = row.status();
    drop(row);
    state.voice_channels.collect_retired(&state.voice_handoff);
    Ok(status)
}
pub(crate) async fn settle_admitted_setup(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<Status, Refused> {
    finish_admitted(state, owner, executor, Closure::Setup).await
}
pub(crate) async fn collect_admitted_channel(
    state: &AppState,
    owner: &str,
    executor: [u8; 32],
) -> Result<Status, Refused> {
    finish_admitted(state, owner, executor, Closure::Channel).await
}
