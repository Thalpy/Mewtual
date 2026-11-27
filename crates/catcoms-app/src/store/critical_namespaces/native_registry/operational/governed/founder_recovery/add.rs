//! A cold founder's existing native runtime owns one exact Add until the
//! signed journal, MLS and Core have selected a durable matching successor.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::persistence::v2_text::{prepare_sdk_admission, PreparedColdGovernedAdd};
use crate::store::{persistence::RecoveredFounderInvite, ServerStore};
use catcoms_rt::{Clock, NativeChatKeyPurpose, NativeChatPreparedKeyCut};
use catcoms_sync::{owner_v2::V2FounderAddNativeStage, V2JoinRequestPacketData};
use zeroize::Zeroizing;

impl NativeTextColdFounderSource {
    /// Finish a suspended exact request or prove whether a new request belongs
    /// to the original covered invitation. The native caller holds the
    /// row's exclusive runtime and the original UI command through this work.
    pub(in crate::store::critical_namespaces::native_registry) fn handle_recovered_join(
        &mut self,
        packet: &V2JoinRequestPacketData<'_>,
        (keys, cuts): (&mut NativeFounderKeys, [NativeChatPreparedKeyCut; 3]),
        (clock, rng): (&dyn Clock, &mut impl CryptoRngCore),
        caller: &RequestCancellation,
    ) -> Result<Vec<u8>, AppError> {
        if caller.is_cancelled() || self.index.is_some() {
            return Err(refused());
        }
        if self.add.is_none() {
            if let Some(delivery) = self.classify_recovered_join_delivery(packet, caller)? {
                return Ok(delivery);
            }
            self.require_recovered_join_packet(packet, keys, clock, caller)?;
            let context = self.outbox_context()?;
            let parent = self.governed.as_deref_mut().ok_or_else(refused)?;
            let sdk = self.sdk.as_ref().ok_or_else(refused)?;
            self.add = Some(Box::new(self.work.with_recovered_founder_store(
                &self.selection,
                &self.source,
                |store| {
                    if let Some(lane) = &self.member {
                        lane.require_quiescent_for_add()?;
                    }
                    let key = Zeroizing::new(store.keys.db_key()?);
                    store
                        .member_outbox_mount()?
                        .require_quiescent_for_add(store, context, &key, rng)
                        .map_err(|_| refused())?;
                    let fetch = sdk.recovered_add_fetch_verifier_data(
                        parent.member_parts()?.0,
                        self.source.data(),
                    )?;
                    PreparedColdGovernedAdd::prepare(
                        store,
                        &self.source,
                        parent,
                        packet,
                        fetch,
                        rng,
                    )
                },
            )?));
        }
        let (native, command, _) = keys.member_entry();
        command
            .preflight(native, NativeChatKeyPurpose::MemberAction)
            .map_err(|_| refused())?;
        let current = command.observe_current();
        let require_native = || {
            if caller.is_cancelled() || !current.is_current() {
                return Err(refused());
            }
            Ok(())
        };
        let pending = self.add.as_deref_mut().ok_or_else(refused)?;
        pending.require_original_packet(&self.source, packet)?;
        let parent = self.governed.as_deref_mut().ok_or_else(refused)?;
        let sdk = self.sdk.as_mut().ok_or_else(refused)?;
        if pending.selected_head_data().is_none() {
            let actor = self.actor.as_ref().ok_or_else(refused)?;
            let (_, _route_fee) = self.work.recovered_founder_resources(32 * 1024)?;
            self.work
                .with_recovered_founder_store(&self.selection, &self.source, |store| {
                    require_native()?;
                    let route = super::invite::original_invite_route(
                        store,
                        &self.selection,
                        self.local,
                        actor,
                        caller,
                    )?;
                    let mut invitation = RecoveredFounderInvite::capture_join_invite(
                        parent.invite_parent_coordinates(store, &self.source)?,
                        &self.source,
                        packet,
                        &route,
                    )?;
                    let mut require_invite = || {
                        require_native()?;
                        invitation.require_current()
                    };
                    if !pending.has_started_selection() {
                        cover_add_controls(
                            (store, &self.source, parent),
                            (sdk, pending),
                            packet,
                            (native.clone(), cuts),
                            (clock, rng),
                            caller,
                            &mut require_invite,
                        )?;
                    }
                    let mut candidate = Some(parent.derive_add_selection(sdk, &self.source)?);
                    let install = sdk.prepare_recovered_add_install(
                        parent.member_parts()?.0,
                        self.source.data(),
                    )?;
                    pending.select_archive(
                        (store, &self.source, parent),
                        (install, &mut candidate),
                        caller,
                        require_invite,
                    )?;
                    Ok(())
                })?;
        }
        // Rebind only the new physical archive. The identical advanced Core
        // stays paired with this SDK. On a late refusal the retained writer
        // remembers selection, so a retry does not attempt the Add again.
        let head = pending.selected_head_data().ok_or_else(refused)?;
        let installed = sdk.fingerprint_data();
        self.work
            .with_recovered_founder_store(&self.selection, &self.source, |store| {
                require_native()?;
                *parent = parent.refresh_selected_add(
                    store,
                    &self.source,
                    self.selection.binding.pin,
                    head,
                    installed,
                )?;
                Ok(())
            })?;
        // Dispose retired key Ends/SDK state outside the physical writer.
        let retired = sdk.take_retired_recovered_add_install()?;
        drop(self.member.take());
        drop(self.received.take());
        drop(retired);
        drop(self.add.take());
        self.invite = Default::default();
        self.classify_recovered_join_delivery(packet, caller)?
            .ok_or_else(refused)
    }
}

