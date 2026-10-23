//! Inactive child Input projection. Only an actual registered Root action lends bytes;
//! metadata/role hints cannot create a packet, a decrypt token, a parent or an engine.
use super::bootstrap::Route;
use super::channel::{self, BodyKind, CompletedReply, Incoming, Operation};
use super::context::{Context, LiveContext};
use super::custody::InputCustody;
use super::factory::Refused;
use super::ledger::{Purpose, ResolvedRow, RowId, Status, Work};
use super::requests::{confirmation::Request, Action, Kind};
use catcoms_sync::voice_v2::crypto_actions::{
    CryptoInput, CryptoOperation, ProtectedCryptoBinding,
};
use catcoms_sync::voice_v2::VoiceError;

/// Fixed tag7 header includes BOTH native child and original parent RowIds. The chain
/// serial remains the original parent's serial; a successor never resets key/nonce state.
pub(super) const FIXED_BYTES: usize = 430;

/// Synchronous resolution deliberately ends the larger borrowing request and parent handle
/// before any await. The returned child itself retains the same actual strong parent.
pub(super) fn resolve(
    context: &LiveContext,
    route: &Route,
    request: &Request<'_>,
) -> Result<ResolvedRow, VoiceError> {
    let parent = context.ledger.resolve(
        route.selected(),
        Purpose::Derive(request.selector),
        request.parent,
    )?;
    context.ledger.lookup_child(
        route.selected(),
        &parent,
        request.purpose,
        Some(request.row.ok_or(VoiceError::Unauthorized)?),
    )
}

