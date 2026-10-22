//! Fixed private codec. Only the enclosing paid channel can call sealing primitives; its
//! consumed PreparedReply owns the already-spent nonce. No raw-key/header sealing API escapes.
use super::Refused;
use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm,
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

pub(super) const HEADER: usize = 40;
pub(super) const ADMISSION: usize = (HEADER + 32) * 2;
const DOMAIN: &[u8] = b"Mewtual native voice handoff v1";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Direction {
    Request = 0,
    Response = 1,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::voice_handoff) enum BodyKind {
    Control = 0,
    Data = 1,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::voice_handoff) enum Operation {
    Input = 1,
    Admit = 2,
    Result = 3,
    ReceiptAck = 4,
    Settle = 5,
    ClosePhase = 6,
    FinalAck = 7,
    BeginDerive = 8,
    Status = 9,
}
impl Operation {
    fn parse(value: u8) -> Result<Self, Refused> {
        match value {
            1 => Ok(Self::Input),
            2 => Ok(Self::Admit),
            3 => Ok(Self::Result),
            4 => Ok(Self::ReceiptAck),
            5 => Ok(Self::Settle),
            6 => Ok(Self::ClosePhase),
            7 => Ok(Self::FinalAck),
            8 => Ok(Self::BeginDerive),
            9 => Ok(Self::Status),
            _ => Err(Refused::Header),
        }
    }
}
/// A Copy header is immutable metadata, NOT authentication/replay/Root authority. Only the
/// enclosing Incoming/PreparedReply owns physical admission and the one outgoing nonce.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Header([u8; HEADER]);
impl Header {
    #[cfg(test)]
    pub(super) fn fixture_raw(&self) -> [u8; HEADER] {
        self.0
    }
    pub(super) fn new(
        direction: Direction,
        operation: Operation,
        kind: BodyKind,
        slot: u8,
        sequence: u64,
        logical: [u8; 16],
        length: usize,
    ) -> Result<Self, Refused> {
        let class = match (direction, operation, kind) {
            (Direction::Request, Operation::Result, BodyKind::Data)
            | (Direction::Response, Operation::Input, BodyKind::Data) => 2048 + 16,
            (Direction::Request, Operation::Result, BodyKind::Control) => {
                return Err(Refused::Header)
            }
            (_, _, BodyKind::Control) => 256 + 16,
            _ => return Err(Refused::Header),
        };
        if !(1..=64).contains(&slot) || sequence == 0 || !(16..=class).contains(&length) {
            return Err(Refused::Header);
        }
        let mut bytes = [0; HEADER];
        bytes[..4].copy_from_slice(b"MWVH");
        bytes[4] = 1;
        bytes[5] = direction as u8;
        bytes[6] = operation as u8;
        bytes[7] = slot;
        bytes[8..16].copy_from_slice(&sequence.to_be_bytes());
        bytes[16..32].copy_from_slice(&logical);
        bytes[32..36].copy_from_slice(&(length as u32).to_be_bytes());
        bytes[36] = kind as u8;
        Ok(Self(bytes))
    }
    fn parse(bytes: [u8; HEADER]) -> Result<Self, Refused> {
        let direction = match bytes[5] {
            0 => Direction::Request,
            1 => Direction::Response,
            _ => return Err(Refused::Header),
        };
        let kind = match bytes[36] {
            0 => BodyKind::Control,
            1 => BodyKind::Data,
            _ => return Err(Refused::Header),
        };
        let operation = Operation::parse(bytes[6])?;
        let header = Self::new(
            direction,
            operation,
            kind,
            bytes[7],
            u64::from_be_bytes(bytes[8..16].try_into().expect("fixed sequence")),
            bytes[16..32].try_into().expect("fixed logical identity"),
            u32::from_be_bytes(bytes[32..36].try_into().expect("fixed length")) as usize,
        )?;
        // Exact reconstruction pins magic, version and reserved bytes as well as every field.
        if header.0 != bytes {
            return Err(Refused::Header);
        }
        Ok(header)
    }
    pub(super) fn sequence(&self) -> u64 {
        u64::from_be_bytes(self.0[8..16].try_into().expect("fixed sequence"))
    }
    pub(super) fn logical(&self) -> [u8; 16] {
        self.0[16..32].try_into().expect("fixed logical identity")
    }
    pub(super) fn length(&self) -> usize {
        u32::from_be_bytes(self.0[32..36].try_into().expect("fixed length")) as usize
    }
    pub(super) fn slot(&self) -> u8 {
        self.0[7]
    }
    pub(super) fn kind(&self) -> BodyKind {
        if self.0[36] == 0 {
            BodyKind::Control
        } else {
            BodyKind::Data
        }
    }
    pub(super) fn operation(&self) -> Operation {
        Operation::parse(self.0[6]).expect("closed header operation")
    }
}
pub(super) struct Keys {
    request_mac: Zeroizing<[u8; 32]>,
    request_body: Zeroizing<[u8; 32]>,
    response_mac: Zeroizing<[u8; 32]>,
    response_body: Zeroizing<[u8; 32]>,
}
impl Keys {
    /// Deliberately signs noncanonical test input so parser/matrix checks cannot hide behind
    /// an invalid old MAC. This constructor is absent from production code.
    #[cfg(test)]
    pub(super) fn fixture_raw_admission(&self, raw: [u8; HEADER]) -> [u8; ADMISSION] {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.request_mac.as_ref()).unwrap();
        mac.update(&raw);
        encode(&Header(raw), &mac.finalize().into_bytes().into())
    }
    pub(super) fn new(seed: &[u8; 32]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(Some(DOMAIN), seed);
        let derive = |label: &[u8]| {
            let mut key = Zeroizing::new([0; 32]);
            hkdf.expand(label, key.as_mut()).expect("fixed HKDF output");
            key
        };
        Self {
            request_mac: derive(b"v1 c2n header HMAC-SHA256"),
            request_body: derive(b"v1 c2n body AES-256-GCM"),
            response_mac: derive(b"v1 n2c header HMAC-SHA256"),
            response_body: derive(b"v1 n2c body AES-256-GCM"),
        }
    }
    pub(super) fn authenticate_request(&self, text: &str) -> Result<Header, Refused> {
        let (header, tag) = decode(text)?;
        if header.0[5] != Direction::Request as u8 {
            return Err(Refused::Header);
        }
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.request_mac.as_ref())
            .expect("fixed HMAC key");
        mac.update(&header.0);
        mac.verify_slice(&tag)
            .map_err(|_| Refused::Authentication)?;
        Ok(header)
    }
    pub(super) fn open_request(&self, header: &Header, body: &mut [u8]) -> Result<(), Refused> {
        if body.len() != header.length() || body.len() < 16 {
            return Err(Refused::Bounds);
        }
        let at = body.len() - 16;
        let tag: [u8; 16] = body[at..].try_into().expect("fixed AEAD tag");
        // Pinned aes-gcm verifies the detached tag before in-place decryption. No Vec growth,
        // borrowed caller plaintext or provider-owned result is returned without its custody.
        let cipher = Aes256Gcm::new_from_slice(self.request_body.as_ref()).expect("fixed AEAD key");
        if cipher
            .decrypt_in_place_detached(&nonce(header), &header.0, &mut body[..at], (&tag).into())
            .is_err()
        {
            body.zeroize();
            return Err(Refused::Body);
        }
        Ok(())
    }
    pub(super) fn seal_response(
        &self,
        header: &Header,
        request: &Header,
        body: &mut [u8],
    ) -> Result<(), Refused> {
        if body.len() != header.length() || body.len() < 16 {
            return Err(Refused::Bounds);
        }
        let at = body.len() - 16;
        let cipher =
            Aes256Gcm::new_from_slice(self.response_body.as_ref()).expect("fixed AEAD key");
        let aad = response_aad(header, request);
        let tag = cipher
            .encrypt_in_place_detached(&nonce(header), &aad, &mut body[..at])
            .map_err(|_| Refused::Body)?;
        body[at..].copy_from_slice(&tag);
        Ok(())
    }
    pub(super) fn response_admission(&self, header: &Header, request: &Header) -> [u8; ADMISSION] {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.response_mac.as_ref())
            .expect("fixed HMAC key");
        mac.update(&response_aad(header, request));
        encode(header, &mac.finalize().into_bytes().into())
    }
    #[cfg(test)]
    pub(super) fn request(&self, header: Header, plaintext: &[u8]) -> ([u8; ADMISSION], Vec<u8>) {
        assert_eq!(header.length(), plaintext.len() + 16);
        let mut body = plaintext.to_vec();
        let tag = Aes256Gcm::new_from_slice(self.request_body.as_ref())
            .unwrap()
            .encrypt_in_place_detached(&nonce(&header), &header.0, &mut body)
            .unwrap();
        body.extend_from_slice(&tag);
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.request_mac.as_ref()).unwrap();
        mac.update(&header.0);
        (encode(&header, &mac.finalize().into_bytes().into()), body)
    }
    #[cfg(test)]
    pub(super) fn response(
        &self,
        text: &str,
        request: &Header,
        body: &mut [u8],
    ) -> Result<Header, Refused> {
        let (header, tag) = decode(text)?;
        if header.0[5] != Direction::Response as u8
            || header.logical() != request.logical()
            || header.operation() != request.operation()
            || header.slot() != request.slot()
            || body.len() != header.length()
        {
            return Err(Refused::Binding);
        }
        let aad = response_aad(&header, request);
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.response_mac.as_ref()).unwrap();
        mac.update(&aad);
        mac.verify_slice(&tag)
            .map_err(|_| Refused::Authentication)?;
        let at = body.len() - 16;
        let tag: [u8; 16] = body[at..].try_into().unwrap();
        Aes256Gcm::new_from_slice(self.response_body.as_ref())
            .unwrap()
            .decrypt_in_place_detached(&nonce(&header), &aad, &mut body[..at], (&tag).into())
            .map_err(|_| Refused::Body)?;
        Ok(header)
    }
}
fn response_aad(header: &Header, request: &Header) -> [u8; HEADER * 2] {
    let mut aad = [0; HEADER * 2];
    aad[..HEADER].copy_from_slice(&header.0);
    aad[HEADER..].copy_from_slice(&request.0);
    aad
}
fn nonce(header: &Header) -> aes_gcm::Nonce<aes_gcm::aead::consts::U12> {
    let mut bytes = [0; 12];
    bytes[..4].copy_from_slice(if header.0[5] == 0 { b"VHQ1" } else { b"VHP1" });
    bytes[4..].copy_from_slice(&header.0[8..16]);
    bytes.into()
}
fn encode(header: &Header, tag: &[u8; 32]) -> [u8; ADMISSION] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = [0; ADMISSION];
    for (index, byte) in header.0.iter().chain(tag).copied().enumerate() {
        result[index * 2] = HEX[(byte >> 4) as usize];
        result[index * 2 + 1] = HEX[(byte & 15) as usize];
    }
    result
}
fn decode(text: &str) -> Result<(Header, [u8; 32]), Refused> {
    if text.len() != ADMISSION {
        return Err(Refused::Header);
    }
    let mut bytes = [0; HEADER + 32];
    let nibble = |byte| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(Refused::Header),
    };
    for (index, chunk) in text.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = nibble(chunk[0])? * 16 + nibble(chunk[1])?;
    }
    let header = Header::parse(bytes[..HEADER].try_into().expect("fixed header"))?;
    Ok((header, bytes[HEADER..].try_into().expect("fixed MAC")))
}
