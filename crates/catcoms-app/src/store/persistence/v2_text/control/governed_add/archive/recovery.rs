//! Authenticate the complete original transition outside writers. The
//! recovered snapshot still needs independent SDK restore and Head2 reproof.
use super::*;
use crate::store::critical_namespaces::{
    archive_current_codec::GenesisPublicData, read_pool::CatalogReadReservation,
};
use crate::store::persistence::v2_text::{
    archive::TextArchiveData,
    head::{TextHeadData, TextPendingData},
};
use crate::store::ServerStore;
use catcoms_mls::persist::{
    text_current_add::TextCurrentAddLimits, TextCurrentSdkGraphReservation,
    TextCurrentSdkRestoreAdmission, TextCurrentSdkRestoreLimits,
};
use catcoms_replication::chat::{
    owner_header::SignedOwnerHeader,
    owner_profile::{AdmissionBody, AdmissionOrigin},
    owner_state::transition::{ReplayBody, ReplayMath},
};
use catcoms_sync::{
    V2AdmissionStepData, V2JoinResponsePacketData, V2MemberAddFrameData, V2MemberAddRequestData,
};

mod chain;
mod cold;
#[cfg(all(test, any(windows, target_os = "linux")))]
mod held;
pub(in crate::store) use cold::{HeldColdFounderAdd, HeldColdFounderGenesis};
#[cfg(all(test, any(windows, target_os = "linux")))]
pub(in crate::store) use held::HeldGovernedArchiveRead;

struct RestoredSdkPayment {
    limits: TextCurrentSdkRestoreLimits,
    _fee: CatalogReadReservation,
}

impl TextCurrentSdkGraphReservation for RestoredSdkPayment {
    fn limits(&self) -> TextCurrentSdkRestoreLimits {
        self.limits
    }
}

fn restore_admission(store: &ServerStore) -> Result<TextCurrentSdkRestoreAdmission, AppError> {
    let limits = TextCurrentAddLimits::governed_text_v2();
    let max_typed_bytes = limits.restore_typed_bytes + 128 * 1024;
    let bytes = max_typed_bytes
        + limits.max_provider_backing_bytes
        + limits.max_provider_payload_bytes
        + 2 * limits.max_snapshot_bytes;
    let pool = &store
        .archive_bootstrap_policy
        .as_ref()
        .ok_or_else(refused)?
        .pending
        .pool;
    let fee = pool.try_reserve(bytes)?;
    fee.validate(pool, bytes)?;
    Ok(TextCurrentSdkRestoreAdmission::new(RestoredSdkPayment {
        limits: TextCurrentSdkRestoreLimits {
            max_typed_bytes,
            max_work: limits.restore_work,
        },
        _fee: fee,
    })?)
}

pub(in crate::store) struct RecoveredGovernedAdd {
    pub(in crate::store) request: Zeroizing<Vec<u8>>,
    pub(in crate::store) response: Zeroizing<Vec<u8>>,
    pub(in crate::store) header: SignedOwnerHeader,
    pub(in crate::store) admission: AdmissionBody,
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) parent: TextHeadData,
    consumed_nonce: [u8; 16],
    entry: [u8; ENTRY_BYTES],
    outcome: Zeroizing<Vec<u8>>,
}

struct RecoveredAddJournal {
    request: Zeroizing<Vec<u8>>,
    response: Zeroizing<Vec<u8>>,
    header: SignedOwnerHeader,
    admission: AdmissionBody,
    nonce: [u8; 16],
    entry: [u8; ENTRY_BYTES],
    outcome: Zeroizing<Vec<u8>>,
    fetch_verifier: Option<[u8; 32]>,
}

impl RecoveredAddJournal {
    fn encode_member_add(&self, request: &V2MemberAddRequestData) -> Result<Vec<u8>, AppError> {
        let AdmissionOrigin::Add { before_epoch, .. } = self.admission.spec().origin else {
            return Err(refused());
        };
        if request.parent_data() != (self.header.spec().parent, before_epoch)
            || !self
                .fetch_verifier
                .is_some_and(|expected| request.matches_verifier_data(expected))
        {
            return Err(refused());
        }
        let image = outcome::governing_bodies(&self.entry, &self.outcome)?[2];
        let commit = outcome::archive_commit(&self.entry, &self.outcome)?;
        Ok(V2MemberAddFrameData::encode(
            V2AdmissionStepData {
                header: &self.header,
                admission: &self.admission,
                image,
            },
            commit,
        )?)
    }
}

/// Replay of one physical archive line. Each nonce is retained only after its
/// original signed request and complete governing journal have verified.
struct AddReplay {
    core: ReplayMath,
    nonces: Vec<[u8; 16]>,
    selected: Option<(TextHeadIdData, ArchiveHeadId)>,
}

impl AddReplay {
    fn from_genesis(trusted: &GenesisPublicData<'_>) -> Result<Self, AppError> {
        let core = ReplayMath::from_genesis(
            trusted.profile,
            trusted.admission,
            trusted.header,
            trusted.image,
        )
        .map_err(|_| refused())?;
        Ok(Self {
            core,
            nonces: Vec::new(),
            selected: None,
        })
    }

