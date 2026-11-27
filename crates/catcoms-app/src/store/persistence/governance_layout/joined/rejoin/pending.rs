//! One covered A2 attempt under the exact selected rejoin catalog.
use super::*;
use crate::store::critical_namespaces::joined_pin::pending::{
    JoinedTransportReservation, RecoveredPinnedPending,
};
use crate::store::NativeTextInvitePacketData;
use catcoms_rt::{NativeChatKeyEnds, NativeChatPreparedKeyCut, NativeChatSourceObservation};
use catcoms_sync::{
    DirectJoinLimits, NativeV2JoinKeyEntry, PreparedV2JoinRequest, RecoveredV2PendingJoinSource,
    SealedV2PendingJoinSource, V2JoinRequestPacketData, V2PendingJoinContextData,
    MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES, MAX_V2_JOIN_REQUEST_PACKET_BYTES,
};
use catcoms_wire::{Decoder, Encoder};

mod received;
pub use received::NativeTextRejoinReceivedReceipt;
pub(in crate::store) use received::{RejoinReceivedInput, RejoinReceivedOwner};

const ATTEMPT: &str = "chat-critical-namespaces.rejoin-attempt.bin";
const PENDING: &str = "chat-critical-namespaces.rejoin-pending.bin";
const ATTEMPT_PURPOSE: &str = "catcoms/chat-v2/rejoin-attempt/v1";
const PENDING_PURPOSE: &str = "catcoms/chat-v2/rejoin-pending/v1";
const MAX_ATTEMPT: usize = 40 + 36 + 4 + crate::store::MAX_NATIVE_TEXT_INVITE_PACKET_BYTES;
const MAX_SOURCE: usize = 512 * 1024 + MAX_V2_JOIN_REQUEST_PACKET_BYTES + 1024;
const MAX_PENDING: usize = 40 + 36 + 36 + 2 + 28 + 4 + MAX_SOURCE;
const WORK_BYTES: usize = 8 * MAX_PENDING + 4 * MAX_ATTEMPT + 4 * 1024 * 1024;

/// Covered historical data for an exact signed A2 request. It cannot start
/// transport, enter the Welcome SDK or install active membership.
pub struct NativeTextRejoinPendingReceipt {
    pub request_digest: [u8; 32],
    pub context: V2PendingJoinContextData,
}
catcoms_rt::redacted_debug! { NativeTextRejoinPendingReceipt; }

#[derive(Clone, Copy, PartialEq, Eq)]
struct Binding {
    context: V2PendingJoinContextData,
    previous: NativeTextJoinedRegistrationData,
    removal: SelectedRejoinRemoval,
    token: [u8; 32],
}

impl Binding {
    fn new(
        catalog: [u8; 32],
        previous: NativeTextJoinedRegistrationData,
        removal: SelectedRejoinRemoval,
        invite: &NativeTextInvitePacketData<'_>,
        token: [u8; 32],
    ) -> Result<Self, AppError> {
        let spec = invite.binding().spec_data();
        if token == [0; 32]
            || token == previous.invocation
            || spec.anchor.group.as_slice() != previous.group
            || spec.anchor.profile != previous.profile
            || invite.binding().id_data() == previous.invite
            || previous.selected.own_admission != removal.1
            || previous.selected.epoch >= removal.2
        {
            return Err(denied());
        }
        let mut hash = blake3::Hasher::new_derive_key("catcoms/chat-v2/rejoin-context/v1");
        for field in [
            &catalog,
            &previous.context.vault,
            &previous.context.root,
            &previous.context.pin,
            &previous.context.original_pending,
            &previous.request,
            &previous.signer,
            removal.0.as_bytes(),
            removal.1.as_bytes(),
            &token,
        ] {
            hash.update(field);
        }
        hash.update(&removal.2.to_be_bytes());
        hash.update(blake3::hash(invite.encoded_data()).as_bytes());
        Ok(Self {
            context: V2PendingJoinContextData {
                original_pending: *hash.finalize().as_bytes(),
                ..previous.context
            },
            previous,
            removal,
            token,
        })
    }

