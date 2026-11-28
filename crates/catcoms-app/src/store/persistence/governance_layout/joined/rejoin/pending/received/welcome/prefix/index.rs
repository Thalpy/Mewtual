//! Cover one authenticated rejoin Index4. Index3 remains the immutable
//! predecessor used to verify the old registration and terminal A1 proof.
use super::*;
use crate::store::critical_namespaces::native_registry::PreparedNativeRejoinIndex;
use crate::store::NativeTextJoinedRegistryRowData;

mod installed;
use crate::store::persistence::governance_layout::joined::rejoin::pending::replay_catalog::InstalledCatalogWitness;
pub(in crate::store) use installed::InstalledRejoin;

const INDEX: &str = "chat-critical-namespaces.rejoin-native-index.bin";
const INDEX_TAG: &str = "catcoms/chat-v2/rejoin-native-index/v1";
const INDEX_FRAME: usize = 40 + 2 + 3 * 36 + 4 + MAX_INDEX_FRAME;
const INDEX_WORK: usize = 5 * INDEX_FRAME + 4 * MAX_INDEX_FRAME + 128 * 1024;

pub(super) fn require_unstarted(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, INDEX)?)
}

/// Covered replacement registry coordinates. The active router needs the
/// selected SDK/Core/index owners, not these historical labels.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NativeTextRejoinIndexReceipt {
    pub row: NativeTextJoinedRegistryRowData,
    pub index_fingerprint: [u8; 32],
    pub prefix_fingerprint: [u8; 32],
}
catcoms_rt::redacted_debug! { NativeTextRejoinIndexReceipt; }

pub(in crate::store) type RejoinIndexInputs<'a> =
    (RejoinPrefixInputs<'a>, &'a mut Option<RejoinIndexOwner>);

pub(in crate::store) type RejoinIndexRecovery<'a> = (
    &'a mut Option<RejoinPendingOwner>,
    &'a mut Option<RejoinReceivedOwner>,
    &'a mut Option<Box<RejoinWelcomeOwner>>,
    &'a mut Option<Box<RejoinPrefixOwner>>,
    &'a mut Option<RejoinIndexOwner>,
);

pub(in crate::store) struct RejoinIndexOwner {
    record: JournalFrame,
    pending_cookie: std::sync::Arc<()>,
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    fee: CatalogReadReservation,
}

fn encode_index(
    prefix: NativeTextRejoinPrefixReceipt,
    inventory: &UpgradeInventory,
    index: &PreparedNativeRejoinIndex,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut out = Encoder::with_capacity(INDEX_FRAME - 40);
    out.put_u16(VERSION);
    for field in [
        prefix.registration.request,
        prefix.history_fingerprint,
        *blake3::hash(inventory.index()).as_bytes(),
    ] {
        out.put_bytes(&field).map_err(|_| denied())?;
    }
    out.put_bytes(index.encoded()).map_err(|_| denied())?;
    Ok(Zeroizing::new(out.finish()))
}

fn require_index(
    plain: &[u8],
    prefix: NativeTextRejoinPrefixReceipt,
    inventory: &UpgradeInventory,
    index: &PreparedNativeRejoinIndex,
) -> Result<(), AppError> {
    let mut decoder = Decoder::new(plain);
    let bad = |_| denied();
    if decoder.get_u16().map_err(bad)? != VERSION
        || decoder.get_bytes().map_err(bad)? != prefix.registration.request
        || decoder.get_bytes().map_err(bad)? != prefix.history_fingerprint
        || decoder.get_bytes().map_err(bad)? != blake3::hash(inventory.index()).as_bytes()
    {
        return Err(denied());
    }
    let selected = decoder.get_bytes().map_err(bad)?;
    decoder.finish().map_err(bad)?;
    index.require_exact(selected)
}

