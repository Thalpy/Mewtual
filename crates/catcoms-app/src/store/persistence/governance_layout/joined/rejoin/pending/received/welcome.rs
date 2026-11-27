//! Admit one MLS Welcome for the exact covered A2 delivery, then keep the
//! closed result in a distinct Outcome. Neither file installs a member row.
use super::*;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::RequestCancellation;
use catcoms_sync::{RecoveredV2JoinedOutcome, SealedV2JoinedOutcome, V2JoinDeliveryData};

const SDK_ATTEMPT: &str = "chat-critical-namespaces.rejoin-welcome-attempt.bin";
const OUTCOME: &str = "chat-critical-namespaces.rejoin-outcome.bin";
const ATTEMPT_TAG: &str = "catcoms/chat-v2/rejoin-welcome-attempt/v1";
const OUTCOME_TAG: &str = "catcoms/chat-v2/rejoin-outcome/v1";
const ATTEMPT_MAX: usize = 40 + 2 + 5 * 36;
// Include the complete closed provider snapshot, its inner framing, and the
// exact possible-entry hash in the independently covered outer record.
const OUTCOME_MAX: usize = 40 + 2 + 3 * 36 + 8 + 28 + 4 + 512 * 1024 + 64 * 1024;
const OUTCOME_WORK: usize = 6 * OUTCOME_MAX + 4 * MAX_FRAME + 4 * 1024 * 1024;

pub(super) fn require_unstarted(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, SDK_ATTEMPT)?)?;
    absent(&fixed(store, false, OUTCOME)?)
}

/// Historical coordinates of the exact covered closed SDK result.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NativeTextRejoinOutcomeReceipt {
    pub request_digest: [u8; 32],
    pub header: OwnerHeaderId,
    pub epoch: u64,
}
catcoms_rt::redacted_debug! { NativeTextRejoinOutcomeReceipt; }

/// Status of the original replacement Welcome journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextRejoinWelcomeStatus {
    WaitingForTransport,
    Held,
    Covered(NativeTextRejoinOutcomeReceipt),
}

pub(in crate::store) struct RejoinWelcomeKeyEntry<'a> {
    pub(in crate::store) source: &'a NativeChatSourceObservation,
    pub(in crate::store) pending: &'a mut NativeChatPreparedKeyCut,
    pub(in crate::store) welcome: &'a mut NativeChatPreparedKeyCut,
    pub(in crate::store) ends: &'a mut NativeChatKeyEnds,
}

pub(in crate::store) type RejoinWelcomeOwners<'a> = (
    &'a RejoinPendingOwner,
    &'a RejoinReceivedOwner,
    &'a mut Option<Box<RejoinWelcomeOwner>>,
);

struct JournalFrame {
    frame: Zeroizing<Vec<u8>>,
    held: Held,
}

impl JournalFrame {
    fn selected((file, frame): OpenedArchiveObject) -> Self {
        Self {
            held: Held::Selected(file),
            frame,
        }
    }

    fn staged(frame: Zeroizing<Vec<u8>>) -> Self {
        Self {
            held: Held::Stage(None),
            frame,
        }
    }

    fn require(&self, path: &Path) -> Result<(), AppError> {
        match &self.held {
            Held::Stage(Some(file)) => exact_retained(path, file, &self.frame),
            Held::Selected(file) => {
                exact_held(path, file, session_id(file).map_err(io)?, &self.frame)
            }
            Held::Stage(None) => Err(denied()),
        }
    }

    fn cover(&mut self, path: &Path) -> Result<(), AppError> {
        match &mut self.held {
            Held::Stage(file) => cover_new(path, &self.frame, file),
            Held::Selected(file) => cover_existing(path, file, &self.frame),
        }
    }
}

struct WarmGraph {
    request: Box<PreparedV2JoinRequest>,
    entered: bool,
    receipt: Option<NativeTextRejoinOutcomeReceipt>,
}

