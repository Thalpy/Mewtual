//! Optional A1 founder identity memory, never a dial/contact approval.
//! Only the original signed reservation preimage is saved. Existing pin,
//! Pending, Welcome, Current and native registration formats are unchanged.
use super::governance_layout::joined::JoinedPendingIo;
use super::v2_member_outbox::{
    fs::{
        finish_exact_write, observe, open_exact_write, read_bounded_held, remove_listed,
        require_exact_held, HeldDirectory, Presence, WriteMode,
    },
    member_outbox_key,
};
use crate::{
    store::{
        critical_namespaces::{read_pool::CatalogReadReservation, refused},
        frame, unframe, NativeTextInvitePacketData, NativeTextJoinedRegistrationData, ServerStore,
    },
    AppError,
};
use catcoms_crypto::{seal, unseal};
use catcoms_replication::chat::join_bindings::SignedInviteBindingData;
use catcoms_rt::{CryptoRngCore, PeerId};
use catcoms_storage::kept::SessionId;
use catcoms_sync::{V2JoinRequestPacketData, MAX_V2_JOIN_REQUEST_PACKET_BYTES};
use catcoms_wire::{Decoder, Encoder};
use std::fs::File;
use zeroize::Zeroizing;

const NAME: &str = "chat-v2-a1-founder-peer.bin";
const STAGE: &str = "chat-v2-a1-founder-peer.stage";
const TAG: &[u8] = b"mewtual/chat/a1-founder-peer/v1\0";
const KEY_DOMAIN: &str = "mewtual/chat/a1-founder-peer-key/v1";
const SCOPE_DOMAIN: &str = "mewtual/chat/a1-founder-peer-scope/v1";
const MAX_RESERVATION: usize = 1024;
const MAX_PLAIN: usize = 4 + TAG.len() + 2 + 36 + 4 + MAX_RESERVATION;
const MAX_FRAME: usize = MAX_PLAIN + 48;

#[cfg(test)]
thread_local! { static TORN_STAGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(test)]
fn take_torn_stage() -> bool {
    TORN_STAGE.with(|flag| flag.replace(false))
}

/// Exercise an actual admitted stage prefix in the original synchronous
/// producer. No hook supplies a source, registration or successful result.
#[cfg(test)]
pub(in crate::store) fn with_torn_founder_stage_for_test<T>(run: impl FnOnce() -> T) -> (T, bool) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            TORN_STAGE.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(TORN_STAGE.with(|flag| flag.replace(true)));
    let result = run();
    let reached = TORN_STAGE.with(|flag| !flag.get());
    (result, reached)
}
pub(in crate::store) const FOUNDER_ROUTE_FEE: usize = 8 * MAX_FRAME
    + 4 * MAX_V2_JOIN_REQUEST_PACKET_BYTES
    + 8 * (4096 + 256)
    + 2 * std::mem::size_of::<FounderRouteWrite>()
    + 2 * std::mem::size_of::<PreparedRouteFrame>();

/// Same-runtime warm retry owns its exact ciphertext and partial stage
/// File before the first fallible write. The fee drains after both.
pub(in crate::store) struct FounderRouteWrite {
    scope: [u8; 32],
    reservation: Zeroizing<Vec<u8>>,
    frame: Zeroizing<Vec<u8>>,
    selected: Option<(File, SessionId)>,
    stage: Option<(File, SessionId)>,
    _fee: CatalogReadReservation,
}

struct PreparedRouteFrame {
    frame: Zeroizing<Vec<u8>>,
    selected: Option<(File, SessionId)>,
    stage: Option<(File, SessionId)>,
}