    fn require_original(
        &self,
        request: &[u8],
        invite: &NativeTextInvitePacketData<'_>,
    ) -> Result<[u8; 32], AppError> {
        let packet = V2JoinRequestPacketData::decode(request).map_err(|_| denied())?;
        let original = packet.founder_add_data();
        let spec = original.request.spec_data();
        let digest = *blake3::hash(request).as_bytes();
        if original.invite.encode() != invite.binding().encode()
            || spec.request_token != self.token
            || spec.joiner != self.previous.signer
            || digest == self.previous.request
        {
            return Err(denied());
        }
        Ok(digest)
    }
}

fn seal_frame(
    key: &[u8; 32],
    domain: &'static str,
    plain: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(crate::store::frame(&catcoms_crypto::seal(
        &key, plain, rng,
    )?)))
}

fn open_frame(
    key: &[u8; 32],
    domain: &'static str,
    frame: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let key = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(catcoms_crypto::unseal(
        &key,
        &crate::store::unframe(frame)?,
    )?))
}

fn attempt_plain(
    binding: &Binding,
    invite: &NativeTextInvitePacketData<'_>,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut encoded = Encoder::with_capacity(MAX_ATTEMPT - 40);
    encoded
        .put_bytes(&binding.context.original_pending)
        .map_err(|_| denied())?;
    encoded
        .put_bytes(invite.encoded_data())
        .map_err(|_| denied())?;
    Ok(Zeroizing::new(encoded.finish()))
}

struct OpenedPending {
    receipt: NativeTextRejoinPendingReceipt,
    recovered: RecoveredV2PendingJoinSource,
    transport: JoinedTransportReservation,
}

fn verify_pending(
    binding: &Binding,
    invite: &NativeTextInvitePacketData<'_>,
    key: &[u8; 32],
    limits: DirectJoinLimits,
    frame: &[u8],
) -> Result<NativeTextRejoinPendingReceipt, AppError> {
    open_pending(binding, invite, key, limits, frame).map(|original| original.receipt)
}

fn open_pending(
    binding: &Binding,
    invite: &NativeTextInvitePacketData<'_>,
    key: &[u8; 32],
    limits: DirectJoinLimits,
    frame: &[u8],
) -> Result<OpenedPending, AppError> {
    let plain = open_frame(key, PENDING_PURPOSE, frame)?;
    let mut decoded = Decoder::new(&plain);
    let malformed = |_| denied();
    if decoded.get_bytes().map_err(malformed)? != binding.context.original_pending {
        return Err(denied());
    }
    let seed = decoded
        .get_bytes()
        .map_err(malformed)?
        .try_into()
        .map_err(|_| denied())?;
    let port = decoded.get_u16().map_err(malformed)?;
    let nonce = decoded
        .get_bytes()
        .map_err(malformed)?
        .try_into()
        .map_err(|_| denied())?;
    let ciphertext = decoded.get_bytes().map_err(malformed)?;
    if ciphertext.len() > MAX_SOURCE {
        return Err(denied());
    }
    decoded.finish().map_err(malformed)?;
    let transport = JoinedTransportReservation::new(Zeroizing::new(seed), port)?;
    if transport.identity_data()?.0 == binding.previous.transport.0 {
        return Err(denied());
    }
    let sealed =
        SealedV2PendingJoinSource::from_encrypted_parts(nonce, ciphertext).map_err(|_| denied())?;
    let recovered =
        RecoveredV2PendingJoinSource::open(&sealed, key, binding.context, invite.binding(), limits)
            .map_err(|_| denied())?;
    if recovered.rejoin_remove_data() != Some(binding.removal) {
        return Err(denied());
    }
    Ok(OpenedPending {
        receipt: NativeTextRejoinPendingReceipt {
            request_digest: binding.require_original(recovered.request_bytes_data(), invite)?,
            context: binding.context,
        },
        recovered,
        transport,
    })
}

