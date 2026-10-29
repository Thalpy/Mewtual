//! Closed control syntax and immutable-record identity. No operation here owns a
//! live query, admission preclaim, original terminal result or collection proof.
use super::*;
use agreement::{Agreement, Endpoint, RoundRef};

// Agreed inactive grammar: bit0 control7, bit1 encrypted8, bit2 Ready9 forwarding.
// Zero/partial support is an observation, never reservation, key or consent.
const KNOWN_FEATURES: u16 = 7;
const MAX_CAPABILITY: usize = 512;
const TERMINAL_DOMAIN: &[u8] = b"mewtual/voice/admission/terminal/v1\0";

pub(super) struct Control<'a> {
    pub(super) epoch: u64,
    pub(super) operation: Operation<'a>,
}
pub(super) enum Operation<'a> {
    Query {
        target: DeviceId,
        kind: QueryKind,
    },
    Support {
        target: DeviceId,
        query: Nonce,
        features: u16,
    },
    Availability(Availability),
    Request(Request<'a>),
    Grant(Grant),
    Commit(Commit),
    Busy {
        target: DeviceId,
        request: Nonce,
        hint_ms: u16,
    },
    Status {
        target: DeviceId,
        request: Nonce,
    },
    Terminal(TerminalObservation),
}
pub(super) enum QueryKind {
    ProtocolSupport,
    Availability,
}
pub(super) struct Availability {
    target: DeviceId,
    query: Nonce,
    join: Nonce,
    key_ref: Nonce,
    incarnation: Nonce,
    generation: u64,
    serial: u64,
    available: bool,
}
pub(super) struct Request<'a> {
    low: Endpoint,
    high: Endpoint,
    low_nonce: Nonce,
    capability: &'a [u8],
    capability_expires: u64,
}
pub(super) struct Grant {
    target: DeviceId,
    request: Nonce,
    high_nonce: Nonce,
    commit_by_wall: u64,
}
pub(super) struct Commit {
    target: DeviceId,
    request: Nonce,
    grant: Nonce,
    round: Nonce,
    low_nonce: Nonce,
    high_nonce: Nonce,
}

/// Unknown is an observation, NEVER a recorded rejected/accepted result. It has
/// no record/digest conversion and cannot replace known history or prove return.
pub(super) enum TerminalObservation {
    Unknown {
        target: DeviceId,
        request: Nonce,
    },
    Recorded {
        record: Recorded,
        observed: Observed,
    },
}
#[derive(Clone, Copy)]
pub(super) struct Recorded {
    target: DeviceId,
    request: Nonce,
    grant: Option<Nonce>,
    round: Option<Nonce>,
    decision: Decision,
}
#[derive(Clone, Copy)]
enum Decision {
    Accepted { at_wall: u64 },
    Rejected(Reason),
}
#[derive(Clone, Copy)]
enum Reason {
    Cancelled = 1,
    Expired = 2,
    Superseded = 3,
    AuthorityChanged = 4,
    ProtocolRejected = 5,
}
#[derive(Clone, Copy)]
pub(super) enum Observed {
    HistoricalOnly,
    Negotiating,
    Established,
    Closing,
    Expired,
    Collected,
}
redacted_debug!(Control<'_>, TerminalObservation, Recorded);

impl<'a> Control<'a> {
    pub(super) fn decode(payload: &'a [u8], common: &Header) -> Result<Self, VoiceError> {
        let mut raw = Raw::new(payload);
        if raw.u8()? != 1 {
            return Err(VoiceError::Malformed);
        }
        let operation = raw.u8()?;
        if raw.u8()? != ADMISSION_PROFILE {
            return Err(VoiceError::Malformed);
        }
        let epoch = raw.u64()?;
        let operation = match operation {
            0 => Operation::Query {
                target: device(&mut raw)?,
                kind: match raw.u8()? {
                    0 => QueryKind::ProtocolSupport,
                    1 => QueryKind::Availability,
                    _ => return Err(VoiceError::Malformed),
                },
            },
            1 => {
                let target = device(&mut raw)?;
                let query = raw.array()?;
                let features = raw.u16()?;
                if features & !KNOWN_FEATURES != 0 {
                    return Err(VoiceError::Malformed);
                }
                Operation::Support {
                    target,
                    query,
                    features,
                }
            }
            2 => {
                let value = Availability {
                    target: device(&mut raw)?,
                    query: raw.array()?,
                    join: raw.array()?,
                    key_ref: raw.array()?,
                    incarnation: raw.array()?,
                    generation: raw.u64()?,
                    serial: raw.u64()?,
                    available: raw.boolean()?,
                };
                let layout = raw.u8()?;
                if value.generation == 0 || value.serial == 0 || layout != u8::from(value.available)
                {
                    return Err(VoiceError::Malformed);
                }
                Operation::Availability(value)
            }
            3 => Operation::Request(Request::decode(&mut raw, common, epoch)?),
            4 => {
                let value = Grant {
                    target: device(&mut raw)?,
                    request: raw.array()?,
                    high_nonce: raw.array()?,
                    commit_by_wall: raw.u64()?,
                };
                if raw.u16()? != 1 {
                    return Err(VoiceError::Malformed);
                }
                value.check_window(common.issued, common.expires)?;
                Operation::Grant(value)
            }
            5 => Operation::Commit(Commit {
                target: device(&mut raw)?,
                request: raw.array()?,
                grant: raw.array()?,
                round: raw.array()?,
                low_nonce: raw.array()?,
                high_nonce: raw.array()?,
            }),
            6 => {
                let target = device(&mut raw)?;
                let request = raw.array()?;
                let hint_ms = raw.u16()?;
                // A scheduling hint cannot renew the original attempts/deadline.
                if hint_ms > 1000 {
                    return Err(VoiceError::Bounds);
                }
                Operation::Busy {
                    target,
                    request,
                    hint_ms,
                }
            }
            7 => Operation::Status {
                target: device(&mut raw)?,
                request: raw.array()?,
            },
            8 => Operation::Terminal(TerminalObservation::decode(&mut raw, common, epoch)?),
            _ => return Err(VoiceError::Malformed),
        };
        raw.finish()?;
        if operation.target() == common.issuer {
            return Err(VoiceError::Malformed);
        }
        Ok(Self { epoch, operation })
    }
}
impl Operation<'_> {
    pub(super) fn target(&self) -> DeviceId {
        match self {
            Self::Query { target, .. }
            | Self::Support { target, .. }
            | Self::Busy { target, .. }
            | Self::Status { target, .. } => *target,
            Self::Availability(value) => value.target,
            Self::Request(value) => value.high.device,
            Self::Grant(value) => value.target,
            Self::Commit(value) => value.target,
            Self::Terminal(TerminalObservation::Unknown { target, .. }) => *target,
            Self::Terminal(TerminalObservation::Recorded { record, .. }) => record.target,
        }
    }
}
fn device(raw: &mut Raw<'_>) -> Result<DeviceId, VoiceError> {
    Ok(DeviceId::from_bytes(raw.array()?))
}
fn optional_digest(raw: &mut Raw<'_>) -> Result<Option<Nonce>, VoiceError> {
    let present = raw.boolean()?;
    let bytes = raw.array()?;
    if !present && bytes != [0; 32] {
        return Err(VoiceError::Malformed);
    }
    Ok(present.then_some(bytes))
}

