//! Native confirmation continuation under the original Derive parent. The request reserves
//! a finite purpose before Actor work; the admitted task outlives its caller and retains the
//! UI commit through publication. Native maintenance and the authenticated child adapter
//! share the actual publication routine; neither can manufacture a Root action from IDs.
use super::*;
use crate::voice_handoff::ledger::{ConfirmationPurpose, ResolvedRow};

pub(in crate::voice_handoff) async fn ensure_confirmation_row(
    context: &Context,
    route: &Route,
    parent: &ResolvedRow,
    request: ConfirmationPurpose,
    physical: PhysicalHold,
) -> Result<RowId, Refused> {
    if matches!(request, ConfirmationPurpose::Incoming(_)) {
        // Incoming actions require the actual authenticated Root intake result. A public
        // role number, parent handle or cached descriptor cannot manufacture that token.
        return Err(Refused::Closed);
    }
    ensure(context, route, parent, request, None, None, physical).await
}

/// Register the ACTUAL action from the original participant's authenticated Root intake.
/// This cannot be called with an id/body from IPC; parent containment and exact role claim
/// still precede exposing its input. A lost caller retains the detached original worker.
pub(in crate::voice_handoff) async fn ensure_incoming_row(
    context: &Context,
    route: &Route,
    parent: &ResolvedRow,
    action: Arc<catcoms_sync::voice_v2::crypto_actions::CryptoAction>,
    physical: PhysicalHold,
) -> Result<RowId, Refused> {
    if action.operation()
        != catcoms_sync::voice_v2::crypto_actions::CryptoOperation::DecryptConfirmation
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = ConfirmationPurpose::Incoming(action.protected_row()?);
    ensure(
        context,
        route,
        parent,
        request,
        Some(action),
        None,
        physical,
    )
    .await
}

/// Retry attachment from the SAME already-retained publication, without requiring a
/// rotating pending snapshot to rediscover an action whose input may already be settled.
/// Unknown or uncertain rows cannot reserve a purpose or reach a role-only Root factory.
pub(in crate::voice_handoff) async fn ensure_known_incoming_row(
    context: &Context,
    route: &Route,
    parent: &ResolvedRow,
    child: &ResolvedRow,
    request: ConfirmationPurpose,
    physical: PhysicalHold,
) -> Result<RowId, Refused> {
    if !matches!(request, ConfirmationPurpose::Incoming(_)) {
        return Err(VoiceError::Unauthorized.into());
    }
    ensure(context, route, parent, request, None, Some(child), physical).await
}

async fn ensure(
    context: &Context,
    route: &Route,
    parent: &ResolvedRow,
    request: ConfirmationPurpose,
    incoming: Option<Arc<catcoms_sync::voice_v2::crypto_actions::CryptoAction>>,
    known: Option<&ResolvedRow>,
    physical: PhysicalHold,
) -> Result<RowId, Refused> {
    if !physical.belongs_to(route.session()) {
        return Err(Refused::Closed);
    }
    let selected = route.selected().clone();
    let commit = preflight_binding(context, route).await?;
    let commit = commit.lock_owned().await;
    let context = context.upgrade().ok_or(Refused::Closed)?;
    context
        .gate()
        .require_unlocked()
        .await
        .map_err(|_| Refused::Closed)?;
    let actor = {
        let servers = context.servers.lock().await;
        let binding = selected.binding();
        servers
            .get(&binding.server)
            .filter(|entry| entry.instance == binding.instance && entry.group_id == binding.group)
            .map(|entry| entry.actor.clone())
            .ok_or(Refused::Closed)?
    };
    {
        let authority = context.authority.lock().unwrap_or_else(|e| e.into_inner());
        require_current_selection(context.gate(), &authority, &selected)?;
        route.require_open().map_err(|_| Refused::Closed)?;
    }
    let grant = selected.grant()?;
    let handshake = parent.confirmation_handshake()?;
    let (begin, trigger) = if let Some(child) = known {
        // Compare the actual caller-supplied parent AND child allocations, plus full
        // purpose, under one Index. Re-resolving only child.id() here would incorrectly
        // accept an equal-label handle from another original ledger.
        (
            context
                .ledger
                .resume_known_child(selected.clone(), parent, child, request)?,
            None,
        )
    } else {
        context
            .ledger
            .begin_child(selected.clone(), parent, request)?
    };
    let registration = match begin {
        Begin::Existing(id) => return Ok(id),
        Begin::Factory(registration) => registration,
    };
    let credit = selected.resources().charge(
        selected.binding().scope.group,
        FACTORY_BYTES,
        ResourceKind::Retained,
    )?;
    drop(context);
    let work = run_confirmation(
        registration,
        selected,
        actor,
        grant,
        handshake,
        request,
        incoming.or(trigger),
        commit,
        FactoryCustody {
            _physical: physical,
            _credit: credit,
        },
    );
    if worker_layout(&work) > FACTORY_BYTES {
        return Err(Refused::Protocol(VoiceError::Capacity));
    }
    tokio::spawn(work)
        .await
        .map_err(|_| Refused::Stopped)?
        .result
}

#[allow(clippy::too_many_arguments)] // all original affine owners travel with one paid task
async fn run_confirmation(
    registration: Registration,
    _selected: Arc<SelectedHandoffOwner>,
    actor: ServerActor,
    grant: Arc<CallGrant>,
    handshake: [u8; 32],
    request: ConfirmationPurpose,
    trigger: Option<Arc<catcoms_sync::voice_v2::crypto_actions::CryptoAction>>,
    _commit: tokio::sync::OwnedMutexGuard<()>,
    custody: FactoryCustody,
) -> Completion {
    let result =
        publish_confirmation(registration, actor, grant, handshake, request, trigger).await;
    Completion {
        result,
        _custody: custody,
    }
}

/// Actual Actor publication shared by the separately paid physical and native producers.
/// Admission, UI commit, exact selected owner, and real drain custody belong to the caller
/// for this entire future. A known publication retries attachment without calling Actor.
pub(super) async fn publish_confirmation(
    registration: Registration,
    actor: ServerActor,
    grant: Arc<CallGrant>,
    handshake: [u8; 32],
    request: ConfirmationPurpose,
    trigger: Option<Arc<catcoms_sync::voice_v2::crypto_actions::CryptoAction>>,
) -> Result<RowId, Refused> {
    if registration.has_publication()? {
        return registration
            .attach_published(DERIVATION_HANDOFF_BYTES)
            .map_err(Refused::from);
    }
    registration.mark_handoff()?;
    let action = match request {
        ConfirmationPurpose::Start(predecessor) => {
            actor
                .start_voice_confirmation(grant, handshake, predecessor)
                .await?
        }
        ConfirmationPurpose::Respond(_) => {
            actor
                .respond_voice_confirmation(
                    grant,
                    handshake,
                    trigger.ok_or(VoiceError::Unauthorized)?,
                )
                .await?
        }
        ConfirmationPurpose::Incoming(_) => trigger.ok_or(VoiceError::Unauthorized)?,
    };
    registration
        .attach_child(action, DERIVATION_HANDOFF_BYTES)
        .map_err(Refused::from)
}
