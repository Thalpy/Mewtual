//! One original founder invitation, from retained Clock/RNG/key custody.
//!
//! This is the private producer's DATA half. Root must bind the supplied native
//! observation to THIS original node, authenticate its routing plan and current
//! founder authority, and cover the unsigned intent BEFORE calling the key edge.
//! No Saved receipt, nonce consumption or publication authority is issued here.
use super::*;
use catcoms_replication::chat::join_bindings::{
    GenesisAnchorBindingData, InviteBindingSpecData, JoinPayloadKindData, SignedInviteBindingData,
    MAX_JOIN_BINDING_BYTES,
};
use catcoms_rt::{
    NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatKeyRefusal, NativeChatPreparedKeyCut,
    NativeChatSourceObservation,
};

/// Initial issuer profile. The original retained Clock fixes the expiry once;
/// a retry never extends it. This is a bound, not a claim of clock accuracy.
pub const MAX_FOUNDER_INVITE_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// Local request DATA. `token` is distinct from the random wire nonce. The
/// route commitment must come from Root's actual original routing reservation;
/// a nonzero caller hash establishes no route or invite authority by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct V2FounderInviteRequestData {
    pub token: [u8; 32],
    pub lifetime_ms: u64,
    pub public_routing_plan: [u8; 32],
}
impl V2FounderInviteRequestData {
    fn validate(self) -> Result<(), SyncError> {
        if self.token == [0; 32]
            || self.public_routing_plan == [0; 32]
            || self.lifetime_ms == 0
            || self.lifetime_ms > MAX_FOUNDER_INVITE_LIFETIME_MS
        {
            return Err(unavailable());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Draw {
    Captured,
    ClockEntered,
    Timed,
    NonceEntered,
    Drawn,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Unstarted,
    Attempting,
    KnownLocalRefusal,
    Entered,
    Raw,
}

/// Same-operation owner, installed in Root before any Clock/RNG/key seam. No
/// raw SDK, native issuer, setter, cloning or decoded pending adoption exists.
/// Unknown entered seams remain sticky. Cached output is historical DATA even
/// after source revocation and never a permission to publish or run Add.
pub struct PreparedV2FounderInviteData {
    request: V2FounderInviteRequestData,
    original: OriginalV2GenesisSeedObservation,
    native: NativeChatSourceObservation,
    draw: Draw,
    expires_at_ms: Option<u64>,
    nonce: Option<[u8; 16]>,
    spec: Option<InviteBindingSpecData>,
    policy_bytes: Option<Vec<u8>>,
    bytes: Option<Vec<u8>>,
    key: Key,
    attempts: u64,
    raw: Option<[u8; 64]>,
    signed: Option<SignedInviteBindingData>,
}
impl std::fmt::Debug for PreparedV2FounderInviteData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreparedV2FounderInviteData { .. }")
    }
}

/// Named logical buffers/layout/signature scratch, independent of actual Root
/// payment, immutable journal quotas, SDK memory and native metadata custody.
/// Reserve before the plan/spec/preimage allocations; retain through output.
pub const FOUNDER_INVITE_DATA_BYTES: usize = 4 * std::mem::size_of::<PreparedV2FounderInviteData>()
    + 12 * MAX_JOIN_BINDING_BYTES
    + 4 * MAX_GROUP_ID_BYTES
    + 16 * 1024;

impl PreparedV2FounderInviteData {
    /// The exact Root-local request, not the random invite nonce or wire ID.
    pub fn request_data(&self) -> V2FounderInviteRequestData {
        self.request
    }

    /// Exact preimage DATA for Root's covering intent. No Clock, RNG, encoding
    /// or signature occurs on this loan, including after cancellation.
    pub fn unsigned_data(&self) -> Result<(&InviteBindingSpecData, &[u8]), SyncError> {
        Ok((
            self.spec.as_ref().ok_or_else(unavailable)?,
            self.bytes.as_deref().ok_or_else(unavailable)?,
        ))
    }

    /// The exact original policy envelope committed by the invitation.
    pub fn policy_bytes_data(&self) -> Result<&[u8], SyncError> {
        self.policy_bytes.as_deref().ok_or_else(unavailable)
    }

    /// Retained signature for the original covering journal, not a Saved proof.
    pub fn raw_signature_data(&self) -> Result<&[u8; 64], SyncError> {
        self.raw.as_ref().ok_or_else(unavailable)
    }

    /// Reconstruct and verify OUTSIDE writers using only the retained raw64.
    /// Failure leaves the original bytes/raw intact, without a new key call.
    pub fn materialize_data(&mut self) -> Result<(), SyncError> {
        if self.signed.is_some() {
            return Ok(());
        }
        let raw = self.raw.ok_or_else(unavailable)?;
        let (spec, expected) = self.unsigned_data()?;
        let signed = SignedInviteBindingData::sign_using(spec.clone(), |bytes| {
            if bytes == expected {
                Ok(raw)
            } else {
                Err(catcoms_replication::chat::ChatError::Dependency)
            }
        })
        .map_err(|_| unavailable())?;
        self.signed = Some(signed);
        Ok(())
    }

