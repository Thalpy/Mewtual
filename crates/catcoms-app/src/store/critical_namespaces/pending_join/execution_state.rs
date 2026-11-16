//! Closed execution DATA grammar for the explicit private checkpoint network policy.
//! No decoder/builder below issues a grant, submission ticket or accepted join. Only the
//! actual configured reader, durable arm producer and once-bound network owner consume it.
use super::refused;
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};

const TAG: &str = "catcoms/destination-join-execution/v1";
pub(in crate::store::critical_namespaces) const MAX_GRANT: u32 = 64;
pub(in crate::store::critical_namespaces) const MAX_BODY: usize = 2 * 1024 * 1024;
// Inner authenticated source/arm framing has its own fixed allowance; it must NOT be counted
// as if the AEAD contained only body bytes. Whole JOIN payload remains separately16MiB.
pub(in crate::store::critical_namespaces) const MAX_INPUT_PLAIN: usize = MAX_BODY + 1024;
pub(in crate::store::critical_namespaces) const MAX_INPUT_CIPHER: usize = MAX_INPUT_PLAIN + 16;
pub(in crate::store::critical_namespaces) const MAX_RECORD: usize = MAX_INPUT_CIPHER + 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces) struct SourceIdentity {
    pub(in crate::store::critical_namespaces) binding: [u8; 32],
    pub(in crate::store::critical_namespaces) request: [u8; 32],
    pub(in crate::store::critical_namespaces) package: [u8; 32],
}

/// Durable invocation labels ONLY. Cold restore always treats this as possibly spent. None
/// of these bytes identifies a live context Arc, proves a current connection or creates a ticket.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces) struct Arm {
    pub(in crate::store::critical_namespaces) source: SourceIdentity,
    pub(in crate::store::critical_namespaces) grant: u64,
    pub(in crate::store::critical_namespaces) ordinal: u64,
    pub(in crate::store::critical_namespaces) scheduling_revision: u64,
    pub(in crate::store::critical_namespaces) activation: [u8; 32],
    pub(in crate::store::critical_namespaces) invocation: [u8; 32],
    pub(in crate::store::critical_namespaces) deadline_ms: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces) struct InputSummary {
    pub(in crate::store::critical_namespaces) arm: Arm,
    pub(in crate::store::critical_namespaces) body_length: u32,
    pub(in crate::store::critical_namespaces) body_digest: [u8; 32],
    pub(in crate::store::critical_namespaces) ciphertext_digest: [u8; 32],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces) struct State {
    pub(in crate::store::critical_namespaces) grant: u64,
    pub(in crate::store::critical_namespaces) limit: u32,
    pub(in crate::store::critical_namespaces) spent: u32,
    pub(in crate::store::critical_namespaces) ordinal: u64,
    pub(in crate::store::critical_namespaces) arm: Option<Arm>,
    pub(in crate::store::critical_namespaces) input: Option<InputSummary>,
}
impl State {
    pub(in crate::store::critical_namespaces) fn not_granted() -> Self {
        Self {
            grant: 0,
            limit: 0,
            spent: 0,
            ordinal: 0,
            arm: None,
            input: None,
        }
    }

    fn validate(&self, source: &SourceIdentity, scheduling_revision: u64) -> Result<(), AppError> {
        if scheduling_revision == 0 {
            return Err(refused());
        }
        if self.grant == 0 {
            return if *self == Self::not_granted() {
                Ok(())
            } else {
                Err(refused())
            };
        }
        if self.limit == 0
            || self.limit > MAX_GRANT
            || self.spent > self.limit
            || self.ordinal < u64::from(self.spent)
        {
            return Err(refused());
        }
        match self.arm {
            None if self.ordinal == 0 && self.spent == 0 && self.input.is_none() => {}
            Some(arm) => {
                validate_arm(&arm, source, scheduling_revision)?;
                if arm.ordinal != self.ordinal
                    || arm.grant > self.grant
                    || (self.spent > 0 && arm.grant != self.grant)
                    || (self.spent == 0 && arm.grant >= self.grant)
                {
                    return Err(refused());
                }
            }
            _ => return Err(refused()),
        }
        if let Some(input) = self.input {
            validate_arm(&input.arm, source, scheduling_revision)?;
            let latest = self.arm.ok_or_else(refused)?;
            if input.arm.ordinal > self.ordinal
                || input.arm.grant > self.grant
                // An ordinal has ONE original durable arm. Later zero-submit grants cannot
                // create a different activation/invocation/deadline for that old ordinal.
                || (input.arm.ordinal == latest.ordinal && input.arm != latest)
                || (input.arm.ordinal < latest.ordinal
                    && (input.arm.grant > latest.grant
                        || input.arm.scheduling_revision > latest.scheduling_revision))
                || input.body_length == 0
                || input.body_length as usize > MAX_BODY
            {
                return Err(refused());
            }
        }
        Ok(())
    }
}