impl ServerStore {
    pub(in crate::store) fn prepare_a1_founder_route(
        &self,
        pending: &JoinedPendingIo,
        invite: &NativeTextInvitePacketData<'_>,
        fee: CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<FounderRouteWrite, AppError> {
        let _lease = self._session.retain_for_physical_work();
        fee.validate(
            &self
                .governance_layout_policy
                .as_ref()
                .ok_or_else(refused)?
                .current
                .archive
                .pending
                .pool,
            FOUNDER_ROUTE_FEE,
        )?;
        let original = pending.warm_registration_data(self)?;
        self.with_warm_a1_invite(pending, original, |binding| {
            if binding.id_data() != invite.binding().id_data() {
                return Err(refused());
            }
            crate::store::native_text_invite::checked_founder_peer(
                binding,
                invite.reservation_bytes(),
            )
            .map(drop)
            .map_err(|_| refused())
        })?;
        let scope = scope(original);
        let vault_key = member_outbox_key(self)?;
        let key = scoped_key(&vault_key, scope);
        let root = HeldDirectory::open(self.dir.clone())?;
        let PreparedRouteFrame {
            frame,
            selected,
            stage,
        } = prepare_frame(&root, &key, scope, invite.reservation_bytes(), rng)?;
        self.with_warm_a1_invite(pending, original, |_| Ok(()))?;
        Ok(FounderRouteWrite {
            scope,
            reservation: Zeroizing::new(invite.reservation_bytes().to_vec()),
            frame,
            selected,
            stage,
            _fee: fee,
        })
    }

    pub(in crate::store) fn cover_a1_founder_route(
        &self,
        pending: &JoinedPendingIo,
        plan: &mut FounderRouteWrite,
        admit: impl FnOnce() -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let _lease = self._session.retain_for_physical_work();
        plan._fee.validate(
            &self
                .governance_layout_policy
                .as_ref()
                .ok_or_else(refused)?
                .current
                .archive
                .pending
                .pool,
            FOUNDER_ROUTE_FEE,
        )?;
        let original = pending.warm_registration_data(self)?;
        if scope(original) != plan.scope {
            return Err(refused());
        }
        self.with_warm_a1_invite(pending, original, |binding| {
            crate::store::native_text_invite::checked_founder_peer(binding, &plan.reservation)
                .map(drop)
                .map_err(|_| refused())
        })?;
        let root = HeldDirectory::open(self.dir.clone())?;
        let vault_key = member_outbox_key(self)?;
        let key = scoped_key(&vault_key, plan.scope);
        cover_frame(
            &root,
            &key,
            plan,
            || self.with_warm_a1_invite(pending, original, |_| Ok(())),
            admit,
        )
    }

    /// Optional cold read. The caller catches only this optional feature's
    /// error; the ordinary registered A1 still has its own startup proof.
    /// A stage alone is never a selected route. An exact byte-identical
    /// redundant stage may remain after create-new final output and a crash;
    /// reading that final retains/rechecks both Files and repairs neither.
    pub(in crate::store) fn read_a1_founder_peer(
        &self,
        original: NativeTextJoinedRegistrationData,
        fee: &CatalogReadReservation,
    ) -> Result<Option<PeerId>, AppError> {
        let _lease = self._session.retain_for_physical_work();
        fee.validate(
            &self
                .governance_layout_policy
                .as_ref()
                .ok_or_else(refused)?
                .current
                .archive
                .pending
                .pool,
            FOUNDER_ROUTE_FEE,
        )?;
        self.with_original_a1_invite(original, |_| Ok(()))?;
        let vault_key = member_outbox_key(self)?;
        let key = scoped_key(&vault_key, scope(original));
        let root = HeldDirectory::open(self.dir.clone())?;
        let Some(mut selected) = read_selected_frame(&root)? else {
            if observe(&root.child(STAGE))? != Presence::Absent {
                return Err(refused());
            }
            return Ok(None);
        };
        let reservation = decode_frame(&key, scope(original), &selected.frame)?;
        let peer = self.with_original_a1_invite(original, |binding| {
            crate::store::native_text_invite::checked_founder_peer(binding, &reservation)
                .map_err(|_| refused())
        })?;
        require_selected_frame(&root, &mut selected)?;
        Ok(Some(peer))
    }

    fn with_original_a1_invite<T>(
        &self,
        original: NativeTextJoinedRegistrationData,
        inspect: impl FnOnce(&SignedInviteBindingData) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        // ID9/A2 is a distinct authority family. Neither old file presence
        // nor an equal founder label may attach this A1 hint to it.
        crate::store::critical_namespaces::joined_pin::require_a1_founder_route_profile(self)?;
        if self.recovered_join_registration_data()? != Some(original) {
            return Err(refused());
        }
        let mut request = Zeroizing::new(Vec::with_capacity(MAX_V2_JOIN_REQUEST_PACKET_BYTES));
        self.copy_recovered_join_request(&mut request)?;
        with_binding_from_request(&request, original, inspect)
    }

    fn with_warm_a1_invite<T>(
        &self,
        pending: &JoinedPendingIo,
        original: NativeTextJoinedRegistrationData,
        inspect: impl FnOnce(&SignedInviteBindingData) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        // Warm outcome custody belongs to this actual Pending owner. It
        // must not be replaced by the Store's cold-only Welcome cache.
        crate::store::critical_namespaces::joined_pin::require_a1_founder_route_profile(self)?;
        if pending.warm_registration_data(self)? != original {
            return Err(refused());
        }
        let result = with_binding_from_request(pending.request_data(self)?, original, inspect)?;
        if pending.warm_registration_data(self)? != original {
            return Err(refused());
        }
        Ok(result)
    }
}

fn with_binding_from_request<T>(
    request: &[u8],
    original: NativeTextJoinedRegistrationData,
    inspect: impl FnOnce(&SignedInviteBindingData) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let packet = V2JoinRequestPacketData::decode(request).map_err(|_| refused())?;
    let add = packet.founder_add_data();
    let binding = add.invite;
    if binding.id_data() != original.invite || !binding.verify() {
        return Err(refused());
    }
    inspect(binding)
}

fn scope(value: NativeTextJoinedRegistrationData) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key(SCOPE_DOMAIN);
    for field in [
        value.context.vault,
        value.context.root,
        value.context.original_pending,
        value.context.pin,
        value.profile,
        value.signer,
        value.invocation,
        value.invite,
        value.request,
        *value.transport.0.as_bytes(),
        value.transport.2,
        *value.selected.governing_add.as_bytes(),
        *value.selected.own_admission.as_bytes(),
        value.selected.state,
        *value.selected.profile.as_bytes(),
        *value.selected.initial_tenure.as_bytes(),
        value.selected.profile_creation,
    ] {
        hash.update(&field);
    }
    hash.update(&value.group);
    hash.update(&value.transport.1.to_be_bytes());
    hash.update(&value.selected.epoch.to_be_bytes());
    *hash.finalize().as_bytes()
}

