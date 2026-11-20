//! Immutable bridge from the reviewed borrowed grammar to operational Control7.
//! It exposes no mutable signed projection or general signing/payload command.
#![warn(dead_code)] // This operational bridge may not inherit staged-code exemptions.
use super::*;
use control::{Operation, QueryKind};

pub(crate) const MAX_CONTROL: usize = 2048;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtocolOperation {
    Query,
    Support { query: Nonce, features: u16 },
    Other,
}
#[derive(Clone, Copy)]
pub(crate) struct Metadata {
    scope: Scope,
    issuer: DeviceId,
    public_key: [u8; 32],
    id: MessageId,
    issued: u64,
    expires: u64,
    epoch: u64,
    target: DeviceId,
    operation: ProtocolOperation,
}
impl Metadata {
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }
    pub(crate) fn issuer(&self) -> DeviceId {
        self.issuer
    }
    pub(crate) fn public_key(&self) -> [u8; 32] {
        self.public_key
    }
    pub(crate) fn id(&self) -> MessageId {
        self.id
    }
    pub(crate) fn issued(&self) -> u64 {
        self.issued
    }
    pub(crate) fn expires(&self) -> u64 {
        self.expires
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
    pub(crate) fn target(&self) -> DeviceId {
        self.target
    }
    pub(crate) fn operation(&self) -> ProtocolOperation {
        self.operation
    }
    pub(crate) fn fresh(&self, wall: u64) -> Result<(), VoiceError> {
        freshness(self.issued, self.expires, wall)
    }
}
pub(crate) struct Verified<'a> {
    bytes: &'a [u8],
    metadata: Metadata,
}
impl<'a> Verified<'a> {
    pub(crate) fn decode(bytes: &'a [u8]) -> Result<Self, VoiceError> {
        if bytes.len() > MAX_CONTROL {
            return Err(VoiceError::Bounds);
        }
        let signed = SignedAdmitted::decode(bytes)?;
        let AdmittedBody::Control(value) = signed.body() else {
            return Err(VoiceError::Malformed);
        };
        let operation = match &value.operation {
            Operation::Query {
                kind: QueryKind::ProtocolSupport,
                ..
            } => ProtocolOperation::Query,
            Operation::Support {
                query, features, ..
            } => ProtocolOperation::Support {
                query: *query,
                features: *features,
            },
            _ => ProtocolOperation::Other,
        };
        let header = signed.header();
        Ok(Self {
            bytes,
            metadata: Metadata {
                scope: header.scope,
                issuer: header.issuer,
                public_key: header.public_key,
                id: header.id,
                issued: header.issued,
                expires: header.expires,
                epoch: value.epoch,
                target: value.operation.target(),
                operation,
            },
        })
    }
    pub(crate) fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
    pub(crate) fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub(crate) fn hash(&self) -> Nonce {
        *blake3::hash(self.bytes).as_bytes()
    }
}

/// Original canonical bytes. Callers must prepay their complete retained/temporary
/// graph BEFORE copying here; immutable getters cannot pair another header/hash.
pub(crate) struct Object {
    bytes: Box<[u8]>,
    metadata: Metadata,
    hash: Nonce,
}
impl Object {
    pub(crate) fn copy(verified: &Verified<'_>) -> Self {
        Self {
            bytes: verified.bytes().to_vec().into_boxed_slice(),
            metadata: verified.metadata,
            hash: verified.hash(),
        }
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub(crate) fn hash(&self) -> Nonce {
        self.hash
    }
    pub(crate) fn query(
        scope: Scope,
        epoch: u64,
        target: DeviceId,
        signer: &dyn VoiceSigner,
        id: Nonce,
        issued: u64,
        expires: u64,
    ) -> Result<Self, VoiceError> {
        let mut payload = RawOutput::<44>::new();
        payload.put(&[1, 0, ADMISSION_PROFILE]);
        payload.put(&epoch.to_be_bytes());
        payload.put(target.as_bytes());
        payload.put(&[0]);
        Self::issue(scope, signer, id, issued, expires, &payload.finish())
    }
    #[allow(clippy::too_many_arguments)] // Exact original Query identity is separate from this signed wrapper.
    pub(crate) fn support(
        scope: Scope,
        epoch: u64,
        target: DeviceId,
        query: Nonce,
        features: u16,
        signer: &dyn VoiceSigner,
        id: Nonce,
        issued: u64,
        expires: u64,
    ) -> Result<Self, VoiceError> {
        if features & !7 != 0 {
            return Err(VoiceError::Malformed);
        }
        let mut payload = RawOutput::<77>::new();
        payload.put(&[1, 1, ADMISSION_PROFILE]);
        payload.put(&epoch.to_be_bytes());
        payload.put(target.as_bytes());
        payload.put(&query);
        payload.put(&features.to_be_bytes());
        Self::issue(scope, signer, id, issued, expires, &payload.finish())
    }
    fn issue(
        scope: Scope,
        signer: &dyn VoiceSigner,
        id: Nonce,
        issued: u64,
        expires: u64,
        payload: &[u8],
    ) -> Result<Self, VoiceError> {
        let public_key = signer.voice_public_key()?;
        let mut e = Encoder::with_capacity(512);
        e.put_str(OBJECT_DOMAIN).expect("constant");
        e.put_u8(VERSION);
        e.put_u8(7);
        scope.put(&mut e);
        fixed(
            &mut e,
            DeviceId::from_public_key_bytes(&public_key).as_bytes(),
        );
        fixed(&mut e, &public_key);
        fixed(&mut e, &id);
        e.put_u64(issued);
        e.put_u64(expires);
        fixed(&mut e, payload);
        let unsigned = e.finish();
        let signature = signer.sign_voice(&unsigned)?;
        let mut out = Encoder::with_capacity(unsigned.len() + 72);
        fixed(&mut out, &unsigned);
        fixed(&mut out, &signature);
        let bytes = out.finish().into_boxed_slice();
        let checked = Verified::decode(&bytes)?;
        let metadata = checked.metadata;
        let hash = checked.hash();
        Ok(Self {
            bytes,
            metadata,
            hash,
        })
    }
}
redacted_debug!(ProtocolOperation, Metadata, Verified<'_>, Object);
