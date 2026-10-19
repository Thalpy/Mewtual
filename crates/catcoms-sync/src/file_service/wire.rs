//! Closed signed file-page frames. Parsing/signature verification establish no current
//! membership, connection, replay, rate, deadline or application authority. The Root must check
//! those independently; these codecs neither register a handler nor enable member forwarding.
use catcoms_crypto::{verify_with_public_bytes, DeviceId};
use catcoms_mls::MlsDevice;
use catcoms_rt::{
    PeerId, FILE_BRIDGE_KIND, FILE_SERVICE_REQUEST_MAX_BYTES, FILE_SERVICE_RESPONSE_MAX_BYTES,
    FILE_SOURCE_KIND,
};
use catcoms_storage::Cid;
use catcoms_wire::{Decoder, Encoder, WireError};
use thiserror::Error;

use super::read::MAX_SOURCE_CHUNK;

pub const MAX_PAGE_BYTES: usize = 64 * 1024;
pub const MAX_SOURCES: usize = 4;
// Derived from canonical field widths, including all length prefixes, the tag, key and signature.
pub const MAX_GRANT_BYTES: usize = 707;
pub const MAX_DELIVERY_BYTES: usize = 893;
pub const MAX_SOURCE_REPLY_BYTES: usize = MAX_PAGE_BYTES + 155;
pub const MAX_HELPER_REPLY_BYTES: usize = MAX_SOURCE_REPLY_BYTES + 151;
const SOURCE_REPLY: u8 = 0xE1;
const HELPER_REPLY: u8 = 0xE2;
const GRANT_DOMAIN: &str = "mewtual/file/bridge-grant/v1";
const DELIVERY_DOMAIN: &str = "mewtual/file/bridge-delivery/v1";
const SOURCE_DOMAIN: &str = "mewtual/file/source-page/v1";
const HELPER_DOMAIN: &str = "mewtual/file/helper-page/v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("file service frame exceeds its bound")]
    Bounds,
    #[error("file service frame is malformed")]
    Malformed,
    #[error("file service signature does not match")]
    Signature,
    #[error("file service frame could not be signed")]
    Signing,
}
impl From<WireError> for FrameError {
    fn from(_: WireError) -> Self {
        Self::Malformed
    }
}
fn fixed<const N: usize>(d: &mut Decoder<'_>) -> Result<[u8; N], FrameError> {
    d.get_bytes()?.try_into().map_err(|_| FrameError::Malformed)
}
fn field(e: &mut Encoder, value: &[u8]) {
    e.put_bytes(value).expect("closed field");
}
fn transcript(domain: &str, body: &[u8]) -> Vec<u8> {
    let mut e = Encoder::with_capacity(8 + domain.len() + body.len());
    e.put_str(domain).expect("fixed domain");
    field(&mut e, body);
    e.finish()
}

#[derive(Clone, Copy)]
struct Signed<'a> {
    raw: &'a [u8],
    body: &'a [u8],
    key: &'a [u8; 32],
    signature: &'a [u8; 64],
}
impl<'a> Signed<'a> {
    fn parse(raw: &'a [u8], tag: u8, max: usize) -> Result<Self, FrameError> {
        if raw.len() > max {
            return Err(FrameError::Bounds);
        }
        let mut d = Decoder::new(raw);
        if d.get_u8()? != tag {
            return Err(FrameError::Malformed);
        }
        let body = d.get_bytes()?;
        let key = d
            .get_bytes()?
            .try_into()
            .map_err(|_| FrameError::Malformed)?;
        let signature = d
            .get_bytes()?
            .try_into()
            .map_err(|_| FrameError::Malformed)?;
        d.finish()?;
        Ok(Self {
            raw,
            body,
            key,
            signature,
        })
    }
    fn signer(&self) -> DeviceId {
        DeviceId::from_public_key_bytes(self.key)
    }
    fn verify(&self, domain: &str) -> bool {
        verify_with_public_bytes(self.key, &transcript(domain, self.body), self.signature)
    }
    fn issue(
        tag: u8,
        body: &[u8],
        domain: &str,
        signer: &MlsDevice,
        max: usize,
    ) -> Result<Vec<u8>, FrameError> {
        if body.len().checked_add(109).is_none_or(|size| size > max) {
            return Err(FrameError::Bounds);
        }
        let signature = signer
            .sign(&transcript(domain, body))
            .map_err(|_| FrameError::Signing)?;
        let key = signer.public_key_bytes();
        if key.len() != 32 {
            return Err(FrameError::Signing);
        }
        let mut e = Encoder::with_capacity(body.len() + 109);
        e.put_u8(tag);
        field(&mut e, body);
        field(&mut e, &key);
        field(&mut e, &signature);
        let bytes = e.finish();
        if bytes.len() > max {
            return Err(FrameError::Bounds);
        }
        Ok(bytes)
    }
}