/// Original physical custody precedes every input/work/reply owner in this function's
/// destruction order. Cancellation cannot reopen the channel ahead of those real owners.
pub(super) async fn pull(
    context: &Context,
    route: &Route,
    incoming: Incoming,
) -> Result<CompletedReply, channel::Refused> {
    let _physical = incoming.hold_position()?;
    match prepare(context, route, &incoming).await {
        Ok(Prepared::Input { work, input }) => serialize_data(incoming, work, input),
        other => serialize_control(incoming, other),
    }
}
enum Prepared {
    Input { work: Work, input: InputCustody },
    History(RowId, Status),
}
async fn prepare(
    context: &Context,
    route: &Route,
    incoming: &Incoming,
) -> Result<Prepared, Refused> {
    let row = {
        let live = context.upgrade().ok_or(Refused::Closed)?;
        let row = decode_input(&live, route, incoming)?;
        {
            let status = row.reconcile()?;
            if status.engine.receipt.is_some() || status.engine.settled || status.drained {
                return Ok(Prepared::History(row.id(), status));
            }
        }
        row
    };
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

/// This synchronous call frame owns the full request and transient parent resolver. It
/// returns only the actual child before prepare's first suspension; neither request nor
/// parent temporaries can coexist with a later Data/Control serializer call frame.
fn decode_input(
    context: &LiveContext,
    route: &Route,
    incoming: &Incoming,
) -> Result<ResolvedRow, Refused> {
    if !incoming.belongs_to(route.session()) || incoming.operation() != Operation::Input {
        return Err(VoiceError::Unauthorized.into());
    }
    let request = Request::decode(
        incoming.operation(),
        incoming.kind(),
        incoming.logical(),
        incoming.plain().map_err(|_| VoiceError::Malformed)?,
    )?;
    if !matches!(request.kind, Kind::Input) {
        return Err(VoiceError::Malformed.into());
    }
    Ok(resolve(context, route, &request)?)
}

struct Projection<'a> {
    binding: ProtectedCryptoBinding,
    role: u8,
    operation: u8,
    sequence: u64,
    aad: &'a [u8],
    body: &'a [u8],
}
impl<'a> Projection<'a> {
    fn new(work: &Work, input: &'a InputCustody) -> Result<Self, VoiceError> {
        let binding = work.action().protected_binding()?;
        let role = work.action().protected_row()?;
        if !(1..=9).contains(&role) {
            return Err(VoiceError::Unauthorized);
        }
        let seal = if binding.local_is_low {
            (role - 1) % 3 != 1
        } else {
            (role - 1) % 3 == 1
        };
        let (operation, sequence, aad, body) = match input.input() {
            CryptoInput::EncryptConfirmation(flight)
                if seal
                    && work.action().operation() == CryptoOperation::EncryptConfirmation
                    && flight.handshake == binding.handshake =>
            {
                (
                    1,
                    flight.reservation.sequence,
                    flight.reservation.aad.as_slice(),
                    flight.plaintext.as_slice(),
                )
            }
            CryptoInput::DecryptConfirmation(packet)
                if !seal && work.action().operation() == CryptoOperation::DecryptConfirmation =>
            {
                (2, packet.header.sequence, packet.aad(), packet.ciphertext())
            }
            _ => return Err(VoiceError::Unauthorized),
        };
        // CryptoFlight.action is the pair's flight serial, whereas CryptoAction.serial
        // also counts Derive/decrypt actions. They need not be equal. InputCustody already
        // borrowed this exact Work action through its actual Phase; never bind by numbers
        // from these different serial spaces (normal first Hello already differs).
        let ciphertext_bytes = body
            .len()
            .checked_add(if seal { 16 } else { 0 })
            .ok_or(VoiceError::Bounds)?;
        if sequence == 0
            || aad.len() > 4096
            || !(16..=528).contains(&ciphertext_bytes)
            || FIXED_BYTES + aad.len() + body.len() > 2048
            || work.action().engine_bytes() != 4 * (aad.len() + ciphertext_bytes) + 3 * 512 + 2048
        {
            return Err(VoiceError::Bounds);
        }
        Ok(Self {
            binding,
            role,
            operation,
            sequence,
            aad,
            body,
        })
    }
    fn encode(&self, out: &mut [u8], work: &Work) -> Result<(), VoiceError> {
        if out.len() != FIXED_BYTES + self.aad.len() + self.body.len() {
            return Err(VoiceError::Bounds);
        }
        let b = &self.binding;
        let mut w = Writer { out, offset: 0 };
        w.put(&[1, 7])?;
        w.put(&work.id().logical())?;
        let action = Action {
            serial: work.action().serial(),
            version: work.action().version(),
            input_tag: work.action().input_tag(),
        };
        w.put(&action.serial.to_be_bytes())?;
        w.put(&action.version.to_be_bytes())?;
        w.put(&action.input_tag)?;
        w.put(&work.confirmation_parent_id()?.logical())?;
        w.put(&b.scope.group)?;
        w.put(&[u8::from(b.scope.policy.is_some())])?;
        w.put(&b.scope.policy.unwrap_or([0; 32]))?;
        w.put(&b.scope.room.to_be_bytes())?;
        w.put(&b.epoch.to_be_bytes())?;
        for field in [
            b.local.as_bytes(),
            &b.local_join,
            &b.local_key,
            b.peer.as_bytes(),
            &b.peer_join,
            &b.peer_key,
            &b.handshake,
        ] {
            w.put(field)?;
        }
        w.put(&b.generation.to_be_bytes())?;
        w.put(&b.deadline.to_be_bytes())?;
        w.put(&[u8::from(b.local_is_low)])?;
        w.put(
            &u32::try_from(work.action().engine_bytes())
                .map_err(|_| VoiceError::Bounds)?
                .to_be_bytes(),
        )?;
        w.put(&[self.operation, self.role])?;
        w.put(&self.sequence.to_be_bytes())?;
        w.put(&(self.aad.len() as u16).to_be_bytes())?;
        w.put(&(self.body.len() as u16).to_be_bytes())?;
        w.put(self.aad)?;
        w.put(self.body)?;
        if w.offset != w.out.len() {
            return Err(VoiceError::Bounds);
        }
        Ok(())
    }
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
fn serialize_data(
    incoming: Incoming,
    work: Work,
    input: InputCustody,
) -> Result<CompletedReply, channel::Refused> {
    let projection = Projection::new(&work, &input).map_err(|_| channel::Refused::Binding)?;
    let mut reply = incoming.reply(
        BodyKind::Data,
        FIXED_BYTES + projection.aad.len() + projection.body.len(),
        Some(work.phase()),
    )?;
    projection
        .encode(reply.plain_mut(), &work)
        .map_err(|_| channel::Refused::Body)?;
    reply.retain_input(input, work).seal()
}
#[allow(clippy::large_enum_variant)]
enum Encoded {
    History([u8; super::replies::STATUS_BYTES]),
    Refused([u8; 3]),
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

#[cfg(test)]
pub(super) fn layout_bytes() -> usize {
    fn future<F: std::future::Future>(
        _: impl FnOnce(&'static Context, &'static Route, Incoming) -> F,
    ) -> usize {
        std::mem::size_of::<F>() + std::mem::size_of::<F::Output>()
    }
    let decode = std::mem::size_of::<Request<'static>>() + 2 * std::mem::size_of::<ResolvedRow>();
    let data = std::mem::size_of::<Writer<'static>>()
        + std::mem::size_of::<Projection<'static>>()
        + std::mem::size_of::<Action>()
        + std::mem::size_of::<RowId>();
    let control = std::mem::size_of::<Encoded>();
    // Actual outer/prepare futures and completed output remain additive, including all
    // retained physical/Input/Work owners. Only these separate synchronous call frames
    // are mutually exclusive: decode_input returns BEFORE the first await; serializers
    // begin AFTER prepare returns (and its LiveContext/guards have dropped). The earlier
    // sum counted the dead decoder alongside the later projection and overstated peak.
    future(pull)
        + std::mem::size_of::<LiveContext>()
        + std::mem::size_of::<Prepared>()
        + decode.max(data).max(control)
        + 128
}

#[cfg(test)]
mod tests;
