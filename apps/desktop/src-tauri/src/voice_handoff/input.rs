//! Original-row derivation input. Serialization writes directly into the already-paid Data
//! reply; its actual CryptoInputLease, Phase and Work survive all IO/output collection.
//! This adapter is not yet admitted by production ingress.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::Context;
use super::factory::Refused;
use super::ledger::{Purpose, RowId};
use super::requests::{Action, Kind, Request};
use catcoms_sync::voice_v2::crypto_actions::{CryptoInput, ProtectedCryptoBinding};
use catcoms_sync::voice_v2::presence::PairSetup;
use catcoms_sync::voice_v2::VoiceError;

pub(super) const FIXED_BYTES: usize = 541;
// The selected WebCrypto HKDF provider has a1024-byte info bound. Refuse before exposing
// work which this adapter cannot execute, while preserving Root's larger protocol limit.
const MAX_TRANSCRIPT: usize = 1024 - b"mewtual/voice/v2/aes-gcm/high-to-low\0".len();

/// No owned transcript/point copy is constructed here. The caller's original paid reply
/// covers every written byte, including full u128 room and exact u64 identities.
fn encode(
    out: &mut [u8],
    id: RowId,
    action: Action,
    engine_bytes: usize,
    binding: &ProtectedCryptoBinding,
    local_point: &[u8; 65],
    setup: &PairSetup,
) -> Result<(), VoiceError> {
    if action.serial == 0
        || action.version == 0
        || out.len() != FIXED_BYTES + setup.transcript.len()
        || setup.transcript.len() > MAX_TRANSCRIPT
        || setup.handshake != binding.handshake
        || setup.peer != binding.peer
        || setup.local_is_low != binding.local_is_low
        || local_point[0] != 4
        || setup.peer_key[0] != 4
        || catcoms_sync::voice_v2::key_reference(local_point) != binding.local_key
        || catcoms_sync::voice_v2::key_reference(&setup.peer_key) != binding.peer_key
    {
        return Err(VoiceError::Malformed);
    }
    let logical = id.logical();
    let mut w = Writer { out, offset: 0 };
    w.put(&[1, 5])?;
    w.put(&logical)?;
    w.put(&action.serial.to_be_bytes())?;
    w.put(&action.version.to_be_bytes())?;
    w.put(&action.input_tag)?;
    // Native-scoped adapter chain identity: the ORIGINAL derivation row serial. Root has
    // no public plan serial. Subsequent confirmation rows must keep this original identity.
    w.put(&logical[8..])?;
    w.put(&binding.scope.group)?;
    w.put(&[u8::from(binding.scope.policy.is_some())])?;
    w.put(&binding.scope.policy.unwrap_or([0; 32]))?;
    w.put(&binding.scope.room.to_be_bytes())?;
    w.put(&binding.epoch.to_be_bytes())?;
    for field in [
        binding.local.as_bytes(),
        &binding.local_join,
        &binding.local_key,
        binding.peer.as_bytes(),
        &binding.peer_join,
        &binding.peer_key,
        &binding.handshake,
    ] {
        w.put(field)?;
    }
    w.put(&binding.generation.to_be_bytes())?;
    w.put(&binding.deadline.to_be_bytes())?;
    w.put(&[u8::from(binding.local_is_low)])?;
    w.put(
        &u32::try_from(engine_bytes)
            .map_err(|_| VoiceError::Bounds)?
            .to_be_bytes(),
    )?;
    w.put(&[0])?; // closed input-operation tag0: Derive
    w.put(local_point)?;
    w.put(&setup.peer_key)?;
    w.put(
        &u16::try_from(setup.transcript.len())
            .map_err(|_| VoiceError::Bounds)?
            .to_be_bytes(),
    )?;
    w.put(&setup.transcript)?;
    if w.offset != w.out.len() {
        return Err(VoiceError::Bounds);
    }
    Ok(())
}
struct Writer<'a> {
    out: &'a mut [u8],
    offset: usize,
}
impl Writer<'_> {
    fn put(&mut self, value: &[u8]) -> Result<(), VoiceError> {
        let end = self
            .offset
            .checked_add(value.len())
            .ok_or(VoiceError::Bounds)?;
        self.out
            .get_mut(self.offset..end)
            .ok_or(VoiceError::Bounds)?
            .copy_from_slice(value);
        self.offset = end;
        Ok(())
    }
}

