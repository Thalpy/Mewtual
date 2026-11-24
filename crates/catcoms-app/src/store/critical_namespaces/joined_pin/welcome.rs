//! Exact private records for possible Welcome entry and its completed result.
use super::*;
use catcoms_replication::chat::cold_selected_history::{
    SelectedHistoryNodePage, MAX_SELECTED_HISTORY_NODE_PAGE_BYTES,
};
use catcoms_replication::chat::{
    owner_header::{HeaderKind, OwnerHeaderId, SignedOwnerHeader, MAX_HEADER_BYTES},
    V2PreparedSelectedHistoryData,
};
use catcoms_sync::{
    PreparedV2JoinedEviction, PreparedV2JoinedMembership, RetiredV2JoinedAdd,
    RetiredV2JoinedEviction, SealedV2JoinedOutcome, V2JoinDeliveryData,
    V2MemberTransitionFrameData, V2PendingJoinContextData, MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES,
    MAX_V2_MEMBER_TRANSITION_FRAME_BYTES,
};

const ENTRY_TAG: &[u8] = b"catcoms/private-chat-v2-welcome-entry/v1\0";
const ENTRY_KEY: &str = "catcoms/private-chat-v2-welcome-entry-key/v1";
const OUTCOME_TAG: &[u8] = b"catcoms/private-chat-v2-welcome-outcome/v1\0";
const OUTCOME_KEY: &str = "catcoms/private-chat-v2-welcome-outcome-key/v1";
const MEMBER_ADD_TAG: &[u8] = b"catcoms/private-chat-v2-joined-member-add/v1\0";
const MEMBER_ADD_KEY: &str = "catcoms/private-chat-v2-joined-member-add-key/v1";
const MEMBER_HEADER_TAG: &[u8] = b"catcoms/private-chat-v2-joined-member-header/v1\0";
const MEMBER_HEADER_KEY: &str = "catcoms/private-chat-v2-joined-member-header-key/v1";
const MEMBER_SELECTION_TAG: &[u8] = b"catcoms/private-chat-v2-joined-member-selected/v1\0";
const MEMBER_SELECTION_KEY: &str = "catcoms/private-chat-v2-joined-member-selected-key/v1";
const VERSION: u16 = 1;
const LATER_ENTRY: u16 = 2;
const OUTCOME_WITH_HISTORY: u16 = 2;
const MEMBER_REMOVAL: u16 = 2;
const MEMBER_EVICTION: u16 = 3;
const SEALED_OVERHEAD: usize = 40;
const DIGESTS: usize = 5 * 36;
const MAX_OUTCOME_CIPHERTEXT: usize = 512 * 1024 + 4096;
pub(in crate::store) const ENTRY_FRAME: usize =
    4 + ENTRY_TAG.len() + 2 + DIGESTS + 4 + MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES + SEALED_OVERHEAD;
pub(in crate::store) const OUTCOME_FRAME: usize = 4
    + OUTCOME_TAG.len()
    + 2
    + DIGESTS
    + 4
    + 24
    + 4
    + MAX_OUTCOME_CIPHERTEXT
    + 4
    + MAX_SELECTED_HISTORY_NODE_PAGE_BYTES
    + SEALED_OVERHEAD;
pub(in crate::store) const MEMBER_ADD_FRAME: usize = 4
    + MEMBER_ADD_TAG.len()
    + 2
    + DIGESTS
    + 4
    + 8
    + 4
    + 32
    + 4
    + MAX_V2_MEMBER_TRANSITION_FRAME_BYTES
    + 4
    + MAX_SELECTED_HISTORY_NODE_PAGE_BYTES
    + SEALED_OVERHEAD;
pub(in crate::store) const MEMBER_HEADER_FRAME: usize = 4
    + MEMBER_HEADER_TAG.len()
    + 2
    + DIGESTS
    + 4
    + 8
    + 4
    + 1
    + 4
    + MAX_HEADER_BYTES
    + SEALED_OVERHEAD;

pub(in crate::store) const MEMBER_SELECTION_FRAME: usize =
    4 + MEMBER_SELECTION_TAG.len() + 2 + DIGESTS + 4 + 8 + 4 + 32 + SEALED_OVERHEAD;