    /// Historical signed DATA only. The owning Root caller must separately
    /// prove covering persistence and final actual native/current admission.
    pub fn signed_data(&self) -> Result<&SignedInviteBindingData, SyncError> {
        self.signed.as_ref().ok_or_else(unavailable)
    }
}

impl<T, R> UnboundV2GenesisNode<T, R> {
    fn require_invite_source(&self) -> Result<(), SyncError> {
        ordinary_create::source(self).map_err(|_| unavailable())?;
        if self._parts.body.warm.is_none() || self._parts.body.fresh.is_none() {
            return Err(unavailable());
        }
        Ok(())
    }

    /// Full SDK/source check OUTSIDE final writers. Root repeats its real
    /// selected-parent/verifier checks at entry; this DATA preflight is not a
    /// substitute for them and supplies no positive current capability.
    pub fn preflight_invite_signing_data(
        &self,
        prepared: &PreparedV2FounderInviteData,
    ) -> Result<(), SyncError> {
        prepared.original.require_same_seed_data(&self.seed)?;
        self.require_invite_source()?;
        prepared.unsigned_data()?;
        Ok(())
    }

    /// Stable original-lifetime negatives only. The full SDK graph, roster and
    /// public-key allocation are deliberately confined to the preflight above.
    fn require_invite_stable_negative(&self) -> Result<(), SyncError> {
        let s = &self.seed.body;
        let (Some(warm), Some(fresh)) = (
            self._parts.body.warm.as_ref(),
            self._parts.body.fresh.as_ref(),
        ) else {
            return Err(unavailable());
        };
        let spec = s.context.spec_data();
        if warm.source != s.source
            || warm.context != s.context
            || !s.metadata.scope.genesis_attachment_live()
            || !warm.scope.matches_genesis_attachment(&s.metadata.scope)
            || !warm
                .fresh
                .matches_governed_origin_life(fresh, spec.group, &spec.signer)
        {
            return Err(unavailable());
        }
        Ok(())
    }

