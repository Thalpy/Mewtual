//! Borrowed, signature-checked new-family views. No old Body/Source enum grows.
//! Signature validity and bounded syntax are not membership, currentness or a
//! committed native round. The caller must retain/pay the SAME immutable input.
use super::*;
use control::Control;

const MAX_CONTROL: usize = 2048;
const MAX_ENCRYPTED: usize = 8864;
const MAX_READY: usize = 693;

#[derive(Clone, Copy)]
pub(super) struct Header {
    pub(super) scope: Scope,
    pub(super) issuer: DeviceId,
    pub(super) public_key: [u8; 32],
    pub(super) id: MessageId,
    pub(super) issued: u64,
    pub(super) expires: u64,
}
pub(super) enum AdmittedBody<'a> {
    Control(Control<'a>),
    Encrypted(Encrypted<'a>),
    Ready(Ready),
}
pub(super) struct SignedAdmitted<'a> {
    header: Header,
    body: AdmittedBody<'a>,
    canonical: &'a [u8],
    aad: Option<&'a [u8]>,
}
redacted_debug!(Header, SignedAdmitted<'_>);

impl<'a> SignedAdmitted<'a> {
    pub(super) fn decode(bytes: &'a [u8]) -> Result<Self, VoiceError> {
        Self::decode_expected(bytes, false)
    }
    pub(super) fn decode_availability(bytes: &'a [u8]) -> Result<Self, VoiceError> {
        Self::decode_expected(bytes, true)
    }
    fn decode_expected(bytes: &'a [u8], availability_only: bool) -> Result<Self, VoiceError> {
        if bytes.len() > MAX_ENCRYPTED {
            return Err(VoiceError::Bounds);
        }
        let mut outer = Decoder::new(bytes);
        let unsigned = outer.get_bytes()?;
        let signature: [u8; 64] = array(&mut outer)?;
        outer.finish()?;
        let mut d = Decoder::new(unsigned);
        if d.get_str()? != OBJECT_DOMAIN || d.get_u8()? != VERSION {
            return Err(VoiceError::Malformed);
        }
        let kind = d.get_u8()?;
        let maximum = match kind {
            7 => MAX_CONTROL,
            8 => MAX_ENCRYPTED,
            9 => MAX_READY,
            _ => return Err(VoiceError::Malformed),
        };
        if bytes.len() > maximum {
            return Err(VoiceError::Bounds);
        }
        let header = Header {
            scope: Scope::get(&mut d)?,
            issuer: DeviceId::from_bytes(array(&mut d)?),
            public_key: array(&mut d)?,
            id: array(&mut d)?,
            issued: d.get_u64()?,
            expires: d.get_u64()?,
        };
        if availability_only {
            // Select the sole fixed nested operation BEFORE signature/body work.
            // A nested Request never enters its own parser, even if signed.
            if kind != 7 {
                return Err(VoiceError::Malformed);
            }
            let mut peek = d.clone();
            let payload = peek.get_bytes()?;
            peek.finish()?;
            if payload.len() != 189 || payload.get(1) != Some(&2) {
                return Err(VoiceError::Malformed);
            }
        }
        lifetime_valid(header.issued, header.expires, MAX_PACKET_LIFETIME_MS)?;
        if header.issuer != DeviceId::from_public_key_bytes(&header.public_key)
            || !verify_with_public_bytes(&header.public_key, unsigned, &signature)
        {
            return Err(VoiceError::Signature);
        }
        // Authenticate the outer object before Request can verify its one nested
        // capability. Nested decoding is limited to a fixed Availability operation.
        let (body, aad) = match kind {
            7 => {
                let payload = d.get_bytes()?;
                (
                    AdmittedBody::Control(Control::decode(payload, &header)?),
                    None,
                )
            }
            8 => {
                if d.get_u8()? != ADMISSION_PROFILE {
                    return Err(VoiceError::Malformed);
                }
                let mut encrypted = Encrypted::get_header(&mut d)?;
                encrypted.validate(&header)?;
                let aad = &unsigned[..unsigned.len() - d.remaining()];
                let ciphertext = d.get_bytes()?;
                if ciphertext.len() < 16 || ciphertext.len() > encrypted.max_plaintext() + 16 {
                    return Err(VoiceError::Bounds);
                }
                encrypted.ciphertext = ciphertext;
                (AdmittedBody::Encrypted(encrypted), Some(aad))
            }
            9 => {
                if d.get_u8()? != ADMISSION_PROFILE {
                    return Err(VoiceError::Malformed);
                }
                let ready = Ready {
                    header: ready::ReadyPeek::get(&mut d)?,
                    sequence: d.get_u64()?,
                };
                if d.get_u8()? != ADMITTED_SDP_PROFILE {
                    return Err(VoiceError::Malformed);
                }
                ready.validate(&header)?;
                (AdmittedBody::Ready(ready), None)
            }
            _ => unreachable!("closed admitted kind"),
        };
        d.finish()?;
        Ok(Self {
            header,
            body,
            canonical: bytes,
            aad,
        })
    }
    pub(super) fn hash(&self) -> Nonce {
        *blake3::hash(self.canonical).as_bytes()
    }
    pub(super) fn encode(&self) -> Vec<u8> {
        self.canonical.to_vec()
    }
    // Keep the verified projection immutable outside this decoder. Siblings may
    // not change a tuple while retaining an unrelated original canonical hash.
    pub(super) fn header(&self) -> &Header {
        &self.header
    }
    pub(super) fn body(&self) -> &AdmittedBody<'a> {
        &self.body
    }
    fn aad(&self) -> Result<&'a [u8], VoiceError> {
        self.aad.ok_or(VoiceError::Malformed)
    }
}

