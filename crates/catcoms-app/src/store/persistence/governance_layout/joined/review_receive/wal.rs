//! Encrypted write-ahead framing for one exact lane update. The physical
//! writer checks prior membership and authenticates a separate admission tag
//! after covering the prepared bytes.
use super::{
    header::{MAX_FORK_BYTES, MAX_HEADER_BYTES},
    index::{Update, MAX_PAGES},
    lane_head::HEAD_BYTES,
    manifest::MAX_MANIFEST_BYTES,
    *,
};
use crate::store::persistence::{v2_member_outbox::fs::MAX_FILE, v2_text::TextRootData};
use catcoms_replication::chat::cold_index::{NodeRefData, MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT};
use catcoms_replication::chat::{join_bindings::GenesisAnchorBindingData, MAX_GROUP_ID_BYTES};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const TAG: &[u8] = b"catcoms/private-chat-review-receiver-write-ahead/v1\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-review-receiver-write-ahead-key/v1";
const FRAME_TAG: &[u8; 8] = b"CTV2RRW\x01";
const FRAME_HEADER: usize = 12;
const ADMISSION_BYTES: usize = 32;
const MAX_PIN: usize = 320 + MAX_GROUP_ID_BYTES;
const REF_BYTES: usize = 1 + 1 + 8 + 3 * 36;
const MAX_OLD: usize = MAX_INDEX_HEIGHT as usize + 1;
const MAX_DELTA: usize =
    2 + MAX_OLD * REF_BYTES + MAX_PAGES * (REF_BYTES + 4 + MAX_INDEX_FRAME_BYTES);
const MAX_CANDIDATE_BYTES: usize = FRAME_HEADER
    + 40
    + 4
    + TAG.len()
    + 36
    + 4
    + MAX_PIN
    + 2 * (4 + MAX_MANIFEST_BYTES)
    + 4
    + HEAD_BYTES
    + 1
    + 4
    + MAX_FORK_BYTES
    + 2 * MAX_DELTA;
pub(super) const MAX_WAL_BYTES: usize = MAX_CANDIDATE_BYTES + ADMISSION_BYTES;
const _: () = assert!(MAX_WAL_BYTES <= MAX_FILE);

pub(super) struct JournalKey(Zeroizing<[u8; 32]>);

impl JournalKey {
    pub(super) fn is_incomplete_scratch(raw: &[u8]) -> bool {
        if raw.len() < FRAME_TAG.len() {
            return FRAME_TAG.starts_with(raw);
        }
        if !raw.starts_with(FRAME_TAG) {
            return false;
        }
        if raw.len() < FRAME_HEADER {
            return true;
        }
        Self::length(raw).is_ok_and(|declared| raw.len() < declared)
    }

    pub(super) fn new(vault_key: &[u8; 32], marker: &[u8; 32]) -> Self {
        let mut kdf = blake3::Hasher::new_derive_key(KEY_DOMAIN);
        kdf.update(vault_key);
        kdf.update(marker);
        Self(Zeroizing::new(*kdf.finalize().as_bytes()))
    }

    pub(super) fn prepare(
        &self,
        (scope, pin): (&[u8; 32], &GenesisAnchorBindingData),
        manifests: [&[u8]; 2],
        head: &[u8],
        update: PreparedEffect<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        check_source(scope, &manifests, head)?;
        let pin = Zeroizing::new(pin.encode_data().map_err(|_| denied())?);
        let mut input = Encoder::with_capacity(
            4 + TAG.len()
                + 36
                + 4
                + pin.len()
                + 4 * 4
                + manifests[0].len()
                + manifests[1].len()
                + head.len()
                + update.output_bytes()?,
        );
        for field in [
            TAG,
            scope.as_slice(),
            &pin,
            manifests[0],
            manifests[1],
            head,
        ] {
            input.put_bytes(field).map_err(|_| denied())?;
        }
        match update {
            PreparedEffect::Header { frame, paths } => {
                input.put_u8(0);
                input.put_bytes(frame).map_err(|_| denied())?;
                for path in paths {
                    encode_path(&mut input, path)?;
                }
            }
            PreparedEffect::Fork(frame) => {
                input.put_u8(1);
                input.put_bytes(frame).map_err(|_| denied())?;
            }
        }
        let plain = Zeroizing::new(input.finish());
        let raw = frame(&seal(&self.0, &plain, rng)?);
        let len = FRAME_HEADER + raw.len();
        if len > MAX_CANDIDATE_BYTES {
            return Err(denied());
        }
        let mut covered = Zeroizing::new(Vec::with_capacity(len + ADMISSION_BYTES));
        covered.extend_from_slice(FRAME_TAG);
        covered.extend_from_slice(&(len as u32).to_le_bytes());
        covered.extend_from_slice(&raw);
        Ok(covered)
    }

