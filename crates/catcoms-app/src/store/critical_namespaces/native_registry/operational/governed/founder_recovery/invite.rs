//! Restore the original TCP routing data and invoke the real cold issuer.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder::NativeFounderKeys;
use crate::store::NativeTextSavedInvite;
use catcoms_replication::chat::join_bindings::JoinPayloadKindData;
use catcoms_rt::Clock;
use catcoms_sync::owner_v2::V2FounderInviteRequestData;

impl NativeTextColdFounderSource {
    pub(in crate::store::critical_namespaces::native_registry) fn save_recovered_invite(
        &mut self,
        token: [u8; 32],
        lifetime_ms: u64,
        keys: &mut NativeFounderKeys,
        runtime: (&dyn Clock, &mut impl CryptoRngCore),
        caller: &RequestCancellation,
    ) -> Result<NativeTextSavedInvite, AppError> {
        self.restore_first_add(caller)?;
        let actor = self.actor.as_ref().ok_or_else(refused)?;
        let sdk = self.sdk.as_mut().ok_or_else(refused)?;
        let governed = self.governed.as_deref_mut().ok_or_else(refused)?;
        let pending = &mut self.invite;
        let (_, _route_fee) = self.work.recovered_founder_resources(32 * 1024)?;
        let saved =
            self.work
                .with_recovered_founder_store(&self.selection, &self.source, |store| {
                    let reserved = self.selection.network.restore_reservation(store)?;
                    let public = catcoms_net::seeded_transport_identity_data(&reserved.seed)
                        .map_err(|_| refused())?;
                    if (reserved.peer, reserved.port) != self.local
                        || reserved.port == 0
                        || public.phase0_peer_data() != reserved.peer
                        || actor.borrow()?.local_peer() != reserved.peer
                        || caller.is_cancelled()
                    {
                        return Err(refused());
                    }
                    let route = crate::store::native_text_invite::encode_reservation(
                        &self.selection.binding.group,
                        &public,
                        reserved.port,
                    )
                    .map_err(|_| refused())?;
                    drop(reserved);
                    let request = V2FounderInviteRequestData {
                        token,
                        lifetime_ms,
                        public_routing_plan: JoinPayloadKindData::RoutingPlan
                            .commitment_data(&route)
                            .map_err(|_| refused())?,
                    };
                    pending.save(
                        (store, &self.source, governed, sdk),
                        (request, &route),
                        keys,
                        runtime,
                        caller,
                    )
                })?;
        self.restore_first_add(caller)?;
        Ok(saved)
    }
}