/// The separate profile2 header deliberately cannot be passed to old packet
/// factories. round/ready bytes here are untrusted labels, not native capabilities.
pub(super) struct Encrypted<'a> {
    source_join: Nonce,
    target: DeviceId,
    target_join: Nonce,
    source_key: Nonce,
    target_key: Nonce,
    round: Nonce,
    low_to_high: bool,
    kind: u8,
    sequence: u64,
    offer: Option<OfferId>,
    ready: Option<Nonce>,
    ciphertext: &'a [u8],
}
impl<'a> Encrypted<'a> {
    fn get_header(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        let source_join = array(d)?;
        let target = DeviceId::from_bytes(array(d)?);
        let target_join = array(d)?;
        let source_key = array(d)?;
        let target_key = array(d)?;
        if d.get_u8()? != SUITE {
            return Err(VoiceError::Malformed);
        }
        let round = array(d)?;
        let low_to_high = boolean(d)?;
        let kind = d.get_u8()?;
        if !matches!(kind, 1 | 2 | 3 | 7 | 8 | 9) {
            return Err(VoiceError::Malformed);
        }
        let sequence = d.get_u64()?;
        let offer = if boolean(d)? {
            Some(OfferId::get(d)?)
        } else {
            None
        };
        let ready = if matches!(kind, 8 | 9) {
            if d.get_u8()? != ADMITTED_SDP_PROFILE {
                return Err(VoiceError::Malformed);
            }
            Some(array(d)?)
        } else {
            None
        };
        Ok(Self {
            source_join,
            target,
            target_join,
            source_key,
            target_key,
            round,
            low_to_high,
            kind,
            sequence,
            offer,
            ready,
            ciphertext: &[],
        })
    }
    fn validate(&self, common: &Header) -> Result<(), VoiceError> {
        if common.issuer == self.target
            || self.sequence == 0
            || self.low_to_high != (common.issuer < self.target)
        {
            return Err(VoiceError::Malformed);
        }
        match (self.kind, self.offer) {
            (1 | 3, None) if self.low_to_high => Ok(()),
            (2, None) if !self.low_to_high => Ok(()),
            (7, None) => Ok(()),
            (8, Some(offer))
                if self.low_to_high
                    && offer.issuer == common.issuer
                    && offer.message == common.id =>
            {
                Ok(())
            }
            (9, Some(offer)) if !self.low_to_high && offer.issuer == self.target => Ok(()),
            _ => Err(VoiceError::Malformed),
        }
    }
    fn max_plaintext(&self) -> usize {
        match self.kind {
            8 | 9 => MAX_SDP_PLAINTEXT,
            7 => 128,
            _ => 512,
        }
    }
}

pub(super) struct Ready {
    header: ready::ReadyPeek,
    sequence: u64,
}
impl Ready {
    fn validate(&self, common: &Header) -> Result<(), VoiceError> {
        let h = &self.header;
        if self.sequence == 0
            || common.issuer <= h.target
            || (common.scope.policy.is_none() && h.helpers[0].is_some())
            || h.helpers
                .iter()
                .flatten()
                .any(|id| *id == common.issuer || *id == h.target)
            || matches!(h.helpers, [Some(a), Some(b)] if a >= b)
        {
            return Err(VoiceError::Malformed);
        }
        Ok(())
    }
}

// decode borrows canonical unsigned/ciphertext/capability fields and uses fixed
// <=2-helper storage. Request verifies at most ONE nested<=512-byte Availability;
// that operation cannot contain another nested object. Signature-library scratch
// and all value/frame copies still belong in a future paid consumer's inventory.
// encode explicitly allocates ONE exact <=8864-byte canonical output copy; no
// production signing/unsigned-construction or envelope/Source adapter exists here.

#[cfg(test)]
mod tests;
