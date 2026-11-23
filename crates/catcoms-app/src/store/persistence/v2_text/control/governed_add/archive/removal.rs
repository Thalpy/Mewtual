//! A named Remove archive on the same immutable encrypted Head chain.
use super::*;
use catcoms_mls::group::text_current_add::MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES;
use catcoms_mls::persist::text_current_add::TextCurrentAddLimits;
use catcoms_replication::chat::{
    cold_records::{encode_header_row, HeaderRow, MAX_HEADER_ROW_BYTES},
    owner_bodies::{RemovalBody, MAX_REMOVAL_BODY_BYTES},
    owner_header::SignedOwnerHeader,
};
use catcoms_sync::owner_v2::V2FounderRemoveOutcomeData;
use catcoms_sync::{V2MemberAddRequestData, V2MemberTransitionFrameData};
use selected_head::{GovernedHeadData, GovernedHeadKind};

const TAG: &[u8] = b"catcoms/private-chat-text-governed-remove-archive/v1\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-text-governed-remove-archive-key/v1";
const ID_DOMAIN: &str = "catcoms/private-chat-text-governed-remove-archive-id/v1";
const VERSION: u16 = 1;
const MAX_SNAPSHOT: usize = TextCurrentAddLimits::governed_text_v2().max_snapshot_bytes;
const FIXED_BYTES: usize = 4 + TAG.len() + 2 + 7 * 4 + selected_head::FRAME_LEN + 2 * 32 + 40;
const MAX_REMOVE_ARCHIVE: usize = FIXED_BYTES
    + MAX_HEADER_ROW_BYTES
    + MAX_REMOVAL_BODY_BYTES
    + MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES
    + MAX_SNAPSHOT;
const _: () = assert!(MAX_REMOVE_ARCHIVE <= MAX_BYTES);

pub(super) struct RemovalRecord<'a> {
    pub(super) parent_head: &'a [u8],
    pub(super) row: HeaderRow<'a>,
    pub(super) removal: RemovalBody,
    pub(super) commit: &'a [u8],
    pub(super) snapshot: &'a [u8],
    pub(super) fetch_verifier: [u8; 32],
    authenticator: [u8; 32],
}

pub(super) fn archive_id(raw: &[u8]) -> ArchiveHeadId {
    ArchiveHeadId::from_bytes(blake3::derive_key(ID_DOMAIN, raw))
}

pub(super) fn seal_data(
    key: &[u8; 32],
    parent_head: &[u8],
    fetch_verifier: [u8; 32],
    header: &SignedOwnerHeader,
    outcome: &V2FounderRemoveOutcomeData<'_>,
    rng: &mut dyn CryptoRngCore,
) -> Result<LaterArchive, AppError> {
    let removal = outcome.unsigned.body_data().encode();
    let row = encode_header_row(header, outcome.unsigned.image_data()).map_err(|_| refused())?;
    let commit = outcome.result.commit_data();
    let snapshot = outcome.result.secret_snapshot_data();
    let (epoch, authenticator) = outcome.result.selected_epoch_data();
    if header.spec() != outcome.unsigned.header_spec_data()
        || outcome.unsigned.body_data().spec().after_epoch != epoch
        || &outcome.unsigned.body_data().spec().subject != outcome.result.target_data()
    {
        return Err(refused());
    }
    seal_fields(
        key,
        [
            parent_head,
            &row,
            &removal,
            commit,
            snapshot,
            &fetch_verifier,
            &authenticator,
        ],
        rng,
    )
}

fn seal_fields(
    key: &[u8; 32],
    fields: [&[u8]; 7],
    rng: &mut dyn CryptoRngCore,
) -> Result<LaterArchive, AppError> {
    let caps = [
        selected_head::FRAME_LEN,
        MAX_HEADER_ROW_BYTES,
        MAX_REMOVAL_BODY_BYTES,
        MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES,
        MAX_SNAPSHOT,
        32,
        32,
    ];
    let size = fields.iter().zip(caps).try_fold(
        FIXED_BYTES - 40 - selected_head::FRAME_LEN - 2 * 32,
        |size, (field, cap)| {
            if field.is_empty() || field.len() > cap {
                return Err(refused());
            }
            size.checked_add(field.len()).ok_or_else(refused)
        },
    )?;
    let parent_head = fields[0];
    let parent = GovernedHeadData::open_governed_data(key, parent_head)?;
    let mut encoder = Encoder::with_capacity(size);
    put(&mut encoder, TAG)?;
    encoder.put_u16(VERSION);
    for value in fields {
        put(&mut encoder, value)?;
    }
    let plain = Zeroizing::new(encoder.finish());
    let record = RemovalRecord::decode(&plain)?;
    if record.row.header().spec().parent != parent.transition.header {
        return Err(refused());
    }
    let labels = transition(&parent, &record);
    let archive_key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let archive = Zeroizing::new(frame(&seal(&archive_key, &plain, rng)?));
    let head = GovernedHeadData::after_successor(
        key,
        parent_head,
        labels,
        archive_id(&archive),
        GovernedHeadKind::Remove,
    )?
    .seal_kind(key, rng, GovernedHeadKind::Remove)?;
    Ok(LaterArchive { archive, head })
}