/// Warm partial work retains the original private graph. A cold Attempt
/// never creates a warm graph or re-enters the Welcome SDK.
pub(in crate::store) struct RejoinWelcomeOwner {
    warm: Option<WarmGraph>,
    entry: JournalFrame,
    outcome: Option<JournalFrame>,
    receipt: Option<NativeTextRejoinOutcomeReceipt>,
    pending_cookie: std::sync::Arc<()>,
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    fee: CatalogReadReservation,
}

fn entry_plain(
    binding: &Binding,
    (request, delivery): ([u8; 32], [u8; 32]),
    (pending, received): (&[u8], &[u8]),
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut out = Encoder::with_capacity(ATTEMPT_MAX - 40);
    out.put_u16(VERSION);
    for hash in [
        binding.context.original_pending,
        request,
        delivery,
        *blake3::hash(pending).as_bytes(),
        *blake3::hash(received).as_bytes(),
    ] {
        out.put_bytes(&hash).map_err(|_| denied())?;
    }
    Ok(Zeroizing::new(out.finish()))
}

fn seal_outcome(
    key: &[u8; 32],
    attempt: &[u8],
    receipt: NativeTextRejoinOutcomeReceipt,
    snapshot: &SealedV2JoinedOutcome,
    rng: &mut impl CryptoRngCore,
) -> Result<JournalFrame, AppError> {
    let (nonce, bytes) = snapshot.encrypted_parts();
    let mut out = Encoder::with_capacity(OUTCOME_MAX - 40);
    out.put_u16(VERSION);
    for hash in [
        *blake3::hash(attempt).as_bytes(),
        receipt.request_digest,
        *receipt.header.as_bytes(),
    ] {
        out.put_bytes(&hash).map_err(|_| denied())?;
    }
    out.put_u64(receipt.epoch);
    out.put_bytes(nonce).map_err(|_| denied())?;
    out.put_bytes(bytes).map_err(|_| denied())?;
    let plain = Zeroizing::new(out.finish());
    if plain.len() > OUTCOME_MAX - 40 {
        return Err(denied());
    }
    Ok(JournalFrame::staged(seal_frame(
        key,
        OUTCOME_TAG,
        &plain,
        rng,
    )?))
}

fn open_outcome(
    key: &[u8; 32],
    attempt: &[u8],
    frame: &[u8],
    original: &OpenedPending,
) -> Result<(SealedV2JoinedOutcome, NativeTextRejoinOutcomeReceipt), AppError> {
    let plain = open_frame(key, OUTCOME_TAG, frame)?;
    let mut decoder = Decoder::new(&plain);
    let malformed = |_| denied();
    if decoder.get_u16().map_err(malformed)? != VERSION
        || decoder.get_bytes().map_err(malformed)? != blake3::hash(attempt).as_bytes()
        || decoder.get_bytes().map_err(malformed)? != original.receipt.request_digest
    {
        return Err(denied());
    }
    let header = OwnerHeaderId::from_bytes(
        decoder
            .get_bytes()
            .map_err(malformed)?
            .try_into()
            .map_err(|_| denied())?,
    );
    let epoch = decoder.get_u64().map_err(malformed)?;
    let nonce = decoder
        .get_bytes()
        .map_err(malformed)?
        .try_into()
        .map_err(|_| denied())?;
    let ciphertext = decoder.get_bytes().map_err(malformed)?;
    decoder.finish().map_err(malformed)?;
    let snapshot =
        SealedV2JoinedOutcome::from_encrypted_parts(nonce, ciphertext).map_err(|_| denied())?;
    Ok((
        snapshot,
        NativeTextRejoinOutcomeReceipt {
            request_digest: original.receipt.request_digest,
            header,
            epoch,
        },
    ))
}

