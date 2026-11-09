//! Chat V2 ordinary-member signing seams on the ACTUAL whole node.
//!
//! An ordinary member (including the Profile founder acting as an ordinary member) signs its
//! own Create/reply, own Edit/Delete and Join with its retained MLS leaf key. Nothing here
//! signs an owner Header, reaches the founder A/I/S/B Append lane, exports a channel secret,
//! mutates the sender ratchet or extracts a key. Two node families expose the same seams:
//!
//! - the founder's closed genesis node (`UnboundV2GenesisNode`, in
//!   `founder/bootstrap_source/genesis_seed/ordinary_member.rs`), used by the founder vertical;
//! - a joined member's live `ChannelSync` (below), bound to its exact process incarnation by
//!   `RegistrySyncInstance`. Its App owner is the separate joined-member source, which must
//!   supply the independently trusted V2 pin/Profile/governing prefix itself.
//!
//! Seams, in the order the App host uses them:
//! 1. `draw_ordinary_draft_inputs` (outside every writer) binds the request's
//!    [`V2MemberDraftInputs`] to THIS node's original identity, then makes the request's one
//!    Clock read and one 16-byte RNG fill through this node's own runtime seams, caching each
//!    BEFORE the next negative. A retry never draws again; another node cannot draw into it.
//! 2. `ordinary_member_signer_data` (inside the host's Current guard, BEFORE Core
//!    `admit_signing`): full source negatives, which may consult SDK group state, and the full
//!    retained key that the host passes to `admit_signing`.
//! 3. `sign_ordinary_admitted` (Core's original group reader held by the guard): only
//!    allocation-free, SDK-free negatives (same original node as the draw, retained key equals
//!    the Draft signer), then the ONE bounded key call, and raw64 cached in the Plan at once.
//!
//! None of these is authorization. Core issues the Plan only for a current OPEN Admission;
//! the App host holds its native/source/Current guard outside the Core reader.
use crate::owner_v2::{OriginalV2GenesisSeedObservation, UnboundV2GenesisSeed};
use crate::*;
use catcoms_replication::chat::{V2OrdinarySigningGuard, MAX_ACTION_BYTES};

/// Closed refusal classes for the ordinary-member seams. They carry no key, text or source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V2MemberSignRefusal {
    /// The node is not (or no longer) the live original source for this request: another
    /// node/incarnation than the one that drew it, an inactive/removed group, a closed scope,
    /// or no local leaf for the signer.
    Source,
    /// The admitted Draft names a different full signer than this node's retained key.
    Key,
    /// A once-only seam already entered without a result (lost/unwound return) or the device
    /// refused after the key entry. The original request can never draw or sign again.
    Terminal,
}

// Each seam is marked BEFORE the external call. An unwind or lost return between the mark
// and the cached result leaves the slot in the entered stage, which is terminal: the same
// request can never call the Clock or RNG a second time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawStage {
    Captured,
    Clock,
    Time,
    Nonce,
    Drawn,
    Failed,
}

/// The exact original node a request was drawn on. Weak/pointer identities only: they keep
/// nothing alive and a replacement node with equal key/group labels never matches them.
enum DrawOrigin {
    Founder(OriginalV2GenesisSeedObservation),
    Joined(RegistrySyncInstance),
}

/// One request's once-only presented-time and nonce, bound to its original node. Owned by the
/// App request custody for the request's whole life (including across every error), never
/// rebuilt from a token. Not Clone: two copies could both reach `Captured` and draw twice.
pub struct V2MemberDraftInputs {
    stage: DrawStage,
    time: Option<u64>,
    nonce: Option<[u8; 16]>,
    origin: Option<DrawOrigin>,
}

impl Default for V2MemberDraftInputs {
    fn default() -> Self {
        Self::new()
    }
}

impl V2MemberDraftInputs {
    pub fn new() -> Self {
        Self {
            stage: DrawStage::Captured,
            time: None,
            nonce: None,
            origin: None,
        }
    }

    /// Cached result only; never draws. `None` until both values are cached.
    pub fn drawn_data(&self) -> Option<(u64, [u8; 16])> {
        match (self.stage, self.time, self.nonce) {
            (DrawStage::Drawn, Some(time), Some(nonce)) => Some((time, nonce)),
            _ => None,
        }
    }