/// Application preparation must derive this bound from the exact authorized encrypted FileRef,
/// never a remote size or arbitrary renderer field. No file-layer key appears in a grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub cid: Cid,
    pub max_cipher_bytes: u32,
    pub offset: u32,
    pub length: u32,
}
impl Window {
    fn valid(&self) -> bool {
        self.max_cipher_bytes != 0
            && self.max_cipher_bytes as usize <= MAX_SOURCE_CHUNK
            && self.offset < self.max_cipher_bytes
            && self.length != 0
            && self.length as usize <= MAX_PAGE_BYTES
    }
}

/// Exact immutable original C grant. C, helper B and each permitted source must be distinct.
/// Freshness is strictly less than20 seconds, enforced by the service ledger, not this codec.
pub struct GrantFields<'a> {
    pub group: &'a [u8],
    pub epoch: u64,
    pub helper: DeviceId,
    pub helper_peer: PeerId,
    pub requester_peer: PeerId,
    pub window: Window,
    pub sources: &'a [DeviceId],
    pub created_ms: u64,
    pub nonce: [u8; 16],
}
impl GrantFields<'_> {
    fn valid(&self, requester: DeviceId) -> bool {
        !self.group.is_empty()
            && self.group.len() <= 256
            && self.window.valid()
            && self.helper != requester
            && self.helper_peer != self.requester_peer
            && !self.sources.is_empty()
            && self.sources.len() <= MAX_SOURCES
            && self.sources.windows(2).all(|pair| pair[0] < pair[1])
            && self
                .sources
                .iter()
                .all(|id| *id != requester && *id != self.helper)
    }
    /// Native-only signing with a caller-owned device. This creates no standing or read intent;
    /// the originating actor must prepay and authorize the exact file/route before issuing it.
    pub fn issue(&self, requester: &MlsDevice) -> Result<Vec<u8>, FrameError> {
        if !self.valid(requester.device_id()) {
            return Err(FrameError::Malformed);
        }
        let mut e = Encoder::with_capacity(598);
        e.put_u8(1);
        field(&mut e, self.group);
        e.put_u64(self.epoch);
        field(&mut e, self.helper.as_bytes());
        field(&mut e, self.helper_peer.as_bytes());
        field(&mut e, self.requester_peer.as_bytes());
        field(&mut e, self.window.cid.as_bytes());
        e.put_u32(self.window.max_cipher_bytes);
        e.put_u32(self.window.offset);
        e.put_u32(self.window.length);
        e.put_u8(self.sources.len() as u8);
        for source in self.sources {
            field(&mut e, source.as_bytes());
        }
        e.put_u64(self.created_ms);
        field(&mut e, &self.nonce);
        Signed::issue(
            FILE_BRIDGE_KIND,
            &e.finish(),
            GRANT_DOMAIN,
            requester,
            MAX_GRANT_BYTES,
        )
    }
}