impl ServerStore {
    pub(in crate::store) fn cover_rejoin_welcome(
        &self,
        inventory: &UpgradeInventory,
        (registration, removal): (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        (pending, received, slot): RejoinWelcomeOwners<'_>,
        (invite, token): (&NativeTextInvitePacketData<'_>, [u8; 32]),
        (mut native, caller): (Option<RejoinWelcomeKeyEntry<'_>>, &RequestCancellation),
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<NativeTextRejoinWelcomeStatus, AppError> {
        require_current()?;
        let catalog = self.cover_selected_rejoin_catalog(registration, removal, inventory)?;
        let pool = &self
            .governance_layout_policy
            .as_ref()
            .ok_or_else(denied)?
            .current
            .archive
            .pending
            .pool;
        pending.invoice().validate(pool, WORK_BYTES)?;
        received._fee.validate(pool, WORK)?;
        if !pending.matches_cookie(&received.pending_cookie) {
            return Err(denied());
        }
        let binding = Binding::new(catalog, registration, removal, invite, token)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let attempt_path = fixed(self, false, ATTEMPT)?;
        let pending_path = fixed(self, false, PENDING)?;
        let received_path = fixed(self, false, RECEIVED)?;
        let paths = [attempt_path.as_path(), pending_path.as_path()];
        let files = pending.files(&binding)?;
        let limits = pending_limits()?;
        let original = self.open_covered_rejoin_files(
            inventory,
            &binding,
            (&key, invite, limits),
            paths,
            files,
        )?;
        received.require(&received_path)?;
        let plaintext = open_frame(&key, PURPOSE, &received.frame)?;
        let delivery = delivery_bytes(&binding, &original, &plaintext)?;
        original
            .recovered
            .validate_pinned_join_delivery_data(delivery)
            .map_err(|_| denied())?;
        let expected = entry_plain(
            &binding,
            (
                original.receipt.request_digest,
                *blake3::hash(delivery).as_bytes(),
            ),
            (files[1].1, &received.frame),
        )?;
        let require = || {
            require_current()?;
            require_held_rejoin_files(paths, files)?;
            self.require_recovered_joined_removal(registration, removal)?;
            absent(&fixed(self, false, REJOIN_STAGE)?)?;
            inventory.require(self)?;
            received.require(&received_path)?;
            require_current()
        };
        let entry_path = fixed(self, false, SDK_ATTEMPT)?;
        let outcome_path = fixed(self, false, OUTCOME)?;
        if slot.is_none() {
            let fee = pool.try_reserve(OUTCOME_WORK)?;
            let (entry, outcome, warm) = if let Some(entry) = read(&entry_path, ATTEMPT_MAX)? {
                if open_frame(&key, ATTEMPT_TAG, &entry.1)? != expected {
                    return Err(denied());
                }
                let outcome = read(&outcome_path, OUTCOME_MAX)?.map(JournalFrame::selected);
                (JournalFrame::selected(entry), outcome, None)
            } else {
                absent(&outcome_path)?;
                let native = native.as_mut().ok_or_else(denied)?;
                require()?;
                let graph_fee = super::super::super::super::pending::reserve_sdk_graph(self)?;
                let mut request = original
                    .sealed
                    .restore_native_pre_welcome(
                        &key,
                        binding.context,
                        invite.binding(),
                        limits,
                        NativeV2JoinKeyEntry::new(native.source, native.pending, native.ends),
                    )
                    .map_err(|_| denied())?;
                request
                    .fund_warm_sdk_graph(graph_fee)
                    .map_err(|_| denied())?;
                let entry = JournalFrame::staged(seal_frame(&key, ATTEMPT_TAG, &expected, rng)?);
                (
                    entry,
                    None,
                    Some(WarmGraph {
                        request: Box::new(request),
                        entered: false,
                        receipt: None,
                    }),
                )
            };
            require()?;
            *slot = Some(Box::new(RejoinWelcomeOwner {
                warm,
                entry,
                outcome,
                receipt: None,
                pending_cookie: pending.cookie().clone(),
                _lease: self._session.retain_for_physical_work(),
                fee,
            }));
        }
        let original_welcome = slot.as_mut().ok_or_else(denied)?;
        original_welcome.fee.validate(pool, OUTCOME_WORK)?;
        if !pending.matches_cookie(&original_welcome.pending_cookie)
            || open_frame(&key, ATTEMPT_TAG, &original_welcome.entry.frame)? != expected
        {
            return Err(denied());
        }
        require()?;
        original_welcome.entry.cover(&entry_path)?;
        inventory.sync_rejoin_catalog_parent(self)?;
        original_welcome.entry.require(&entry_path)?;
        require()?;
        if original_welcome.outcome.is_none() {
            absent(&outcome_path)?;
            let Some(warm) = original_welcome.warm.as_mut() else {
                original_welcome.entry.require(&entry_path)?;
                require_current()?;
                return Ok(NativeTextRejoinWelcomeStatus::Held);
            };
            let Some(native) = native.as_mut() else {
                original_welcome.entry.require(&entry_path)?;
                require_current()?;
                return Ok(NativeTextRejoinWelcomeStatus::Held);
            };
            if warm.receipt.is_none() {
                if warm.entered {
                    original_welcome.entry.require(&entry_path)?;
                    require_current()?;
                    return Ok(NativeTextRejoinWelcomeStatus::Held);
                }
                let delivery_data = V2JoinDeliveryData::decode(delivery).map_err(|_| denied())?;
                let ((_, header), _) = delivery_data.proof_data().map_err(|_| denied())?;
                require()?;
                original_welcome.entry.require(&entry_path)?;
                require_current()?;
                warm.entered = true;
                let completion = warm
                    .request
                    .finish_pinned_join_delivery(
                        delivery,
                        Some(caller),
                        Some(NativeV2JoinKeyEntry::new(
                            native.source,
                            native.welcome,
                            native.ends,
                        )),
                    )
                    .map(|candidate| candidate.facts_data().join_epoch_data());
                match completion {
                    Ok(epoch) => {
                        warm.receipt = Some(NativeTextRejoinOutcomeReceipt {
                            request_digest: original.receipt.request_digest,
                            header,
                            epoch,
                        })
                    }
                    Err(_) => {
                        // Only normal return can prove a pre-entry refusal.
                        // After an unwind the sticky original remains held.
                        warm.entered = warm.request.welcome_entered_data();
                        return Err(denied());
                    }
                }
            }
            let receipt = warm.receipt.ok_or_else(denied)?;
            require()?;
            original_welcome.entry.require(&entry_path)?;
            absent(&outcome_path)?;
            let snapshot = warm
                .request
                .seal_joined_outcome(&key, binding.context, rng)
                .map_err(|_| denied())?;
            original_welcome.outcome = Some(seal_outcome(
                &key,
                &original_welcome.entry.frame,
                receipt,
                snapshot,
                rng,
            )?);
            original_welcome.receipt = Some(receipt);
        }
        let outcome = original_welcome.outcome.as_mut().ok_or_else(denied)?;
        let (snapshot, receipt) = open_outcome(
            &key,
            &original_welcome.entry.frame,
            &outcome.frame,
            &original,
        )?;
        if let Some(expected) = original_welcome.receipt {
            if expected != receipt {
                return Err(denied());
            }
        } else {
            let graph_fee = super::super::super::super::pending::reserve_sdk_graph(self)?;
            let graph = RecoveredV2JoinedOutcome::open(
                &snapshot,
                &key,
                &original.recovered,
                delivery,
                graph_fee,
            )
            .map_err(|_| denied())?;
            if (graph.governing_head_data(), graph.epoch_data()) != (receipt.header, receipt.epoch)
            {
                return Err(denied());
            }
            original_welcome.receipt = Some(receipt);
        }
        require()?;
        original_welcome.entry.require(&entry_path)?;
        outcome.cover(&outcome_path)?;
        inventory.sync_rejoin_catalog_parent(self)?;
        require()?;
        original_welcome.entry.require(&entry_path)?;
        outcome.require(&outcome_path)?;
        require_current()?;
        Ok(NativeTextRejoinWelcomeStatus::Covered(receipt))
    }
}