pub(in crate::store) fn seal_member_selection(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    epoch: u64,
    predecessor: &[u8],
    selected: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if epoch < 2
        || predecessor.is_empty()
        || selected.is_empty()
        || selected.len() > MEMBER_ADD_FRAME
    {
        return Err(refused());
    }
    seal_record(
        key,
        MEMBER_SELECTION_KEY,
        MEMBER_SELECTION_TAG,
        VERSION,
        context_fields(context, predecessor),
        &[&epoch.to_be_bytes(), blake3::hash(selected).as_bytes()],
        rng,
    )
}

pub(in crate::store) fn require_member_selection(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    epoch: u64,
    predecessor: &[u8],
    selected: &[u8],
    raw: &[u8],
) -> Result<(), AppError> {
    if epoch < 2
        || predecessor.is_empty()
        || selected.is_empty()
        || selected.len() > MEMBER_ADD_FRAME
    {
        return Err(refused());
    }
    let plain = open_record(key, MEMBER_SELECTION_KEY, raw, MEMBER_SELECTION_FRAME)?;
    let (version, mut reader) = decoder(
        &plain,
        MEMBER_SELECTION_TAG,
        context_fields(context, predecessor),
    )?;
    let bad = |_| refused();
    if version != VERSION
        || reader.get_bytes().map_err(bad)? != epoch.to_be_bytes()
        || reader.get_bytes().map_err(bad)? != blake3::hash(selected).as_bytes()
    {
        return Err(refused());
    }
    reader.finish().map_err(bad)
}

pub(in crate::store) fn seal_member_header(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    predecessor: &[u8],
    (epoch, conflict): (u64, bool),
    header: &SignedOwnerHeader,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if predecessor.is_empty() || epoch < 2 {
        return Err(refused());
    }
    seal_record(
        key,
        MEMBER_HEADER_KEY,
        MEMBER_HEADER_TAG,
        VERSION,
        context_fields(context, predecessor),
        &[
            &epoch.to_be_bytes(),
            &[u8::from(conflict)],
            &header.encode(),
        ],
        rng,
    )
}

pub(in crate::store) fn open_member_header(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    predecessor: &[u8],
    (epoch, conflict): (u64, bool),
    raw: &[u8],
) -> Result<SignedOwnerHeader, AppError> {
    if predecessor.is_empty() || epoch < 2 {
        return Err(refused());
    }
    let plain = open_record(key, MEMBER_HEADER_KEY, raw, MEMBER_HEADER_FRAME)?;
    let (version, mut decoder) = decoder(
        &plain,
        MEMBER_HEADER_TAG,
        context_fields(context, predecessor),
    )?;
    let bad = |_| refused();
    if version != VERSION
        || decoder.get_bytes().map_err(bad)? != epoch.to_be_bytes()
        || decoder.get_bytes().map_err(bad)? != [u8::from(conflict)]
    {
        return Err(refused());
    }
    let header =
        SignedOwnerHeader::decode(decoder.get_bytes().map_err(bad)?).map_err(|_| refused())?;
    decoder.finish().map_err(bad)?;
    Ok(header)
}

pub(in crate::store) struct OpenedMemberAdd {
    pub(in crate::store) delivery: Vec<u8>,
    history: Zeroizing<Vec<u8>>,
    binding: (u64, [u8; 32]),
    version: u16,
}

#[expect(
    clippy::large_enum_variant,
    reason = "The bounded candidate carries the paid original transition inline through selection."
)]
pub(in crate::store) enum JoinedMembershipCandidate {
    Continuing(Option<PreparedV2JoinedMembership>),
    Eviction(Option<PreparedV2JoinedEviction>),
}

impl JoinedMembershipCandidate {
    pub(in crate::store) fn header_data(&self) -> &SignedOwnerHeader {
        match self {
            Self::Continuing(candidate) => candidate
                .as_ref()
                .expect("held membership candidate")
                .header_data(),
            Self::Eviction(candidate) => candidate
                .as_ref()
                .expect("held eviction candidate")
                .header_data(),
        }
    }