fn cover_existing(path: &Path, original: &File, frame: &[u8]) -> Result<(), AppError> {
    let identity = session_id(original).map_err(io)?;
    let (file, opened) = open_exact_write(path, WriteMode::Existing)?;
    if opened != identity {
        return Err(denied());
    }
    exact_held(path, &file, identity, frame)?;
    sync_selected_catalog(&file).map_err(io)?;
    exact_held(path, &file, identity, frame)
}

pub(in crate::store) type RejoinKeyEntry<'a> = (
    &'a NativeChatSourceObservation,
    &'a mut NativeChatPreparedKeyCut,
    &'a mut NativeChatKeyEnds,
);

pub(in crate::store) struct WarmRejoinPending {
    cookie: std::sync::Arc<()>,
    binding: Binding,
    attempt: Zeroizing<Vec<u8>>,
    attempt_file: Option<CreatedStage>,
    pending: Option<Zeroizing<Vec<u8>>>,
    pending_file: Option<CreatedStage>,
    request: Option<PreparedV2JoinRequest>,
    transport: Option<JoinedTransportReservation>,
    key_may_have_entered: bool,
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

pub(in crate::store) struct ColdRejoinPending {
    cookie: std::sync::Arc<()>,
    binding: Binding,
    attempt: OpenedArchiveObject,
    pending: OpenedArchiveObject,
    _lease: catcoms_storage::VaultPhysicalWorkLease,
    fee: CatalogReadReservation,
}

pub(in crate::store) enum RejoinPendingOwner {
    Warm(Box<WarmRejoinPending>),
    Cold(Box<ColdRejoinPending>),
}

impl RejoinPendingOwner {
    fn invoice(&self) -> &CatalogReadReservation {
        match self {
            Self::Warm(original) => &original._fee,
            Self::Cold(original) => &original.fee,
        }
    }

    fn cookie(&self) -> &std::sync::Arc<()> {
        match self {
            Self::Warm(original) => &original.cookie,
            Self::Cold(original) => &original.cookie,
        }
    }

    pub(in crate::store) fn matches_cookie(&self, original: &std::sync::Arc<()>) -> bool {
        std::sync::Arc::ptr_eq(self.cookie(), original)
    }