#[cfg(test)]
mod tests;

pub(super) fn open_data(
    key: &[u8; 32],
    selected: &GovernedHeadData,
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if selected.kind != GovernedHeadKind::Remove
        || !(FIXED_BYTES..=MAX_REMOVE_ARCHIVE).contains(&ciphertext.len())
        || archive_id(ciphertext) != selected.archive
    {
        return Err(refused());
    }
    let archive_key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let plain = Zeroizing::new(unseal(&archive_key, &unframe(ciphertext)?)?);
    let record = RemovalRecord::decode(&plain)?;
    let original = GovernedHeadData::open_governed_data(key, record.parent_head)?;
    let transition = transition(&original, &record);
    let expected = GovernedHeadData::after_successor(
        key,
        record.parent_head,
        transition,
        selected.archive,
        GovernedHeadKind::Remove,
    )?;
    if expected != *selected || record.row.header().spec().parent != original.transition.header {
        return Err(refused());
    }
    Ok(plain)
}

impl<'a> RemovalRecord<'a> {
    pub(super) fn decode(plain: &'a [u8]) -> Result<Self, AppError> {
        let mut d = Decoder::new(plain);
        if field(&mut d, TAG.len())? != TAG || d.get_u16().map_err(|_| refused())? != VERSION {
            return Err(refused());
        }
        let parent_head = field(&mut d, selected_head::FRAME_LEN)?;
        let row = HeaderRow::decode(field(&mut d, MAX_HEADER_ROW_BYTES)?).map_err(|_| refused())?;
        let removal =
            RemovalBody::decode(field(&mut d, MAX_REMOVAL_BODY_BYTES)?).map_err(|_| refused())?;
        let commit = field(&mut d, MAX_TEXT_RECEIVED_ADD_COMMIT_BYTES)?;
        let snapshot = field(&mut d, MAX_SNAPSHOT)?;
        let fetch_verifier = b32(&mut d)?;
        let authenticator = b32(&mut d)?;
        d.finish().map_err(|_| refused())?;
        removal
            .verify_header_binding(row.header())
            .map_err(|_| refused())?;
        if parent_head.len() != selected_head::FRAME_LEN
            || snapshot.is_empty()
            || fetch_verifier == [0; 32]
            || authenticator == [0; 32]
            || commit.is_empty()
            || blake3::hash(commit).as_bytes() != &removal.spec().commit
        {
            return Err(refused());
        }
        Ok(Self {
            parent_head,
            row,
            removal,
            commit,
            snapshot,
            fetch_verifier,
            authenticator,
        })
    }

    pub(super) fn encode_member_transition(
        &self,
        request: &V2MemberAddRequestData,
    ) -> Result<Vec<u8>, AppError> {
        if request.parent_data()
            != (
                self.row.header().spec().parent,
                self.removal.spec().before_epoch,
            )
            || !request.matches_verifier_data(self.fetch_verifier)
        {
            return Err(refused());
        }
        Ok(V2MemberTransitionFrameData::encode_removal(
            self.row.header(),
            &self.removal,
            self.row.image(),
            self.commit,
        )?)
    }
}

fn transition(parent: &GovernedHeadData, record: &RemovalRecord<'_>) -> GovernedArchiveLabels {
    GovernedArchiveLabels {
        context: parent.transition.context,
        before_head: TextHeadIdData::of_data(record.parent_head),
        before_archive: parent.archive,
        header: record.row.header().id(),
        state: record.row.header().spec().state,
        epoch: record.removal.spec().after_epoch,
        authenticator: record.authenticator,
    }
}
