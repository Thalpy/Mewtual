//! Bounded full-prefix data for a Welcome after the first governing Add.
use super::*;
use catcoms_replication::chat::{
    cold_records::{encode_header_row, HeaderRow},
    owner_bodies::{AppendBody, RemovalBody},
    owner_facts::OwnerFactId,
    owner_header::{HeaderKind, OwnerHeaderId, SignedOwnerHeader},
    owner_profile::{AdmissionBody, AdmissionOrigin},
    owner_state::transition::{ReplayBody, ReplayMath},
};

const TAG: &[u8] = b"mewtual/chat/governing-join-prefix/v2\0";
pub const MAX_V2_GOVERNING_JOIN_STEPS: usize = 64;
pub const MAX_V2_GOVERNING_JOIN_PREFIX_BYTES: usize = 512 * 1024;

/// One signed governing successor and the exact resulting public image.
pub struct V2GoverningJoinStepData<'a> {
    pub header: &'a SignedOwnerHeader,
    pub image: &'a [u8],
    pub body: ReplayBody<'a>,
}

enum BodyData {
    Admission(AdmissionBody),
    Append(AppendBody),
    Removal(RemovalBody),
}

impl BodyData {
    fn read(kind: HeaderKind, raw: &[u8]) -> Result<Self, SyncError> {
        let invalid = |_| SyncError::JoinRejected;
        match kind {
            HeaderKind::Admission => AdmissionBody::decode(raw)
                .map(Self::Admission)
                .map_err(invalid),
            HeaderKind::AcceptanceAppend => {
                AppendBody::decode(raw).map(Self::Append).map_err(invalid)
            }
            HeaderKind::Removal => RemovalBody::decode(raw).map(Self::Removal).map_err(invalid),
            HeaderKind::ReviewDecision => Err(SyncError::JoinRejected),
        }
    }

    fn replay_data(&self) -> ReplayBody<'_> {
        match self {
            Self::Admission(body) => ReplayBody::Admission(body),
            Self::Append(body) => ReplayBody::Append(body),
            Self::Removal(body) => ReplayBody::Removal(body),
        }
    }
}

/// Complete ordered records from the already pinned genesis to a later Add.
/// A decoded frame supplies only data. The caller must bind the final Add to
/// its authenticated Welcome and independently check the genuine MLS outcome.
pub struct V2GoverningJoinPrefixData<'a> {
    first: V2FirstAddProofData<'a>,
    successors: Vec<(HeaderRow<'a>, BodyData)>,
    bytes: usize,
}

impl fmt::Debug for V2GoverningJoinPrefixData<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("V2GoverningJoinPrefixData")
            .finish_non_exhaustive()
    }
}

impl<'a> V2GoverningJoinPrefixData<'a> {
    pub fn encode(
        first: &[u8],
        successors: &[V2GoverningJoinStepData<'_>],
    ) -> Result<Vec<u8>, SyncError> {
        if successors.is_empty() || successors.len() >= MAX_V2_GOVERNING_JOIN_STEPS {
            return Err(SyncError::Malformed);
        }
        V2FirstAddProofData::decode(first)?;
        let malformed = |_| SyncError::Malformed;
        let mut e = Encoder::new();
        e.put_bytes(TAG).map_err(malformed)?;
        e.put_u16(VERSION);
        e.put_bytes(first).map_err(malformed)?;
        e.put_u16(successors.len() as u16);
        for original in successors {
            let body = match (original.header.spec().kind, &original.body) {
                (HeaderKind::Admission, ReplayBody::Admission(body)) => body.encode(),
                (HeaderKind::AcceptanceAppend, ReplayBody::Append(body)) => body.encode(),
                (HeaderKind::Removal, ReplayBody::Removal(body)) => body.encode(),
                _ => return Err(SyncError::JoinRejected),
            };
            let row = encode_header_row(original.header, original.image)
                .map_err(|_| SyncError::JoinRejected)?;
            if e.len() + 8 + row.len() + body.len() > MAX_V2_GOVERNING_JOIN_PREFIX_BYTES {
                return Err(SyncError::Malformed);
            }
            e.put_bytes(&row).map_err(malformed)?;
            e.put_bytes(&body).map_err(malformed)?;
        }
        let bytes = e.finish();
        V2GoverningJoinPrefixData::decode(&bytes)?;
        Ok(bytes)
    }

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        if raw.len() > MAX_V2_GOVERNING_JOIN_PREFIX_BYTES {
            return Err(SyncError::Malformed);
        }
        let malformed = |_| SyncError::Malformed;
        let mut input = Decoder::new(raw);
        if input.get_bytes().map_err(malformed)? != TAG
            || input.get_u16().map_err(malformed)? != VERSION
        {
            return Err(SyncError::Malformed);
        }
        let first = V2FirstAddProofData::decode(input.get_bytes().map_err(malformed)?)?;
        let length = usize::from(input.get_u16().map_err(malformed)?);
        if !(1..MAX_V2_GOVERNING_JOIN_STEPS).contains(&length) {
            return Err(SyncError::Malformed);
        }
        let mut successors = Vec::with_capacity(length);
        for _ in 0..length {
            let header = HeaderRow::decode(input.get_bytes().map_err(malformed)?)
                .map_err(|_| SyncError::JoinRejected)?;
            let body = BodyData::read(
                header.header().spec().kind,
                input.get_bytes().map_err(malformed)?,
            )?;
            successors.push((header, body));
        }
        input.finish().map_err(malformed)?;
        if !matches!(successors.last(), Some((_, BodyData::Admission(_)))) {
            return Err(SyncError::JoinRejected);
        }
        Ok(Self {
            first,
            successors,
            bytes: raw.len(),
        })
    }

    /// Recompute each exact successor from the accepted genesis. The join
    /// must name the final Admission and Header, including when another valid
    /// Add occurs earlier in the same prefix.
    pub fn verify_selected_add(
        &self,
        accepted: &GenesisAnchorBindingData,
        selected_header: OwnerHeaderId,
        selected_admission: OwnerFactId,
    ) -> Result<ReplayMath, SyncError> {
        let (last, admission) = self.last_admission()?;
        let AdmissionOrigin::Add { after_epoch, .. } = admission.spec().origin else {
            return Err(SyncError::JoinRejected);
        };
        if last.header().id() != selected_header || admission.admission_id() != selected_admission {
            return Err(SyncError::JoinRejected);
        }
        let mut replay = self.first.replay_through_add(accepted)?;
        for (header, body) in &self.successors {
            replay = replay
                .derive_successor(header.header(), body.replay_data(), header.image())
                .map_err(|_| SyncError::JoinRejected)?
                .next;
        }
        if replay.mls_epoch_data() != after_epoch {
            return Err(SyncError::JoinRejected);
        }
        Ok(replay)
    }

    pub(super) fn last_admission(&self) -> Result<(&HeaderRow<'a>, &AdmissionBody), SyncError> {
        match self.successors.last() {
            Some((header, BodyData::Admission(admission))) => Ok((header, admission)),
            _ => Err(SyncError::JoinRejected),
        }
    }

    pub(super) fn frame_bytes(&self) -> usize {
        self.bytes
    }
}