pub(in crate::store::critical_namespaces) struct Input<'a> {
    pub(in crate::store::critical_namespaces) summary: InputSummary,
    pub(in crate::store::critical_namespaces) nonce: &'a [u8; 24],
    pub(in crate::store::critical_namespaces) ciphertext: &'a [u8],
}
pub(in crate::store::critical_namespaces) struct Parsed<'a> {
    pub(in crate::store::critical_namespaces) state: State,
    pub(in crate::store::critical_namespaces) input: Option<Input<'a>>,
}

fn array<const N: usize>(input: &mut Decoder<'_>) -> Result<[u8; N], AppError> {
    input
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())
}
fn parse_arm(bytes: &[u8]) -> Result<Arm, AppError> {
    // Fixed five32-byte labels plus fouru64 values, each label u32-framed.
    if bytes.len() != 5 * 36 + 4 * 8 {
        return Err(refused());
    }
    let mut input = Decoder::new(bytes);
    let arm = Arm {
        source: SourceIdentity {
            binding: array(&mut input)?,
            request: array(&mut input)?,
            package: array(&mut input)?,
        },
        grant: input.get_u64().map_err(|_| refused())?,
        ordinal: input.get_u64().map_err(|_| refused())?,
        scheduling_revision: input.get_u64().map_err(|_| refused())?,
        activation: array(&mut input)?,
        invocation: array(&mut input)?,
        deadline_ms: input.get_u64().map_err(|_| refused())?,
    };
    input.finish().map_err(|_| refused())?;
    Ok(arm)
}
fn validate_arm(
    arm: &Arm,
    source: &SourceIdentity,
    scheduling_revision: u64,
) -> Result<(), AppError> {
    if &arm.source != source
        || arm.grant == 0
        || arm.ordinal == 0
        || arm.scheduling_revision == 0
        || arm.scheduling_revision > scheduling_revision
        || arm.deadline_ms == 0
    {
        return Err(refused());
    }
    Ok(())
}

pub(in crate::store::critical_namespaces) fn parse<'a>(
    bytes: &'a [u8],
    source: &SourceIdentity,
    scheduling_revision: u64,
) -> Result<Parsed<'a>, AppError> {
    if bytes.len() > MAX_RECORD || scheduling_revision == 0 {
        return Err(refused());
    }
    let mut input = Decoder::new(bytes);
    if input.get_str().map_err(|_| refused())? != TAG {
        return Err(refused());
    }
    let mut state = State {
        grant: input.get_u64().map_err(|_| refused())?,
        limit: input.get_u32().map_err(|_| refused())?,
        spent: input.get_u32().map_err(|_| refused())?,
        ordinal: input.get_u64().map_err(|_| refused())?,
        arm: None,
        input: None,
    };
    state.arm = match input.get_u8().map_err(|_| refused())? {
        0 => None,
        1 => Some(parse_arm(input.get_bytes().map_err(|_| refused())?)?),
        _ => return Err(refused()),
    };
    let retained_input = match input.get_u8().map_err(|_| refused())? {
        0 => None,
        1 => {
            let mut frame = Decoder::new(input.get_bytes().map_err(|_| refused())?);
            let arm = parse_arm(frame.get_bytes().map_err(|_| refused())?)?;
            let body_length = frame.get_u32().map_err(|_| refused())?;
            let body_digest = array(&mut frame)?;
            let ciphertext_digest = array(&mut frame)?;
            let nonce = frame
                .get_bytes()
                .map_err(|_| refused())?
                .try_into()
                .map_err(|_| refused())?;
            let ciphertext = frame.get_bytes().map_err(|_| refused())?;
            frame.finish().map_err(|_| refused())?;
            if ciphertext.len() < 16
                || ciphertext.len() > MAX_INPUT_CIPHER
                || cipher_digest(nonce, ciphertext) != ciphertext_digest
            {
                return Err(refused());
            }
            let summary = InputSummary {
                arm,
                body_length,
                body_digest,
                ciphertext_digest,
            };
            state.input = Some(summary);
            Some(Input {
                summary,
                nonce,
                ciphertext,
            })
        }
        _ => return Err(refused()),
    };
    input.finish().map_err(|_| refused())?;
    state.validate(source, scheduling_revision)?;
    Ok(Parsed {
        state,
        input: retained_input,
    })
}

