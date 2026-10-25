//! Detached custody for the actual first-round peer after canonical SDP completion.
//! Preparation is capacity only. The first admission still needs the original Ready,
//! native Session/rows/key and current Actor origin; no renderer receipt mints this owner.
//! This does not retire Source/Lower work, prove media connectivity or activate App V2.
use super::*;
use crate::voice_v2::crypto_actions::{CryptoAction, CryptoOperation, CryptoOutcome};
use crate::voice_v2::resources::{ByteBudget, Credit, Reservation, ResourceKind};
use std::sync::Weak;

/// Partitioned from unused original64KiB native handoff capacity. FourKiB pays fixed native
/// ownership/identity retention. Actual future work capacity is BYTES minus METADATA minus
/// the child's non-lendable Reservation/Arc header; it is STRICTLY LESS than fourKiB.
/// No process/group limit grows and there is no new ordinary-pool allocation at admission.
pub(in crate::voice_v2) const BYTES: usize = 8 * 1024;
const METADATA: usize = 4 * 1024;
const PREPARED: u8 = 0;
const ADMITTED: u8 = 1;
const CLOSED: u8 = 2;

/// One inert preparation becomes one admitted media identity. It owns no strong intent,
/// Phase, SDK future, Action, native Row, Workflow or Exchange. The weak identity pins the
/// original Intent ALLOCATION after its last strong owner; its entire layout is separately
/// included in this record's metadata payment, rather than relying on the dropped effect.
/// The original fixed PlanBinding observation keeps its own existing payment to last Drop.
pub struct SdpMedia {
    source: Weak<InitialSdpIntent>,
    original: OriginalPlanObservation,
    round: SdpRound,
    offer: OfferId,
    seal: u64,
    open: u64,
    state: AtomicU8,
    _reserve: Arc<Reservation>,
    // LAST: all retained allocations/observations precede their independent fixed payment.
    _metadata: Credit,
}
redacted_debug!(SdpMedia, MediaAdmission<'_>);

impl InitialSdpIntent {
    /// Prepare OUTSIDE final authority guards after the SAME actual Seal/Open are accepted,
    /// settled and acknowledged and Open is Applied. Native must also require its original
    /// registered rows' localDrained and exact Session/PC/key at first admission. None of
    /// those facts alone proves that browser SDK/RTC references or transport work drained.
    ///
    /// A native claimant must publish preparation exclusion BEFORE this call and retain the
    /// returned SAME record across Capacity, lost replies and retries. The underlying split
    /// is one-shot: dropping it cannot restore the parent's original available capacity.
    /// `wall`/`mono` are preparation observations, not caller-supplied final authority.
    pub fn prepare_media(
        self: &Arc<Self>,
        phase: &Arc<Phase>,
        seal: &Arc<CryptoAction>,
        open: &Arc<CryptoAction>,
        wall: u64,
        mono: u64,
    ) -> Result<Arc<SdpMedia>, VoiceError> {
        if !self.uses_phase(phase)
            || !seal.uses_sdp_intent(self)
            || !open.uses_sdp_intent(self)
            || seal.operation() != CryptoOperation::SealSdp
            || open.operation() != CryptoOperation::OpenSdp
        {
            return Err(VoiceError::Unauthorized);
        }
        // These calls can sample original clocks. Keep them outside Phase and all native
        // admission locks; repeat original pure currency after the last callback below.
        let sealed = seal.handoff_status();
        let opened = open.handoff_status();
        let delivery = open
            .recover_sdp_open_history()?
            .and_then(|history| history.delivery)
            .ok_or(VoiceError::Unauthorized)?;
        let seal_binding = seal.sdp_binding().ok_or(VoiceError::Unauthorized)?;
        let open_binding = open.sdp_binding().ok_or(VoiceError::Unauthorized)?;
        if !sealed.engine.settled
            || !sealed.acknowledged
            || !sealed.engine.receipt.as_ref().is_some_and(|receipt| {
                !receipt.retired && matches!(receipt.outcome, CryptoOutcome::Sealed { .. })
            })
            || !opened.engine.settled
            || !opened.acknowledged
            || !opened
                .engine
                .receipt
                .as_ref()
                .is_some_and(|receipt| !receipt.retired && receipt.outcome == CryptoOutcome::Opened)
            || !delivery.was_applied()
            || seal_binding.offer() != open_binding.offer()
            || delivery.offer() != open_binding.offer()
            || !self.current_at(wall, mono)
        {
            return Err(VoiceError::Unauthorized);
        }
        let reserve = phase.partition_media()?;
        let metadata = ByteBudget::Reserved(reserve.clone()).charge(
            self.original.resources(),
            self.original.binding().scope.group,
            METADATA,
            ResourceKind::Retained,
        )?;
        Ok(Arc::new(SdpMedia {
            source: Arc::downgrade(self),
            original: self.original.clone(),
            round: self.round,
            offer: seal_binding.offer(),
            seal: seal.serial(),
            open: open.serial(),
            state: AtomicU8::new(PREPARED),
            _reserve: reserve,
            _metadata: metadata,
        }))
    }
}

impl SdpMedia {
    /// Remaining declared byte allowance, not a work permit. A future native consumer must
    /// prepay its measured whole graph AND enforce finite concrete work ownership within it.
    pub fn work_capacity() -> usize {
        BYTES - METADATA - Reservation::partition_metadata_bytes()
    }
    /// Actual original allocation equality, independent of equal serialized room/round IDs.
    pub fn uses_intent(&self, intent: &Arc<InitialSdpIntent>) -> bool {
        self.source.as_ptr() == Arc::as_ptr(intent)
    }
    pub(in crate::voice_v2) fn matches_actions(
        &self,
        intent: &Arc<InitialSdpIntent>,
        seal: &Arc<CryptoAction>,
        open: &Arc<CryptoAction>,
    ) -> bool {
        self.uses_intent(intent)
            && self.seal == seal.serial()
            && self.open == open.serial()
            && seal.uses_sdp_intent(intent)
            && open.uses_sdp_intent(intent)
    }
    /// Historical fact only. A native lost-reply reader may recover this after End, but
    /// must not substitute it for fresh current admission before transferring an actual PC.
    pub fn was_admitted(&self) -> bool {
        self.state.load(Ordering::Acquire) & ADMITTED != 0
    }
    /// Currency of ALREADY admitted media. Intentionally independent of expired Ready and
    /// old phase/successor cleanup; never use it to grant a first media admission.
    pub fn is_current(&self) -> bool {
        self.state.load(Ordering::Acquire) == ADMITTED && self.original.media_current()
    }
    /// Negative-only irreversible fence. Closing does not refund custody: native/browser
    /// actual PC/callback/SDK drainage must precede releasing their separately paid owners.
    pub fn close(&self) {
        self.state.fetch_or(CLOSED, Ordering::AcqRel);
    }
    pub fn round(&self) -> SdpRound {
        self.round
    }
    pub fn offer(&self) -> OfferId {
        self.offer
    }
    pub(in crate::voice_v2) fn scope(&self) -> Scope {
        self.original.binding().scope
    }
    pub(in crate::voice_v2) fn peer(&self) -> DeviceId {
        self.original.binding().peer
    }
    pub(in crate::voice_v2) fn first_current_at(
        &self,
        intent: &Arc<InitialSdpIntent>,
        wall: u64,
        mono: u64,
    ) -> bool {
        self.uses_intent(intent)
            && self.state.load(Ordering::Acquire) == PREPARED
            && intent.current_at(wall, mono)
            && self.original.current()
    }
    pub(in crate::voice_v2) fn admit(&self) -> Result<(), VoiceError> {
        // Actual origin publication plus native original-row guards exclude a competing
        // media release. CAS still preserves CLOSED if an earlier negative fence won.
        self.state
            .compare_exchange(PREPARED, ADMITTED, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| VoiceError::Unauthorized)
    }
    /// Capacity only, prepared outside final guards and measured by the native consumer.
    /// This has no Phase/effect backedge and remains usable for cleanup after Close. It
    /// cannot stand in for the separate current native/Actor admission for a media effect.
    #[cfg(test)]
    pub(in crate::voice_v2) fn prepay_work(&self, bytes: usize) -> Result<Credit, VoiceError> {
        ByteBudget::Reserved(self._reserve.clone()).charge(
            self.original.resources(),
            self.scope().group,
            bytes,
            ResourceKind::Retained,
        )
    }
}

/// Root's synchronous borrowed origin/admission frame. Native/App consumers must add their
/// actual overlapping queue/callback/argument/return graphs; this is no allocation or grant.
pub const fn initial_media_admission_bytes() -> usize {
    std::mem::size_of::<crate::voice_v2::Context>()
        + std::mem::size_of::<crate::voice_v2::helper::RoomAuthority>()
        + std::mem::size_of::<crate::voice_v2::packets::Authority<'static>>()
        + std::mem::size_of::<crate::voice_v2::runtime::PreparedLocalOrigin<'static>>()
        + std::mem::size_of::<MediaAdmission<'static>>()
        + std::mem::size_of::<[Option<DeviceId>; 4]>()
        + 12 * std::mem::size_of::<Arc<()>>()
        + 4 * std::mem::size_of::<std::sync::MutexGuard<'static, ()>>()
        + 256 // borrowed closure/final owner, samples, discriminator and result wrappers
}

/// A borrowed first-media final tail on the actual Actor. It cannot escape the exclusive
/// call or authorize a later RTC operation; native surrounds commit with original lexical
/// UI/Coordinator/route/row admission and installs its exact preallocated claim immediately.
pub struct MediaAdmission<'a> {
    media: &'a Arc<SdpMedia>,
    intent: &'a Arc<InitialSdpIntent>,
    wall: u64,
    mono: u64,
    commit: &'a mut dyn FnMut() -> Result<(), VoiceError>,
}
impl<'a> MediaAdmission<'a> {
    pub(crate) fn new(
        media: &'a Arc<SdpMedia>,
        intent: &'a Arc<InitialSdpIntent>,
        wall: u64,
        mono: u64,
        commit: &'a mut dyn FnMut() -> Result<(), VoiceError>,
    ) -> Self {
        Self {
            media,
            intent,
            wall,
            mono,
            commit,
        }
    }
    pub fn matches(&self, media: &Arc<SdpMedia>, intent: &Arc<InitialSdpIntent>) -> bool {
        Arc::ptr_eq(self.media, media) && Arc::ptr_eq(self.intent, intent)
    }
    /// Native's final pure original-row gate uses THESE actual Actor preparation samples.
    /// Sampling another Clock beneath UI/row/Group/Policy is forbidden; these observations
    /// alone grant nothing and commit still repeats all original currency checks.
    pub fn observed_time(&self) -> (u64, u64) {
        (self.wall, self.mono)
    }
    pub fn commit(self) -> Result<(), VoiceError> {
        (self.commit)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_media_metadata_pays_its_record_and_entire_weak_intent_allocation() {
        let bytes = std::mem::size_of::<SdpMedia>()
            + 2 * std::mem::size_of::<usize>()
            + std::mem::size_of::<InitialSdpIntent>()
            + 2 * std::mem::size_of::<usize>();
        // The rest is reserved for the complete native claim/preparation graph. Its actual
        // consumer must add its measured graph before publishing, not silently use4KiB.
        assert!(bytes <= 2048, "fixed media graph {bytes}/2048");
        assert!(BYTES > METADATA);
    }
}
