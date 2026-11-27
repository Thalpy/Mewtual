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

fn verify_pending(
    binding: &Binding,
    invite: &NativeTextInvitePacketData<'_>,
    key: &[u8; 32],
    limits: DirectJoinLimits,
    frame: &[u8],
) -> Result<NativeTextRejoinPendingReceipt, AppError> {
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
    Ok(NativeTextRejoinPendingReceipt {
        request_digest: binding.require_original(recovered.request_bytes_data(), invite)?,
        context: binding.context,
    })
}

fn cover_existing(path: &Path, original: &OpenedArchiveObject) -> Result<(), AppError> {
    let identity = session_id(&original.0).map_err(io)?;
    let (file, opened) = open_exact_write(path, WriteMode::Existing)?;
    if opened != identity {
        return Err(denied());
    }
    exact_held(path, &file, identity, &original.1)?;
    sync_selected_catalog(&file).map_err(io)?;
    exact_held(path, &file, identity, &original.1)
}

pub(in crate::store) type RejoinKeyEntry<'a> = (
    &'a NativeChatSourceObservation,
    &'a mut NativeChatPreparedKeyCut,
    &'a mut NativeChatKeyEnds,
);

pub(in crate::store) struct WarmRejoinPending {
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

impl ServerStore {
    /// The physical caller holds the original native registry cell and its
    /// inventory. No socket or Welcome work may follow from this receipt.
    pub(in crate::store) fn cover_rejoin_pending(
        &self,
        inventory: &UpgradeInventory,
        (registration, removal): (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        warm: &mut Option<Box<WarmRejoinPending>>,
        (invite, token, now_ms): (&NativeTextInvitePacketData<'_>, [u8; 32], u64),
        (native, cut, ends): RejoinKeyEntry<'_>,
        rng: &mut impl CryptoRngCore,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<NativeTextRejoinPendingReceipt, AppError> {
        require_current()?;
        let catalog = self.cover_selected_rejoin_catalog(registration, removal, inventory)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let pool = &policy.current.archive.pending.pool;
        let fee = if let Some(original) = warm.as_ref() {
            // The original invoice covers both the retained request and
            // this attempt's transient sealing and read overlap.
            original._fee.validate(pool, WORK_BYTES)?;
            None
        } else {
            Some(pool.try_reserve(WORK_BYTES)?)
        };
        let binding = Binding::new(catalog, registration, removal, invite, token)?;
        let attempt = attempt_plain(&binding, invite)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let attempt_path = fixed(self, false, ATTEMPT)?;
        let pending_path = fixed(self, false, PENDING)?;
        let limits = DirectJoinLimits::new(
            512 * 1024,
            2 * 1024 * 1024,
            MAX_V2_GOVERNING_JOIN_DELIVERY_BYTES,
        )
        .map_err(|_| denied())?;
        let require = || {
            require_current()?;
            inventory.require(self)?;
            self.require_recovered_joined_removal(registration, removal)?;
            absent(&fixed(self, false, REJOIN_STAGE)?)
        };

        if warm.is_none() {
            if let Some(covered_attempt) = read(&attempt_path, MAX_ATTEMPT)? {
                if open_frame(&key, ATTEMPT_PURPOSE, &covered_attempt.1)? != attempt {
                    return Err(denied());
                }
                // A durable attempt without all of the private output is
                // ambiguous after restart. Preserve it without new key work.
                let covered_pending = read(&pending_path, MAX_PENDING)?.ok_or_else(denied)?;
                let result = verify_pending(&binding, invite, &key, limits, &covered_pending.1)?;
                cover_existing(&attempt_path, &covered_attempt)?;
                cover_existing(&pending_path, &covered_pending)?;
                inventory.sync_rejoin_catalog_parent(self)?;
                for (path, file) in [
                    (&attempt_path, &covered_attempt),
                    (&pending_path, &covered_pending),
                ] {
                    exact_held(path, &file.0, session_id(&file.0).map_err(io)?, &file.1)?;
                }
                require()?;
                return Ok(result);
            }
            absent(&pending_path)?;
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
            *warm = Some(Box::new(WarmRejoinPending {
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
            }));
        }
        let original = warm.as_mut().ok_or_else(denied)?;
        if original.binding != binding {
            return Err(denied());
        }
        require()?;
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
