//! Match the exact closed Welcome with its signed Core history before the
//! replacement can be presented to the native registry writer.
use super::*;
use crate::store::persistence::governance_layout::joined::pending::JoinedCore;
use catcoms_replication::chat::{
    cold_selected_history::MAX_SELECTED_HISTORY_NODE_PAGE_BYTES, V2HostSelectionExpectation,
};

mod index;
pub use index::NativeTextRejoinIndexReceipt;
pub(in crate::store) use index::{
    InstalledRejoin, RejoinIndexInputs, RejoinIndexOwner, RejoinIndexRecovery,
};

const PREFIX: &str = "chat-critical-namespaces.rejoin-initial-prefix.bin";
const PREFIX_TAG: &str = "catcoms/chat-v2/rejoin-initial-prefix/v1";
const MAX_PREFIX: usize = 40 + 2 + 3 * 36 + 8 + 4 + MAX_SELECTED_HISTORY_NODE_PAGE_BYTES;
const PREFIX_WORK: usize = 5 * MAX_PREFIX + 2 * MAX_FRAME + 128 * 1024;

/// Covered initial history for a replacement, still under the retired vault.
#[derive(Clone, Copy)]
pub struct NativeTextRejoinPrefixReceipt {
    pub registration: NativeTextJoinedRegistrationData,
    pub history_fingerprint: [u8; 32],
}
catcoms_rt::redacted_debug! { NativeTextRejoinPrefixReceipt; }

pub(super) fn require_unstarted(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, PREFIX)?)?;
    index::require_unstarted(store)
}

pub(in crate::store) struct RejoinPrefixOwner {
    core: std::sync::Arc<JoinedCore>,
    frame: Option<JournalFrame>,
    // Keep the receipt after finish consumes the pending Core transition.
    receipt: Option<NativeTextRejoinPrefixReceipt>,
    pending_cookie: std::sync::Arc<()>,
    bind: [u8; 32],
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    fee: CatalogReadReservation,
}

pub(in crate::store) type RejoinPrefixInputs<'a> = (
    &'a RejoinPendingOwner,
    &'a RejoinReceivedOwner,
    &'a mut RejoinWelcomeOwner,
    &'a mut Option<Box<RejoinPrefixOwner>>,
);

fn prefix_plain(
    (pending_context, bind): ([u8; 32], [u8; 32]),
    receipt: NativeTextRejoinOutcomeReceipt,
    history: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if history.is_empty() || history.len() > MAX_SELECTED_HISTORY_NODE_PAGE_BYTES {
        return Err(denied());
    }
    let mut out = Encoder::with_capacity(MAX_PREFIX - 40);
    out.put_u16(VERSION);
    for digest in [pending_context, bind, *receipt.header.as_bytes()] {
        out.put_bytes(&digest).map_err(|_| denied())?;
    }
    out.put_u64(receipt.epoch);
    out.put_bytes(history).map_err(|_| denied())?;
    Ok(Zeroizing::new(out.finish()))
}