    pub(in crate::store) fn matches_frame_data(&self, delivery: &[u8]) -> bool {
        match self {
            Self::Continuing(candidate) => candidate
                .as_ref()
                .is_some_and(|candidate| candidate.matches_frame_data(delivery)),
            Self::Eviction(candidate) => candidate
                .as_ref()
                .is_some_and(|candidate| candidate.matches_frame_data(delivery)),
        }
    }

    fn record_version(&self) -> Result<u16, AppError> {
        match self {
            Self::Continuing(_) => member_transition_version(self.header_data().spec().kind),
            Self::Eviction(_) => Ok(MEMBER_EVICTION),
        }
    }

    fn history_data(&self) -> &V2PreparedSelectedHistoryData {
        match self {
            Self::Continuing(candidate) => candidate
                .as_ref()
                .expect("held membership candidate")
                .selected_history_data(),
            Self::Eviction(candidate) => candidate
                .as_ref()
                .expect("held eviction candidate")
                .selected_history_data(),
        }
    }

    /// A continuing row retains the selected MLS authenticator. An eviction
    /// retains the exact Commit digest and grants no successor SDK state.
    fn binding_data(&self) -> (u64, [u8; 32]) {
        match self {
            Self::Continuing(candidate) => candidate
                .as_ref()
                .expect("held membership candidate")
                .selected_epoch_data(),
            Self::Eviction(candidate) => candidate
                .as_ref()
                .expect("held eviction candidate")
                .removed_data(),
        }
    }
}

#[must_use = "retire the displaced joined continuation outside the host writers"]
#[expect(
    clippy::large_enum_variant,
    reason = "Installing a selected transition returns its whole retired custody without allocating."
)]
pub(in crate::store) enum RetiredJoinedMembership {
    Continuing {
        _retired: RetiredV2JoinedAdd,
    },
    Evicted {
        terminal: (OwnerHeaderId, u64),
        _retired: RetiredV2JoinedEviction,
    },
}

impl RetiredJoinedMembership {
    pub(in crate::store) fn terminal_data(&self) -> Option<(OwnerHeaderId, u64)> {
        match self {
            Self::Continuing { .. } => None,
            Self::Evicted { terminal, .. } => Some(*terminal),
        }
    }
}

impl OpenedMemberAdd {
    pub(in crate::store) fn require_original_candidate(
        &self,
        vault: &[u8; 32],
        candidate: &JoinedMembershipCandidate,
    ) -> Result<(u64, [u8; 32]), AppError> {
        if !candidate.matches_frame_data(&self.delivery)
            || self.version != candidate.record_version()?
            || self.binding != candidate.binding_data()
        {
            return Err(refused());
        }
        SelectedHistoryNodePage::decode(&self.history)
            .and_then(|page| page.verify_exact_append_data(vault, candidate.history_data()))
            .map_err(|_| refused())?;
        Ok(self.binding)
    }

    #[cfg(test)]
    pub(in crate::store) fn relabel_as_eviction_for_test(
        &self,
        key: &[u8; 32],
        context: V2PendingJoinContextData,
        predecessor: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        seal_record(
            key,
            MEMBER_ADD_KEY,
            MEMBER_ADD_TAG,
            MEMBER_EVICTION,
            context_fields(context, predecessor),
            &[
                &self.binding.0.to_be_bytes(),
                &self.binding.1,
                &self.delivery,
                &self.history,
            ],
            rng,
        )
    }
}

pub(in crate::store) fn seal_member_add(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    predecessor: &[u8],
    delivery: &[u8],
    candidate: &JoinedMembershipCandidate,
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if predecessor.is_empty() || !candidate.matches_frame_data(delivery) {
        return Err(refused());
    }
    let (epoch, binding) = candidate.binding_data();
    let version = candidate.record_version()?;
    let history =
        catcoms_replication::chat::cold_selected_history::encode_selected_history_node_page(
            &context.vault,
            candidate.history_data(),
        )
        .map_err(|_| refused())?;
    seal_record(
        key,
        MEMBER_ADD_KEY,
        MEMBER_ADD_TAG,
        version,
        context_fields(context, predecessor),
        &[&epoch.to_be_bytes(), &binding, delivery, &history],
        rng,
    )
}