    fn length(raw: &[u8]) -> Result<usize, AppError> {
        if raw.len() < FRAME_HEADER || !raw.starts_with(FRAME_TAG) {
            return Err(denied());
        }
        let size =
            u32::from_le_bytes(raw[8..FRAME_HEADER].try_into().map_err(|_| denied())?) as usize;
        if !(FRAME_HEADER + 41..=MAX_CANDIDATE_BYTES).contains(&size) {
            return Err(denied());
        }
        Ok(size)
    }

    fn admission(&self, candidate: &[u8]) -> [u8; ADMISSION_BYTES] {
        let mut hash = blake3::Hasher::new_keyed(&self.0);
        hash.update(b"catcoms/private-chat-review-receiver-admitted/v1\0");
        hash.update(candidate);
        *hash.finalize().as_bytes()
    }

    /// Issued only after the selected native row and historical coordinates
    /// are rechecked against the prepared candidate.
    pub(super) fn mark_admitted(&self, candidate: &mut Zeroizing<Vec<u8>>) {
        let admission = self.admission(candidate);
        candidate.extend_from_slice(&admission);
    }

    pub(super) fn open_admitted(&self, raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let size = Self::length(raw)?;
        if raw.len() != size + ADMISSION_BYTES {
            return Err(denied());
        }
        let (candidate, tag) = raw.split_at(size);
        if tag != self.admission(candidate) {
            return Err(denied());
        }
        self.open_candidate(candidate)
    }

    pub(super) fn open_scratch(&self, raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let size = Self::length(raw)?;
        if raw.len() < size || raw.len() > size + ADMISSION_BYTES {
            return Err(denied());
        }
        let (candidate, tag) = raw.split_at(size);
        if !self.admission(candidate).starts_with(tag) {
            return Err(denied());
        }
        self.open_candidate(candidate)
    }

    pub(super) fn open_candidate(&self, raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if Self::length(raw)? != raw.len() {
            return Err(denied());
        }
        Ok(Zeroizing::new(unseal(
            &self.0,
            &unframe(&raw[FRAME_HEADER..])?,
        )?))
    }
}

pub(super) enum PreparedEffect<'a> {
    Header {
        frame: &'a [u8],
        paths: &'a [Update; 2],
    },
    Fork(&'a [u8]),
}

impl PreparedEffect<'_> {
    fn output_bytes(&self) -> Result<usize, AppError> {
        match self {
            Self::Header { frame, paths } => {
                require_frame(frame, MAX_HEADER_BYTES)?;
                paths.iter().try_fold(1 + frame.len(), |total, path| {
                    require_path_count(path.old.len(), path.new.len())?;
                    let cost = total + 2 + path.old.len() * REF_BYTES;
                    path.new.iter().try_fold(cost, |done, page| {
                        let raw = page.frame.as_bytes();
                        require_frame(raw, MAX_INDEX_FRAME_BYTES)?;
                        Ok(done + REF_BYTES + 4 + raw.len())
                    })
                })
            }
            Self::Fork(frame) => {
                require_frame(frame, MAX_FORK_BYTES)?;
                Ok(1 + frame.len())
            }
        }
    }
}

pub(super) struct WriteAhead<'a> {
    pub(super) scope: [u8; 32],
    pub(super) pin: GenesisAnchorBindingData,
    pub(super) manifests: [&'a [u8]; 2],
    pub(super) head: &'a [u8],
    pub(super) effect: Effect<'a>,
}

