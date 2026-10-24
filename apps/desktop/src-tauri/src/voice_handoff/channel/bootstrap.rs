//! Native-only local-channel key agreement. There is no raw-seed constructor/export here.
//! A registry must retain its Pending ticket before construction, then atomically install the
//! result under the original selected-owner/generation fence. These keys grant no Root effect.
use super::{
    Counters, Keys, Refused, Session, SessionIdentity, BOOTSTRAP_BYTES, BROWSER_CONTROL_BYTES,
    CONTROL_BYTES,
};
use crate::voice_authority::{Binding, SelectedHandoffOwner};
use catcoms_rt::rng::RngError;
use catcoms_rt::{CryptoRng, CryptoRngCore, RngCore};
use catcoms_sync::voice_v2::resources::{Credit, ResourceKind, WorkLane};
use hkdf::Hkdf;
use p256::{ecdh::EphemeralSecret, elliptic_curve::sec1::ToEncodedPoint, PublicKey};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

pub(in crate::voice_handoff) const TRANSCRIPT_BYTES: usize = 1024;
pub(in crate::voice_handoff) const SETUP_WORK_BYTES: usize = 8 * 1024;
const DOMAIN: &[u8] = b"Mewtual native voice bootstrap\0";
const SEED_LABEL: &[u8] = b"Mewtual native voice bootstrap seed v1";

/// Public immutable setup only. It is not a Session, sequence allocator or permission to
/// perform an action. The trusted bridge may project only transcript[..length], never padding.
/// The selected browser independently parses it before deriving the existing framing keys.
#[derive(PartialEq, Eq)]
pub(crate) struct PublicSetup {
    pub session: [u8; 16],
    pub slot: u8,
    pub transcript: [u8; TRANSCRIPT_BYTES],
    pub length: usize,
    pub hash: [u8; 32],
}
impl PublicSetup {
    pub fn transcript(&self) -> &[u8] {
        &self.transcript[..self.length]
    }
}

/// Ordinary group bytes AND an ordinary Crypto slot precede native scalar arithmetic. The
/// SAME setup work charge transfers to the registry through actual browser setup, until an
/// exact settlement/complete authenticated Status. Session's control lane is independent.
/// This owner must never be constructed merely to recover an already Pending/Ready row.
pub(in crate::voice_handoff) struct Prepared {
    selected: Arc<SelectedHandoffOwner>,
    session: [u8; 16],
    slot: u8,
    device: [u8; 32],
    control: Credit,
    bootstrap: Credit,
    browser_control: Credit,
    control_lane: Arc<WorkLane>,
    work: Credit,
}
impl Prepared {
    pub fn new(
        selected: Arc<SelectedHandoffOwner>,
        session: [u8; 16],
        slot: u8,
        device: [u8; 32],
    ) -> Result<Self, Refused> {
        if session == [0; 16]
            || !(1..=64).contains(&slot)
            || selected.id().is_empty()
            || selected.id().len() > 128
            || selected.binding().group.is_empty()
            || selected.binding().group.len() > 256
            || !selected.lease().is_live()
        {
            return Err(Refused::Binding);
        }
        let resources = selected.resources();
        let group = selected.binding().scope.group;
        // A proven pre-admission refusal may drop these ordinary resource owners under the
        // Coordinator/Index guards. They contain no active engine, affine grant or callback;
        // no row/serial/RNG effect has happened. Actual-work/output owners retire outside
        // those guards after Pending admission. Preserve this distinction when adding fields.
        let pay = |bytes, kind| {
            resources
                .charge(group, bytes, kind)
                .map_err(|_| Refused::Capacity)
        };
        Ok(Self {
            control: pay(CONTROL_BYTES, ResourceKind::Retained)?,
            bootstrap: pay(BOOTSTRAP_BYTES, ResourceKind::Retained)?,
            browser_control: pay(BROWSER_CONTROL_BYTES, ResourceKind::Retained)?,
            control_lane: resources
                .reserve_crypto_lane(group)
                .map_err(|_| Refused::Capacity)?,
            work: pay(SETUP_WORK_BYTES, ResourceKind::Crypto)?,
            selected,
            session,
            slot,
            device,
        })
    }