impl<'a> Request<'a> {
    fn decode(raw: &mut Raw<'a>, common: &Header, epoch: u64) -> Result<Self, VoiceError> {
        let low_device = device(raw)?;
        let high_device = device(raw)?;
        let low_join = raw.array()?;
        let high_join = raw.array()?;
        let low_key = raw.array()?;
        let high_key = raw.array()?;
        let low_incarnation = raw.array()?;
        let high_incarnation = raw.array()?;
        let low_generation = raw.u64()?;
        let high_generation = raw.u64()?;
        let low_nonce = raw.array()?;
        let length = usize::from(raw.u16()?);
        if length > MAX_CAPABILITY {
            return Err(VoiceError::Bounds);
        }
        let capability = raw.take(length)?;
        if low_device >= high_device
            || common.issuer != low_device
            || low_generation == 0
            || high_generation == 0
        {
            return Err(VoiceError::Scope);
        }
        let signed = SignedAdmitted::decode_availability(capability)?;
        let AdmittedBody::Control(Control {
            epoch: cap_epoch,
            operation: Operation::Availability(cap),
        }) = signed.body()
        else {
            return Err(VoiceError::Malformed);
        };
        if signed.header().scope != common.scope
            || *cap_epoch != epoch
            || signed.header().issuer != high_device
            || cap.target != low_device
            || !cap.available
            || cap.join != high_join
            || cap.key_ref != high_key
            || cap.incarnation != high_incarnation
            || cap.generation != high_generation
        {
            return Err(VoiceError::Scope);
        }
        // A nested capability can only be a fixed189-byte Availability. It cannot
        // contain another Request or recursively allocate/parser-stack-grow.
        Ok(Self {
            low: Endpoint {
                device: low_device,
                join: low_join,
                key_ref: low_key,
                incarnation: low_incarnation,
                generation: low_generation,
            },
            high: Endpoint {
                device: high_device,
                join: high_join,
                key_ref: high_key,
                incarnation: high_incarnation,
                generation: high_generation,
            },
            low_nonce,
            capability,
            capability_expires: signed.header().expires,
        })
    }
}
impl Grant {
    fn check_window(&self, issued: u64, upper: u64) -> Result<(), VoiceError> {
        if self.commit_by_wall < issued.saturating_sub(CLOCK_SKEW_MS) || self.commit_by_wall > upper
        {
            Err(VoiceError::Freshness)
        } else {
            Ok(())
        }
    }
}

