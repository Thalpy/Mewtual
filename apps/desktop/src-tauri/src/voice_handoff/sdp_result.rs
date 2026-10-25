//! Private authenticated SDP Result adapter. Exact original output crosses the
//! existing paid Body RPC with no plaintext clone. Production SDP remains disabled.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, PhysicalHold};
use super::context::Context;
use super::factory::{self, Refused};
use super::ledger::{ResolvedRow, SdpHistory, SdpInputTarget as Target, SdpSubmissionOwners};
use super::requests::sdp_action::{Kind, Request};
use super::requests::Action;
use super::sdp_publication::TASK_BYTES;
use catcoms_app::{
    VoiceSdpBody, VoiceSdpResultInput, VoiceSdpResultPublisher, VOICE_SDP_RPC_METADATA_BYTES,
};
use catcoms_sync::voice_v2::chain::sdp_handoff::{Copy, TaskCopy};
use catcoms_sync::voice_v2::crypto_actions::{CryptoAction, CryptoResultReceipt};
use catcoms_sync::voice_v2::sdp::{SdpOpenOutput, SdpResultAdmission, SdpResultIdentity};
use catcoms_sync::voice_v2::VoiceError;
use std::sync::Arc;

struct Custody {
    _physical: PhysicalHold,
    // LAST: whole concrete native graph remains paid through send/callback cleanup.
    _task: TaskCopy,
}
impl std::fmt::Debug for Custody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SdpResultCustody { .. }")
    }
}
struct Publisher {
    context: Context,
    route: Route,
    target: Arc<Target>,
    identity: SdpResultIdentity,
    // LAST. No Target/Ledger backedge is attached to the original Body/Copy.
    _custody: Arc<Custody>,
}
impl VoiceSdpResultPublisher for Publisher {
    fn publish(self: Box<Self>, step: SdpResultAdmission<'_>) -> Result<(), VoiceError> {
        if !step.matches(&self.target.action, self.identity) {
            return Err(VoiceError::Unauthorized);
        }
        let live = self.context.upgrade().ok_or(VoiceError::Unauthorized)?;
        let commit = live
            .commit
            .clone()
            .try_lock_owned()
            .map_err(|_| VoiceError::Capacity)?;
        factory::require_unlocked_now(&live, self.route.selected()).map_err(
            |error| match error {
                Refused::Protocol(error) => error,
                _ => VoiceError::Unauthorized,
            },
        )?;
        let authority = match live.authority.try_lock() {
            Ok(authority) => authority,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(VoiceError::Capacity),
        };
        let admission = super::transition::Admission::new(&live, &authority, &self.route, &commit);
        // LEXICAL UI: Root may recover/history-sample/drop unused preparation after
        // this hook returns. The engine, ACK, physical IO and SDK lifetimes are separate.
        self.route
            .try_with_open(|| self.target.admit(&admission, || step.commit()))
    }
}
struct Task {
    context: Context,
    route: Route,
    target: Arc<Target>,
    action: Action,
    identity: SdpResultIdentity,
    seal: bool,
    body: Option<VoiceSdpBody>,
    metadata: Option<Copy>,
    // LAST, also on unpolled future cancellation and failed mailbox send.
    custody: Arc<Custody>,
}
impl Task {
    fn history(&self) -> Result<Option<Box<SdpHistory>>, Refused> {
        let history = self
            .target
            .row()
            .sdp_history(self.route.session().identity())?;
        check_action(&history, self.action)?;
        let Some(receipt) = &history.status().engine.receipt else {
            return Ok(None);
        };
        match self.identity {
            SdpResultIdentity::Bytes(tag) if receipt.output_tag != tag => {
                return Err(VoiceError::RetryMismatch.into())
            }
            SdpResultIdentity::AuthenticationFailed => {
                let past = self
                    .target
                    .action
                    .recover_sdp_open_result(SdpOpenOutput::AuthenticationFailed)?
                    .ok_or(VoiceError::Capacity)?;
                if !past.receipt.same_transition(receipt) {
                    return Err(VoiceError::RetryMismatch.into());
                }
            }
            _ => {}
        }
        Ok(Some(Box::new(history)))
    }
    async fn execute(&mut self) -> Result<Box<SdpHistory>, Refused> {
        if let Some(history) = self.history()? {
            return Ok(history);
        }
        let (actor, grant) = factory::native::observation_actor(&self.context, &self.route)?;
        let input = match self.identity {
            SdpResultIdentity::Bytes(tag) if self.seal => VoiceSdpResultInput::Sealed {
                body: self.body.as_ref().expect("original sealed output"),
                tag,
            },
            SdpResultIdentity::Bytes(tag) => VoiceSdpResultInput::Opened {
                body: self.body.as_ref().expect("original Open output"),
                tag,
            },
            SdpResultIdentity::AuthenticationFailed => VoiceSdpResultInput::AuthenticationFailed(
                self.metadata.take().expect("original metadata payment"),
            ),
        };
        let publisher = Box::new(Publisher {
            context: self.context.clone(),
            route: self.route.clone(),
            target: self.target.clone(),
            identity: self.identity,
            _custody: self.custody.clone(),
        });
        let pending = actor
            .queue_voice_sdp_result_scoped(
                grant,
                self.target.intent.clone(),
                self.target.action.clone(),
                input,
                publisher,
            )
            .await;
        let original_error = match pending {
            Ok(pending) => match pending.complete().await {
                Ok(completion) => completion.result().err().map(Refused::from),
                Err(error) => Some(error.into()),
            },
            Err(error) => Some(error.into()),
        };
        // The existing reply is sent only after the entire scoped Root call returns.
        // Its independent sender Work can still be retiring: this is NOT funding0.
        // Canonical positive history wins over lost/failed callback completion.
        self.history()?
            .ok_or_else(|| original_error.unwrap_or(VoiceError::Capacity.into()))
    }
}
struct Construction {
    target: Option<Arc<Target>>,
    owners: Option<SdpSubmissionOwners>,
    metadata: Option<Copy>,
    custody: Option<Arc<Custody>>,
    physical: Option<PhysicalHold>,
    task: Option<TaskCopy>,
}
fn construct(
    context: &Context,
    route: &Route,
    incoming: &mut Incoming,
    row: ResolvedRow,
    expected: Expected,
    original: &Arc<CryptoAction>,
) -> Result<Box<Task>, Refused> {
    if task_layout() > TASK_BYTES {
        return Err(VoiceError::Capacity.into());
    }
    let mut prepared = Construction {
        target: None,
        owners: None,
        metadata: None,
        custody: None,
        physical: Some(
            incoming
                .hold_position()
                .map_err(|_| VoiceError::Unauthorized)?,
        ),
        task: None,
    };
    prepared.owners = Some(row.sdp_input_owners(route.session().identity(), expected.action)?);
    let owners = prepared.owners.as_ref().expect("original row funding");
    prepared.task = Some(owners.phase.task_copy(TASK_BYTES)?);
    prepared
        .task
        .as_mut()
        .expect("original native graph")
        .bind_native_history(owners.funding.clone())?;
    prepared.target = Some(Arc::new(row.sdp_input_target(
        route.session().identity(),
        expected.action,
        owners,
        prepared.task.as_ref().unwrap(),
    )?));
    if !Arc::ptr_eq(&prepared.target.as_ref().unwrap().action, original) {
        return Err(VoiceError::Unauthorized.into());
    }
    if expected.identity == SdpResultIdentity::AuthenticationFailed {
        prepared.metadata = Some(owners.phase.copy(VOICE_SDP_RPC_METADATA_BYTES)?);
        prepared
            .metadata
            .as_mut()
            .unwrap()
            .bind_native_history(owners.funding.clone())?;
    }
    prepared.custody = Some(Arc::new(Custody {
        _physical: prepared.physical.take().unwrap(),
        _task: prepared.task.take().unwrap(),
    }));
    if let Some(metadata) = prepared.metadata.as_mut() {
        // One-way ownership only: the original Copy keeps its native Task/physical
        // custody alive through the actual Actor sender. Custody retains no Copy,
        // Buffer, Target or Ledger, so it cannot keep itself in a cycle.
        metadata.attach_keepalive(prepared.custody.as_ref().unwrap().clone())?;
    }
    // The original payload is moved only AFTER every other fallible admission. If the
    // Body constructor refuses, Incoming restores the identical opened buffer/position.
    let body = if expected.identity != SdpResultIdentity::AuthenticationFailed {
        let mut body = incoming
            .sdp_body(owners.intent.clone(), &owners.funding)
            .map_err(|error| match error {
                channel::Refused::Capacity => VoiceError::Capacity,
                _ => VoiceError::Unauthorized,
            })?;
        body.attach_publication(prepared.custody.as_ref().unwrap().clone())?;
        Some(body)
    } else {
        None
    };
    Ok(Box::new(Task {
        context: context.clone(),
        route: route.clone(),
        target: prepared.target.take().unwrap(),
        action: expected.action,
        identity: expected.identity,
        seal: expected.seal,
        body,
        metadata: prepared.metadata.take(),
        custody: prepared.custody.take().unwrap(),
    }))
}
#[derive(Clone, Copy)]
struct Expected {
    action: Action,
    identity: SdpResultIdentity,
    seal: bool,
}
fn check_action(history: &SdpHistory, expected: Action) -> Result<(), VoiceError> {
    if history.status().action == Some((expected.serial, expected.version, expected.input_tag)) {
        Ok(())
    } else {
        Err(VoiceError::Unauthorized)
    }
}
fn resolve(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<
    (
        ResolvedRow,
        Arc<CryptoAction>,
        Expected,
        Option<CryptoResultReceipt>,
    ),
    Refused,
> {
    if !incoming.belongs_to(route.session()) {
        return Err(VoiceError::Unauthorized.into());
    }
    let Request::Existing(request) = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?
    else {
        return Err(VoiceError::Malformed.into());
    };
    let (expected, output) = match request.kind {
        Kind::Sealed { action, tag, bytes } => (
            Expected {
                action,
                identity: SdpResultIdentity::Bytes(tag),
                seal: true,
            },
            Some(bytes),
        ),
        Kind::Opened { action, tag, bytes } => (
            Expected {
                action,
                identity: SdpResultIdentity::Bytes(tag),
                seal: false,
            },
            Some(bytes),
        ),
        Kind::AuthenticationFailed(action) => (
            Expected {
                action,
                identity: SdpResultIdentity::AuthenticationFailed,
                seal: false,
            },
            None,
        ),
        _ => return Err(VoiceError::Malformed.into()),
    };
    let row = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        live.channels
            .require_cleanup_route(route)
            .map_err(|_| Refused::Closed)?;
        live.ledger.resolve(
            route.selected(),
            super::ledger::Purpose::Sdp {
                parent: request.parent,
                purpose: request.purpose,
            },
            request.row,
        )?
    };
    let action = row.sdp_result_action(route.session().identity(), expected.action)?;
    // Hash the ACTUAL bounded immutable request before any buffer transfer or native
    // admission guard. A claimed tag cannot make changed bytes an exact retry.
    let past = match expected.identity {
        SdpResultIdentity::Bytes(tag) => {
            if action.output_tag(output.expect("selected byte result"))? != tag {
                return Err(VoiceError::RetryMismatch.into());
            }
            action.recover_result(output.unwrap(), tag)?
        }
        SdpResultIdentity::AuthenticationFailed => action
            .recover_sdp_open_result(SdpOpenOutput::AuthenticationFailed)?
            .map(|past| past.receipt),
    };
    Ok((row, action, expected, past))
}

