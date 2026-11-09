//! The founder's whole closed genesis node acting as an ORDINARY member.
//!
//! The Profile founder is also a member with its own founder Admission. Its ordinary actions
//! are signed by the SAME retained MLS device held in this exact node, under a Core
//! `V2OrdinarySigningPlan` issued by a general verifier. This file deliberately shares nothing
//! with the founder A/I/S/B Append certification lane (`ordinary_append.rs`): no Header,
//! intent, Append plan, channel-secret export, seal or ratchet use.
//!
//! Why the genesis-only node is a sound source HERE (and only here): the founder Create
//! producer's closed source check is reused unchanged, so this node is epoch 0, sole member,
//! designated committer, with its live original scope reservation. The App host additionally
//! requires its selected operational O to still be the empty generation-1 promotion, so the
//! governing prefix is exactly genesis and the founder's Admission is the only current one.
//! When any of those stops holding, every seam below (or the host's guard) refuses. A joined
//! member's source is the separate `ChannelSync` seam in `owner_v2/member.rs`.
use super::ordinary_create;
use super::*;
use crate::owner_v2::member::sign_admitted_with_device;
use crate::owner_v2::{V2MemberDraftInputs, V2MemberSignRefusal};
use catcoms_replication::chat::V2OrdinarySigningGuard;

impl<T, R> UnboundV2GenesisNode<T, R> {
    /// Full closed-source negatives (consults SDK group state; may copy the public key).
    /// Pre-admission only, never under Core's held reader.
    fn ordinary_member_source(&self) -> Result<[u8; 32], V2MemberSignRefusal> {
        ordinary_create::source(self).map_err(|_| V2MemberSignRefusal::Source)?;
        let signer = self.seed.body.context.spec_data().signer;
        if !self.seed.body.device.retained_ed25519_key_matches(&signer) {
            return Err(V2MemberSignRefusal::Source);
        }
        Ok(signer)
    }

    /// The founder's full retained device key, which is also its founder Admission subject.
    /// The App host calls this inside its Current guard, BEFORE Core `admit_signing`.
    pub fn ordinary_member_signer_data(&self) -> Result<[u8; 32], V2MemberSignRefusal> {
        self.ordinary_member_source()
    }

    /// Key cut for an already-admitted Core guard. Under the held reader only allocation-free,
    /// SDK-free negatives run: the request was drawn on THIS exact seed (Weak pointer check),
    /// the genesis scope reservation is still live, and the Draft signer is this context's
    /// signer. Then the shared tail in `owner_v2::member` makes the one key call.
    pub fn sign_ordinary_admitted<'a>(
        &self,
        inputs: &V2MemberDraftInputs,
        guard: V2OrdinarySigningGuard<'a>,
    ) -> Result<V2OrdinarySigningGuard<'a>, V2MemberSignRefusal> {
        let s = &self.seed.body;
        inputs.require_founder_origin(&self.seed)?;
        if !s.metadata.scope.genesis_attachment_live()
            || s.context.spec_data().signer != *guard.signer_data()
        {
            return Err(V2MemberSignRefusal::Source);
        }
        let result = sign_admitted_with_device(&s.device, inputs, guard);
        // Test-only cancellation cut AFTER the raw was cached in the Plan, never before.
        #[cfg(test)]
        after_member_sign();
        result
    }
}

impl<T: MeshTransport, R: CryptoRngCore> UnboundV2GenesisNode<T, R> {
    /// Bind `slot` to this exact seed, then the request's one Clock read and one RNG nonce
    /// through THIS node's retained runtime seams (the same `RuntimeBody` the founder Create
    /// producer uses). Each result is cached in `slot` before the next negative; a cached slot
    /// calls neither seam again, and a slot bound to another seed refuses.
    pub fn draw_ordinary_draft_inputs(
        &mut self,
        slot: &mut V2MemberDraftInputs,
    ) -> Result<(u64, [u8; 16]), V2MemberSignRefusal> {
        self.ordinary_member_source()?;
        slot.bind_founder_origin(&self.seed)?;
        if slot.enter_clock()? {
            slot.cache_time(self._parts.body._clock.now_ms());
        }
        self.ordinary_member_source()?;
        if slot.enter_nonce()? {
            let mut nonce = [0; 16];
            self._parts.body._rng.fill_bytes(&mut nonce);
            slot.cache_nonce(nonce);
        }
        self.ordinary_member_source()?;
        slot.drawn()
    }
}

// Detached before invocation so a hook that cancels the source cannot re-enter a held cell.
#[cfg(test)]
std::thread_local! {
    static AFTER_MEMBER_SIGN: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn after_member_sign() {
    let hook = AFTER_MEMBER_SIGN.with(|h| h.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
pub(in crate::owner_v2) fn after_member_sign_for_test(hook: Box<dyn FnOnce()>) {
    AFTER_MEMBER_SIGN.with(|h| *h.borrow_mut() = Some(hook));
}
