//! Covered unsigned invite preparation on the actual retained native runtime.
//! No signed invite, SavedInvite receipt or network publication.
use super::*;
use crate::store::V2MemberSaveRefusal;
use catcoms_rt::RequestCancellation;

#[cfg(test)]
std::thread_local! {
    pub(super) static AFTER_COVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

impl<T: MeshTransport, R: CryptoRngCore> member::NativeTextMemberJob<T, R> {
    /// Prepare one founder invite's unsigned original through this same-row
    /// job. Reuses the paid member-operation capture; the Host pays its retained
    /// invite separately. The UI supplies only token/lifetime, never a route,
    /// policy, anchor, native observation, clock, nonce or signing callback.
    ///
    /// Returns (preimage hash, expiry) as historical DATA. It is deliberately
    /// NOT an invite or a SavedInvite acknowledgement. The isolated first
    /// journal profile covers this exact unsigned original; durable KeyAttempt,
    /// signature coverage and final publication admission are still required.
    pub fn prepare_invite_intent(
        mut self,
        token: [u8; 32],
        lifetime_ms: u64,
        negative: &RequestCancellation,
    ) -> Result<([u8; 32], u64), V2MemberSaveRefusal> {
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
        let result = body
            .work
            .prepare_native_invite_intent(
                driver,
                catcoms_sync::owner_v2::V2FounderInviteRequestData {
                    token,
                    lifetime_ms,
                    public_routing_plan:
                        catcoms_replication::chat::join_bindings::JoinPayloadKindData::RoutingPlan
                            .commitment_data(&route)
                            .map_err(|_| Error::Failed)?,
                },
                &route,
                &mut inner.keys,
                body.rng.as_mut().ok_or(Error::Unavailable)?,
            )
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