/// Staged authenticated handler only; the shipping Host still refuses19/20/21.
pub(super) async fn apply(
    context: &Context,
    route: &Route,
    mut incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let result = prepare(context, route, &mut incoming).await;
    // The observer's Task/Body/Target references retire before fixed serialization.
    // Sender Work may still retain its Body/Copy and attached native Task custody;
    // all remain paid until their actual destruction. Physical output independently
    // keeps its request position. None of this establishes SDK settlement/funding0.
    let mut reply = incoming.reply(
        BodyKind::Control,
        if result.is_ok() {
            super::sdp_replies::HISTORY_BYTES
        } else {
            4
        },
        None,
    )?;
    match result {
        Ok(history) => reply.plain_mut().copy_from_slice(
            &super::sdp_replies::history(&history).map_err(|_| channel::Refused::Binding)?,
        ),
        Err(error) => reply
            .plain_mut()
            .copy_from_slice(&super::sdp_replies::refused(error)),
    }
    reply.seal()
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &mut Incoming,
) -> Result<Box<SdpHistory>, Refused> {
    let (row, original, expected, past) = resolve(context, route, incoming)?;
    if let Some(receipt) = past {
        let history = row.sdp_history(route.session().identity())?;
        check_action(&history, expected.action)?;
        if history
            .status()
            .engine
            .receipt
            .as_ref()
            .is_none_or(|canonical| !canonical.same_transition(&receipt))
        {
            return Err(VoiceError::RetryMismatch.into());
        }
        return Ok(Box::new(history));
    }
    let mut task = construct(context, route, incoming, row, expected, &original)?;
    task.execute().await
}