/// Borrowed, structurally checked original grant. `verify_signature` proves only its key's
/// signature. Root separately checks that key's current membership, actual C Noise binding,
/// group/epoch, fixed helper, replay/rate admission and original deadline.
pub struct Grant<'a> {
    signed: Signed<'a>,
    group: &'a [u8],
    epoch: u64,
    helper: DeviceId,
    helper_peer: PeerId,
    requester_peer: PeerId,
    window: Window,
    sources: [DeviceId; MAX_SOURCES],
    source_count: usize,
    created_ms: u64,
    nonce: [u8; 16],
}
impl<'a> Grant<'a> {
    pub fn parse(raw: &'a [u8]) -> Result<Self, FrameError> {
        let signed = Signed::parse(raw, FILE_BRIDGE_KIND, MAX_GRANT_BYTES)?;
        let mut d = Decoder::new(signed.body);
        if d.get_u8()? != 1 {
            return Err(FrameError::Malformed);
        }
        let group = d.get_bytes()?;
        let epoch = d.get_u64()?;
        let helper = DeviceId::from_bytes(fixed(&mut d)?);
        let helper_peer = PeerId::new(fixed(&mut d)?);
        let requester_peer = PeerId::new(fixed(&mut d)?);
        let window = Window {
            cid: Cid::from_bytes(fixed(&mut d)?),
            max_cipher_bytes: d.get_u32()?,
            offset: d.get_u32()?,
            length: d.get_u32()?,
        };
        let source_count = d.get_u8()? as usize;
        if source_count == 0 || source_count > MAX_SOURCES {
            return Err(FrameError::Bounds);
        }
        let mut sources = [DeviceId::from_bytes([0; 32]); MAX_SOURCES];
        for id in &mut sources[..source_count] {
            *id = DeviceId::from_bytes(fixed(&mut d)?);
        }
        let created_ms = d.get_u64()?;
        let nonce = fixed(&mut d)?;
        d.finish()?;
        let view = Self {
            signed,
            group,
            epoch,
            helper,
            helper_peer,
            requester_peer,
            window,
            sources,
            source_count,
            created_ms,
            nonce,
        };
        if !(GrantFields {
            group,
            epoch,
            helper,
            helper_peer,
            requester_peer,
            window,
            sources: view.sources(),
            created_ms,
            nonce,
        })
        .valid(view.requester())
        {
            return Err(FrameError::Malformed);
        }
        Ok(view)
    }
    pub fn group(&self) -> &'a [u8] {
        self.group
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn helper(&self) -> DeviceId {
        self.helper
    }
    pub fn helper_peer(&self) -> PeerId {
        self.helper_peer
    }
    pub fn requester_peer(&self) -> PeerId {
        self.requester_peer
    }
    pub fn window(&self) -> Window {
        self.window
    }
    pub fn created_ms(&self) -> u64 {
        self.created_ms
    }
    pub fn nonce(&self) -> [u8; 16] {
        self.nonce
    }
    pub fn requester(&self) -> DeviceId {
        self.signed.signer()
    }
    pub fn public_key(&self) -> &[u8; 32] {
        self.signed.key
    }
    pub fn sources(&self) -> &[DeviceId] {
        &self.sources[..self.source_count]
    }
    pub fn raw(&self) -> &'a [u8] {
        self.signed.raw
    }
    /// Hash the COMPLETE signed frame: changed signature/key/body cannot reuse this response.
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(self.raw()).as_bytes()
    }
    pub fn verify_signature(&self) -> bool {
        self.signed.verify(GRANT_DOMAIN)
    }
}