    /// Bind (first draw) or require (any later use) the founder node's original seed.
    pub(crate) fn bind_founder_origin(
        &mut self,
        seed: &UnboundV2GenesisSeed,
    ) -> Result<(), V2MemberSignRefusal> {
        if self.origin.is_none() {
            self.origin = Some(DrawOrigin::Founder(seed.observe_original_seed_data()));
        }
        self.require_founder_origin(seed)
    }
    /// Allocation-free: a Weak upgrade and pointer comparison.
    pub(crate) fn require_founder_origin(
        &self,
        seed: &UnboundV2GenesisSeed,
    ) -> Result<(), V2MemberSignRefusal> {
        match &self.origin {
            Some(DrawOrigin::Founder(original)) => original
                .require_same_seed_data(seed)
                .map_err(|_| V2MemberSignRefusal::Source),
            _ => Err(V2MemberSignRefusal::Source),
        }
    }
    /// `node` already matched `instance`; a bound slot must name that same incarnation.
    fn bind_joined_origin<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        node: &ChannelSync<T, R>,
        instance: &RegistrySyncInstance,
    ) -> Result<(), V2MemberSignRefusal> {
        match &self.origin {
            None => {
                self.origin = Some(DrawOrigin::Joined(instance.clone()));
                Ok(())
            }
            Some(DrawOrigin::Joined(original)) if node.matches_registry_instance(original) => {
                Ok(())
            }
            Some(_) => Err(V2MemberSignRefusal::Source),
        }
    }
    /// Allocation-free pointer comparison with the live node's own incarnation.
    fn require_joined_origin<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<(), V2MemberSignRefusal> {
        match &self.origin {
            Some(DrawOrigin::Joined(original)) if node.matches_registry_instance(original) => {
                Ok(())
            }
            _ => Err(V2MemberSignRefusal::Source),
        }
    }

    /// `Ok(true)` when the caller must make the one Clock call now (the stage is marked
    /// first); `Ok(false)` when time is already cached. An entered-without-result slot fails.
    pub(crate) fn enter_clock(&mut self) -> Result<bool, V2MemberSignRefusal> {
        match self.stage {
            DrawStage::Captured => {
                self.stage = DrawStage::Clock;
                Ok(true)
            }
            DrawStage::Time | DrawStage::Nonce | DrawStage::Drawn => Ok(false),
            DrawStage::Clock | DrawStage::Failed => {
                self.stage = DrawStage::Failed;
                Err(V2MemberSignRefusal::Terminal)
            }
        }
    }
    pub(crate) fn cache_time(&mut self, time: u64) {
        self.time = Some(time);
        self.stage = DrawStage::Time;
    }
    /// Same contract as [`Self::enter_clock`] for the one RNG fill.
    pub(crate) fn enter_nonce(&mut self) -> Result<bool, V2MemberSignRefusal> {
        match self.stage {
            DrawStage::Time => {
                self.stage = DrawStage::Nonce;
                Ok(true)
            }
            DrawStage::Drawn => Ok(false),
            DrawStage::Captured | DrawStage::Clock | DrawStage::Nonce | DrawStage::Failed => {
                self.stage = DrawStage::Failed;
                Err(V2MemberSignRefusal::Terminal)
            }
        }
    }
    pub(crate) fn cache_nonce(&mut self, nonce: [u8; 16]) {
        self.nonce = Some(nonce);
        self.stage = DrawStage::Drawn;
    }
    pub(crate) fn drawn(&self) -> Result<(u64, [u8; 16]), V2MemberSignRefusal> {
        self.drawn_data().ok_or(V2MemberSignRefusal::Terminal)
    }
}

impl std::fmt::Debug for V2MemberDraftInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The nonce and presented time are request DATA; keep them out of logs.
        f.debug_struct("V2MemberDraftInputs")
            .finish_non_exhaustive()
    }
}

