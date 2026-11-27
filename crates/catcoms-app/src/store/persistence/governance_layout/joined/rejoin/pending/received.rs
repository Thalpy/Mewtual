//! Store authenticated A2 reply data before any Welcome key operation.
use super::*;

mod welcome;
pub use welcome::{
    NativeTextRejoinOutcomeReceipt, NativeTextRejoinPrefixReceipt, NativeTextRejoinWelcomeStatus,
};
pub(in crate::store) use welcome::{
    RejoinPrefixInputs, RejoinPrefixOwner, RejoinWelcomeKeyEntry, RejoinWelcomeOwner,
    RejoinWelcomeOwners,
};

const RECEIVED: &str = "chat-critical-namespaces.rejoin-received.bin";
const PURPOSE: &str = "catcoms/chat-v2/rejoin-received/v1";
const VERSION: u16 = 1;
const MAX_FRAME: usize = 40 + 2 + 2 * 36 + 4 + MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES;
const WORK: usize = 5 * MAX_FRAME + 128 * 1024;

pub(super) fn require_absent(store: &ServerStore) -> Result<(), AppError> {
    absent(&fixed(store, false, RECEIVED)?)?;
    welcome::require_unstarted(store)
}

/// Exact covered reply data. It grants no authority to use the Welcome SDK.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NativeTextRejoinReceivedReceipt {
    pub request_digest: [u8; 32],
    pub delivery_digest: [u8; 32],
}
catcoms_rt::redacted_debug! { NativeTextRejoinReceivedReceipt; }

pub(in crate::store) enum RejoinReceivedInput<'a> {
    Connected {
        pending_cookie: &'a std::sync::Arc<()>,
        delivery: &'a [u8],
    },
    LocalRetry,
}

enum Held {
    Stage(Option<CreatedStage>),
    Selected(File),
}

/// A warm retry cannot adopt an equal-byte file or a new Pending owner.
pub(in crate::store) struct RejoinReceivedOwner {
    held: Held,
    frame: Zeroizing<Vec<u8>>,
    pending_cookie: std::sync::Arc<()>,
    covered: bool,
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl RejoinReceivedOwner {
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

fn encode_delivery(
    binding: &Binding,
    request_digest: [u8; 32],
    delivery: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if delivery.is_empty() || delivery.len() > MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES {
        return Err(denied());
    }
    let mut out = Encoder::with_capacity(MAX_FRAME - 40);
    out.put_u16(VERSION);
    out.put_bytes(&binding.context.original_pending)
        .map_err(|_| denied())?;
    out.put_bytes(&request_digest).map_err(|_| denied())?;
    out.put_bytes(delivery).map_err(|_| denied())?;
    Ok(Zeroizing::new(out.finish()))
}

fn delivery_bytes<'a>(
    binding: &Binding,
    original: &OpenedPending,
    plain: &'a [u8],
) -> Result<&'a [u8], AppError> {
    let mut decoder = Decoder::new(plain);
    let invalid = |_| denied();
    if decoder.get_u16().map_err(invalid)? != VERSION
        || decoder.get_bytes().map_err(invalid)? != binding.context.original_pending
        || decoder.get_bytes().map_err(invalid)? != original.receipt.request_digest
    {
        return Err(denied());
    }
    let delivery = decoder.get_bytes().map_err(invalid)?;
    decoder.finish().map_err(invalid)?;
    if delivery.is_empty() || delivery.len() > MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES {
        return Err(denied());
    }
    Ok(delivery)
}

fn verify_delivery(
    binding: &Binding,
    original: &OpenedPending,
    plain: &[u8],
    received: Option<&[u8]>,
) -> Result<NativeTextRejoinReceivedReceipt, AppError> {
    let delivery = delivery_bytes(binding, original, plain)?;
    if received.is_some_and(|bytes| bytes != delivery) {
        return Err(denied());
    }
    original
        .recovered
        .validate_pinned_join_delivery_data(delivery)
        .map_err(|_| denied())?;
    Ok(NativeTextRejoinReceivedReceipt {
        request_digest: original.receipt.request_digest,
        delivery_digest: *blake3::hash(delivery).as_bytes(),
    })
}

