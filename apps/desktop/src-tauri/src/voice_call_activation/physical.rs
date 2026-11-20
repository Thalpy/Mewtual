//! One irreversible physical-setup history for the ORIGINAL admitted activation.
//!
//! This is negative/local provenance, not a call winner, operational admission,
//! SDK-return claim or Root history certificate. It has no Handle/Row/Actor
//! backedge. The parent and physical Row retain the SAME outside plain payer.
use super::*;
use catcoms_sync::voice_v2::resources::Credit;

/// Independent, bounded physical-history/receipt graph. Native's activation,
/// generation and Row payments still cover their own actual owners. The complete
/// production invoice below must pass BEFORE charging or allocating this body.
pub(crate) const PHYSICAL_HISTORY_BYTES: usize = 8 * 1024;
pub(crate) const PHYSICAL_REPLY_BYTES: usize = 1024;

struct HistoryFunding {
    _credit: Credit,
}

/// The final extraction frees the funding Arc allocation BEFORE returning its
/// credit. Plain Arc::drop would destroy the credit first and leave an unpaid
/// header tail. No raw Arc/Weak escapes; into_inner also gives exactly one winner
/// when the two final wrappers are destroyed concurrently.
struct FundingOwner(Option<Arc<HistoryFunding>>);
impl Clone for FundingOwner {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl FundingOwner {
    fn release_with(&mut self, after_deallocation: impl FnOnce()) {
        if let Some(original) = self.0.take() {
            if let Some(funding) = Arc::into_inner(original) {
                after_deallocation();
                drop(funding);
            }
        }
    }
}
impl Drop for FundingOwner {
    fn drop(&mut self) {
        self.release_with(|| {});
    }
}

/// Plain prepayment is acquired before the generation can consume its serial.
/// A local capacity refusal cannot leave a new key row or physical intent.
pub(crate) struct PreparedPhysicalHistory {
    credit: Credit,
}

#[derive(Clone)]
pub(crate) struct PhysicalHistory {
    body: Arc<HistoryBody>,
    // LAST: body, original observer and receipt aliases deallocate first.
    funding: FundingOwner,
}

struct HistoryBody {
    owner: String,
    original: ActivationObserver,
    generation: u64,
    executor: [u8; 32],
    join: [u8; 32],
    permit: String,
    state: StdMutex<HistoryState>,
}

/// There is deliberately no transition back to Unstarted. In particular, a
/// collected/missing Row cannot make a prior admitted setup look unentered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryState {
    Unstarted,
    Admitted([u8; 16]),
    NotStarted,
    ActuallyCollected([u8; 16]),
    Acknowledged(PhysicalTerminal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhysicalTerminal {
    NotStarted,
    Collected([u8; 16]),
}

/// Parent-inline purpose is selected once, before browser SDK permission. A
/// closed original can prove NoIntentYet only from this real initial state;
/// absence of a parent/history record is never such evidence.
pub(super) enum PhysicalPurpose {
    NoIntentYet,
    KeyOnly,
    Physical(PhysicalHistory),
}

impl PhysicalHistory {
    pub(crate) fn prepay(handle: &Handle) -> Result<PreparedPhysicalHistory, VoiceError> {
        if physical_history_layout_bytes() > PHYSICAL_HISTORY_BYTES || handle.id().len() > 64 {
            return Err(VoiceError::Bounds);
        }
        let credit = handle.original().reserve_retained(PHYSICAL_HISTORY_BYTES)?;
        // No callback or await occurs between this final phase check and the
        // caller's UI -> Coordinator -> Cell installation. Closed is permanent.
        if handle.original().is_closed() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(PreparedPhysicalHistory { credit })
    }
}
impl PreparedPhysicalHistory {
    /// The private Coordinator supplies the newly constructed canonical row's
    /// bounded permit. No callback, await or fallible allocation follows binding.
    pub(crate) fn bind(
        self,
        handle: &Handle,
        executor: [u8; 32],
        join: [u8; 32],
        permit: &str,
    ) -> PhysicalHistory {
        assert!(
            permit.len() <= 64,
            "native generation bounds its original permit"
        );
        let funding = FundingOwner(Some(Arc::new(HistoryFunding {
            _credit: self.credit,
        })));
        PhysicalHistory {
            body: Arc::new(HistoryBody {
                owner: handle.id().to_owned(),
                original: handle.original().clone(),
                generation: handle.binding().generation,
                executor,
                join,
                permit: permit.to_owned(),
                state: StdMutex::new(HistoryState::Unstarted),
            }),
            funding,
        }
    }
}
impl PhysicalHistory {
    pub(crate) fn matches(
        &self,
        handle: &Handle,
        executor: [u8; 32],
        join: [u8; 32],
        permit: &str,
    ) -> bool {
        self.body.original.is_same(handle.original())
            && self.body.owner == handle.id()
            && self.body.generation == handle.binding().generation
            && self.body.executor == executor
            && self.body.join == join
            && self.body.permit == permit
    }
    pub(crate) fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.body, &other.body)
    }
    pub(crate) fn owner(&self) -> &str {
        &self.body.owner
    }
    pub(crate) fn executor(&self) -> [u8; 32] {
        self.body.executor
    }
    pub(crate) fn join(&self) -> [u8; 32] {
        self.body.join
    }
    pub(crate) fn original(&self) -> &ActivationObserver {
        &self.body.original
    }