pub(super) enum Effect<'a> {
    Header {
        frame: &'a [u8],
        paths: [PathDelta<'a>; 2],
    },
    Fork(&'a [u8]),
}

pub(super) struct PathDelta<'a> {
    pub(super) old: Vec<NodeRefData>,
    pub(super) exact: Vec<(NodeRefData, &'a [u8])>,
}

impl<'a> WriteAhead<'a> {
    pub(super) fn decode(plain: &'a [u8]) -> Result<Self, AppError> {
        if plain.len() > MAX_CANDIDATE_BYTES - 40 - FRAME_HEADER {
            return Err(denied());
        }
        let mut input = Decoder::new(plain);
        if field(&mut input)? != TAG {
            return Err(denied());
        }
        let scope = field(&mut input)?.try_into().map_err(|_| denied())?;
        let pin = field(&mut input)?;
        if pin.len() > MAX_PIN {
            return Err(denied());
        }
        let pin = GenesisAnchorBindingData::decode_data(pin).map_err(|_| denied())?;
        let manifests = [field(&mut input)?, field(&mut input)?];
        let head = field(&mut input)?;
        check_source(&scope, &manifests, head)?;
        let kind = input.get_u8().map_err(|_| denied())?;
        let output = field(&mut input)?;
        let effect = match kind {
            0 => {
                require_frame(output, MAX_HEADER_BYTES)?;
                Effect::Header {
                    frame: output,
                    paths: [read_path(&mut input)?, read_path(&mut input)?],
                }
            }
            1 => {
                require_frame(output, MAX_FORK_BYTES)?;
                Effect::Fork(output)
            }
            _ => return Err(denied()),
        };
        input.finish().map_err(|_| denied())?;
        Ok(Self {
            scope,
            pin,
            manifests,
            head,
            effect,
        })
    }
}

fn encode_path(output: &mut Encoder, path: &Update) -> Result<(), AppError> {
    output.put_u8(path.old.len() as u8);
    for node in &path.old {
        TextRootData::Present(*node).encode(output)?;
    }
    output.put_u8(path.new.len() as u8);
    for page in &path.new {
        TextRootData::Present(page.reference).encode(output)?;
        output
            .put_bytes(page.frame.as_bytes())
            .map_err(|_| denied())?;
    }
    Ok(())
}

fn read_path<'a>(input: &mut Decoder<'a>) -> Result<PathDelta<'a>, AppError> {
    let old_len = usize::from(input.get_u8().map_err(|_| denied())?);
    if old_len > MAX_OLD {
        return Err(denied());
    }
    let mut old = Vec::with_capacity(old_len);
    for _ in 0..old_len {
        let node = read_ref(input)?;
        if old
            .iter()
            .any(|prior: &NodeRefData| prior.page == node.page)
        {
            return Err(denied());
        }
        old.push(node);
    }
    let new_len = usize::from(input.get_u8().map_err(|_| denied())?);
    require_path_count(old_len, new_len)?;
    let mut exact = Vec::with_capacity(new_len);
    for _ in 0..new_len {
        let node = read_ref(input)?;
        let frame = field(input)?;
        require_frame(frame, MAX_INDEX_FRAME_BYTES)?;
        if old.iter().any(|prior| prior.page == node.page)
            || exact
                .iter()
                .any(|(prior, _): &(NodeRefData, &[u8])| prior.page == node.page)
        {
            return Err(denied());
        }
        exact.push((node, frame));
    }
    Ok(PathDelta { old, exact })
}

fn require_path_count(old: usize, new: usize) -> Result<(), AppError> {
    if old > MAX_OLD || !(1..=MAX_PAGES).contains(&new) || (old == 0 && new != 1) {
        return Err(denied());
    }
    Ok(())
}

fn check_source(scope: &[u8; 32], manifests: &[&[u8]; 2], head: &[u8]) -> Result<(), AppError> {
    if scope == &[0; 32] || manifests[0] == manifests[1] || head.len() != HEAD_BYTES {
        return Err(denied());
    }
    for bytes in manifests {
        require_frame(bytes, MAX_MANIFEST_BYTES)?;
    }
    Ok(())
}

fn require_frame(bytes: &[u8], maximum: usize) -> Result<(), AppError> {
    if !(41..=maximum).contains(&bytes.len()) {
        return Err(denied());
    }
    Ok(())
}

fn field<'a>(input: &mut Decoder<'a>) -> Result<&'a [u8], AppError> {
    input.get_bytes().map_err(|_| denied())
}

fn read_ref(input: &mut Decoder<'_>) -> Result<NodeRefData, AppError> {
    match TextRootData::decode(input)? {
        TextRootData::Present(node) => Ok(node),
        TextRootData::Empty => Err(denied()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn recipient_wal_candidate_cannot_recover_without_its_exact_admission_tag() {
        let key = JournalKey::new(&[1; 32], &[2; 32]);
        let mut rng = ChaCha20Rng::seed_from_u64(881);
        let pin = GenesisAnchorBindingData {
            group: vec![3; 16],
            founder: [4; 32],
            profile: [5; 32],
            admission: [6; 32],
            tenure: [7; 32],
            header: [8; 32],
            state: [9; 32],
        };
        let mut candidate = key
            .prepare(
                (&[3; 32], &pin),
                [&[10; 41], &[11; 41]],
                &[12; HEAD_BYTES],
                PreparedEffect::Fork(&[13; 41]),
                &mut rng,
            )
            .unwrap();
        let size = candidate.len();
        assert!(key.open_admitted(&candidate).is_err());
        assert!(key.open_scratch(&candidate).is_ok());
        key.mark_admitted(&mut candidate);
        assert!(key.open_admitted(&candidate[..size + 31]).is_err());
        assert!(key.open_scratch(&candidate[..size + 31]).is_ok());
        let opened = key.open_admitted(&candidate).unwrap();
        assert!(WriteAhead::decode(&opened).unwrap().pin == pin);
        let mut another = key
            .prepare(
                (&[3; 32], &pin),
                [&[10; 41], &[11; 41]],
                &[12; HEAD_BYTES],
                PreparedEffect::Fork(&[14; 41]),
                &mut rng,
            )
            .unwrap();
        another.extend_from_slice(&candidate[size..]);
        assert!(key.open_admitted(&another).is_err());
        assert!(JournalKey::new(&[1; 32], &[3; 32])
            .open_admitted(&candidate)
            .is_err());
        candidate[size] ^= 0x80;
        assert!(key.open_admitted(&candidate).is_err());
        assert!(key.open_scratch(&candidate).is_err());
    }
}
