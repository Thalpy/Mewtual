//! Paid original-history DATA returned only after whole runtime restoration.
use super::*;
use crate::store::persistence::v2_member_outbox::{
    DispositionCode, MemberEntryStatus, MemberHistoryItem, MemberHistoryPage, MemberHistoryRead,
};
use catcoms_replication::chat::SignedAction;

/// Local outbox disposition only. Submitted says bytes reached local transport,
/// never that another member received, accepted or durably saved them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextMemberHistoryStatus {
    Pending,
    Submitted,
    Held,
}

/// An owning bounded page of this member's actual saved originals. Contains no
/// Store, SDK, source issuer, physical lease or positive native permission.
/// Native serialization/copies require their own independently bounded payment.
pub struct NativeTextMemberHistoryPage {
    inner: MemberHistoryPage,
}
pub struct NativeTextMemberHistoryItem<'a> {
    inner: &'a MemberHistoryItem,
}
impl std::fmt::Debug for NativeTextMemberHistoryItem<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextMemberHistoryItem")
            .finish_non_exhaustive()
    }
}
impl NativeTextMemberHistoryPage {
    pub(in crate::store::critical_namespaces::native_registry) fn from_original(
        inner: MemberHistoryPage,
    ) -> Self {
        Self { inner }
    }
    pub fn next(&self) -> u64 {
        self.inner.next()
    }
    pub fn end_exclusive(&self) -> u64 {
        self.inner.end_exclusive()
    }
    pub fn items(&self) -> impl ExactSizeIterator<Item = NativeTextMemberHistoryItem<'_>> {
        self.inner
            .items()
            .iter()
            .map(|inner| NativeTextMemberHistoryItem { inner })
    }
}
impl NativeTextMemberHistoryItem<'_> {
    pub fn ordinal(&self) -> u64 {
        self.inner.receipt().ordinal
    }
    pub fn action(&self) -> &SignedAction {
        self.inner.action()
    }
    pub fn original_bytes(&self) -> &[u8] {
        self.inner.original_bytes()
    }
    pub fn status(&self) -> NativeTextMemberHistoryStatus {
        match self.inner.status() {
            MemberEntryStatus::Pending => NativeTextMemberHistoryStatus::Pending,
            MemberEntryStatus::Settled(DispositionCode::Submitted) => {
                NativeTextMemberHistoryStatus::Submitted
            }
            MemberEntryStatus::Settled(DispositionCode::Held) => {
                NativeTextMemberHistoryStatus::Held
            }
        }
    }
}
impl std::fmt::Debug for NativeTextMemberHistoryPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextMemberHistoryPage")
            .field("items", &self.inner.items().len())
            .finish_non_exhaustive()
    }
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberJob<T, R> {
    /// Blocking preparation outside all native guards: bounded context setup
    /// or recovery, then the saved-original page. max is 1..=8; reuse the first
    /// page's end_exclusive for finite continuation. Full account+row negatives
    /// are checked before/after work; the real native caller still gates reply
    /// delivery against its exact original UI generation.
    pub fn prepare_member_history(
        mut self,
        start: u64,
        end_exclusive: Option<u64>,
        maximum: usize,
        negative: &RequestCancellation,
    ) -> Result<NativeTextMemberHistoryPage, V2MemberSaveRefusal> {
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }
        self.inner
            .require_current()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        let body = self
            .inner
            .loan
            .body
            .as_mut()
            .ok_or(V2MemberSaveRefusal::Unavailable)?;
        let request = MemberHistoryRead {
            start,
            end: end_exclusive,
            maximum,
        };
        let rng = body.rng.as_mut().ok_or(V2MemberSaveRefusal::Unavailable)?;
        let result = if let Some(founder) = body.recovered_founder.as_mut() {
            founder.read_own_history(request, negative, rng)
        } else {
            body.work.prepare_native_member_history(
                body.driver
                    .as_mut()
                    .ok_or(V2MemberSaveRefusal::Unavailable)?,
                request,
                negative,
                rng,
                &mut self.inner.keys,
            )
        };
        #[cfg(all(test, any(windows, target_os = "linux")))]
        super::tests::after_history();
        self.inner.loan.recorded = true; // known return, never new signing authority
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }
        self.inner
            .require_current()
            .map_err(|_| V2MemberSaveRefusal::Cancelled)?;
        result.map(NativeTextMemberHistoryPage::from_original)
        // Consuming self restores the whole runtime BEFORE the paid page leaves.
    }
}