    /// No Coordinator/registry guard may cross this call: injected RNG and scalar arithmetic
    /// are outside final authority locks. Caller must mark its Pending ticket started BEFORE
    /// this call; panic/error then closes that original purpose rather than retrying fresh RNG.
    pub fn compute(
        self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(Arc<Session>, PublicSetup, Credit), Refused> {
        // The older selected-point publication validates encoding/reference only. Validate
        // this actual curve point before scalar multiplication, never just its 04 prefix.
        let point = self.selected.point();
        if point[0] != 4 {
            return Err(Refused::Binding);
        }
        let browser = PublicKey::from_sec1_bytes(&point).map_err(|_| Refused::Binding)?;
        let ephemeral = EphemeralSecret::random(&mut BorrowedRng(rng));
        let encoded = ephemeral.public_key().to_encoded_point(false);
        let native: [u8; 65] = encoded
            .as_bytes()
            .try_into()
            .map_err(|_| Refused::Binding)?;
        let public = self.transcript(native)?;
        let keys = agree(&ephemeral, &browser, &public.hash)?;
        // The pinned types zeroize their own scalar/shared storage on Drop. This is not a
        // claim about arithmetic temporaries, provider heaps or later join-key compromise.
        drop(ephemeral);
        let session = Arc::new(Session {
            identity: Arc::new(SessionIdentity(public.hash)),
            resources: self.selected.resources(),
            group: self.selected.binding().scope.group,
            slot: self.slot,
            keys,
            counters: Mutex::new(Counters {
                closed: false,
                busy: false,
                native_busy: false,
                request_floor: 0,
                response_sequence: 0,
            }),
            idle: tokio::sync::Notify::new(),
            _control: self.control,
            _bootstrap: self.bootstrap,
            _browser_control: self.browser_control,
            control_lane: self.control_lane,
            #[cfg(test)]
            input_pressure: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            result_preflight_sequence: std::sync::atomic::AtomicU64::new(0),
        });
        // The browser has not performed ECDH/HKDF/sealing merely because native arithmetic
        // finished. Install this same credit before exposing public setup; do not re-charge.
        Ok((session, public, self.work))
    }

    fn transcript(&self, native: [u8; 65]) -> Result<PublicSetup, Refused> {
        encode_transcript(Transcript {
            binding: self.selected.binding(),
            owner: self.selected.id(),
            join: self.selected.join(),
            executor: self.selected.executor(),
            device: self.device,
            browser: self.selected.point(),
            native,
            session: self.session,
            slot: self.slot,
        })
    }
}

/// Pure metadata encoding is separate so Rust and WebCrypto fixtures can pin the exact same
/// bytes. Neither constructing these fields nor computing their hash grants native authority.
struct Transcript<'a> {
    binding: &'a Binding,
    owner: &'a str,
    join: [u8; 32],
    executor: [u8; 32],
    device: [u8; 32],
    browser: [u8; 65],
    native: [u8; 65],
    session: [u8; 16],
    slot: u8,
}
fn encode_transcript(fields: Transcript<'_>) -> Result<PublicSetup, Refused> {
    if fields.owner.is_empty()
        || fields.owner.len() > 128
        || fields.binding.group.is_empty()
        || fields.binding.group.len() > 256
        || fields.session == [0; 16]
        || !(1..=64).contains(&fields.slot)
    {
        return Err(Refused::Bounds);
    }
    let mut public = PublicSetup {
        session: fields.session,
        slot: fields.slot,
        transcript: [0; TRANSCRIPT_BYTES],
        length: 0,
        hash: [0; 32],
    };
    let mut append = |part: &[u8]| -> Result<(), Refused> {
        let end = public
            .length
            .checked_add(part.len())
            .ok_or(Refused::Bounds)?;
        public
            .transcript
            .get_mut(public.length..end)
            .ok_or(Refused::Bounds)?
            .copy_from_slice(part);
        public.length = end;
        Ok(())
    };
    let binding = fields.binding;
    append(DOMAIN)?;
    append(&[1, 1])?; // version1 / suite1: P256 + HKDF-SHA256 + framing-v1
    append(&fields.session)?;
    append(&[fields.slot])?;
    for value in [binding.generation, binding.server, binding.instance] {
        append(&value.to_be_bytes())?;
    }
    append(&(binding.group.len() as u16).to_be_bytes())?;
    append(&binding.group)?;
    append(&binding.scope.group)?;
    append(&[u8::from(binding.scope.policy.is_some())])?;
    if let Some(policy) = binding.scope.policy {
        append(&policy)?;
    }
    append(&binding.scope.room.to_be_bytes())?;
    append(&fields.device)?;
    append(&fields.join)?;
    append(&(fields.owner.len() as u16).to_be_bytes())?;
    append(fields.owner.as_bytes())?;
    append(&fields.executor)?;
    append(&fields.browser)?;
    append(&fields.native)?;
    public.hash = Sha256::digest(public.transcript()).into();
    Ok(public)
}

fn agree(
    ephemeral: &EphemeralSecret,
    browser: &PublicKey,
    hash: &[u8; 32],
) -> Result<Keys, Refused> {
    let shared = ephemeral.diffie_hellman(browser);
    // Fixed-width big-endian field element, including leading zeroes. HKDF extracts it under
    // the complete injective transcript; this value is never used directly as an AES key.
    let mut seed = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(hash), shared.raw_secret_bytes())
        .expand(SEED_LABEL, seed.as_mut())
        .map_err(|_| Refused::Binding)?;
    Ok(Keys::new(&seed))
}

/// Pinned p256 takes a Sized rand_core0.6 RNG. This borrowed adapter preserves the sanctioned
/// injected catcoms-rt seam; it never seeds another RNG or reaches ambient OS randomness.
struct BorrowedRng<'a>(&'a mut dyn CryptoRngCore);
impl RngCore for BorrowedRng<'_> {
    fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }
    fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        self.0.fill_bytes(bytes);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), RngError> {
        self.0.try_fill_bytes(bytes)
    }
}
impl CryptoRng for BorrowedRng<'_> {}

#[cfg(test)]
mod tests;