impl ServerStore {
    /// Cover received data against the exact retained Pending pair and their
    /// original terminal membership. LocalRetry can only recover that data.
    pub(in crate::store) fn cover_rejoin_received(
        &self,
        inventory: &UpgradeInventory,
        (registration, removal): (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        (pending, slot): (&RejoinPendingOwner, &mut Option<RejoinReceivedOwner>),
        (invite, token): (&NativeTextInvitePacketData<'_>, [u8; 32]),
        input: RejoinReceivedInput<'_>,
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<Option<NativeTextRejoinReceivedReceipt>, AppError> {
        require_current()?;
        let received = match input {
            RejoinReceivedInput::Connected {
                pending_cookie,
                delivery,
            } => {
                if !pending.matches_cookie(pending_cookie) {
                    return Err(denied());
                }
                Some(delivery)
            }
            RejoinReceivedInput::LocalRetry => None,
        };
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
        let binding = Binding::new(catalog, registration, removal, invite, token)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let attempt_path = fixed(self, false, ATTEMPT)?;
        let pending_path = fixed(self, false, PENDING)?;
        let paths = [attempt_path.as_path(), pending_path.as_path()];
        let files = pending.files(&binding)?;
        let original = self.open_covered_rejoin_files(
            inventory,
            &binding,
            (&key, invite, pending_limits()?),
            paths,
            files,
        )?;
        let path = fixed(self, false, RECEIVED)?;
        let require = || {
            require_current()?;
            require_held_rejoin_files(paths, files)?;
            self.require_recovered_joined_removal(registration, removal)?;
            absent(&fixed(self, false, REJOIN_STAGE)?)?;
            inventory.require(self)
        };
        if slot.is_none() {
            let fee = pool.try_reserve(WORK)?;
            let candidate = if let Some((file, frame)) = read(&path, MAX_FRAME)? {
                // Verify before holding an existing file or changing anything.
                let plain = open_frame(&key, PURPOSE, &frame)?;
                verify_delivery(&binding, &original, &plain, received)?;
                (Held::Selected(file), frame)
            } else {
                // A later journal binds the exact original Received frame.
                // Its missing parent cannot be rebuilt with a new nonce.
                welcome::require_unstarted(self)?;
                if let Some(delivery) = received {
                    let plain =
                        encode_delivery(&binding, original.receipt.request_digest, delivery)?;
                    verify_delivery(&binding, &original, &plain, Some(delivery))?;
                    require()?;
                    welcome::require_unstarted(self)?;
                    (Held::Stage(None), seal_frame(&key, PURPOSE, &plain, rng)?)
                } else {
                    require()?;
                    welcome::require_unstarted(self)?;
                    absent(&path)?;
                    require_current()?;
                    return Ok(None);
                }
            };
            require()?;
            *slot = Some(RejoinReceivedOwner {
                held: candidate.0,
                frame: candidate.1,
                pending_cookie: pending.cookie().clone(),
                covered: false,
                _lease: self._session.retain_for_physical_work(),
                _fee: fee,
            });
        }
        let record = slot.as_mut().ok_or_else(denied)?;
        if !std::sync::Arc::ptr_eq(&record.pending_cookie, pending.cookie()) {
            return Err(denied());
        }
        record._fee.validate(pool, WORK)?;
        let plain = open_frame(&key, PURPOSE, &record.frame)?;
        let receipt = verify_delivery(&binding, &original, &plain, received)?;
        // The barrier operates on the held file. Recheck the two original
        // request files and vault owner on both sides of covering the reply.
        require()?;
        record.cover(&path)?;
        inventory.sync_rejoin_catalog_parent(self)?;
        record.require(&path)?;
        require()?;
        // The final source proof can itself block on physical reads.
        // Return only while its reply still names the held original file.
        record.require(&path)?;
        require_current()?;
        // A matching name alone cannot prove the earlier durability barrier.
        record.covered = true;
        Ok(Some(receipt))
    }
}
