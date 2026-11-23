//! Authenticate the complete original transition outside writers. The
//! recovered snapshot still needs independent SDK restore and Head2 reproof.
use super::*;
use crate::store::critical_namespaces::archive_current_codec::GenesisPublicData;
use crate::store::persistence::v2_text::{
    archive::TextArchiveData,
    head::{TextHeadData, TextPendingData},
};
use catcoms_replication::chat::{
    owner_header::SignedOwnerHeader,
    owner_profile::{AdmissionBody, AdmissionOrigin},
    owner_state::transition::{ReplayBody, ReplayMath},
};
use catcoms_sync::V2JoinResponsePacketData;

mod cold;
mod held;
pub(in crate::store) use cold::{HeldColdFounderAdd, HeldColdFounderGenesis};
pub(in crate::store) use held::HeldGovernedArchiveRead;

pub(in crate::store) struct RecoveredGovernedAdd {
    pub(in crate::store) request: Zeroizing<Vec<u8>>,
    pub(in crate::store) response: Zeroizing<Vec<u8>>,
    pub(in crate::store) header: SignedOwnerHeader,
    pub(in crate::store) admission: AdmissionBody,
    pub(in crate::store) parent: TextHeadData,
    entry: [u8; ENTRY_BYTES],
    outcome: Zeroizing<Vec<u8>>,
}

struct RecoveredAddJournal {
    request: Zeroizing<Vec<u8>>,
    response: Zeroizing<Vec<u8>>,
    header: SignedOwnerHeader,
    admission: AdmissionBody,
    entry: [u8; ENTRY_BYTES],
    outcome: Zeroizing<Vec<u8>>,
}

impl RecoveredGovernedAdd {
    pub(in crate::store) fn snapshot_data(&self) -> Result<&[u8], AppError> {
        outcome::archive_snapshot(&self.entry, &self.outcome)
    }

    pub(in crate::store) fn image_data(&self) -> Result<&[u8], AppError> {
        Ok(outcome::governing_bodies(&self.entry, &self.outcome)?[2])
    }
}

fn authenticate_selected(
    key: &[u8; 32],
    head: &selected_head::GovernedHeadData,
    raw_archive: &[u8],
    context: TextContextIdData,
    trusted: &GenesisPublicData<'_>,
) -> Result<RecoveredGovernedAdd, AppError> {
    let archive = open_data(key, head.archive, raw_archive)?;
    let (labels, frames) = decode_data(&archive)?;
    if labels != head.transition || labels.context != context {
        return Err(refused());
    }
    let parent = parent_head(key, head, &frames, trusted)?;
    let prior = ReplayMath::from_genesis(
        trusted.profile,
        trusted.admission,
        trusted.header,
        trusted.image,
    )
    .map_err(|_| refused())?;
    let journal = authenticate_add_journal(key, labels, &frames, trusted, &prior)?;
    Ok(RecoveredGovernedAdd {
        request: journal.request,
        response: journal.response,
        header: journal.header,
        admission: journal.admission,
        parent,
        entry: journal.entry,
        outcome: journal.outcome,
    })
}

