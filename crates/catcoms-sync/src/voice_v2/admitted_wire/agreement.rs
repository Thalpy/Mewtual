//! Canonical context only: these bytes authenticate no current member, committed
//! native round, key owner or RTC effect. Point checks below are SHAPE/hash checks;
//! actual original-key ownership and on-curve WebCrypto import remain mandatory.
use super::*;
use sha2::{Digest, Sha256};

const AGREEMENT_BYTES: usize = 503;
const TRANSCRIPT_BYTES: usize = 789;
const SALT_INPUT_BYTES: usize = 830;
const INFO_BYTES: usize = 839;
const ROUND_DOMAIN: &[u8] = b"mewtual/voice/admission/round/v1\0";
const PAIR_DOMAIN: &str = "mewtual/voice/pair/admitted/v1";
const SALT_DOMAIN: &[u8] = b"mewtual/voice/v2/admission/v1/transcript\0";
const LOW_INFO_DOMAIN: &[u8] = b"mewtual/voice/v2/admission/v1/aes-gcm/low-to-high\0";
const HIGH_INFO_DOMAIN: &[u8] = b"mewtual/voice/v2/admission/v1/aes-gcm/high-to-low\0";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Endpoint {
    pub(super) device: DeviceId,
    pub(super) join: Nonce,
    pub(super) key_ref: Nonce,
    pub(super) incarnation: Nonce,
    pub(super) generation: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Agreement {
    pub(super) scope: Scope,
    pub(super) epoch: u64,
    pub(super) low: Endpoint,
    pub(super) high: Endpoint,
    pub(super) low_nonce: Nonce,
    pub(super) high_nonce: Nonce,
    pub(super) request_digest: Nonce,
    pub(super) grant_digest: Nonce,
    pub(super) commit_by_wall: u64,
}
redacted_debug!(Endpoint, Agreement);

impl Agreement {
    fn validate(&self) -> Result<(), VoiceError> {
        if self.low.device >= self.high.device
            || self.low.generation == 0
            || self.high.generation == 0
        {
            return Err(VoiceError::Malformed);
        }
        Ok(())
    }
    pub(super) fn encode(&self) -> Result<[u8; AGREEMENT_BYTES], VoiceError> {
        self.validate()?;
        let mut out = RawOutput::new();
        out.put(&[1, ADMISSION_PROFILE, ADMITTED_SDP_PROFILE, SUITE]);
        put_raw_scope(&mut out, self.scope);
        out.put(&self.epoch.to_be_bytes());
        for field in [self.low.device.as_bytes(), self.high.device.as_bytes()] {
            out.put(field);
        }
        for field in [&self.low.join, &self.high.join] {
            out.put(field);
        }
        for field in [&self.low.key_ref, &self.high.key_ref] {
            out.put(field);
        }
        for field in [&self.low.incarnation, &self.high.incarnation] {
            out.put(field);
        }
        out.put(&self.low.generation.to_be_bytes());
        out.put(&self.high.generation.to_be_bytes());
        for field in [
            &self.low_nonce,
            &self.high_nonce,
            &self.request_digest,
            &self.grant_digest,
        ] {
            out.put(field);
        }
        out.put(&self.commit_by_wall.to_be_bytes());
        out.put(&1u16.to_be_bytes());
        Ok(out.finish())
    }
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, VoiceError> {
        if bytes.len() != AGREEMENT_BYTES {
            return Err(VoiceError::Bounds);
        }
        let mut raw = Raw::new(bytes);
        if raw.array::<4>()? != [1, ADMISSION_PROFILE, ADMITTED_SDP_PROFILE, SUITE] {
            return Err(VoiceError::Malformed);
        }
        let scope = get_raw_scope(&mut raw)?;
        let epoch = raw.u64()?;
        let low_device = DeviceId::from_bytes(raw.array()?);
        let high_device = DeviceId::from_bytes(raw.array()?);
        let low_join = raw.array()?;
        let high_join = raw.array()?;
        let low_key = raw.array()?;
        let high_key = raw.array()?;
        let low_incarnation = raw.array()?;
        let high_incarnation = raw.array()?;
        let low_generation = raw.u64()?;
        let high_generation = raw.u64()?;
        let value = Self {
            scope,
            epoch,
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
            low_nonce: raw.array()?,
            high_nonce: raw.array()?,
            request_digest: raw.array()?,
            grant_digest: raw.array()?,
            commit_by_wall: raw.u64()?,
        };
        if raw.u16()? != 1 {
            return Err(VoiceError::Malformed);
        }
        raw.finish()?;
        value.validate()?;
        Ok(value)
    }
}

/// Derived digest, not a parsed caller assertion and not a native RoundGate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct RoundRef(Nonce);
redacted_debug!(RoundRef);
impl RoundRef {
    pub(super) fn derive(agreement: &Agreement) -> Result<Self, VoiceError> {
        let bytes = agreement.encode()?;
        let mut hash = blake3::Hasher::new();
        hash.update(ROUND_DOMAIN);
        hash.update(&bytes);
        Ok(Self(*hash.finalize().as_bytes()))
    }
    pub(super) fn bytes(&self) -> &Nonce {
        &self.0
    }
}

struct EndpointKeyBytes {
    ed25519: [u8; 32],
    point: [u8; 65],
}
impl EndpointKeyBytes {
    fn validate_shape_and_reference(&self, endpoint: Endpoint) -> Result<(), VoiceError> {
        if self.point[0] != 4
            || key_reference(&self.point) != endpoint.key_ref
            || DeviceId::from_public_key_bytes(&self.ed25519) != endpoint.device
        {
            return Err(VoiceError::Scope);
        }
        // No P-256 on-curve or private-key ownership proof is made here.
        Ok(())
    }
}

/// Fixed domain-separated KDF input. It derives no key and grants no engine use.
struct Transcript([u8; TRANSCRIPT_BYTES]);
redacted_debug!(Transcript);
#[derive(Clone, Copy)]
enum Direction {
    LowToHigh,
    HighToLow,
}
impl Transcript {
    fn prepare(
        agreement: &Agreement,
        low: &EndpointKeyBytes,
        high: &EndpointKeyBytes,
    ) -> Result<Self, VoiceError> {
        low.validate_shape_and_reference(agreement.low)?;
        high.validate_shape_and_reference(agreement.high)?;
        let agreement_bytes = agreement.encode()?;
        let round = RoundRef::derive(agreement)?;
        let mut e = Encoder::with_capacity(TRANSCRIPT_BYTES);
        e.put_str(PAIR_DOMAIN)?;
        e.put_u8(SUITE);
        e.put_u8(ADMISSION_PROFILE);
        fixed(&mut e, round.bytes());
        fixed(&mut e, &agreement_bytes);
        for endpoint in [low, high] {
            fixed(&mut e, &endpoint.ed25519);
            fixed(&mut e, &endpoint.point);
        }
        let bytes = e.finish().try_into().map_err(|_| VoiceError::Bounds)?;
        Ok(Self(bytes))
    }
    fn decode(bytes: &[u8]) -> Result<Self, VoiceError> {
        if bytes.len() != TRANSCRIPT_BYTES {
            return Err(VoiceError::Bounds);
        }
        let mut d = Decoder::new(bytes);
        if d.get_str()? != PAIR_DOMAIN || d.get_u8()? != SUITE || d.get_u8()? != ADMISSION_PROFILE {
            return Err(VoiceError::Malformed);
        }
        let round: Nonce = array(&mut d)?;
        let agreement = Agreement::decode(d.get_bytes()?)?;
        let low = EndpointKeyBytes {
            ed25519: array(&mut d)?,
            point: array(&mut d)?,
        };
        let high = EndpointKeyBytes {
            ed25519: array(&mut d)?,
            point: array(&mut d)?,
        };
        d.finish()?;
        low.validate_shape_and_reference(agreement.low)?;
        high.validate_shape_and_reference(agreement.high)?;
        if round != *RoundRef::derive(&agreement)?.bytes() {
            return Err(VoiceError::Scope);
        }
        Ok(Self(bytes.try_into().map_err(|_| VoiceError::Bounds)?))
    }
    fn salt_input(&self) -> [u8; SALT_INPUT_BYTES] {
        let mut out = RawOutput::new();
        out.put(SALT_DOMAIN);
        out.put(&self.0);
        out.finish()
    }
    fn salt(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(SALT_DOMAIN);
        hash.update(self.0.as_slice());
        hash.finalize().into()
    }
    fn info(&self, direction: Direction) -> [u8; INFO_BYTES] {
        let mut out = RawOutput::new();
        out.put(match direction {
            Direction::LowToHigh => LOW_INFO_DOMAIN,
            Direction::HighToLow => HIGH_INFO_DOMAIN,
        });
        out.put(&self.0);
        out.finish()
    }
}

// These are wire/algorithm buffers, NOT a native inventory pass or a new payer.
// prepare holds the raw503-byte A, one exact789-byte Encoder backing and its
// 789-byte returned array during conversion. decode borrows its789-byte input;
// salt hashes incrementally, while explicit salt_input/info return830/839 bytes.
// A future paid action must additionally count all structs, SHA/BLAKE3 scratch,
// caller/IPC/SDK copies, simultaneous outputs and actual transition frames.

#[cfg(test)]
mod tests;