fn task_layout() -> usize {
    #[repr(C)]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    fn executing<F: std::future::Future>(_: fn(&'static mut Task) -> F) -> usize {
        std::mem::size_of::<F>()
    }
    let retained = std::mem::size_of::<ArcAllocation<Custody>>()
        + std::mem::size_of::<ArcAllocation<Target>>()
        + std::mem::size_of::<Publisher>()
        + std::mem::size_of::<Task>();
    let construction = std::mem::size_of::<Construction>()
        + std::mem::size_of::<Target>()
        + std::mem::size_of::<Task>()
        + 256;
    let callback = std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<crate::UiSessionGate<'static>>()
        + std::mem::size_of::<super::transition::Admission<'static>>()
        + std::mem::size_of::<std::sync::MutexGuard<'static, crate::voice_authority::Coordinator>>(
        )
        + 5 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 192;
    let execute = executing(Task::execute) + 192;
    retained + construction.max(callback).max(execute)
}
#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn measured<F: std::future::Future>(
        _: fn(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let lookup = std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<SdpHistory>()
        + 256
        + catcoms_sync::voice_v2::sdp::sdp_result_hash_frame_bytes();
    let authentication_failure = catcoms_sync::voice_v2::sdp::sdp_result_hash_frame_bytes()
        + std::mem::size_of::<Option<catcoms_sync::voice_v2::sdp::SdpOpenResult>>()
        + 64;
    let history = std::mem::size_of::<SdpHistory>()
        + 192
        + super::ledger::sdp_history_layout_bytes().max(authentication_failure);
    let serialization =
        std::mem::size_of::<SdpHistory>() + 96 + super::sdp_replies::serialization_layout_bytes();
    let bytes = measured(apply) + lookup.max(history).max(serialization);
    eprintln!(
        "SDP Result physical adapter {bytes}; native task {}",
        task_layout()
    );
    bytes
}
#[cfg(test)]
pub(super) mod tests;
#[cfg(test)]
pub(super) use tests::{exercise, exercise_open};