pub(in crate::store) fn open_member_add(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    predecessor: &[u8],
    raw: &[u8],
) -> Result<OpenedMemberAdd, AppError> {
    if predecessor.is_empty() {
        return Err(refused());
    }
    let plain = open_record(key, MEMBER_ADD_KEY, raw, MEMBER_ADD_FRAME)?;
    let (version, mut d) = decoder(&plain, MEMBER_ADD_TAG, context_fields(context, predecessor))?;
    let bad = |_| refused();
    let epoch = u64::from_be_bytes(
        d.get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| refused())?,
    );
    let binding = d
        .get_bytes()
        .map_err(bad)?
        .try_into()
        .map_err(|_| refused())?;
    let delivery = d.get_bytes().map_err(bad)?;
    let history = d.get_bytes().map_err(bad)?;
    d.finish().map_err(bad)?;
    let frame = V2MemberTransitionFrameData::decode(delivery)?;
    let version_matches = version == member_transition_version(frame.kind_data())?
        || (version == MEMBER_EVICTION && frame.kind_data() == HeaderKind::Removal);
    if !version_matches || frame.kind_data() != frame.header_data().spec().kind || epoch < 2 {
        return Err(refused());
    }
    SelectedHistoryNodePage::decode(history).map_err(|_| refused())?;
    Ok(OpenedMemberAdd {
        delivery: delivery.to_vec(),
        history: Zeroizing::new(history.to_vec()),
        binding: (epoch, binding),
        version,
    })
}

fn member_transition_version(kind: HeaderKind) -> Result<u16, AppError> {
    match kind {
        HeaderKind::Admission => Ok(VERSION),
        HeaderKind::Removal => Ok(MEMBER_REMOVAL),
        _ => Err(refused()),
    }
}

fn context_fields(context: V2PendingJoinContextData, parent: &[u8]) -> [[u8; 32]; 5] {
    [
        context.vault,
        context.root,
        context.original_pending,
        context.pin,
        *blake3::hash(parent).as_bytes(),
    ]
}

fn seal_record(
    key: &[u8; 32],
    domain: &str,
    tag: &[u8],
    version: u16,
    labels: [[u8; 32]; 5],
    fields: &[&[u8]],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let size =
        4 + tag.len() + 2 + DIGESTS + fields.iter().map(|field| 4 + field.len()).sum::<usize>();
    let mut e = Encoder::with_capacity(size);
    let bad = |_| refused();
    e.put_bytes(tag).map_err(bad)?;
    e.put_u16(version);
    for field in labels {
        e.put_bytes(&field).map_err(bad)?;
    }
    for field in fields {
        e.put_bytes(field).map_err(bad)?;
    }
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}

fn open_record(
    key: &[u8; 32],
    domain: &str,
    frame_bytes: &[u8],
    cap: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(SEALED_OVERHEAD + 1..=cap).contains(&frame_bytes.len()) {
        return Err(refused());
    }
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(unseal(&key, &unframe(frame_bytes)?)?))
}

fn decoder<'a>(
    raw: &'a [u8],
    tag: &[u8],
    labels: [[u8; 32]; 5],
) -> Result<(u16, Decoder<'a>), AppError> {
    let mut d = Decoder::new(raw);
    let bad = |_| refused();
    if d.get_bytes().map_err(bad)? != tag {
        return Err(refused());
    }
    let version = d.get_u16().map_err(bad)?;
    for label in labels {
        if d.get_bytes().map_err(bad)? != label {
            return Err(refused());
        }
    }
    Ok((version, d))
}

fn entry_version(delivery: &[u8]) -> Result<u16, AppError> {
    Ok(
        if V2JoinDeliveryData::later_frame_data(delivery).map_err(|_| refused())? {
            LATER_ENTRY
        } else {
            VERSION
        },
    )
}

pub(in crate::store) fn seal_entry(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    pending: &[u8],
    delivery: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    seal_record(
        key,
        ENTRY_KEY,
        ENTRY_TAG,
        entry_version(delivery)?,
        context_fields(context, pending),
        &[delivery],
        rng,
    )
}