    fn verify_add(
        &mut self,
        key: &[u8; 32],
        labels: GovernedArchiveLabels,
        frames: &GovernedArchiveFrames<'_>,
        trusted: &GenesisPublicData<'_>,
    ) -> Result<RecoveredAddJournal, AppError> {
        if self.nonces.len() >= catcoms_sync::MAX_V2_GOVERNING_JOIN_STEPS {
            return Err(refused());
        }
        let (journal, next) = authenticate_add_journal(key, labels, frames, trusted, &self.core)?;
        if self.nonces.contains(&journal.nonce) {
            return Err(refused());
        }
        self.nonces.push(journal.nonce);
        self.core = next;
        Ok(journal)
    }

    fn verify_later_add(
        &mut self,
        key: &[u8; 32],
        raw_head: &[u8],
        raw: &[u8],
        trusted: &GenesisPublicData<'_>,
    ) -> Result<RecoveredAddJournal, AppError> {
        let head = selected_head::GovernedHeadData::open_governed_data(key, raw_head)?;
        if self.selected != Some((head.transition.before_head, head.transition.before_archive)) {
            return Err(refused());
        }
        let archive = open_later_data(key, &head, raw)?;
        let (labels, frames) = decode_later_data(&archive)?;
        let original =
            selected_head::GovernedHeadData::open_governed_data(key, frames.parent_head)?;
        let prior = self.core.header();
        if original.transition.header != prior.id()
            || original.transition.state != prior.spec().state
            || original.transition.epoch != self.core.mls_epoch_data()
        {
            return Err(refused());
        }
        let journal = self.verify_add(key, labels, &frames, trusted)?;
        self.selected = Some((TextHeadIdData::of_data(raw_head), head.archive));
        Ok(journal)
    }
}

impl RecoveredGovernedAdd {
    pub(in crate::store) fn snapshot_data(&self) -> Result<&[u8], AppError> {
        outcome::archive_snapshot(&self.entry, &self.outcome)
    }

    pub(in crate::store) fn image_data(&self) -> Result<&[u8], AppError> {
        Ok(outcome::governing_bodies(&self.entry, &self.outcome)?[2])
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn consumed_invite(&self, packet: &V2JoinRequestPacketData<'_>) -> bool {
        packet.founder_add_data().invite.spec_data().nonce == self.consumed_nonce
    }
}

fn authenticate_selected(
    key: &[u8; 32],
    raw_head: &[u8],
    raw_archive: &[u8],
    context: TextContextIdData,
    trusted: &GenesisPublicData<'_>,
) -> Result<RecoveredGovernedAdd, AppError> {
    authenticate_first_archive(key, raw_head, raw_archive, context, trusted).map(|(first, _)| first)
}

fn authenticate_first_archive(
    key: &[u8; 32],
    raw_head: &[u8],
    raw_archive: &[u8],
    context: TextContextIdData,
    trusted: &GenesisPublicData<'_>,
) -> Result<(RecoveredGovernedAdd, AddReplay), AppError> {
    let head = selected_head::GovernedHeadData::open_data(key, raw_head)?;
    let archive = open_data(key, head.archive, raw_archive)?;
    let (labels, frames) = decode_data(&archive)?;
    if labels != head.transition || labels.context != context {
        return Err(refused());
    }
    let _parent = parent_head(key, &head, &frames, trusted)?;
    let mut replay = AddReplay::from_genesis(trusted)?;
    let journal = replay.verify_add(key, labels, &frames, trusted)?;
    replay.selected = Some((TextHeadIdData::of_data(raw_head), head.archive));
    Ok((
        RecoveredGovernedAdd {
            request: journal.request,
            response: journal.response,
            header: journal.header,
            admission: journal.admission,
            #[cfg(all(test, any(windows, target_os = "linux")))]
            parent: _parent,
            consumed_nonce: journal.nonce,
            entry: journal.entry,
            outcome: journal.outcome,
        },
        replay,
    ))
}

/// Verify a complete Add journal against the replay of its authenticated
/// parent. The caller owns the separate proof of physical head selection.
fn authenticate_add_journal(
    key: &[u8; 32],
    labels: GovernedArchiveLabels,
    frames: &GovernedArchiveFrames<'_>,
    trusted: &GenesisPublicData<'_>,
    prior: &ReplayMath,
) -> Result<(RecoveredAddJournal, ReplayMath), AppError> {
    let reservation = GovernedAddFenceData::open_data(key, frames.fence)?;
    reservation.require_parent(labels.context, labels.before_head)?;
    let request::OpenedRequest {
        packet: request,
        fetch_verifier,
    } = request::open_record(key, frames.fence, frames.request)?;
    let packet = V2JoinRequestPacketData::decode(&request)?;
    reservation.require_packet(&packet)?;
    let input = packet.founder_add_data();
    let nonce = input.invite.spec_data().nonce;
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
        commit,
        ..
    } = admission.spec().origin
    else {
        return Err(refused());
    };
    if before_epoch != prior.mls_epoch_data()
        || before_epoch.checked_add(1) != Some(after_epoch)
        || after_epoch != labels.epoch
        || (before_epoch == 0 && fetch_verifier.is_some())
        || blake3::hash(outcome::archive_commit(frames.entry, &outcome)?).as_bytes() != &commit
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
    let next = prior
        .derive_successor(&header, ReplayBody::Admission(&admission), bodies[2])
        .map_err(|_| refused())?
        .next;
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
    Ok((
        RecoveredAddJournal {
            request,
            response,
            header,
            admission,
            nonce,
            entry,
            outcome,
            fetch_verifier,
        },
        next,
    ))
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
