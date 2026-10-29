//! The agreed one-helper envelope/permit bytes with an independently typed
//! Control7 object. Old Envelope/Body APIs remain closed to these bytes.
use super::*;

#[derive(Clone)]
pub(crate) struct Permit(RoutePermit);
impl Permit {
    pub(crate) fn issue(
        object: &Object,
        helper: DeviceId,
        signer: &dyn VoiceSigner,
        issued: u64,
        expires: u64,
    ) -> Result<Self, VoiceError> {
        let public_key = signer.voice_public_key()?;
        let mut value = RoutePermit {
            scope: object.metadata().scope(),
            issuer: DeviceId::from_public_key_bytes(&public_key),
            public_key,
            packet: object.hash(),
            helper,
            final_scope: FinalScope::Endpoint(object.metadata().target()),
            issued,
            expires,
            signature: [0; 64],
        };
        Self::matches(&value, object.metadata(), object.hash())?;
        let unsigned = value.unsigned();
        value.signature = signer.sign_voice(&unsigned)?;
        if !verify_with_public_bytes(&public_key, &unsigned, &value.signature) {
            return Err(VoiceError::Signature);
        }
        Ok(Self(value))
    }
    fn matches(value: &RoutePermit, metadata: &Metadata, hash: Nonce) -> Result<(), VoiceError> {
        value.validate()?;
        if value.scope != metadata.scope()
            || value.issuer != metadata.issuer()
            || value.public_key != metadata.public_key()
            || value.packet != hash
            || value.final_scope != FinalScope::Endpoint(metadata.target())
            || value.expires > metadata.expires()
            || value.issued < metadata.issued()
        {
            return Err(VoiceError::Scope);
        }
        Ok(())
    }
    pub(crate) fn helper(&self) -> DeviceId {
        self.0.helper
    }
    pub(crate) fn expires(&self) -> u64 {
        self.0.expires
    }
}