fn scoped_key(master: &[u8; 32], scope: [u8; 32]) -> Zeroizing<[u8; 32]> {
    let domain = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, master));
    Zeroizing::new(*blake3::keyed_hash(&domain, &scope).as_bytes())
}

fn decode_frame(
    key: &[u8; 32],
    scope: [u8; 32],
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if bytes.len() > MAX_FRAME {
        return Err(refused());
    }
    let blob = unframe(bytes).map_err(|_| refused())?;
    let plain = Zeroizing::new(unseal(key, &blob).map_err(|_| refused())?);
    if plain.len() > MAX_PLAIN {
        return Err(refused());
    }
    let mut d = Decoder::new(&plain);
    if d.get_bytes().map_err(|_| refused())? != TAG
        || d.get_u16().map_err(|_| refused())? != 1
        || d.get_bytes().map_err(|_| refused())? != scope
    {
        return Err(refused());
    }
    let reservation = d.get_bytes().map_err(|_| refused())?;
    if reservation.is_empty() || reservation.len() > MAX_RESERVATION {
        return Err(refused());
    }
    d.finish().map_err(|_| refused())?;
    Ok(Zeroizing::new(reservation.to_vec()))
}

type HeldFrame = ((File, SessionId), Zeroizing<Vec<u8>>);
fn read_frame(root: &HeldDirectory, name: &str) -> Result<Option<HeldFrame>, AppError> {
    if observe(&root.child(name))? == Presence::Absent {
        return Ok(None);
    }
    let (file, id, bytes) = read_bounded_held(&root.child(name), MAX_FRAME, false)?;
    Ok(Some(((file, id), bytes)))
}

/// Create-new final output can leave two complete names at a power cut.
/// The final supplies the selection; Stage is only redundant evidence and
/// must be exactly identical, not merely independently valid ciphertext.
/// Keep both native identities through the caller's authentication waits.
fn read_selected_frame(root: &HeldDirectory) -> Result<Option<PreparedRouteFrame>, AppError> {
    let Some((selected, frame)) = read_frame(root, NAME)? else {
        return Ok(None);
    };
    let stage = match read_frame(root, STAGE)? {
        Some((stage, bytes)) if bytes == frame => Some(stage),
        Some(_) => return Err(refused()),
        None => None,
    };
    let mut result = PreparedRouteFrame {
        frame,
        selected: Some(selected),
        stage,
    };
    require_selected_frame(root, &mut result)?;
    Ok(Some(result))
}

fn require_selected_frame(
    root: &HeldDirectory,
    selected: &mut PreparedRouteFrame,
) -> Result<(), AppError> {
    require_exact_held(
        &root.child(NAME),
        selected.selected.as_mut().ok_or_else(refused)?,
        &selected.frame,
    )?;
    if let Some(stage) = selected.stage.as_mut() {
        require_exact_held(&root.child(STAGE), stage, &selected.frame)?;
    } else if observe(&root.child(STAGE))? != Presence::Absent {
        return Err(refused());
    }
    root.require()
}