/// Helper B's exact one-hop delivery to one of C's explicitly named sources. A must verify
/// both signatures, immediate B Noise binding, own target device/peer and current MLS scope.
/// A forwarded C signature never establishes a connection/proof/standing for C at A.
pub struct Delivery<'a> {
    signed: Signed<'a>,
    grant: Grant<'a>,
    source: DeviceId,
    source_peer: PeerId,
}
impl<'a> Delivery<'a> {
    pub fn grant(&self) -> &Grant<'a> {
        &self.grant
    }
    pub fn source(&self) -> DeviceId {
        self.source
    }
    pub fn source_peer(&self) -> PeerId {
        self.source_peer
    }
    fn valid(grant: &Grant<'_>, source: DeviceId, peer: PeerId, signer: DeviceId) -> bool {
        signer == grant.helper
            && grant.sources().contains(&source)
            && peer != grant.helper_peer
            && peer != grant.requester_peer
    }
    pub fn issue(
        grant: &Grant<'_>,
        source: DeviceId,
        source_peer: PeerId,
        helper: &MlsDevice,
    ) -> Result<Vec<u8>, FrameError> {
        if !Self::valid(grant, source, source_peer, helper.device_id()) {
            return Err(FrameError::Malformed);
        }
        let mut e = Encoder::with_capacity(784);
        e.put_u8(1);
        field(&mut e, grant.raw());
        field(&mut e, source.as_bytes());
        field(&mut e, source_peer.as_bytes());
        Signed::issue(
            FILE_SOURCE_KIND,
            &e.finish(),
            DELIVERY_DOMAIN,
            helper,
            MAX_DELIVERY_BYTES,
        )
    }
    pub fn parse(raw: &'a [u8]) -> Result<Self, FrameError> {
        let signed = Signed::parse(raw, FILE_SOURCE_KIND, MAX_DELIVERY_BYTES)?;
        let mut d = Decoder::new(signed.body);
        if d.get_u8()? != 1 {
            return Err(FrameError::Malformed);
        }
        let grant = Grant::parse(d.get_bytes()?)?;
        let source = DeviceId::from_bytes(fixed(&mut d)?);
        let source_peer = PeerId::new(fixed(&mut d)?);
        d.finish()?;
        if !Self::valid(&grant, source, source_peer, signed.signer()) {
            return Err(FrameError::Malformed);
        }
        Ok(Self {
            signed,
            grant,
            source,
            source_peer,
        })
    }
    pub fn public_key(&self) -> &[u8; 32] {
        self.signed.key
    }
    /// Checks only B's delivery signature. Independently verify the original C grant signature.
    pub fn verify_signature(&self) -> bool {
        self.signed.verify(DELIVERY_DOMAIN)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    Present = 1,
    Absent = 2,
    Unavailable = 3,
}
impl SourceStatus {
    fn parse(value: u8) -> Result<Self, FrameError> {
        match value {
            1 => Ok(Self::Present),
            2 => Ok(Self::Absent),
            3 => Ok(Self::Unavailable),
            _ => Err(FrameError::Malformed),
        }
    }
}
/// A source-signed page bound to C's entire original grant. Parsing is borrowed; it never
/// authenticates current membership or says that ciphertext opens under C's file reference.
pub struct SourcePage<'a> {
    signed: Signed<'a>,
    status: SourceStatus,
    total: u32,
    page: &'a [u8],
}
impl<'a> SourcePage<'a> {
    pub fn status(&self) -> SourceStatus {
        self.status
    }
    pub fn total(&self) -> u32 {
        self.total
    }
    pub fn page(&self) -> &'a [u8] {
        self.page
    }
    fn valid(
        grant: &Grant<'_>,
        signer: DeviceId,
        status: SourceStatus,
        total: u32,
        page: &[u8],
    ) -> bool {
        if !grant.sources().contains(&signer) {
            return false;
        }
        if status != SourceStatus::Present {
            return total == 0 && page.is_empty();
        }
        total != 0
            && total <= grant.window.max_cipher_bytes
            && grant.window.offset < total
            && !page.is_empty()
            && page.len() == grant.window.length.min(total - grant.window.offset) as usize
    }
    pub fn issue(
        grant: &Grant<'_>,
        status: SourceStatus,
        total: u32,
        page: &[u8],
        source: &MlsDevice,
    ) -> Result<Vec<u8>, FrameError> {
        if !Self::valid(grant, source.device_id(), status, total, page) {
            return Err(FrameError::Malformed);
        }
        let mut e = Encoder::with_capacity(46 + page.len());
        e.put_u8(1);
        field(&mut e, &grant.hash());
        e.put_u8(status as u8);
        e.put_u32(total);
        field(&mut e, page);
        Signed::issue(
            SOURCE_REPLY,
            &e.finish(),
            SOURCE_DOMAIN,
            source,
            MAX_SOURCE_REPLY_BYTES,
        )
    }
    pub fn parse(raw: &'a [u8], grant: &Grant<'_>) -> Result<Self, FrameError> {
        let signed = Signed::parse(raw, SOURCE_REPLY, MAX_SOURCE_REPLY_BYTES)?;
        let mut d = Decoder::new(signed.body);
        if d.get_u8()? != 1 || fixed::<32>(&mut d)? != grant.hash() {
            return Err(FrameError::Malformed);
        }
        let status = SourceStatus::parse(d.get_u8()?)?;
        let total = d.get_u32()?;
        let page = d.get_bytes()?;
        d.finish()?;
        if !Self::valid(grant, signed.signer(), status, total, page) {
            return Err(FrameError::Malformed);
        }
        Ok(Self {
            signed,
            status,
            total,
            page,
        })
    }
    pub fn source(&self) -> DeviceId {
        self.signed.signer()
    }
    pub fn public_key(&self) -> &[u8; 32] {
        self.signed.key
    }
    pub fn raw(&self) -> &'a [u8] {
        self.signed.raw
    }
    pub fn verify_signature(&self) -> bool {
        self.signed.verify(SOURCE_DOMAIN)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperStatus {
    Found = 1,
    Absent = 2,
    Unavailable = 3,
}
/// Helper-only negative evidence is not proof that no group member holds a file. A Found result
/// contains the exact unmodified source Present frame; C must verify BOTH current member keys.
pub struct HelperPage<'a> {
    signed: Signed<'a>,
    status: HelperStatus,
    source: Option<SourcePage<'a>>,
}
impl<'a> HelperPage<'a> {
    pub fn status(&self) -> HelperStatus {
        self.status
    }
    pub fn source(&self) -> Option<&SourcePage<'a>> {
        self.source.as_ref()
    }
    pub fn issue(
        grant: &Grant<'_>,
        status: HelperStatus,
        source: Option<&SourcePage<'_>>,
        helper: &MlsDevice,
    ) -> Result<Vec<u8>, FrameError> {
        if helper.device_id() != grant.helper || (status == HelperStatus::Found) != source.is_some()
        {
            return Err(FrameError::Malformed);
        }
        if let Some(source) = source {
            // Reparse against THIS grant so a typed view from another request cannot be wrapped.
            let same = SourcePage::parse(source.raw(), grant)?;
            if same.status != SourceStatus::Present || !same.verify_signature() {
                return Err(FrameError::Signature);
            }
        }
        let mut e = Encoder::with_capacity(42 + source.map_or(0, |page| page.raw().len()));
        e.put_u8(1);
        field(&mut e, &grant.hash());
        e.put_u8(status as u8);
        field(&mut e, source.map_or(&[], |page| page.raw()));
        Signed::issue(
            HELPER_REPLY,
            &e.finish(),
            HELPER_DOMAIN,
            helper,
            MAX_HELPER_REPLY_BYTES,
        )
    }
    pub fn parse(raw: &'a [u8], grant: &Grant<'_>) -> Result<Self, FrameError> {
        let signed = Signed::parse(raw, HELPER_REPLY, MAX_HELPER_REPLY_BYTES)?;
        if signed.signer() != grant.helper {
            return Err(FrameError::Malformed);
        }
        let mut d = Decoder::new(signed.body);
        if d.get_u8()? != 1 || fixed::<32>(&mut d)? != grant.hash() {
            return Err(FrameError::Malformed);
        }
        let status = match d.get_u8()? {
            1 => HelperStatus::Found,
            2 => HelperStatus::Absent,
            3 => HelperStatus::Unavailable,
            _ => return Err(FrameError::Malformed),
        };
        let source = d.get_bytes()?;
        d.finish()?;
        let source = if status == HelperStatus::Found {
            let page = SourcePage::parse(source, grant)?;
            if page.status != SourceStatus::Present {
                return Err(FrameError::Malformed);
            }
            Some(page)
        } else {
            if !source.is_empty() {
                return Err(FrameError::Malformed);
            }
            None
        };
        Ok(Self {
            signed,
            status,
            source,
        })
    }
    pub fn public_key(&self) -> &[u8; 32] {
        self.signed.key
    }
    /// Checks B only; verify the nested A signature and current scope independently.
    pub fn verify_signature(&self) -> bool {
        self.signed.verify(HELPER_DOMAIN)
    }
}

// Keep the complete signed domains inside the actual pre-allocation transport ceilings.
const _: () = assert!(MAX_GRANT_BYTES <= FILE_SERVICE_REQUEST_MAX_BYTES);
const _: () = assert!(MAX_DELIVERY_BYTES <= FILE_SERVICE_REQUEST_MAX_BYTES);
const _: () = assert!(MAX_SOURCE_REPLY_BYTES <= FILE_SERVICE_RESPONSE_MAX_BYTES);
const _: () = assert!(MAX_HELPER_REPLY_BYTES <= FILE_SERVICE_RESPONSE_MAX_BYTES);

// Diagnostics must not accidentally dump file addresses, full member/group identities, signed
// grants or ciphertext pages. The actor's scoped status observations are the reporting surface.
impl std::fmt::Debug for GrantFields<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GrantFields { .. }")
    }
}
impl std::fmt::Debug for Grant<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Grant { .. }")
    }
}
impl std::fmt::Debug for Delivery<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Delivery { .. }")
    }
}
impl std::fmt::Debug for SourcePage<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SourcePage { .. }")
    }
}
impl std::fmt::Debug for HelperPage<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HelperPage { .. }")
    }
}

#[cfg(test)]
mod tests;