impl ServerStore {
    pub(in crate::store) fn cover_rejoin_prefix(
        &self,
        inventory: &UpgradeInventory,
        (registration, removal): (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        (pending, received, welcome, slot): RejoinPrefixInputs<'_>,
        (invite, token): (&NativeTextInvitePacketData<'_>, [u8; 32]),
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<NativeTextRejoinPrefixReceipt, AppError> {
        self.select_rejoin_prefix(
            inventory,
            (registration, removal),
            (pending, received, welcome, slot),
            (invite, token),
            Some(rng),
            require_current,
        )
    }

    fn select_rejoin_prefix(
        &self,
        inventory: &UpgradeInventory,
        (registration, removal): (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        (pending, received, welcome, slot): RejoinPrefixInputs<'_>,
        (invite, token): (&NativeTextInvitePacketData<'_>, [u8; 32]),
        rng: Option<&mut impl CryptoRngCore>,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<NativeTextRejoinPrefixReceipt, AppError> {
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
        welcome.fee.validate(pool, OUTCOME_WORK)?;
        if !received.covered
            || !welcome.covered
            || !pending.matches_cookie(&received.pending_cookie)
            || !pending.matches_cookie(&welcome.pending_cookie)
        {
            return Err(denied());
        }
        let binding = Binding::new(catalog, registration, removal, invite, token)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let attempt_path = fixed(self, false, ATTEMPT)?;
        let pending_path = fixed(self, false, PENDING)?;
        let received_path = fixed(self, false, RECEIVED)?;
        let entry_path = fixed(self, false, SDK_ATTEMPT)?;
        let outcome_path = fixed(self, false, OUTCOME)?;
        let prefix_path = fixed(self, false, PREFIX)?;
        let paths = [attempt_path.as_path(), pending_path.as_path()];
        let files = pending.files(&binding)?;
        let original = self.open_covered_rejoin_files(
            inventory,
            &binding,
            (&key, invite, pending_limits()?),
            paths,
            files,
        )?;
        let outcome = welcome.outcome.as_ref().ok_or_else(denied)?;
        let receipt = welcome.receipt.ok_or_else(denied)?;
        let require = || {
            require_current()?;
            require_held_rejoin_files(paths, files)?;
            self.require_recovered_joined_removal(registration, removal)?;
            absent(&fixed(self, false, REJOIN_STAGE)?)?;
            inventory.require(self)?;
            received.require(&received_path)?;
            welcome.entry.require(&entry_path)?;
            outcome.require(&outcome_path)?;
            require_current()
        };
        require()?;
        let plain = open_frame(&key, PURPOSE, &received.frame)?;
        let delivery = delivery_bytes(&binding, &original, &plain)?;
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
        if open_frame(&key, ATTEMPT_TAG, &welcome.entry.frame)? != expected
            || receipt.request_digest != original.receipt.request_digest
        {
            return Err(denied());
        }
        let mut hash = blake3::Hasher::new_derive_key("catcoms/chat-v2/rejoin-prefix-parent/v1");
        for frame in [
            files[0].1,
            files[1].1,
            &received.frame,
            &welcome.entry.frame,
            &outcome.frame,
        ] {
            hash.update(blake3::hash(frame).as_bytes());
        }
        let bind = *hash.finalize().as_bytes();
        if slot.is_none() {
            let fee = pool.try_reserve(PREFIX_WORK)?;
            // A cold missing parent cannot be regenerated under a chosen
            // registry successor, even if the signed history could be replayed.
            if read(&prefix_path, MAX_PREFIX)?.is_none() {
                if rng.is_none() {
                    return Err(denied());
                }
                index::require_unstarted(self)?;
            }
            let core = if let Some(warm) = welcome.warm.as_mut() {
                JoinedCore::prepare(self, &registration.group, delivery, |view, origin| {
                    warm.request
                        .prepare_initial_prefix(delivery, view, origin)
                        .map_err(|_| denied())
                })?
            } else {
                let graph = welcome.recovered.as_mut().ok_or_else(denied)?;
                let expected = V2HostSelectionExpectation::from_trusted_host(
                    graph.governing_head_data(),
                    graph.governing_state_data(),
                );
                JoinedCore::prepare(self, &registration.group, delivery, |view, origin| {
                    Ok((
                        graph
                            .prepare_initial_prefix(delivery, view, origin)
                            .map_err(|_| denied())?,
                        expected,
                    ))
                })?
            };
            // Sync issues this particular prefix only once per closed graph.
            // Keep it before any further file or lifecycle work can refuse.
            *slot = Some(Box::new(RejoinPrefixOwner {
                core: std::sync::Arc::new(core),
                frame: None,
                receipt: None,
                pending_cookie: pending.cookie().clone(),
                bind,
                _lease: self._session.retain_for_physical_work(),
                fee,
            }));
        }
        let prefix = slot.as_mut().ok_or_else(denied)?;
        prefix.fee.validate(pool, PREFIX_WORK)?;
        if !pending.matches_cookie(&prefix.pending_cookie) || prefix.bind != bind {
            return Err(denied());
        }
        let existing_only = rng.is_none();
        if prefix.frame.is_none() {
            let history = prefix.core.encode_initial_history(&binding.context.vault)?;
            let expected =
                prefix_plain((binding.context.original_pending, bind), receipt, &history)?;
            let original = if let Some(original) = read(&prefix_path, MAX_PREFIX)? {
                if open_frame(&key, PREFIX_TAG, &original.1)? != expected {
                    return Err(denied());
                }
                JournalFrame::selected(original)
            } else {
                require()?;
                // A registry record without its original prefix is ambiguous.
                index::require_unstarted(self)?;
                absent(&prefix_path)?;
                let rng = rng.ok_or_else(denied)?;
                JournalFrame::staged(seal_frame(&key, PREFIX_TAG, &expected, rng)?)
            };
            prefix.frame = Some(original);
        }
        let frame = prefix.frame.as_mut().ok_or_else(denied)?;
        require()?;
        if existing_only {
            frame.require(&prefix_path)?;
        }
        frame.cover(&prefix_path)?;
        inventory.sync_rejoin_catalog_parent(self)?;
        require()?;
        frame.require(&prefix_path)?;
        if prefix.receipt.is_none() {
            std::sync::Arc::get_mut(&mut prefix.core)
                .ok_or_else(denied)?
                .finish()?;
            let selected = prefix.core.selected_data().ok_or_else(denied)?;
            if (selected.governing_add, selected.epoch) != (receipt.header, receipt.epoch)
                || selected.own_admission == removal.1
            {
                return Err(denied());
            }
            let data = NativeTextJoinedRegistrationData::from_selection(
                original.recovered.request_bytes_data(),
                binding.context,
                original.transport.identity_data()?,
                selected,
            )?;
            prefix.receipt = Some(NativeTextRejoinPrefixReceipt {
                registration: data,
                history_fingerprint: *blake3::hash(&frame.frame).as_bytes(),
            });
        }
        require()?;
        frame.require(&prefix_path)?;
        require_current()?;
        prefix.receipt.ok_or_else(denied)
    }
}
