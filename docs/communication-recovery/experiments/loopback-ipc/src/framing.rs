//! Fixture-only encrypted framing. These keys authorize no account/action/RTC effect.
//! A fixed authenticated header precedes body allocation; response authentication includes the
//! entire original request header. This is a deliberately sequential protocol, not a production
//! transport or a bound on the allocator, cryptographic library, browser or kernel heap.
use aes_gcm::{
    aead::{AeadInPlace, KeyInit},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

pub const HEADER: usize = 40;
pub const TAG: usize = 32;
pub const MAX_PLAIN: usize = 8192;
pub const MAX_CIPHER: usize = MAX_PLAIN + 16;
pub const MAX_ADMISSIONS: u64 = 64;
pub const REQUEST: u8 = 0;
pub const RESPONSE: u8 = 1;
const SALT: &[u8] = b"Mewtual IPC fixture v1";

/// Fresh, immutable keys for one of the two finite bootstrap grants. Separate seeds prevent
/// independently initialized browser counters from ever encrypting under the same key/nonce.
pub struct Keys {
    request_mac: Zeroizing<[u8; 32]>,
    request_body: Zeroizing<[u8; 32]>,
    response_mac: Zeroizing<[u8; 32]>,
    response_body: Zeroizing<[u8; 32]>,
}
impl Keys {
    pub fn new(seed: &[u8; 32]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(Some(SALT), seed);
        let derive = |label: &[u8]| {
            let mut key = Zeroizing::new([0; 32]);
            hkdf.expand(label, key.as_mut()).expect("fixed HKDF size");
            key
        };
        Self {
            request_mac: derive(b"v1 c2n header HMAC-SHA256"),
            request_body: derive(b"v1 c2n body AES-256-GCM"),
            response_mac: derive(b"v1 n2c header HMAC-SHA256"),
            response_body: derive(b"v1 n2c body AES-256-GCM"),
        }
    }
    fn mac(&self, direction: u8, header: &Header, request: Option<&Header>) -> [u8; TAG] {
        let key = if direction == REQUEST {
            &self.request_mac
        } else {
            &self.response_mac
        };
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_ref()).expect("fixed HMAC key");
        mac.update(&header.0);
        if let Some(request) = request {
            mac.update(&request.0);
        }
        mac.finalize().into_bytes().into()
    }
    pub fn authenticate_request(&self, admission: &str) -> Result<Header, &'static str> {
        let (header, tag) = decode(admission)?;
        if header.direction() != REQUEST {
            return Err("wrong frame direction");
        }
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.request_mac.as_ref())
            .expect("fixed HMAC key");
        mac.update(&header.0);
        mac.verify_slice(&tag)
            .map_err(|_| "frame header authentication refused")?;
        Ok(header)
    }
    pub fn response_admission(&self, header: &Header, request: &Header) -> String {
        encode(header, &self.mac(RESPONSE, header, Some(request)))
    }
    pub fn open_request(&self, header: &Header, bytes: &mut Vec<u8>) -> Result<(), &'static str> {
        let cipher = Aes256Gcm::new_from_slice(self.request_body.as_ref()).expect("fixed AEAD key");
        cipher
            .decrypt_in_place(&nonce(header), &header.0, bytes)
            .map_err(|_| "frame body authentication refused")
    }
    pub fn seal_response(
        &self,
        header: &Header,
        request: &Header,
        bytes: &mut Vec<u8>,
    ) -> Result<(), &'static str> {
        let cipher =
            Aes256Gcm::new_from_slice(self.response_body.as_ref()).expect("fixed AEAD key");
        let mut aad = [0; HEADER * 2];
        aad[..HEADER].copy_from_slice(&header.0);
        aad[HEADER..].copy_from_slice(&request.0);
        cipher
            .encrypt_in_place(&nonce(header), &aad, bytes)
            .map_err(|_| "fixture encryption refused")
    }
    #[cfg(test)]
    pub fn test_request(&self, sequence: u64, bytes: &mut Vec<u8>) -> String {
        let header = Header::new(REQUEST, 1, sequence, [9; 16], bytes.len() + 16).unwrap();
        Aes256Gcm::new_from_slice(self.request_body.as_ref())
            .unwrap()
            .encrypt_in_place(&nonce(&header), &header.0, bytes)
            .unwrap();
        encode(&header, &self.mac(REQUEST, &header, None))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header([u8; HEADER]);
impl Header {
    pub fn new(
        direction: u8,
        grant: u8,
        sequence: u64,
        id: [u8; 16],
        cipher_bytes: usize,
    ) -> Result<Self, &'static str> {
        if direction > RESPONSE
            || !(1..=2).contains(&grant)
            || sequence == 0
            || !(16..=MAX_CIPHER).contains(&cipher_bytes)
        {
            return Err("invalid fixed frame header");
        }
        let mut bytes = [0; HEADER];
        bytes[..4].copy_from_slice(b"MWIP");
        bytes[4] = 1;
        bytes[5] = direction;
        bytes[6] = grant;
        bytes[8..16].copy_from_slice(&sequence.to_be_bytes());
        bytes[16..32].copy_from_slice(&id);
        bytes[32..36].copy_from_slice(&(cipher_bytes as u32).to_be_bytes());
        Ok(Self(bytes))
    }
    pub fn grant(&self) -> usize {
        self.0[6] as usize - 1
    }
    pub fn direction(&self) -> u8 {
        self.0[5]
    }
    pub fn sequence(&self) -> u64 {
        u64::from_be_bytes(self.0[8..16].try_into().expect("fixed field"))
    }
    pub fn id(&self) -> [u8; 16] {
        self.0[16..32].try_into().expect("fixed field")
    }
    pub fn cipher_bytes(&self) -> usize {
        u32::from_be_bytes(self.0[32..36].try_into().expect("fixed field")) as usize
    }
}
fn nonce(header: &Header) -> Nonce<aes_gcm::aead::consts::U12> {
    let mut bytes = [0; 12];
    bytes[..4].copy_from_slice(if header.direction() == REQUEST {
        b"REQ1"
    } else {
        b"RSP1"
    });
    bytes[4..].copy_from_slice(&header.0[8..16]);
    bytes.into()
}
/// Untrusted slot selection only. It grants nothing until that immutable slot verifies HMAC.
pub fn claimed_grant(text: &str) -> Result<usize, &'static str> {
    Ok(decode(text)?.0.grant())
}
fn decode(text: &str) -> Result<(Header, [u8; TAG]), &'static str> {
    if text.len() != (HEADER + TAG) * 2 {
        return Err("invalid admission length");
    }
    let mut bytes = [0; HEADER + TAG];
    let nibble = |byte| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err("noncanonical admission"),
    };
    for (index, chunk) in text.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = nibble(chunk[0])? * 16 + nibble(chunk[1])?;
    }
    let raw: [u8; HEADER] = bytes[..HEADER].try_into().expect("fixed field");
    let parsed = Header(raw);
    if raw[..4] != *b"MWIP"
        || raw[4] != 1
        || raw[7] != 0
        || raw[36..] != [0; 4]
        || Header::new(
            parsed.direction(),
            raw[6],
            parsed.sequence(),
            parsed.id(),
            parsed.cipher_bytes(),
        )? != parsed
    {
        return Err("invalid fixed frame header");
    }
    Ok((parsed, bytes[HEADER..].try_into().expect("fixed field")))
}
fn encode(header: &Header, tag: &[u8; TAG]) -> String {
    let mut text = String::with_capacity((HEADER + TAG) * 2);
    const HEX: &[u8] = b"0123456789abcdef";
    for byte in header.0.iter().chain(tag) {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    text
}

/// One process-wide synchronous admission transaction, called only after header authentication
/// and frame-permit acquisition. Both grant replay floors share one finite execution allowance.
/// It deliberately rejects reordered requests. No EOF, bad body or timeout refunds a sequence.
#[derive(Default)]
pub struct Sequences {
    requests: [u64; 2],
    responses: [u64; 2],
    admissions: u64,
}
impl Sequences {
    pub fn admit(&mut self, header: &Header) -> Result<(), &'static str> {
        if self.admissions == MAX_ADMISSIONS || header.sequence() <= self.requests[header.grant()] {
            return Err("frame replay or admission allowance exhausted");
        }
        self.requests[header.grant()] = header.sequence();
        self.admissions += 1;
        Ok(())
    }
    pub fn response(
        &mut self,
        request: &Header,
        plain_bytes: usize,
    ) -> Result<Header, &'static str> {
        let sequence = self.responses[request.grant()]
            .checked_add(1)
            .ok_or("response sequence exhausted")?;
        let header = Header::new(
            RESPONSE,
            (request.grant() + 1) as u8,
            sequence,
            request.id(),
            plain_bytes + 16,
        )?;
        // Burn before encryption or writing. A failed or lost response never reuses this nonce.
        self.responses[request.grant()] = sequence;
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn header_mac_rejects_tampering_cross_grant_direction_and_noncanonical_encoding() {
        let keys = Keys::new(&[7; 32]);
        let request = Header::new(REQUEST, 1, 1, [9; 16], MAX_CIPHER).unwrap();
        let encoded = encode(&request, &keys.mac(REQUEST, &request, None));
        assert_eq!(keys.authenticate_request(&encoded).unwrap(), request);
        for index in [0, 8, 10, 12, 15, 16, 32, 64, 72, 143] {
            let mut bad = encoded.clone().into_bytes();
            bad[index] = if bad[index] == b'0' { b'1' } else { b'0' };
            assert!(keys
                .authenticate_request(std::str::from_utf8(&bad).unwrap())
                .is_err());
        }
        assert!(Keys::new(&[8; 32]).authenticate_request(&encoded).is_err());
        assert!(keys.authenticate_request(&encoded.to_uppercase()).is_err());
        assert!(Header::new(REQUEST, 0, 1, [0; 16], 16).is_err());
        assert!(Header::new(REQUEST, 1, 0, [0; 16], 16).is_err());
        assert!(Header::new(REQUEST, 1, 1, [0; 16], MAX_CIPHER + 1).is_err());
    }
    #[test]
    fn admission_is_global_finite_and_never_refunds_or_accepts_reordering() {
        let mut sequences = Sequences::default();
        let header = |grant, seq| Header::new(REQUEST, grant, seq, [0; 16], 16).unwrap();
        sequences.admit(&header(1, 2)).unwrap();
        assert!(sequences.admit(&header(1, 1)).is_err());
        assert!(sequences.admit(&header(1, 2)).is_err());
        for index in 1..MAX_ADMISSIONS {
            sequences.admit(&header(2, index)).unwrap();
        }
        assert!(
            sequences.admit(&header(1, 3)).is_err(),
            "other grant cannot reset the global allowance"
        );
        let a = sequences.response(&header(1, 2), 0).unwrap();
        let b = sequences.response(&header(1, 2), 0).unwrap();
        assert_eq!(b.sequence(), a.sequence() + 1);
    }
    #[test]
    fn maximum_body_and_response_authentication_bind_the_entire_request() {
        let keys = Keys::new(&[7; 32]);
        let request = Header::new(REQUEST, 1, 1, [9; 16], MAX_CIPHER).unwrap();
        let mut bytes = vec![42; MAX_PLAIN];
        Aes256Gcm::new_from_slice(keys.request_body.as_ref())
            .unwrap()
            .encrypt_in_place(&nonce(&request), &request.0, &mut bytes)
            .unwrap();
        let mut corrupt = bytes.clone();
        corrupt[0] ^= 1;
        assert!(keys.open_request(&request, &mut corrupt).is_err());
        keys.open_request(&request, &mut bytes).unwrap();
        assert_eq!(bytes.len(), MAX_PLAIN);
        let response = Header::new(RESPONSE, 1, 1, request.id(), MAX_CIPHER).unwrap();
        keys.seal_response(&response, &request, &mut bytes).unwrap();
        let newer_same_id = Header::new(REQUEST, 1, 2, request.id(), MAX_CIPHER).unwrap();
        assert_ne!(
            keys.response_admission(&response, &request),
            keys.response_admission(&response, &newer_same_id)
        );
        let mut wrong_aad = [0; HEADER * 2];
        wrong_aad[..HEADER].copy_from_slice(&response.0);
        wrong_aad[HEADER..].copy_from_slice(&newer_same_id.0);
        let cipher = Aes256Gcm::new_from_slice(keys.response_body.as_ref()).unwrap();
        assert!(cipher
            .decrypt_in_place(&nonce(&response), &wrong_aad, &mut bytes.clone())
            .is_err());
        wrong_aad[HEADER..].copy_from_slice(&request.0);
        cipher
            .decrypt_in_place(&nonce(&response), &wrong_aad, &mut bytes)
            .unwrap();
        assert_eq!(bytes, vec![42; MAX_PLAIN]);
    }
}