/// Bind the opened request to its exact original Session/purpose/row before any input owner.
/// Terminal history is returned as Control instead: it cannot reconstruct browser keys.
pub(super) async fn pull(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    // Declared before preparation so every local input/work/reply on an error or cancelled
    // future drops before this original hold. In particular, reply construction/encoding
    // refusal must not clear busy or wake shutdown ahead of its still-live input owners.
    let _physical = incoming.hold_position()?;
    // Errors retain the original Incoming so even refusals use its one spent response nonce.
    let prepared = prepare(context, route, &incoming).await;
    match prepared {
        Ok(Prepared::Input { work, input }) => serialize_data(route, incoming, work, input),
        other => serialize_control(incoming, other),
    }
}
/// Keep the synchronous Data and Control serializers in separate call frames. They never
/// coexist: Data retains actual input/Work; Control constructs only fixed historical bytes.
/// Neither path borrows an extra variable buffer or carries serializer locals across awaits.
fn serialize_data(
    route: &Route,
    incoming: Incoming,
    work: super::ledger::Work,
    input: super::custody::InputCustody,
) -> Result<CompletedReply, channel::Refused> {
    let binding = work
        .action()
        .protected_binding()
        .map_err(|_| channel::Refused::Binding)?;
    let CryptoInput::Derive(setup) = input.input() else {
        return Err(channel::Refused::Binding);
    };
    let mut reply = incoming.reply(
        BodyKind::Data,
        FIXED_BYTES + setup.transcript.len(),
        Some(work.phase()),
    )?;
    encode(
        reply.plain_mut(),
        work.id(),
        Action {
            serial: work.action().serial(),
            version: work.action().version(),
            input_tag: work.action().input_tag(),
        },
        work.action().engine_bytes(),
        &binding,
        &route.selected().point(),
        setup,
    )
    .map_err(|_| channel::Refused::Body)?;
    reply.retain_input(input, work).seal()
}
fn serialize_control(
    incoming: Incoming,
    prepared: Result<Prepared, Refused>,
) -> Result<CompletedReply, channel::Refused> {
    let encoded = match prepared {
        Ok(Prepared::History(id, status)) => super::replies::status(id, &status)
            .map(Encoded::History)
            .unwrap_or_else(|e| Encoded::Refused(super::replies::refused(e.into()))),
        Err(error) => Encoded::Refused(super::replies::refused(error)),
        Ok(Prepared::Input { .. }) => unreachable!(),
    };
    let bytes = match &encoded {
        Encoded::History(b) => b.as_slice(),
        Encoded::Refused(b) => b.as_slice(),
    };
    let mut reply = incoming.reply(BodyKind::Control, bytes.len(), None)?;
    reply.plain_mut().copy_from_slice(bytes);
    reply.seal()
}
#[allow(clippy::large_enum_variant)]
enum Encoded {
    History([u8; super::replies::STATUS_BYTES]),
    Refused([u8; 3]),
}
enum Prepared {
    Input {
        work: super::ledger::Work,
        input: super::custody::InputCustody,
    },
    History(RowId, super::ledger::Status),
}

async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<Prepared, Refused> {
    if !incoming.belongs_to(route.session())
        || incoming.operation() != Operation::Input
        || incoming.kind() != BodyKind::Control
    {
        return Err(VoiceError::Unauthorized.into());
    }
    let selector = {
        let request = Request::decode(
            incoming.operation(),
            incoming.plain().map_err(|_| VoiceError::Malformed)?,
        )?;
        if !matches!(request.kind, Kind::Input) {
            return Err(VoiceError::Malformed.into());
        }
        request.selector
    };
    let row = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let row = live.ledger.resolve(
            route.selected(),
            Purpose::Derive(selector),
            RowId::from_logical(incoming.logical())?,
        )?;
        {
            let status = row.reconcile()?;
            if status.engine.receipt.is_some() || status.engine.settled || status.drained {
                return Ok(Prepared::History(row.id(), status));
            }
        }
        row
    };
    // Request decoding and the historical snapshot end before suspension. Neither belongs
    // to the binding/commit future or duplicates its later actual Work/input owners.
    let commit = super::factory::preflight_binding(context, route).await?;
    let _commit = commit.lock_owned().await;
    let context = context.upgrade().ok_or(Refused::Closed)?;
    context
        .gate()
        .require_unlocked()
        .await
        .map_err(|_| Refused::Closed)?;
    let work = row.work()?;
    let input = {
        let authority = context.authority.lock().unwrap_or_else(|e| e.into_inner());
        super::factory::require_current_selection(context.gate(), &authority, route.selected())?;
        route
            .with_open(|| work.phase().input(work.action()))
            .map_err(|_| Refused::Closed)??
    };
    Ok(Prepared::Input { work, input })
}

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn future<F: std::future::Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    // Count the actual compiler-generated outer/prepare future, plus synchronous resolver,
    // encoding and completed-output graphs conservatively. The codec separately counts the
    // physical hold/header backing; Data reply bytes borrow the SAME paid Phase copy loan.
    future(pull)
        + std::mem::size_of::<super::context::LiveContext>()
        + std::mem::size_of::<super::ledger::ResolvedRow>()
        + std::mem::size_of::<Request<'static>>()
        + std::mem::size_of::<Prepared>()
        // Separate synchronous serializers above cannot coexist. Count the full larger
        // serializer graph, in addition to Prepared/future/output storage counted above.
        + std::mem::size_of::<Encoded>().max(
            std::mem::size_of::<Writer<'static>>() + std::mem::size_of::<ProtectedCryptoBinding>())
        + 128 // synchronous gate/argument/encoding temporaries, no payload allocation
}

#[cfg(test)]
mod tests;