enum Hop {
    Direct,
    ToHelper(Permit),
    Delivery(Permit),
}
pub(crate) struct Envelope<'a> {
    bytes: &'a [u8],
    object: Verified<'a>,
    hop: Hop,
}
impl<'a> Envelope<'a> {
    /// Family discriminator only, never authority. The caller has already bounded
    /// the frame and prepaid verification before the signature decoder follows.
    pub(crate) fn is_control(bytes: &[u8]) -> bool {
        (|| {
            let mut outer = Decoder::new(bytes);
            if outer.get_u8()? != VERSION || !(1..=3).contains(&outer.get_u8()?) {
                return Err(VoiceError::Malformed);
            }
            let mut signed = Decoder::new(outer.get_bytes()?);
            let mut header = Decoder::new(signed.get_bytes()?);
            if header.get_str()? != OBJECT_DOMAIN || header.get_u8()? != VERSION {
                return Err(VoiceError::Malformed);
            }
            Ok(header.get_u8()? == 7)
        })()
        .unwrap_or(false)
    }
    pub(crate) fn decode(bytes: &'a [u8]) -> Result<Self, VoiceError> {
        if bytes.len() > 2 + 4 + super::super::admitted_wire::protocol::MAX_CONTROL + 4 + 1024 {
            return Err(VoiceError::Bounds);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8()? != VERSION {
            return Err(VoiceError::Malformed);
        }
        let kind = d.get_u8()?;
        let object = Verified::decode(d.get_bytes()?)?;
        let hop = match kind {
            1 => Hop::Direct,
            2 | 3 => {
                let permit = RoutePermit::decode(d.get_bytes()?)?;
                Permit::matches(&permit, object.metadata(), object.hash())?;
                if kind == 2 {
                    Hop::ToHelper(Permit(permit))
                } else {
                    Hop::Delivery(Permit(permit))
                }
            }
            _ => return Err(VoiceError::Malformed),
        };
        d.finish()?;
        Ok(Self { bytes, object, hop })
    }
    pub(crate) fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
    pub(crate) fn object(&self) -> &Verified<'a> {
        &self.object
    }
    pub(crate) fn helper_permit(&self) -> Option<&Permit> {
        if let Hop::ToHelper(permit) = &self.hop {
            Some(permit)
        } else {
            None
        }
    }
    pub(crate) fn authorize(
        &self,
        requester: DeviceId,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
    ) -> Result<Disposition, VoiceError> {
        let (kind, permit) = match &self.hop {
            Hop::Direct => (1, None),
            Hop::ToHelper(value) => (2, Some(value)),
            Hop::Delivery(value) => (3, Some(value)),
        };
        authorize(
            self.object.metadata(),
            self.object.hash(),
            kind,
            permit,
            requester,
            context,
            member,
        )
    }
}
pub(crate) enum EnvelopeRef<'a> {
    Direct(&'a Object),
    ToHelper(&'a Object, &'a Permit),
    Delivery(&'a Object, &'a Permit),
}
impl EnvelopeRef<'_> {
    pub(crate) fn object(&self) -> &Object {
        match self {
            Self::Direct(value) | Self::ToHelper(value, _) | Self::Delivery(value, _) => value,
        }
    }
    pub(crate) fn encode(&self) -> Vec<u8> {
        let (kind, object, permit) = match self {
            Self::Direct(value) => (1, value, None),
            Self::ToHelper(value, permit) => (2, value, Some(permit)),
            Self::Delivery(value, permit) => (3, value, Some(permit)),
        };
        let permit = permit.map(|value| value.0.encode());
        let mut e = Encoder::with_capacity(
            6 + object.bytes().len() + permit.as_ref().map_or(0, |bytes| bytes.len() + 4),
        );
        e.put_u8(VERSION);
        e.put_u8(kind);
        fixed(&mut e, object.bytes());
        if let Some(permit) = permit {
            fixed(&mut e, &permit);
        }
        e.finish()
    }
    pub(crate) fn authorize(
        &self,
        requester: DeviceId,
        context: &Context,
        member: &dyn Fn(DeviceId) -> bool,
    ) -> Result<Disposition, VoiceError> {
        let (kind, permit) = match self {
            Self::Direct(_) => (1, None),
            Self::ToHelper(_, permit) => (2, Some(*permit)),
            Self::Delivery(_, permit) => (3, Some(*permit)),
        };
        authorize(
            self.object().metadata(),
            self.object().hash(),
            kind,
            permit,
            requester,
            context,
            member,
        )
    }
}
fn authorize(
    metadata: &Metadata,
    hash: Nonce,
    kind: u8,
    permit: Option<&Permit>,
    requester: DeviceId,
    context: &Context,
    member: &dyn Fn(DeviceId) -> bool,
) -> Result<Disposition, VoiceError> {
    if metadata.scope() != context.scope {
        return Err(VoiceError::Scope);
    }
    metadata.fresh(context.now)?;
    if metadata.epoch() != context.epoch
        || metadata.issuer() == context.local
        || !member(requester)
        || !member(metadata.issuer())
        || !member(metadata.target())
        || !member(context.local)
    {
        return Err(VoiceError::Unauthorized);
    }
    if kind == 1 {
        return if requester == metadata.issuer() && metadata.target() == context.local {
            Ok(Disposition::Deliver)
        } else {
            Err(VoiceError::Unauthorized)
        };
    }
    let permit = permit.ok_or(VoiceError::Malformed)?;
    if !context.indirect || context.scope.policy.is_none() || !member(permit.helper()) {
        return Err(VoiceError::Unauthorized);
    }
    Permit::matches(&permit.0, metadata, hash)?;
    freshness(permit.0.issued, permit.0.expires, context.now)?;
    match kind {
        2 if requester == metadata.issuer() && permit.helper() == context.local => Ok(
            Disposition::Forward(FinalScope::Endpoint(metadata.target())),
        ),
        3 if requester == permit.helper()
            && permit.helper() != context.local
            && metadata.target() == context.local =>
        {
            Ok(Disposition::Deliver)
        }
        _ => Err(VoiceError::Unauthorized),
    }
}
redacted_debug!(Permit, Envelope<'_>);