impl ServerStore {
    pub(in crate::store) fn cover_rejoin_index(
        &self,
        inventory: &UpgradeInventory,
        original: (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        sources: RejoinIndexInputs<'_>,
        invite: (&NativeTextInvitePacketData<'_>, [u8; 32]),
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<NativeTextRejoinIndexReceipt, AppError> {
        self.select_rejoin_index(
            RejoinReadCatalog::Current {
                inventory,
                original,
            },
            sources,
            invite,
            (rng, None),
            require_current,
        )
    }

    /// Restore only a covered registry selection. Read the held Index before
    /// rebuilding its parents so an interrupted attempt cannot become a new
    /// selection during cold recovery.
    pub(in crate::store) fn recover_rejoin_index(
        &self,
        inventory: &UpgradeInventory,
        original: (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        sources: RejoinIndexRecovery<'_>,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<NativeTextRejoinIndexReceipt, AppError> {
        self.recover_rejoin_index_with_catalog(
            inventory,
            original,
            sources,
            caller,
            rng,
            require_current,
        )
        .map(|(receipt, _)| receipt)
    }

    fn recover_rejoin_index_with_catalog(
        &self,
        inventory: &UpgradeInventory,
        original: (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        (pending, received, welcome, prefix, slot): RejoinIndexRecovery<'_>,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<(NativeTextRejoinIndexReceipt, InstalledCatalogWitness), AppError> {
        require_current()?;
        const CLOSED_ROLES: [(&str, usize); 7] = [
            (ATTEMPT, MAX_ATTEMPT),
            (PENDING, MAX_PENDING),
            (RECEIVED, MAX_FRAME),
            (SDK_ATTEMPT, ATTEMPT_MAX),
            (OUTCOME, OUTCOME_MAX),
            (PREFIX, MAX_PREFIX),
            (INDEX, INDEX_FRAME),
        ];
        let replay = InstalledReplayCatalog::capture(self, inventory, original, CLOSED_ROLES)?;
        let catalog = RejoinReadCatalog::Installed(&replay);
        require_current()?;
        let path = fixed(self, false, INDEX)?;
        let captured = if let Some(owner) = slot.as_ref() {
            let _ = owner
                .record
                .held
                .replay_file(catalog, INDEX, &owner.record.frame)?;
            owner.record.require(&path)?;
            None
        } else {
            let pool = &self
                .governance_layout_policy
                .as_ref()
                .ok_or_else(denied)?
                .current
                .archive
                .pending
                .pool;
            let fee = pool.try_reserve(INDEX_WORK)?;
            let record = read(&path, INDEX_FRAME)?.ok_or_else(denied)?;
            catalog.require_leaf(INDEX, (&record.0, &record.1))?;
            let key = Zeroizing::new(self.keys.db_key()?);
            // Refuse an incomplete or foreign record before replaying the
            // original SDK and Core data. Exact successor proof follows.
            open_frame(&key, INDEX_TAG, &record.1)?;
            Some((record, fee))
        };
        let (attempt, token) = self.recover_saved_rejoin_intent(&replay, pending)?;
        let (_, invite) = decode_saved_attempt(&attempt)?;
        let intent = (&invite, token);
        let pending = pending.as_ref().ok_or_else(denied)?;
        self.select_rejoin_received(
            catalog,
            (pending, received),
            intent,
            RejoinReceivedInput::LocalRetry,
            rng,
            &require_current,
        )?
        .ok_or_else(denied)?;
        let received = received.as_ref().ok_or_else(denied)?;
        if !matches!(
            self.select_rejoin_welcome(
                catalog,
                (pending, received, welcome),
                intent,
                (None, caller),
                rng,
                &require_current,
            )?,
            NativeTextRejoinWelcomeStatus::Covered(_)
        ) {
            return Err(denied());
        }
        let welcome = welcome.as_deref_mut().ok_or_else(denied)?;
        let selected = self.select_rejoin_index(
            catalog,
            ((pending, received, welcome, prefix), slot),
            intent,
            (rng, captured),
            &require_current,
        )?;
        replay.require(self)?;
        require_current()?;
        Ok((selected, replay.into_witness()))
    }

    fn select_rejoin_index<R: CryptoRngCore>(
        &self,
        catalog: RejoinReadCatalog<'_>,
        sources: RejoinIndexInputs<'_>,
        invite: (&NativeTextInvitePacketData<'_>, [u8; 32]),
        (rng, captured): (
            &mut R,
            Option<(OpenedArchiveObject, CatalogReadReservation)>,
        ),
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<NativeTextRejoinIndexReceipt, AppError> {
        let ((pending, received, welcome, prefix), slot) = sources;
        let (inventory, original) = catalog.parts();
        let existing_only = catalog.existing_only();
        let expected = self.select_rejoin_prefix(
            catalog,
            (pending, received, &mut *welcome, &mut *prefix),
            invite,
            (!existing_only).then_some(&mut *rng),
            &require_current,
        )?;
        let pool = &self
            .governance_layout_policy
            .as_ref()
            .ok_or_else(denied)?
            .current
            .archive
            .pending
            .pool;
        let path = fixed(self, false, INDEX)?;
        let (captured, fee) = if let Some(selected) = slot.as_ref() {
            selected.fee.validate(pool, INDEX_WORK)?;
            if !pending.matches_cookie(&selected.pending_cookie) {
                return Err(denied());
            }
            (None, None)
        } else if let Some((original, fee)) = captured {
            let record = JournalFrame::selected(original);
            record.require(&path)?;
            (Some(record), Some(fee))
        } else {
            let fee = pool.try_reserve(INDEX_WORK)?;
            (
                read(&path, INDEX_FRAME)?.map(JournalFrame::selected),
                Some(fee),
            )
        };
        let selected = PreparedNativeRejoinIndex::prepare(
            self,
            inventory,
            original.0,
            original.1,
            expected.registration,
        )?;
        let key = Zeroizing::new(self.keys.db_key()?);
        if slot.is_none() {
            let record = if let Some(original) = captured {
                require_index(
                    &open_frame(&key, INDEX_TAG, &original.frame)?,
                    expected,
                    inventory,
                    &selected,
                )?;
                original
            } else {
                if existing_only {
                    return Err(denied());
                }
                require_current()?;
                absent(&path)?;
                let plain = encode_index(expected, inventory, &selected)?;
                JournalFrame::staged(seal_frame(&key, INDEX_TAG, &plain, rng)?)
            };
            *slot = Some(RejoinIndexOwner {
                record,
                pending_cookie: pending.cookie().clone(),
                _lease: self._session.retain_for_physical_work(),
                fee: fee.ok_or_else(denied)?,
            });
        }
        let owner = slot.as_mut().ok_or_else(denied)?;
        let mut require = |prefix: &mut Option<Box<RejoinPrefixOwner>>| {
            let current = self.select_rejoin_prefix(
                catalog,
                (pending, received, &mut *welcome, prefix),
                invite,
                None::<&mut R>,
                &require_current,
            )?;
            if current.registration != expected.registration
                || current.history_fingerprint != expected.history_fingerprint
            {
                return Err(denied());
            }
            Ok(())
        };
        require_index(
            &open_frame(&key, INDEX_TAG, &owner.record.frame)?,
            expected,
            inventory,
            &selected,
        )?;
        require(prefix)?;
        owner.record.cover_bound(&path, catalog, INDEX)?;
        inventory.sync_rejoin_catalog_parent(self)?;
        require(prefix)?;
        owner.record.require(&path)?;
        require_current()?;
        Ok(NativeTextRejoinIndexReceipt {
            row: selected.row_data(),
            index_fingerprint: *blake3::hash(&owner.record.frame).as_bytes(),
            prefix_fingerprint: expected.history_fingerprint,
        })
    }
}