/// Verify a complete Add journal against the replay of its authenticated
/// parent. The caller owns the separate proof of physical head selection.
fn authenticate_add_journal(
    key: &[u8; 32],
    labels: GovernedArchiveLabels,
    frames: &GovernedArchiveFrames<'_>,
    trusted: &GenesisPublicData<'_>,
    prior: &ReplayMath,
) -> Result<RecoveredAddJournal, AppError> {
    let reservation = GovernedAddFenceData::open_data(key, frames.fence)?;
    reservation.require_parent(labels.context, labels.before_head)?;
    let request = request::open_data(key, frames.fence, frames.request)?;
    let packet = V2JoinRequestPacketData::decode(&request)?;
    reservation.require_packet(&packet)?;
    let input = packet.founder_add_data();
    let anchor = &input.invite.spec_data().anchor;
    let original = trusted.header.spec();
    if anchor.group != original.group
        || anchor.founder != original.issuer
        || anchor.profile != *trusted.profile.id().as_bytes()
        || anchor.tenure != *trusted.profile.tenure().as_bytes()
        || anchor.header != *trusted.header.id().as_bytes()
        || anchor.state != *original.state.as_bytes()
        || anchor.admission != *trusted.admission.admission_id().as_bytes()
        || prior.profile().id() != trusted.profile.id()
    {
        return Err(refused());
    }
    let entry = sdk_entry::GovernedAddSdkEntryData::from_fence_data(frames.fence)?;
    entry.require_fence_data(key, frames.entry)?;
    let outcome = outcome::open_outcome_data(key, frames.entry, frames.outcome)?;
    let bodies = outcome::governing_bodies(frames.entry, &outcome)?;
    let admission = AdmissionBody::decode(bodies[0]).map_err(|_| refused())?;
    let AdmissionOrigin::Add {
        before_epoch,
        after_epoch,
        ..
    } = admission.spec().origin
    else {
        return Err(refused());
    };
    if before_epoch != prior.mls_epoch_data()
        || before_epoch.checked_add(1) != Some(after_epoch)
        || after_epoch != labels.epoch
    {
        return Err(refused());
    }
    let admitted_at = outcome::verify_archive_fingerprint(
        frames.entry,
        &outcome,
        before_epoch,
        after_epoch,
        &labels.authenticator,
    )?;
    if input.invite.expired_at_data(admitted_at) {
        return Err(refused());
    }
    let raw_header = header_signing::AddSigningParent::from_outcome(frames.outcome)?.open_raw(
        key,
        frames.header_attempt,
        frames.raw_header,
    )?;
    let header = governing_record::open_data(
        key,
        frames.raw_header,
        frames.governing,
        &raw_header,
        bodies,
    )?;
    if header.id() != labels.header || header.spec().state != labels.state {
        return Err(refused());
    }
    prior
        .derive_successor(&header, ReplayBody::Admission(&admission), bodies[2])
        .map_err(|_| refused())?;
    let spec = welcome_intent::open_data(key, frames.governing, frames.welcome_intent)?;
    let payloads = outcome::welcome_bodies(frames.entry, &outcome)?;
    welcome_intent::require_original_binding(
        &spec,
        &packet,
        original.issuer,
        header.id(),
        admission.admission_id(),
        payloads,
    )?;
    let raw_welcome = header_signing::AddSigningParent::from_welcome_intent(frames.welcome_intent)?
        .open_raw(key, frames.welcome_attempt, frames.raw_welcome)?;
    let preimage = spec.signing_bytes_data().map_err(|_| refused())?;
    let response = response::open_data(
        key,
        frames.raw_welcome,
        frames.response,
        response::OriginalResponse {
            preimage: &preimage,
            signature: &raw_welcome,
            payloads,
        },
    )?;
    let decoded = V2JoinResponsePacketData::decode(&response)?;
    decoded
        .response_data()
        .binding
        .check_links_data(
            input.invite,
            input.request,
            trusted.profile,
            &admission,
            &header,
        )
        .map_err(|_| refused())?;
    let entry = frames.entry.try_into().map_err(|_| refused())?;
    Ok(RecoveredAddJournal {
        request,
        response,
        header,
        admission,
        entry,
        outcome,
    })
}

fn parent_head(
    key: &[u8; 32],
    head: &selected_head::GovernedHeadData,
    frames: &GovernedArchiveFrames<'_>,
    trusted: &GenesisPublicData<'_>,
) -> Result<TextHeadData, AppError> {
    let labels = head.transition;
    let parent_kind = TextControlKindData::Head;
    let archive_kind = TextControlKindData::Archive;
    let archive_id = archive_kind.id_data(frames.parent_archive);
    if TextHeadIdData::of_data(frames.parent_head) != labels.before_head
        || archive_id.as_bytes() != labels.before_archive.as_bytes()
    {
        return Err(refused());
    }
    let parent = TextHeadData::decode_data(&open_control_data(
        key,
        parent_kind.id_data(frames.parent_head),
        frames.parent_head,
    )?)?;
    let archive =
        TextArchiveData::decode_data(&open_control_data(key, archive_id, frames.parent_archive)?)?;
    let prior = archive.spec_data();
    if parent.generation != 1
        || Some(head.generation) != parent.generation.checked_add(1)
        || parent.pending != TextPendingData::None
        || parent.context.id_data()? != labels.context
        || parent.archive != labels.before_archive
        || prior.context != labels.context
        || prior.header != trusted.header
        || prior.image != trusted.image
    {
        return Err(refused());
    }
    Ok(parent)
}