pub(in crate::store) fn open_entry(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    pending: &[u8],
    raw: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let plain = open_record(key, ENTRY_KEY, raw, ENTRY_FRAME)?;
    let (version, mut d) = decoder(&plain, ENTRY_TAG, context_fields(context, pending))?;
    let bytes = d.get_bytes().map_err(|_| refused())?;
    d.finish().map_err(|_| refused())?;
    if entry_version(bytes)? != version {
        return Err(refused());
    }
    Ok(Zeroizing::new(bytes.to_vec()))
}

pub(in crate::store) fn seal_outcome(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    entry: &[u8],
    result: &SealedV2JoinedOutcome,
    history: &[u8],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let (nonce, bytes) = result.encrypted_parts();
    if bytes.len() > MAX_OUTCOME_CIPHERTEXT {
        return Err(refused());
    }
    SelectedHistoryNodePage::decode(history).map_err(|_| refused())?;
    seal_record(
        key,
        OUTCOME_KEY,
        OUTCOME_TAG,
        OUTCOME_WITH_HISTORY,
        context_fields(context, entry),
        &[nonce, bytes, history],
        rng,
    )
}

pub(in crate::store) struct OpenedOutcome {
    pub(in crate::store) sealed: SealedV2JoinedOutcome,
    pub(in crate::store) history: Option<Zeroizing<Vec<u8>>>,
}