    fn files(&self, binding: &Binding) -> Result<[(&File, &[u8]); 2], AppError> {
        match self {
            Self::Warm(warm) if &warm.binding == binding => Ok([
                (
                    &warm.attempt_file.as_ref().ok_or_else(denied)?.file,
                    &warm.attempt,
                ),
                (
                    &warm.pending_file.as_ref().ok_or_else(denied)?.file,
                    warm.pending.as_ref().ok_or_else(denied)?,
                ),
            ]),
            Self::Cold(cold) if &cold.binding == binding => Ok([
                (&cold.attempt.0, &cold.attempt.1),
                (&cold.pending.0, &cold.pending.1),
            ]),
            _ => Err(denied()),
        }
    }
}

fn cover_new(path: &Path, frame: &[u8], file: &mut Option<CreatedStage>) -> Result<(), AppError> {
    ensure_native_claim(path, file)?;
    ensure(
        path,
        frame,
        file,
        ArchiveOutputMode::Original,
        #[cfg(test)]
        0,
        #[cfg(test)]
        0,
    )?;
    exact_retained(path, file.as_ref().ok_or_else(denied)?, frame)
}

/// Paid custody issued only after re-covering both exact journal files.
pub(in crate::store) struct PreparedRejoinTransport {
    pub(in crate::store) recovered: RecoveredV2PendingJoinSource,
    pub(in crate::store) seed: Zeroizing<[u8; 32]>,
    pub(in crate::store) local: (catcoms_rt::PeerId, u16),
    pub(in crate::store) receipt: NativeTextRejoinPendingReceipt,
    pub(in crate::store) cookie: std::sync::Arc<()>,
    pub(in crate::store) mount: catcoms_rt::RequestCancellation,
    pub(in crate::store) transport_lease: catcoms_storage::VaultPhysicalWorkLease,
    pub(in crate::store) pool: CriticalCatalogReadPool,
    pub(in crate::store) exchange_fee: CatalogReadReservation,
}

fn pending_limits() -> Result<DirectJoinLimits, AppError> {
    DirectJoinLimits::new(
        512 * 1024,
        2 * 1024 * 1024,
        MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES,
    )
    .map_err(|_| denied())
}

fn existing_rejoin_files(
    attempt_path: &Path,
    pending_path: &Path,
) -> Result<Option<(OpenedArchiveObject, OpenedArchiveObject)>, AppError> {
    let Some(attempt) = read(attempt_path, MAX_ATTEMPT)? else {
        absent(pending_path)?;
        return Ok(None);
    };
    let pending = read(pending_path, MAX_PENDING)?.ok_or_else(denied)?;
    Ok(Some((attempt, pending)))
}

fn require_held_rejoin_files(
    paths: [&Path; 2],
    files: [(&File, &[u8]); 2],
) -> Result<(), AppError> {
    for (path, (file, frame)) in paths.into_iter().zip(files) {
        exact_held(path, file, session_id(file).map_err(io)?, frame)?;
    }
    Ok(())
}

impl ServerStore {
    fn open_covered_rejoin_files(
        &self,
        inventory: &UpgradeInventory,
        binding: &Binding,
        (key, invite, limits): (&[u8; 32], &NativeTextInvitePacketData<'_>, DirectJoinLimits),
        paths: [&Path; 2],
        files: [(&File, &[u8]); 2],
    ) -> Result<OpenedPending, AppError> {
        require_held_rejoin_files(paths, files)?;
        if open_frame(key, ATTEMPT_PURPOSE, files[0].1)? != attempt_plain(binding, invite)? {
            return Err(denied());
        }
        let opened = open_pending(binding, invite, key, limits, files[1].1)?;
        for (path, (file, frame)) in paths.into_iter().zip(files) {
            cover_existing(path, file, frame)?;
        }
        inventory.sync_rejoin_catalog_parent(self)?;
        require_held_rejoin_files(paths, files)?;
        self.require_recovered_joined_removal(binding.previous, binding.removal)?;
        absent(&fixed(self, false, REJOIN_STAGE)?)?;
        inventory.require(self)?;
        Ok(opened)
    }

    /// Reprove a completed replacement request for an explicitly approved
    /// native exchange. This path never admits a device or Welcome cut.
    pub(in crate::store) fn prepare_rejoin_transport(
        &self,
        inventory: &UpgradeInventory,
        (registration, removal): (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        slot: &mut Option<RejoinPendingOwner>,
        (invite, token): (&NativeTextInvitePacketData<'_>, [u8; 32]),
    ) -> Result<PreparedRejoinTransport, AppError> {
        let catalog = self.cover_selected_rejoin_catalog(registration, removal, inventory)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let pool = &policy.current.archive.pending.pool;
        let binding = Binding::new(catalog, registration, removal, invite, token)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let attempt_path = fixed(self, false, ATTEMPT)?;
        let pending_path = fixed(self, false, PENDING)?;
        let paths = [attempt_path.as_path(), pending_path.as_path()];
        let limits = pending_limits()?;
        let opened = if let Some(original) = slot.as_ref() {
            original.invoice().validate(pool, WORK_BYTES)?;
            self.open_covered_rejoin_files(
                inventory,
                &binding,
                (&key, invite, limits),
                paths,
                original.files(&binding)?,
            )?
        } else {
            let fee = pool.try_reserve(WORK_BYTES)?;
            let (attempt, pending) =
                existing_rejoin_files(paths[0], paths[1])?.ok_or_else(denied)?;
            let opened = self.open_covered_rejoin_files(
                inventory,
                &binding,
                (&key, invite, limits),
                paths,
                [(&attempt.0, &attempt.1), (&pending.0, &pending.1)],
            )?;
            *slot = Some(RejoinPendingOwner::Cold(Box::new(ColdRejoinPending {
                cookie: std::sync::Arc::new(()),
                binding,
                attempt,
                pending,
                _lease: self._session.retain_for_physical_work(),
                fee,
            })));
            opened
        };
        let (peer, port, _) = opened.transport.identity_data()?;
        let (seed, _) = opened.transport.sealed_record_data();
        let exchange_fee = pool.try_reserve(
            4 * (opened.recovered.request_bytes_data().len()
                + 1
                + catcoms_rt::CHAT_V2_JOIN_RESPONSE_MAX_BYTES)
                + 4096,
        )?;
        Ok(PreparedRejoinTransport {
            recovered: opened.recovered,
            seed: Zeroizing::new(*seed),
            local: (peer, port),
            receipt: opened.receipt,
            cookie: slot.as_ref().ok_or_else(denied)?.cookie().clone(),
            mount: self.chat_mount.observer(),
            transport_lease: self._session.retain_for_physical_work(),
            pool: pool.clone(),
            exchange_fee,
        })
    }

    /// The physical caller holds the original native registry cell and its
    /// inventory. No socket or Welcome work may follow from this receipt.
    pub(in crate::store) fn cover_rejoin_pending(
        &self,
        inventory: &UpgradeInventory,
        (registration, removal): (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        slot: &mut Option<RejoinPendingOwner>,
        (invite, token, now_ms): (&NativeTextInvitePacketData<'_>, [u8; 32], u64),
        (native, cut, ends): RejoinKeyEntry<'_>,
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<NativeTextRejoinPendingReceipt, AppError> {
        require_current()?;
        let catalog = self.cover_selected_rejoin_catalog(registration, removal, inventory)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let pool = &policy.current.archive.pending.pool;
        let fee = if let Some(original) = slot.as_ref() {
            // The original invoice covers the files, retained request
            // and their transient sealing and read overlap.
            original.invoice().validate(pool, WORK_BYTES)?;
            None
        } else {
            Some(pool.try_reserve(WORK_BYTES)?)
        };
        let binding = Binding::new(catalog, registration, removal, invite, token)?;
        let attempt = attempt_plain(&binding, invite)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let attempt_path = fixed(self, false, ATTEMPT)?;
        let pending_path = fixed(self, false, PENDING)?;
        let limits = pending_limits()?;
        let require = || {
            require_current()?;
            inventory.require(self)?;
            self.require_recovered_joined_removal(registration, removal)?;
            absent(&fixed(self, false, REJOIN_STAGE)?)
        };

        let paths = [attempt_path.as_path(), pending_path.as_path()];
        if let Some(original @ RejoinPendingOwner::Cold(_)) = slot.as_ref() {
            let opened = self.open_covered_rejoin_files(
                inventory,
                &binding,
                (&key, invite, limits),
                paths,
                original.files(&binding)?,
            )?;
            require()?;
            return Ok(opened.receipt);
        }
        if slot.is_none() {
            if let Some((attempt, pending)) = existing_rejoin_files(paths[0], paths[1])? {
                let opened = self.open_covered_rejoin_files(
                    inventory,
                    &binding,
                    (&key, invite, limits),
                    paths,
                    [(&attempt.0, &attempt.1), (&pending.0, &pending.1)],
                )?;
                require()?;
                *slot = Some(RejoinPendingOwner::Cold(Box::new(ColdRejoinPending {
                    cookie: std::sync::Arc::new(()),
                    binding,
                    attempt,
                    pending,
                    _lease: self._session.retain_for_physical_work(),
                    fee: fee.ok_or_else(denied)?,
                })));
                return Ok(opened.receipt);
            }
            // An orphaned or unresolved reply cannot be reassigned to a
            // replacement attempt. Preserve it and refuse before entropy.
            received::require_absent(self)?;
            if invite.binding().expired_at_data(now_ms) {
                return Err(denied());
            }
            self.with_rejoin_donor(registration, |layout, raw| {
                RecoveredPinnedPending::require_rejoin_invite(
                    &key,
                    layout,
                    raw,
                    registration.context,
                    invite.binding(),
                )
            })?;
            let sealed_attempt = seal_frame(&key, ATTEMPT_PURPOSE, &attempt, rng)?;
            *slot = Some(RejoinPendingOwner::Warm(Box::new(WarmRejoinPending {
                cookie: std::sync::Arc::new(()),
                binding,
                attempt: sealed_attempt,
                attempt_file: None,
                pending: None,
                pending_file: None,
                request: None,
                transport: None,
                key_may_have_entered: false,
                _lease: self._session.retain_for_physical_work(),
                _fee: fee.ok_or_else(denied)?,
            })));
        }
        let Some(RejoinPendingOwner::Warm(original)) = slot.as_mut() else {
            return Err(denied());
        };
        if original.binding != binding {
            return Err(denied());
        }
        require()?;
        if original.pending.is_none() {
            received::require_absent(self)?;
        }
        cover_new(&attempt_path, &original.attempt, &mut original.attempt_file)?;
        inventory.sync_rejoin_catalog_parent(self)?;
        if original.request.is_none() {
            if original.key_may_have_entered || invite.binding().expired_at_data(now_ms) {
                return Err(denied());
            }
            absent(&pending_path)?;
            require()?;
            exact_retained(
                &attempt_path,
                original.attempt_file.as_ref().ok_or_else(denied)?,
                &original.attempt,
            )?;
            received::require_absent(self)?;
            original.key_may_have_entered = true;
            let prepared = self.with_rejoin_donor(registration, |layout, raw| {
                RecoveredPinnedPending::prepare_rejoin_from_original(
                    &key,
                    layout,
                    raw,
                    registration,
                    (invite.binding(), token, now_ms),
                    removal,
                    limits,
                    NativeV2JoinKeyEntry::new(native, cut, ends),
                )
            });
            match prepared {
                Ok(Ok(request)) => original.request = Some(request),
                Ok(Err(catcoms_sync::NativeV2RejoinFailure::AfterDeviceEntry(_))) => {
                    return Err(denied());
                }
                // A returned refusal distinguished below the RT cut cannot
                // have issued a KeyPackage. An unwind never reaches here.
                Ok(Err(catcoms_sync::NativeV2RejoinFailure::BeforeDeviceEntry(_))) | Err(_) => {
                    original.key_may_have_entered = false;
                    return Err(denied());
                }
            }
        }
        require()?;
        if original.transport.is_none() {
            original.transport = Some(JoinedTransportReservation::generate(rng)?);
        }
        if original.pending.is_none() {
            received::require_absent(self)?;
            absent(&pending_path)?;
            let request = original.request.as_mut().ok_or_else(denied)?;
            let source = request
                .seal_pending_source(&key, binding.context, rng)
                .map_err(|_| denied())?;
            let (nonce, ciphertext) = source.encrypted_parts();
            let (seed, port) = original
                .transport
                .as_ref()
                .ok_or_else(denied)?
                .sealed_record_data();
            let mut encoded = Encoder::with_capacity(MAX_PENDING - 40);
            encoded
                .put_bytes(&binding.context.original_pending)
                .map_err(|_| denied())?;
            encoded.put_bytes(seed).map_err(|_| denied())?;
            encoded.put_u16(port);
            encoded.put_bytes(nonce).map_err(|_| denied())?;
            encoded.put_bytes(ciphertext).map_err(|_| denied())?;
            original.pending = Some(seal_frame(
                &key,
                PENDING_PURPOSE,
                &Zeroizing::new(encoded.finish()),
                rng,
            )?);
        }
        let pending = original.pending.as_ref().ok_or_else(denied)?;
        let receipt = verify_pending(&binding, invite, &key, limits, pending)?;
        require()?;
        cover_new(&pending_path, pending, &mut original.pending_file)?;
        inventory.sync_rejoin_catalog_parent(self)?;
        exact_retained(
            &attempt_path,
            original.attempt_file.as_ref().ok_or_else(denied)?,
            &original.attempt,
        )?;
        exact_retained(
            &pending_path,
            original.pending_file.as_ref().ok_or_else(denied)?,
            pending,
        )?;
        require()?;
        Ok(receipt)
    }
}