/// Shared key-cut tail for both node families, run under the held Core reader after the
/// caller's allocation-free, SDK-free origin negatives. Order is fixed: compare the Draft
/// signer with the device's RETAINED key (allocation-free), reuse an already cached raw (no
/// key), mark key entry, make the ONE bounded key call, cache raw immediately. No hook, check,
/// allocation or callback runs between the key call and `cache_raw64`.
pub(crate) fn sign_admitted_with_device<'a>(
    device: &MlsDevice,
    guard: V2OrdinarySigningGuard<'a>,
) -> Result<V2OrdinarySigningGuard<'a>, V2MemberSignRefusal> {
    let signer = *guard.signer_data();
    if !device.retained_ed25519_key_matches(&signer) {
        // Dropping the guard releases the reader; the Plan stays Prepared for a retry
        // through the ORIGINAL device. A replacement device can never consume the entry.
        return Err(V2MemberSignRefusal::Key);
    }
    if guard.cached_raw64_data().is_some() {
        // Exact retry after a lost reply: the original raw is reused, the key is not called.
        return Ok(guard);
    }
    let call = guard
        .begin_key()
        .map_err(|_| V2MemberSignRefusal::Terminal)?;
    #[cfg(test)]
    note_key_call();
    match device.sign_retained_ed25519_bounded(&signer, call.signing_bytes_data(), MAX_ACTION_BYTES)
    {
        Ok(raw) => Ok(call.cache_raw64(raw)),
        // Dropping the entered call leaves the Plan terminal KeyEntered: never a second key.
        Err(_) => Err(V2MemberSignRefusal::Terminal),
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Pre-admission live-source negatives: the SAME process incarnation, an active group (a
    /// member removed by an applied commit is inactive), and a local leaf whose signature key
    /// is exactly this device's retained key. These consult SDK group state, so they must run
    /// before Core admission, never under the held reader.
    fn ordinary_member_live(
        &self,
        instance: &RegistrySyncInstance,
        signer: &[u8; 32],
    ) -> Result<(), V2MemberSignRefusal> {
        if !self.matches_registry_instance(instance)
            || !self.group.is_active()
            || !self.group.has_local_signer(signer)
            || !self.device.retained_ed25519_key_matches(signer)
        {
            return Err(V2MemberSignRefusal::Source);
        }
        Ok(())
    }

    /// The full retained device key of this exact live incarnation. Pre-admission only.
    pub fn ordinary_member_signer_data(
        &self,
        instance: &RegistrySyncInstance,
    ) -> Result<[u8; 32], V2MemberSignRefusal> {
        let signer: [u8; 32] = self
            .device
            .public_key_bytes()
            .try_into()
            .map_err(|_| V2MemberSignRefusal::Source)?;
        self.ordinary_member_live(instance, &signer)?;
        Ok(signer)
    }

    /// Bind `slot` to this incarnation, then one Clock read and one RNG fill through THIS
    /// node's own runtime seams, each cached before the next negative. A cached slot returns
    /// the same values and calls neither seam; an entered-without-result slot is terminal.
    pub fn draw_ordinary_draft_inputs(
        &mut self,
        instance: &RegistrySyncInstance,
        slot: &mut V2MemberDraftInputs,
    ) -> Result<(u64, [u8; 16]), V2MemberSignRefusal> {
        let signer = self.ordinary_member_signer_data(instance)?;
        slot.bind_joined_origin(self, instance)?;
        if slot.enter_clock()? {
            slot.cache_time(self.clock.now_ms());
        }
        self.ordinary_member_live(instance, &signer)?;
        if slot.enter_nonce()? {
            let mut nonce = [0; 16];
            self.rng.fill_bytes(&mut nonce);
            slot.cache_nonce(nonce);
        }
        self.ordinary_member_live(instance, &signer)?;
        slot.drawn()
    }

    /// The key cut for an already-admitted Core guard: the request must have been drawn on
    /// THIS incarnation (pointer check), then [`sign_admitted_with_device`]. No SDK lookup.
    /// The caller drops the returned guard before any materialization.
    pub fn sign_ordinary_admitted<'a>(
        &self,
        inputs: &V2MemberDraftInputs,
        guard: V2OrdinarySigningGuard<'a>,
    ) -> Result<V2OrdinarySigningGuard<'a>, V2MemberSignRefusal> {
        inputs.require_joined_origin(self)?;
        sign_admitted_with_device(&self.device, guard)
    }
}

// Test-only key-entry counter. It observes the real device call; it grants nothing.
#[cfg(test)]
std::thread_local! {
    static KEY_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
fn note_key_call() {
    KEY_CALLS.with(|n| n.set(n.get() + 1));
}
#[cfg(test)]
pub(crate) fn key_calls_for_test() -> usize {
    KEY_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) mod governance_fixture;
#[cfg(test)]
mod tests;
