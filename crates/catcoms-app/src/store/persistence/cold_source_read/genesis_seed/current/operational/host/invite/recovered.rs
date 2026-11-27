//! A covered invitation on the original cold founder's selected parent.
use super::*;
use crate::store::persistence::v2_text::HeldColdFounderAdd;
use crate::store::{RetainedBootstrapSourceData, ServerStore};
use catcoms_rt::{Clock, CryptoRngCore, NativeChatKeyPurpose};
use catcoms_sync::owner_v2::PreparedV2FounderRecoveredAdd;

/// One issuing runtime may finish only the files it created. A later opener
/// can reconstruct a complete result or refuse without adopting an attempt.
#[derive(Default)]
pub(in crate::store) struct RecoveredFounderInvite {
    pending: Option<Box<PendingInvite>>,
}

impl RecoveredFounderInvite {
    pub(in crate::store) fn save(
        &mut self,
        (store, source, governed, sdk): (
            &ServerStore,
            &RetainedBootstrapSourceData,
            &mut HeldColdFounderAdd,
            &mut PreparedV2FounderRecoveredAdd,
        ),
        (request, route): (V2FounderInviteRequestData, &[u8]),
        keys: &mut NativeFounderKeys,
        (clock, rng): (&dyn Clock, &mut impl CryptoRngCore),
        caller: &RequestCancellation,
    ) -> Result<NativeTextSavedInvite, AppError> {
        use catcoms_replication::chat::join_bindings::JoinPayloadKindData;

        if caller.is_cancelled()
            || route.is_empty()
            || route.len() > 1024
            || request.token == [0; 32]
            || request.lifetime_ms == 0
            || request.lifetime_ms > MAX_FOUNDER_INVITE_LIFETIME_MS
            || JoinPayloadKindData::RoutingPlan
                .commitment_data(route)
                .map_err(|_| refused())?
                != request.public_routing_plan
        {
            return Err(refused());
        }
        let native = keys.original.clone();
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        governed.with_checked_invite_parent(store, source, |parent, verifier, require| {
            let (head, _) = parent.context_data();
            let require_live = |keys: &mut NativeFounderKeys| {
                if caller.is_cancelled() {
                    return Err(refused());
                }
                keys.invite_cut()
                    .preflight(&native, NativeChatKeyPurpose::JoinInvite)
                    .map_err(|_| refused())
            };
            require_live(keys)?;
            if self.pending.is_none() {
                let mut read = InviteRecoveryRead::prepare(store, Some(head))?;
                read.capture_cold(parent)?;
                let saved = read.recover_cold(request, route, source.data())?;
                require()?;
                read.require_current_cold(parent)?;
                if let Some(signed) = saved {
                    let policy = sdk.read_recovered_invite_policy(
                        verifier,
                        source.data(),
                        &native,
                        keys.invite_cut(),
                    )?;
                    let packet =
                        PreparedNativeTextInvitePacket::prepare(pool, &signed, &policy, route)?;
                    require_live(keys)?;
                    read.cover_current_cold(parent)?;
                    require_live(keys)?;
                    return Ok(packet.into_saved());
                }
                self.pending = Some(Box::new(PendingInvite {
                    request,
                    routing_plan: route.to_vec(),
                    original: None,
                    signing: InviteSigningJournal::default(),
                    journal: InviteIntentJournal::for_parent(Some(head)),
                    fee: pool.try_reserve(INVITE_BYTES)?,
                }));
            }
            let pending = self.pending.as_deref_mut().ok_or_else(refused)?;
            if pending.request != request
                || pending.routing_plan != route
                || pending.journal.parent_data() != Some(head)
            {
                return Err(refused());
            }
            pending.fee.validate(pool, INVITE_BYTES)?;
            let intent = &mut pending.journal;
            intent.bind_cold(parent)?;
            if intent.lookup_bound(store, request, route)?.is_some() && pending.original.is_none() {
                return Err(refused());
            }
            sdk.prepare_recovered_invite_data(
                verifier,
                source.data(),
                request,
                (&native, keys.invite_cut()),
                (clock, &mut *rng),
                &mut pending.original,
            )?;
            let prepared = pending.original.as_mut().ok_or_else(refused)?;
            let (_, bytes) = prepared.unsigned_data()?;
            intent.prepare_data(store, request, route, bytes, rng)?;
            require_live(keys)?;
            require()?;
            intent.bind_cold(parent)?;
            intent.cover_bound(request, route, bytes)?;

            if prepared.raw_signature_data().is_err() {
                pending.signing.prepare_attempt(intent, store, rng)?;
                require_live(keys)?;
                require()?;
                pending.signing.cover_bound_attempt(intent)?;
                require()?;
                pending.signing.require_bound_attempt(intent)?;
                require_live(keys)?;
                sdk.sign_recovered_invite_native(
                    verifier,
                    source.data(),
                    prepared,
                    keys.invite_entry(),
                )?;
            }
            prepared.materialize_data()?;
            pending
                .signing
                .prepare_raw(intent, store, prepared.raw_signature_data()?, rng)?;
            let packet = PreparedNativeTextInvitePacket::prepare(
                pool,
                prepared.signed_data()?,
                prepared.policy_bytes_data()?,
                route,
            );
            require_live(keys)?;
            require()?;
            pending.signing.cover_bound_raw(intent)?;
            require_live(keys)?;
            packet.map(PreparedNativeTextInvitePacket::into_saved)
        })
    }
}
