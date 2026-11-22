//! Invite preparation, signing and read-only recovery on the native runtime.
use super::*;
use crate::store::{NativeTextInviteRecovery, NativeTextSavedInvite, V2MemberSaveRefusal};
use catcoms_rt::RequestCancellation;
use catcoms_sync::V2JoinRequestPacketData;

enum InviteResult {
    Prepared(([u8; 32], u64)),
    Saved(NativeTextSavedInvite),
    Recovered(NativeTextInviteRecovery),
}

enum InviteOperation {
    Prepare,
    Save,
    Inspect,
}

#[cfg(test)]
std::thread_local! {
    pub(super) static AFTER_COVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

impl<T: MeshTransport, R: CryptoRngCore> member::NativeTextMemberJob<T, R> {
    pub(super) fn run_join_request(
        mut self,
        packet: &V2JoinRequestPacketData<'_>,
        cuts: [catcoms_rt::NativeChatPreparedKeyCut; 3],
        negative: &RequestCancellation,
    ) -> Result<crate::store::NativeTextSavedJoinResponse, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Error;
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.inner
            .require_current()
            .map_err(|_| Error::Unavailable)?;
        let inner = &mut self.inner;
        let body = inner.loan.body.as_mut().ok_or(Error::Unavailable)?;
        let driver = body.driver.as_mut().ok_or(Error::Unavailable)?;
        let route = body
            .network
            .as_ref()
            .ok_or(Error::Unavailable)?
            .invite_routing_plan_data(driver.binding_data().ok_or(Error::Unavailable)?)
            .map_err(|_| Error::Unavailable)?;
        let result = body.work.handle_received_join_request(
            driver,
            packet,
            &route,
            &inner.keys,
            cuts,
            body.rng.as_mut().ok_or(Error::Unavailable)?,
            negative,
        );
        inner.loan.recorded = true;
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        inner.require_current().map_err(|_| Error::Unavailable)?;
        result.map_err(|_| Error::Failed)
    }

    /// Prepare one founder invite's unsigned original through this same-row
    /// job. Reuses the paid member-operation capture; the Host pays its retained
    /// invite separately. The UI supplies only token/lifetime, never a route,
    /// policy, anchor, native observation, clock, nonce or signing callback.
    ///
    /// Returns the covered unsigned preimage hash and expiry as historical data.
    pub fn prepare_invite_intent(
        self,
        token: [u8; 32],
        lifetime_ms: u64,
        negative: &RequestCancellation,
    ) -> Result<([u8; 32], u64), V2MemberSaveRefusal> {
        match self.run_invite(token, lifetime_ms, negative, InviteOperation::Prepare)? {
            InviteResult::Prepared(result) => Ok(result),
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }

    /// Saves the signature for the same original token, settings and route.
    /// An exact retry uses the retained signature after any late failure.
    pub fn save_invite(
        self,
        token: [u8; 32],
        lifetime_ms: u64,
        negative: &RequestCancellation,
    ) -> Result<NativeTextSavedInvite, V2MemberSaveRefusal> {
        match self.run_invite(token, lifetime_ms, negative, InviteOperation::Save)? {
            InviteResult::Saved(result) => Ok(result),
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }

    /// Inspect only the original fixed records for this exact request. Even
    /// a complete signature here is data, with no save or publication receipt.
    pub fn inspect_invite(
        self,
        token: [u8; 32],
        lifetime_ms: u64,
        negative: &RequestCancellation,
    ) -> Result<NativeTextInviteRecovery, V2MemberSaveRefusal> {
        match self.run_invite(token, lifetime_ms, negative, InviteOperation::Inspect)? {
            InviteResult::Recovered(result) => Ok(result),
            _ => Err(V2MemberSaveRefusal::Failed),
        }
    }

    /// Authenticates both request signatures and exact package bytes, then
    /// matches the invitation to the original signed journal on this row.
    /// This reads only. Package validation uses the original source clock,
    /// and the owning Add transaction repeats it at actual SDK admission.
    pub fn check_received_join_request<'wire>(
        mut self,
        bytes: &'wire [u8],
        negative: &RequestCancellation,
    ) -> Result<V2JoinRequestPacketData<'wire>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Error;
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.inner
            .require_current()
            .map_err(|_| Error::Unavailable)?;
        let packet = V2JoinRequestPacketData::decode(bytes).map_err(|_| Error::Failed)?;
        let inner = &mut self.inner;
        let body = inner.loan.body.as_ref().ok_or(Error::Unavailable)?;
        let driver = body.driver.as_ref().ok_or(Error::Unavailable)?;
        let route = body
            .network
            .as_ref()
            .ok_or(Error::Unavailable)?
            .invite_routing_plan_data(driver.binding_data().ok_or(Error::Unavailable)?)
            .map_err(|_| Error::Unavailable)?;
        let result =
            body.work
                .require_received_join_request(driver, &packet, &route, &inner.keys, negative);
        inner.loan.recorded = true;
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        inner.require_current().map_err(|_| Error::Unavailable)?;
        result.map_err(|_| Error::Failed)?;
        Ok(packet)
    }

    fn run_invite(
        mut self,
        token: [u8; 32],
        lifetime_ms: u64,
        negative: &RequestCancellation,
        operation: InviteOperation,
    ) -> Result<InviteResult, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Error;
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.inner
            .require_current()
            .map_err(|_| Error::Unavailable)?;
        let inner = &mut self.inner;
        let body = inner.loan.body.as_mut().ok_or(Error::Unavailable)?;
        let driver = body.driver.as_mut().ok_or(Error::Unavailable)?;
        let binding = driver.binding_data().ok_or(Error::Unavailable)?;
        let route = body
            .network
            .as_ref()
            .ok_or(Error::Unavailable)?
            .invite_routing_plan_data(binding)
            .map_err(|_| Error::Unavailable)?;
        let request = catcoms_sync::owner_v2::V2FounderInviteRequestData {
            token,
            lifetime_ms,
            public_routing_plan:
                catcoms_replication::chat::join_bindings::JoinPayloadKindData::RoutingPlan
                    .commitment_data(&route)
                    .map_err(|_| Error::Failed)?,
        };
        let result = match operation {
            InviteOperation::Prepare => body
                .work
                .prepare_native_invite_intent(
                    driver,
                    request,
                    &route,
                    &mut inner.keys,
                    body.rng.as_mut().ok_or(Error::Unavailable)?,
                )
                .map(InviteResult::Prepared),
            InviteOperation::Save => body
                .work
                .save_native_invite(
                    driver,
                    request,
                    &route,
                    &mut inner.keys,
                    body.rng.as_mut().ok_or(Error::Unavailable)?,
                    negative,
                )
                .map(InviteResult::Saved),
            InviteOperation::Inspect => body
                .work
                .inspect_native_invite(driver, request, &route, &inner.keys, negative)
                .map(InviteResult::Recovered),
        }
        .map_err(|_| Error::Failed);
        // The original Host retains every draw before reporting a late error.
        // RuntimeLoan restoration precedes destruction of the outer key pins.
        inner.loan.recorded = true;
        #[cfg(test)]
        if result.is_ok() {
            if let Some(hook) = AFTER_COVER.with(|slot| slot.borrow_mut().take()) {
                hook();
            }
        }
        if negative.is_cancelled() {
            return Err(Error::Cancelled);
        }
        inner.require_current().map_err(|_| Error::Unavailable)?;
        result
    }
}