fn cover_add_controls(
    (store, source, parent): (
        &ServerStore,
        &RetainedBootstrapSourceData,
        &mut HeldColdFounderAdd,
    ),
    (sdk, pending): (
        &mut PreparedV2FounderRecoveredAdd,
        &mut PreparedColdGovernedAdd,
    ),
    packet: &V2JoinRequestPacketData<'_>,
    (native, [add, header, welcome]): (
        catcoms_rt::NativeChatSourceObservation,
        [NativeChatPreparedKeyCut; 3],
    ),
    (clock, rng): (&dyn Clock, &mut impl CryptoRngCore),
    caller: &RequestCancellation,
    require_invite: &mut impl FnMut() -> Result<(), AppError>,
) -> Result<(), AppError> {
    require_invite()?;
    pending.cover_reservation(store, source, parent, caller)?;
    let request = packet.founder_add_data();
    let [retry_sdk, retry_header, retry_welcome] = if sdk.has_recovered_add_pending() {
        [Some(add), Some(header), Some(welcome)]
    } else {
        let admission = prepare_sdk_admission(store)?;
        sdk.prepare_recovered_add_data(
            (parent.member_parts()?.0, source.data()),
            request,
            (native, admission, [add, header, welcome]),
        )
        .map_err(|_| refused())?;
        [None, None, None]
    };
    if sdk.completed_recovered_add_data().is_err() {
        if let Some(cut) = retry_sdk {
            drop(sdk.prepare_recovered_add_native_stage(V2FounderAddNativeStage::Sdk, cut));
        }
        pending.prepare_sdk_entry(store, source, rng)?;
        pending.cover_sdk_entry(store, source, parent, caller)?;
        require_invite()?;
        // MLS snapshots, Add and routing serialization run outside Root.
        sdk.execute_recovered_add_native(
            (parent.member_parts()?.0, source.data()),
            request,
            (clock, rng),
        )?;
    }
    sdk.prepare_recovered_admission_data(parent.member_parts()?.0, source.data())?;
    pending.prepare_outcome(store, source, &sdk.recovered_add_outcome_data()?, rng)?;
    pending.cover_outcome(store, source, parent, caller)?;
    if sdk.raw_recovered_add_header_data().is_err() {
        if let Some(cut) = retry_header {
            drop(
                sdk.prepare_recovered_add_native_stage(
                    V2FounderAddNativeStage::GoverningHeader,
                    cut,
                ),
            );
        }
        pending.prepare_header_attempt(store, source, rng)?;
        let mut primitive =
            sdk.recovered_add_signing_parts(parent.member_parts()?.0, source.data())?;
        pending.cover_header_attempt(
            store,
            source,
            parent,
            caller,
            (&mut *require_invite, || Ok(primitive.sign_header_native()?)),
        )?;
    }
    pending.prepare_raw_header(
        store,
        source,
        &sdk.recovered_add_outcome_data()?,
        sdk.raw_recovered_add_header_data()?,
        rng,
    )?;
    pending.cover_raw_header(store, source, parent, caller)?;
    sdk.prepare_recovered_welcome_data()?;
    let outcome = sdk.recovered_add_outcome_data()?;
    let signed = sdk.recovered_add_header_data()?;
    pending.prepare_governing_record(store, source, (&outcome, signed), rng)?;
    pending.cover_governing_record(store, source, parent, caller)?;
    let (_, intent) = sdk.recovered_welcome_intent_data()?;
    pending.prepare_welcome_intent(store, source, &outcome, intent, rng)?;
    pending.cover_welcome_intent(store, source, parent, caller)?;
    if sdk.raw_recovered_welcome_data().is_err() {
        if let Some(cut) = retry_welcome {
            drop(sdk.prepare_recovered_add_native_stage(V2FounderAddNativeStage::Welcome, cut));
        }
        pending.prepare_welcome_attempt(store, source, rng)?;
        let mut primitive =
            sdk.recovered_add_signing_parts(parent.member_parts()?.0, source.data())?;
        pending.cover_welcome_attempt(
            store,
            source,
            parent,
            caller,
            (
                &mut *require_invite,
                || Ok(primitive.sign_welcome_native()?),
            ),
        )?;
    }
    let (_, intent) = sdk.recovered_welcome_intent_data()?;
    pending.prepare_raw_welcome(
        store,
        source,
        intent,
        sdk.raw_recovered_welcome_data()?,
        rng,
    )?;
    pending.cover_raw_welcome(store, source, parent, caller)?;
    sdk.materialize_recovered_welcome_data()?;
    let (_, intent) = sdk.recovered_welcome_intent_data()?;
    let outcome = sdk.recovered_add_outcome_data()?;
    let (signed, response) = sdk.recovered_join_response_data()?;
    pending.prepare_response(
        (store, source, parent),
        (intent, &outcome, signed, &response),
        rng,
    )?;
    pending.cover_response(store, source, parent, caller)?;
    pending.prepare_archive((store, source, parent), &outcome, rng)
}