pub(in crate::store::critical_namespaces) fn cipher_digest(
    nonce: &[u8; 24],
    ciphertext: &[u8],
) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/checkpoint-join-received-cipher/v1\0");
    hash.update(nonce);
    hash.update(ciphertext);
    *hash.finalize().as_bytes()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::critical_namespaces) enum Change {
    FirstGrant,
    Debit,
    Input,
    RetryGrant,
}

/// Pure negative classification, not admission/authority. The caller must additionally enforce
/// exact original row/source/descriptor/whole-parent checks and the required scheduling change
/// for RetryGrant. Parsed or locally built States cannot create an actual ticket.
pub(in crate::store::critical_namespaces) fn successor(
    old: &State,
    next: &State,
    source: &SourceIdentity,
    old_revision: u64,
    next_revision: u64,
) -> Result<Change, AppError> {
    old.validate(source, old_revision)?;
    next.validate(source, next_revision)?;
    if *old == State::not_granted()
        && next.grant == 1
        && next.spent == 0
        && next.ordinal == 0
        && next.arm.is_none()
        && next.input.is_none()
        && old_revision == next_revision
    {
        return Ok(Change::FirstGrant);
    }
    if old.grant == 0 {
        return Err(refused());
    }
    if next.grant == old.grant
        && next.limit == old.limit
        && old.spent.checked_add(1) == Some(next.spent)
        && old.ordinal.checked_add(1) == Some(next.ordinal)
        && old.input.is_none()
        && next.input.is_none()
        && old_revision == next_revision
        && next.arm.is_some_and(|arm| {
            arm.grant == next.grant
                && arm.ordinal == next.ordinal
                && arm.scheduling_revision == next_revision
        })
    {
        return Ok(Change::Debit);
    }
    if next.grant == old.grant
        && next.limit == old.limit
        && next.spent == old.spent
        && next.ordinal == old.ordinal
        && next.arm == old.arm
        && old.input.is_none()
        && next.input.is_some()
        && old_revision == next_revision
    {
        return Ok(Change::Input);
    }
    if old.grant.checked_add(1) == Some(next.grant)
        && next.spent == 0
        && next.ordinal == old.ordinal
        && next.arm == old.arm
        && next.input == old.input
        && old_revision.checked_add(1) == Some(next_revision)
    {
        return Ok(Change::RetryGrant);
    }
    Err(refused())
}

fn encode_arm(arm: &Arm, output: &mut Encoder) -> Result<(), AppError> {
    for field in [
        &arm.source.binding,
        &arm.source.request,
        &arm.source.package,
    ] {
        output.put_bytes(field).map_err(|_| refused())?;
    }
    output.put_u64(arm.grant);
    output.put_u64(arm.ordinal);
    output.put_u64(arm.scheduling_revision);
    output.put_bytes(&arm.activation).map_err(|_| refused())?;
    output.put_bytes(&arm.invocation).map_err(|_| refused())?;
    output.put_u64(arm.deadline_ms);
    Ok(())
}