    /// Root calls only after full SDK preflight OUTSIDE final writers, covering
    /// this exact unsigned intent and checking
    /// the SAME original source/parent/verifier. The original native binding was
    /// frozen at prepare; a matching foreign observation+cut cannot replace it.
    /// Caller retains cuts and actual Ends OUTSIDE every writer until return.
    /// A cached signature bypasses all key work and is historical DATA only.
    pub fn sign_invite_native_data(
        &self,
        prepared: &mut PreparedV2FounderInviteData,
        cut: &mut NativeChatPreparedKeyCut,
        ends: &mut NativeChatKeyEnds,
    ) -> Result<(), SyncError> {
        prepared.original.require_same_seed_data(&self.seed)?;
        if prepared.raw.is_some() {
            return Ok(());
        }
        self.require_invite_stable_negative()?;
        let signer = self.seed.body.context.spec_data().signer;
        let (spec, _) = prepared.unsigned_data()?;
        if spec.inviter != signer
            || !self.seed.body.device.retained_ed25519_key_matches(&signer)
            || !matches!(prepared.key, Key::Unstarted | Key::KnownLocalRefusal)
        {
            return Err(unavailable());
        }
        // Only a fresh cut may enter. Even a known local refusal may have
        // partially charged UI custody; the owner must retire that old cut/End
        // outside writers and obtain actual fresh current authorization first.
        cut.require_never_entered_data(&prepared.native, NativeChatKeyPurpose::JoinInvite)
            .map_err(|_| unavailable())?;
        prepared.attempts = prepared.attempts.checked_add(1).ok_or_else(unavailable)?;
        prepared.key = Key::Attempting; // before RT: a panic is not a refusal witness
        #[cfg(test)]
        test_cut(TestCut::NativeAttempting);
        match cut.enter(&prepared.native, NativeChatKeyPurpose::JoinInvite, ends) {
            Ok(()) => prepared.key = Key::Entered,
            Err(NativeChatKeyRefusal::Busy | NativeChatKeyRefusal::Unavailable) => {
                prepared.key = Key::KnownLocalRefusal;
                return Err(unavailable());
            }
            Err(_) => return Err(unavailable()),
        }
        #[cfg(test)]
        test_cut(TestCut::KeyEntered);
        let raw = self
            .seed
            .body
            .device
            .sign_retained_ed25519_bounded(
                &signer,
                prepared.bytes.as_deref().ok_or_else(unavailable)?,
                MAX_JOIN_BINDING_BYTES,
            )
            .map_err(|_| unavailable())?;
        prepared.raw = Some(raw); // before any late check or materializer
        prepared.key = Key::Raw;
        #[cfg(test)]
        test_cut(TestCut::RawCached);
        Ok(())
    }
}

impl<T: MeshTransport, R: CryptoRngCore> UnboundV2GenesisNode<T, R> {
    /// Install a paid original before invoking its retained Clock/RNG. Retries
    /// compare every request field and original source BEFORE any cached result.
    /// Root supplies and verifies the real node/native association; these Sync
    /// DATA types cannot authenticate a native source by themselves.
    pub fn prepare_invite_data(
        &mut self,
        request: V2FounderInviteRequestData,
        native: &NativeChatSourceObservation,
        slot: &mut Option<PreparedV2FounderInviteData>,
    ) -> Result<(), SyncError> {
        request.validate()?;
        if let Some(p) = slot.as_ref() {
            p.original.require_same_seed_data(&self.seed)?;
            if p.request != request || !p.native.same_original(native) {
                return Err(unavailable());
            }
            if p.bytes.is_some() {
                return Ok(()); // frozen historical DATA; no authority or fresh draw
            }
        }
        self.require_invite_source()?;
        if slot.is_none() {
            *slot = Some(PreparedV2FounderInviteData {
                request,
                original: self.seed.observe_original_seed_data(),
                native: native.clone(),
                draw: Draw::Captured,
                expires_at_ms: None,
                nonce: None,
                spec: None,
                policy_bytes: None,
                bytes: None,
                key: Key::Unstarted,
                attempts: 0,
                raw: None,
                signed: None,
            });
        }
        let p = slot.as_mut().ok_or_else(unavailable)?;
        if p.draw == Draw::Captured {
            p.draw = Draw::ClockEntered;
            #[cfg(test)]
            test_cut(TestCut::ClockEntered);
            let now = self._parts.body._clock.now_ms();
            p.expires_at_ms = Some(
                now.checked_add(request.lifetime_ms)
                    .ok_or_else(unavailable)?,
            );
            p.draw = Draw::Timed;
        }
        if p.draw == Draw::Timed {
            p.draw = Draw::NonceEntered;
            #[cfg(test)]
            test_cut(TestCut::NonceEntered);
            let mut nonce = [0; 16];
            self._parts.body._rng.fill_bytes(&mut nonce);
            p.nonce = Some(nonce);
            p.draw = Draw::Drawn;
        }
        if p.draw != Draw::Drawn {
            return Err(unavailable()); // unknown Clock/RNG entry cannot redraw
        }
        if p.spec.is_none() {
            let s = &self.seed.body;
            let g = &s.genesis;
            let policy_bytes = s.metadata.policy.as_ref().ok_or_else(unavailable)?.encode();
            if policy_bytes.is_empty() || policy_bytes.capacity() > MAX_JOIN_BINDING_BYTES {
                return Err(unavailable());
            }
            let spec = InviteBindingSpecData {
                anchor: GenesisAnchorBindingData {
                    group: g.profile.spec().group.clone(),
                    founder: g.profile.spec().founder,
                    profile: *g.profile.id().as_bytes(),
                    admission: *g.admission.admission_id().as_bytes(),
                    tenure: *g.profile.tenure().as_bytes(),
                    header: *g.header.id().as_bytes(),
                    state: *g.header.spec().state.as_bytes(),
                },
                inviter: s.context.spec_data().signer,
                nonce: p.nonce.ok_or_else(unavailable)?,
                expires_at_ms: p.expires_at_ms.ok_or_else(unavailable)?,
                policy: JoinPayloadKindData::Policy
                    .commitment_data(&policy_bytes)
                    .map_err(|_| unavailable())?,
                public_routing_plan: request.public_routing_plan,
            };
            p.policy_bytes = Some(policy_bytes);
            p.spec = Some(spec);
        }
        p.bytes = Some(
            p.spec
                .as_ref()
                .ok_or_else(unavailable)?
                .signing_bytes_data()
                .map_err(|_| unavailable())?,
        );
        #[cfg(test)]
        test_cut(TestCut::Prepared);
        self.require_invite_source()?;
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestCut {
    ClockEntered,
    NonceEntered,
    Prepared,
    NativeAttempting,
    KeyEntered,
    RawCached,
}
#[cfg(test)]
type InviteTestHook = (TestCut, Box<dyn FnOnce()>);
#[cfg(test)]
std::thread_local! {
    static TEST_CUT: std::cell::RefCell<Option<InviteTestHook>> =
        const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn test_cut(point: TestCut) {
    let run = TEST_CUT.with(|slot| {
        if slot.borrow().as_ref().is_some_and(|(p, _)| *p == point) {
            slot.borrow_mut().take().map(|(_, f)| f)
        } else {
            None
        }
    });
    if let Some(run) = run {
        run();
    }
}
#[cfg(test)]
mod tests;