    /// Called only under the original UI -> Coordinator -> Index turn, AFTER
    /// the actual Pending Row and its immutable serial have been constructed,
    /// before the same Index turn publishes it and BEFORE RNG/curve work can run.
    /// Exact retry observes the SAME admission.
    pub(crate) fn admit_row(&self, serial: [u8; 16]) -> Result<(), VoiceError> {
        if self.body.original.is_closed() || serial == [0; 16] {
            return Err(VoiceError::Unauthorized);
        }
        let mut state = self.body.state.lock().unwrap_or_else(|e| e.into_inner());
        match *state {
            HistoryState::Unstarted => {
                *state = HistoryState::Admitted(serial);
                Ok(())
            }
            HistoryState::Admitted(original) if original == serial => Ok(()),
            _ => Err(VoiceError::Unauthorized),
        }
    }

    /// Only the outside-guard actual-last-Row destructor calls this AFTER Row
    /// allocation and its Session/output weak tails have really deallocated.
    pub(crate) fn row_actually_returned(&self, serial: [u8; 16]) {
        let mut state = self.body.state.lock().unwrap_or_else(|e| e.into_inner());
        if *state == HistoryState::Admitted(serial) {
            *state = HistoryState::ActuallyCollected(serial);
        }
    }

    /// This closed-original observation never relies on missing registry rows.
    /// It proves physical non-admission only; SDK/Root debts remain separate.
    pub(crate) fn terminal(&self) -> Option<PhysicalTerminal> {
        let mut state = self.body.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.body.original.is_closed() && *state == HistoryState::Unstarted {
            *state = HistoryState::NotStarted;
        }
        match *state {
            HistoryState::NotStarted => Some(PhysicalTerminal::NotStarted),
            HistoryState::ActuallyCollected(serial) => Some(PhysicalTerminal::Collected(serial)),
            HistoryState::Acknowledged(terminal) => Some(terminal),
            _ => None,
        }
    }

    pub(crate) fn acknowledge(&self, expected: PhysicalTerminal) -> Result<(), VoiceError> {
        if self.terminal() != Some(expected) {
            return Err(VoiceError::Unauthorized);
        }
        let mut state = self.body.state.lock().unwrap_or_else(|e| e.into_inner());
        match *state {
            HistoryState::NotStarted if expected == PhysicalTerminal::NotStarted => {}
            HistoryState::ActuallyCollected(serial)
                if expected == PhysicalTerminal::Collected(serial) => {}
            HistoryState::Acknowledged(original) if original == expected => return Ok(()),
            _ => return Err(VoiceError::Unauthorized),
        }
        *state = HistoryState::Acknowledged(expected);
        Ok(())
    }
    pub(crate) fn acknowledged(&self) -> bool {
        matches!(
            *self.body.state.lock().unwrap_or_else(|e| e.into_inner()),
            HistoryState::Acknowledged(_)
        )
    }