fn prepare_frame(
    root: &HeldDirectory,
    key: &[u8; 32],
    scope: [u8; 32],
    reservation: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<PreparedRouteFrame, AppError> {
    if reservation.is_empty() || reservation.len() > MAX_RESERVATION {
        return Err(refused());
    }
    if let Some(mut selected) = read_selected_frame(root)? {
        if decode_frame(key, scope, &selected.frame)?.as_slice() != reservation {
            return Err(refused());
        }
        require_selected_frame(root, &mut selected)?;
        return Ok(selected);
    }
    if let Some((mut stage, frame)) = read_frame(root, STAGE)? {
        if decode_frame(key, scope, &frame)?.as_slice() != reservation {
            return Err(refused());
        }
        require_exact_held(&root.child(STAGE), &mut stage, &frame)?;
        return Ok(PreparedRouteFrame {
            frame,
            selected: None,
            stage: Some(stage),
        });
    }
    let mut e = Encoder::with_capacity(MAX_PLAIN);
    e.put_bytes(TAG).map_err(|_| refused())?;
    e.put_u16(1);
    e.put_bytes(&scope).map_err(|_| refused())?;
    e.put_bytes(reservation).map_err(|_| refused())?;
    let plain = Zeroizing::new(e.finish());
    let blob = seal(key, &plain, rng).map_err(|_| refused())?;
    Ok(PreparedRouteFrame {
        frame: Zeroizing::new(frame(&blob)),
        selected: None,
        stage: None,
    })
}

fn cover_frame(
    root: &HeldDirectory,
    key: &[u8; 32],
    plan: &mut FounderRouteWrite,
    mut current: impl FnMut() -> Result<(), AppError>,
    admit: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    current()?;
    root.require()?;
    if decode_frame(key, plan.scope, &plan.frame)?.as_slice() != plan.reservation.as_slice() {
        return Err(refused());
    }
    if plan.selected.is_none() {
        if observe(&root.child(NAME))? != Presence::Absent {
            return Err(refused());
        }
        if plan.stage.is_none() && observe(&root.child(STAGE))? != Presence::Absent {
            return Err(refused());
        }
        // First writable-open edge follows all prior blocking proof/read
        // work. Only this exact bounded write may finish after cancellation.
        admit()?;
        if plan.stage.is_none() {
            plan.stage = Some(open_exact_write(&root.child(STAGE), WriteMode::Create)?);
        }
        let stage = plan.stage.as_mut().ok_or_else(refused)?;
        #[cfg(test)]
        if take_torn_stage() {
            finish_exact_write(
                &root.child(STAGE),
                stage,
                &plan.frame[..plan.frame.len() / 2],
            )?;
            root.sync()?;
            return Err(refused());
        }
        cover_same_file(root, STAGE, stage, &plan.frame)?;
        root.sync()?;
        current()?;
        require_exact_held(&root.child(STAGE), stage, &plan.frame)?;
        // Create-new is the atomic no-replace boundary. A destination that
        // appears after the last source check is preserved on every OS.
        // Retain the actual final File before the first byte write, so a
        // warm known partial final can finish without a fresh ciphertext.
        plan.selected = Some(open_exact_write(&root.child(NAME), WriteMode::Create)?);
    } else {
        if let Some(stage) = plan.stage.as_mut() {
            require_exact_held(&root.child(STAGE), stage, &plan.frame)?;
        } else if observe(&root.child(STAGE))? != Presence::Absent {
            return Err(refused());
        }
        admit()?;
    }
    let selected = plan.selected.as_mut().ok_or_else(refused)?;
    cover_same_file(root, NAME, selected, &plan.frame)?;
    root.sync()?;
    current()?;
    require_exact_held(&root.child(NAME), selected, &plan.frame)?;
    if let Some(stage) = plan.stage.as_mut() {
        require_exact_held(&root.child(STAGE), stage, &plan.frame)?;
        current()?;
        require_exact_held(&root.child(NAME), selected, &plan.frame)?;
        require_exact_held(&root.child(STAGE), stage, &plan.frame)?;
        remove_listed(&root.child(STAGE))?;
        // Close our last stage handle before the Windows absence barrier.
        drop(plan.stage.take());
        root.sync()?;
    }
    if observe(&root.child(STAGE))? != Presence::Absent {
        return Err(refused());
    }
    require_exact_held(&root.child(NAME), selected, &plan.frame)
}

fn cover_same_file(
    root: &HeldDirectory,
    name: &str,
    original: &mut (File, SessionId),
    expected: &[u8],
) -> Result<(), AppError> {
    // Read-only observed handles need a writer to the SAME object for
    // Windows flush. Exact expected bytes include the cached ciphertext;
    // semantic equality cannot adopt an in-place authenticated replacement.
    let mut writer = open_exact_write(&root.child(name), WriteMode::Existing)?;
    if writer.1 != original.1 {
        return Err(refused());
    }
    finish_exact_write(&root.child(name), &mut writer, expected)?;
    require_exact_held(&root.child(name), original, expected)
}

#[cfg(test)]
mod tests;