pub(in crate::store) fn open_outcome(
    key: &[u8; 32],
    context: V2PendingJoinContextData,
    entry: &[u8],
    raw: &[u8],
) -> Result<OpenedOutcome, AppError> {
    let plain = open_record(key, OUTCOME_KEY, raw, OUTCOME_FRAME)?;
    let (version, mut d) = decoder(&plain, OUTCOME_TAG, context_fields(context, entry))?;
    let nonce: [u8; 24] = d
        .get_bytes()
        .map_err(|_| refused())?
        .try_into()
        .map_err(|_| refused())?;
    let bytes = d.get_bytes().map_err(|_| refused())?;
    if bytes.len() > MAX_OUTCOME_CIPHERTEXT {
        return Err(refused());
    }
    let history = match version {
        VERSION => None,
        OUTCOME_WITH_HISTORY => {
            let page = d.get_bytes().map_err(|_| refused())?;
            SelectedHistoryNodePage::decode(page).map_err(|_| refused())?;
            Some(page)
        }
        _ => return Err(refused()),
    };
    d.finish().map_err(|_| refused())?;
    Ok(OpenedOutcome {
        sealed: SealedV2JoinedOutcome::from_encrypted_parts(nonce, bytes).map_err(|_| refused())?,
        history: history.map(|page| Zeroizing::new(page.to_vec())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn joined_member_header_escrow_binds_the_original_scope_and_witness_slot() {
        use catcoms_replication::chat::{
            owner_facts::OwnerFactId,
            owner_header::{HeaderSpec, OwnerBodyId, OwnerStateCommitment, OwnerTenureId},
        };
        let mut rng = ChaCha20Rng::seed_from_u64(0x6295);
        let signer = catcoms_crypto::DeviceKeypair::from_seed(&[6; 32]);
        let header = SignedOwnerHeader::sign_using(
            HeaderSpec {
                group: vec![7; 16],
                profile: OwnerFactId::from_bytes([2; 32]),
                issuer: signer.verifying_key().to_bytes(),
                tenure: OwnerTenureId::from_bytes([8; 32]),
                parent: OwnerHeaderId::from_bytes([9; 32]),
                sequence: 2,
                kind: HeaderKind::Admission,
                body: OwnerBodyId::from_bytes([10; 32]),
                state: OwnerStateCommitment::from_bytes([11; 32]),
            },
            |bytes| Ok(signer.sign(bytes)),
        )
        .unwrap();
        let context = V2PendingJoinContextData {
            vault: [1; 32],
            root: [2; 32],
            original_pending: [3; 32],
            pin: [4; 32],
        };
        let key = [5; 32];
        let mut record = seal_member_header(
            &key,
            context,
            b"covered Welcome",
            (2, false),
            &header,
            &mut rng,
        )
        .unwrap();
        assert_eq!(
            open_member_header(&key, context, b"covered Welcome", (2, false), &record).unwrap(),
            header
        );
        for (other, predecessor, slot) in [
            (
                V2PendingJoinContextData {
                    original_pending: [12; 32],
                    ..context
                },
                b"covered Welcome".as_slice(),
                (2, false),
            ),
            (context, b"replacement", (2, false)),
            (context, b"covered Welcome", (2, true)),
            (context, b"covered Welcome", (3, false)),
        ] {
            assert!(open_member_header(&key, other, predecessor, slot, &record).is_err());
        }
        *record.last_mut().unwrap() ^= 1;
        assert!(
            open_member_header(&key, context, b"covered Welcome", (2, false), &record).is_err()
        );
    }

    #[test]
    fn governed_joined_pin_welcome_entry_binds_its_delivery_version_and_pending() {
        let mut rng = ChaCha20Rng::seed_from_u64(0x6294);
        let key = [9; 32];
        let context = V2PendingJoinContextData {
            vault: [1; 32],
            root: [2; 32],
            original_pending: [3; 32],
            pin: [4; 32],
        };
        let pending = b"original pending";
        // The outer record classifies the header before the separate signed
        // delivery validator admits any Welcome work.
        for (tag, version, wrong) in [
            (
                b"mewtual/chat/first-join-delivery/v2\0".as_slice(),
                VERSION,
                LATER_ENTRY,
            ),
            (
                b"mewtual/chat/governing-join-delivery/v2\0".as_slice(),
                LATER_ENTRY,
                VERSION,
            ),
        ] {
            let mut e = Encoder::new();
            e.put_bytes(tag).unwrap().put_u16(2);
            let delivery = e.finish();
            let entry = seal_entry(&key, context, pending, &delivery, &mut rng).unwrap();
            assert_eq!(entry_version(&delivery).unwrap(), version);
            assert_eq!(
                &*open_entry(&key, context, pending, &entry).unwrap(),
                &delivery
            );
            assert!(open_entry(&key, context, b"replacement pending", &entry).is_err());
            let relabelled = seal_record(
                &key,
                ENTRY_KEY,
                ENTRY_TAG,
                wrong,
                context_fields(context, pending),
                &[&delivery],
                &mut rng,
            )
            .unwrap();
            assert!(open_entry(&key, context, pending, &relabelled).is_err());
        }
    }

    #[test]
    fn governed_joined_pin_welcome_outcome_reads_legacy_and_requires_the_new_history_page() {
        let mut rng = ChaCha20Rng::seed_from_u64(0x6293);
        let key = [9; 32];
        let context = V2PendingJoinContextData {
            vault: [1; 32],
            root: [2; 32],
            original_pending: [3; 32],
            pin: [4; 32],
        };
        let entry = [5; 80];
        let labels = context_fields(context, &entry);
        let nonce = [6; 24];
        let graph = [7; 4096];
        let mut record = |version, fields: &[&[u8]]| {
            seal_record(
                &key,
                OUTCOME_KEY,
                OUTCOME_TAG,
                version,
                labels,
                fields,
                &mut rng,
            )
            .unwrap()
        };

        let legacy = record(VERSION, &[&nonce, &graph]);
        let opened = open_outcome(&key, context, &entry, &legacy).unwrap();
        assert!(opened.history.is_none());
        assert_eq!(opened.sealed.encrypted_parts(), (&nonce, graph.as_slice()));
        assert!(open_outcome(&key, context, &[8; 80], &legacy).is_err());
        for invalid in [
            record(OUTCOME_WITH_HISTORY, &[&nonce, &graph]),
            record(OUTCOME_WITH_HISTORY, &[&nonce, &graph, b"foreign page"]),
            record(VERSION, &[&nonce, &graph, b"trailing field"]),
            record(3, &[&nonce, &graph]),
        ] {
            assert!(open_outcome(&key, context, &entry, &invalid).is_err());
        }
    }
}