    /// A deterministic identity for THIS latched observation, not a random
    /// capability or a generic caller nonce. All inputs are original native
    /// metadata; replaying another parent's public receipt cannot acknowledge it.
    pub(crate) fn receipt_id(&self, terminal: PhysicalTerminal) -> [u8; 32] {
        use sha2::Digest;
        let mut hash = sha2::Sha256::new();
        hash.update(b"Mewtual admitted physical terminal receipt v1\0");
        hash.update((self.body.owner.len() as u16).to_be_bytes());
        hash.update(self.body.owner.as_bytes());
        hash.update(self.body.generation.to_be_bytes());
        hash.update(self.body.executor);
        hash.update(self.body.join);
        match terminal {
            PhysicalTerminal::NotStarted => hash.update([0]),
            PhysicalTerminal::Collected(serial) => {
                hash.update([1]);
                hash.update(serial);
            }
        }
        hash.finalize().into()
    }
    pub(crate) fn acknowledge_receipt(&self, expected: [u8; 32]) -> Result<(), VoiceError> {
        let terminal = self.terminal().ok_or(VoiceError::Unauthorized)?;
        if self.receipt_id(terminal) != expected {
            return Err(VoiceError::Unauthorized);
        }
        self.acknowledge(terminal)
    }
}

pub(crate) fn physical_history_layout_bytes() -> usize {
    use std::mem::size_of;
    #[repr(C)]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    size_of::<ArcAllocation<HistoryBody>>()
        + size_of::<ArcAllocation<HistoryFunding>>()
        + 64 + 64 // actual owner/permit heaps
        + 6 * size_of::<PhysicalHistory>() // parent/Row/collector/read/reply/return overlap
        + 2 * size_of::<PhysicalTerminal>()
        + PHYSICAL_REPLY_BYTES
        + size_of::<sha2::Sha256>()
        + crate::voice_handoff::commands::physical_receipt_reply_layout_bytes()
        + 2 * (64 + 64 + 64 + 32) // owned bounded reply/hex projections and serializer overlap
        + 512 // fixed serializer/call/return frames; opaque framework copies excluded
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::Clock;
    use catcoms_sync::voice_v2::resources::{ResourceKind, Resources};

    struct ResumeOnDrop(Option<std::sync::mpsc::Sender<()>>);
    impl Drop for ResumeOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    async fn bounded<T>(future: impl Future<Output = T>) -> T {
        tokio::select! {
            value = future => value,
            _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => panic!("physical funding destruction exceeded its bound"),
        }
    }
    fn funding(resources: &Arc<Resources>) -> FundingOwner {
        FundingOwner(Some(Arc::new(HistoryFunding {
            _credit: resources
                .charge([91; 32], PHYSICAL_HISTORY_BYTES, ResourceKind::Retained)
                .unwrap(),
        })))
    }

    #[tokio::test]
    async fn physical_history_funding_payment_survives_actual_final_arc_deallocation() {
        let resources = Arc::new(Resources::default());
        let original = funding(&resources);
        let mut last = original.clone();
        drop(original);
        let (entered, at_deallocation) = tokio::sync::oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let release = ResumeOnDrop(Some(resume));
        let (done, finished) = tokio::sync::oneshot::channel();
        drop(std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                last.release_with(|| {
                    let _ = entered.send(());
                    let _ = resumed.recv();
                });
            }));
            let _ = done.send(result);
        }));
        bounded(at_deallocation).await.unwrap();
        assert_eq!(
            resources.counts(),
            (PHYSICAL_HISTORY_BYTES, 0),
            "funding Arc already deallocated, SAME credit still held outside it"
        );
        drop(release); // RAII also releases the parked producer after an assertion unwind
        bounded(finished).await.unwrap().unwrap();
        assert_eq!(resources.counts(), (0, 0));
    }

    #[tokio::test]
    async fn physical_history_concurrent_final_funding_aliases_extract_and_refund_once() {
        let resources = Arc::new(Resources::default());
        let original = funding(&resources);
        let other = original.clone();
        let extractions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut finished = Vec::with_capacity(2);
        let mut releases = Vec::with_capacity(2);
        for mut owner in [original, other] {
            let observed = extractions.clone();
            let (resume, resumed) = std::sync::mpsc::channel();
            releases.push(ResumeOnDrop(Some(resume)));
            let (done, returned) = tokio::sync::oneshot::channel();
            finished.push(returned);
            drop(std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = resumed.recv();
                    owner.release_with(|| {
                        observed.fetch_add(1, Ordering::AcqRel);
                    });
                }));
                let _ = done.send(result);
            }));
        }
        assert_eq!(resources.counts(), (PHYSICAL_HISTORY_BYTES, 0));
        drop(releases); // concurrent starts; every panic path still releases both
        for returned in finished {
            bounded(returned).await.unwrap().unwrap();
        }
        assert_eq!(extractions.load(Ordering::Acquire), 1);
        assert_eq!(resources.counts(), (0, 0));
    }
}