/// Exact logical output length before any encoding allocation. Saved counters are DATA; this
/// calculation supplies no actual fee, grant or ticket. The producer must reserve its real
/// whole-catalog/source/input overlap first.
pub(in crate::store::critical_namespaces) fn encoded_len(
    state: &State,
    input: Option<&Input<'_>>,
    source: &SourceIdentity,
    revision: u64,
) -> Result<usize, AppError> {
    state.validate(source, revision)?;
    if state.input != input.map(|input| input.summary) {
        return Err(refused());
    }
    let base = 4 + TAG.len() + 8 + 4 + 4 + 8 + 1 + 1;
    let arm_bytes = if state.arm.is_some() {
        4 + 5 * 36 + 4 * 8
    } else {
        0
    };
    let input_bytes = match input {
        Some(input) => {
            if input.ciphertext.len() < 16
                || input.ciphertext.len() > MAX_INPUT_CIPHER
                || cipher_digest(input.nonce, input.ciphertext) != input.summary.ciphertext_digest
            {
                return Err(refused());
            }
            // Outer input frame, framed Arm, body length, two framed32 digests, framed24
            // nonce and framed ciphertext. No temporary input encoder/copy is required.
            (4 + 4 + (5 * 36 + 4 * 8) + 4 + 2 * 36 + 28 + 4usize)
                .checked_add(input.ciphertext.len())
                .ok_or_else(refused)?
        }
        None => 0,
    };
    base.checked_add(arm_bytes)
        .and_then(|length| length.checked_add(input_bytes))
        .filter(|length| *length <= MAX_RECORD)
        .ok_or_else(refused)
}

/// Append into the already-paid whole-catalog output. The borrowed state/input cannot change
/// across this append. No intermediate2MiB record Vec is created. The caller includes the
/// exact encoded_len in its checked whole-output invoice before constructing that encoder.
pub(in crate::store::critical_namespaces) fn encode_into(
    state: &State,
    input: Option<&Input<'_>>,
    source: &SourceIdentity,
    revision: u64,
    output: &mut Encoder,
) -> Result<(), AppError> {
    let length = encoded_len(state, input, source, revision)?;
    let expected = output.len().checked_add(length).ok_or_else(refused)?;
    output.put_str(TAG).map_err(|_| refused())?;
    output.put_u64(state.grant);
    output.put_u32(state.limit);
    output.put_u32(state.spent);
    output.put_u64(state.ordinal);
    match state.arm {
        None => {
            output.put_u8(0);
        }
        Some(arm) => {
            output.put_u8(1);
            output.put_u32((5 * 36 + 4 * 8) as u32);
            encode_arm(&arm, output)?;
        }
    }
    match input {
        None => {
            output.put_u8(0);
        }
        Some(input) => {
            output.put_u8(1);
            let framed = 4 + (5 * 36 + 4 * 8) + 4 + 2 * 36 + 28 + 4 + input.ciphertext.len();
            output.put_u32(framed as u32);
            output.put_u32((5 * 36 + 4 * 8) as u32);
            encode_arm(&input.summary.arm, output)?;
            output.put_u32(input.summary.body_length);
            output
                .put_bytes(&input.summary.body_digest)
                .map_err(|_| refused())?;
            output
                .put_bytes(&input.summary.ciphertext_digest)
                .map_err(|_| refused())?;
            output.put_bytes(input.nonce).map_err(|_| refused())?;
            output.put_bytes(input.ciphertext).map_err(|_| refused())?;
        }
    }
    if output.len() != expected {
        return Err(refused());
    }
    Ok(())
}

#[cfg(test)]
fn encode(
    state: &State,
    input: Option<&Input<'_>>,
    source: &SourceIdentity,
    revision: u64,
) -> Result<Vec<u8>, AppError> {
    let length = encoded_len(state, input, source, revision)?;
    let mut output = Encoder::with_capacity(length);
    encode_into(state, input, source, revision, &mut output)?;
    Ok(output.finish())
}

#[cfg(test)]
mod tests;