impl TerminalObservation {
    fn decode(raw: &mut Raw<'_>, common: &Header, epoch: u64) -> Result<Self, VoiceError> {
        let target = device(raw)?;
        let request = raw.array()?;
        let grant = optional_digest(raw)?;
        let round = optional_digest(raw)?;
        let decision = raw.u8()?;
        let reason = raw.u8()?;
        let at_wall = raw.u64()?;
        let digest: Nonce = raw.array()?;
        let observed = raw.u8()?;
        if decision == 0 {
            if reason != 0
                || grant.is_some()
                || round.is_some()
                || at_wall != 0
                || digest != [0; 32]
                || observed != 6
            {
                return Err(VoiceError::Malformed);
            }
            return Ok(Self::Unknown { target, request });
        }
        let decision = match decision {
            1 if reason == 0 && grant.is_some() && round.is_some() => {
                Decision::Accepted { at_wall }
            }
            2 if at_wall == 0 => Decision::Rejected(match reason {
                1 => Reason::Cancelled,
                2 => Reason::Expired,
                3 => Reason::Superseded,
                4 => Reason::AuthorityChanged,
                5 => Reason::ProtocolRejected,
                _ => return Err(VoiceError::Malformed),
            }),
            _ => return Err(VoiceError::Malformed),
        };
        if round.is_some() && grant.is_none() {
            return Err(VoiceError::Malformed);
        }
        let observed = match observed {
            0 => Observed::HistoricalOnly,
            1 => Observed::Negotiating,
            2 => Observed::Established,
            3 => Observed::Closing,
            4 => Observed::Expired,
            5 => Observed::Collected,
            _ => return Err(VoiceError::Malformed),
        };
        if matches!(decision, Decision::Rejected(_))
            && matches!(observed, Observed::Negotiating | Observed::Established)
        {
            return Err(VoiceError::Malformed);
        }
        let record = Recorded {
            target,
            request,
            grant,
            round,
            decision,
        };
        if record.digest(common.scope, epoch, common.issuer) != digest {
            return Err(VoiceError::Signature);
        }
        // accepted_wall belongs to the first historical decision, not the latest
        // wrapper's issued/expires. A wall rollback cannot rewrite that history.
        Ok(Self::Recorded { record, observed })
    }
}
impl Recorded {
    fn digest(&self, scope: Scope, epoch: u64, issuer: DeviceId) -> Nonce {
        let mut record = RawOutput::<263>::new();
        record.put(&[1, ADMISSION_PROFILE]);
        put_raw_scope(&mut record, scope);
        record.put(&epoch.to_be_bytes());
        record.put(issuer.as_bytes());
        record.put(self.target.as_bytes());
        record.put(&self.request);
        for digest in [self.grant, self.round] {
            record.put(&[u8::from(digest.is_some())]);
            record.put(&digest.unwrap_or([0; 32]));
        }
        match self.decision {
            Decision::Accepted { at_wall } => {
                record.put(&[1, 0]);
                record.put(&at_wall.to_be_bytes());
            }
            Decision::Rejected(reason) => {
                record.put(&[2, reason as u8]);
                record.put(&0u64.to_be_bytes());
            }
        }
        let mut hash = blake3::Hasher::new();
        hash.update(TERMINAL_DOMAIN);
        hash.update(&record.finish());
        *hash.finalize().as_bytes()
    }
}

/// Pure relationship validation only. These verified wrappers still do not prove
/// current presence, an original query/preclaim or native Commit admission.
fn assemble_agreement(
    request: &SignedAdmitted<'_>,
    grant: &SignedAdmitted<'_>,
) -> Result<Agreement, VoiceError> {
    let AdmittedBody::Control(Control {
        epoch,
        operation: Operation::Request(r),
    }) = request.body()
    else {
        return Err(VoiceError::Malformed);
    };
    let AdmittedBody::Control(Control {
        epoch: grant_epoch,
        operation: Operation::Grant(g),
    }) = grant.body()
    else {
        return Err(VoiceError::Malformed);
    };
    if request.header().scope != grant.header().scope
        || epoch != grant_epoch
        || grant.header().issuer != r.high.device
        || g.target != r.low.device
        || g.request != request.hash()
    {
        return Err(VoiceError::Scope);
    }
    g.check_window(
        grant.header().issued,
        request
            .header()
            .expires
            .min(grant.header().expires)
            .min(r.capability_expires),
    )?;
    Ok(Agreement {
        scope: request.header().scope,
        epoch: *epoch,
        low: r.low,
        high: r.high,
        low_nonce: r.low_nonce,
        high_nonce: g.high_nonce,
        request_digest: request.hash(),
        grant_digest: grant.hash(),
        commit_by_wall: g.commit_by_wall,
    })
}
fn check_commit(agreement: &Agreement, commit: &SignedAdmitted<'_>) -> Result<(), VoiceError> {
    let AdmittedBody::Control(Control {
        epoch,
        operation: Operation::Commit(c),
    }) = commit.body()
    else {
        return Err(VoiceError::Malformed);
    };
    if commit.header().scope != agreement.scope
        || *epoch != agreement.epoch
        || commit.header().issuer != agreement.low.device
        || c.target != agreement.high.device
        || c.request != agreement.request_digest
        || c.grant != agreement.grant_digest
        || c.round != *RoundRef::derive(agreement)?.bytes()
        || c.low_nonce != agreement.low_nonce
        || c.high_nonce != agreement.high_nonce
    {
        return Err(VoiceError::Scope);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
