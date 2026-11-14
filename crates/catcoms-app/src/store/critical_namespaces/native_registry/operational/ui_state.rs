//! Concrete local UI-sidecar IO, independent of the index WAL slot. These
//! capsules grant no Tauri, membership, Current, signing or SDK authority.
use super::upgrade_source::owning::UiWork;
use super::*;

const BYTES: usize = 4 * crate::store::MAX_UI_STATE_BYTES + 4096;
const POSITIONS: usize = 4;

/// Trusted local buffer policy. Four independent operations can retain their
/// own credits. Native separately bounds its queue, parser and IPC copies.
#[derive(Clone)]
pub struct UiStateBudget {
    pool: CriticalCatalogReadPool,
}
impl UiStateBudget {
    pub const fn required_capacity() -> usize {
        POSITIONS * BYTES
    }
    pub fn new(pool: CriticalCatalogReadPool) -> Result<Self, AppError> {
        if pool.capacity() < Self::required_capacity() {
            return Err(refused());
        }
        Ok(Self { pool })
    }
    /// Before native copies a payload or acquires Store/commit guards.
    pub fn prepare(&self) -> Result<UiStateCredit, AppError> {
        Ok(UiStateCredit {
            debit: self.pool.try_reserve(BYTES)?,
            pool: self.pool.clone(),
        })
    }
}
/// Unique payment, not a command credential. No Clone or serialization.
pub struct UiStateCredit {
    debit: CatalogReadReservation,
    pool: CriticalCatalogReadPool,
}
impl UiStateCredit {
    fn require(&self) -> Result<(), AppError> {
        self.debit.validate(&self.pool, BYTES)
    }
}
/// Infallible original capture, including Busy/version2. Keep the WHOLE value
/// outside guards before mapping/refusing so no payment or last physical file
/// is destroyed under the native Store guard.
pub struct UiStatePreparation {
    work: Option<UiWork>,
    host: catcoms_rt::NativeChatUiMountObservation,
    credit: UiStateCredit,
}
impl UiStatePreparation {
    pub(super) fn new(
        vault: &NativeTextVault,
        work: Option<UiWork>,
        credit: UiStateCredit,
    ) -> Self {
        Self {
            work,
            host: vault.observe_ui_mount(),
            credit,
        }
    }
    /// Blocking IO only. None means strict NotFound; Some(empty) remains a
    /// present record and MUST reach native's UTF-8/JSON validation.
    pub fn read(mut self) -> Result<UiStateRead, AppError> {
        self.credit.require()?;
        if !self.host.is_open() {
            return Err(refused());
        }
        let work = self.work.take().ok_or_else(refused)?;
        let bytes = work.read()?;
        if !self.host.is_open() || !work.is_live_cached() {
            return Err(refused());
        }
        Ok(UiStateRead {
            bytes,
            work,
            host: self.host,
            _credit: self.credit,
        })
    }
    /// Exact already-validated JSON bytes. Native admits its original session
    /// before starting; this does not itself authenticate a Tauri caller.
    /// Once IO ran an error never promises that the old file was unchanged.
    pub fn save(
        mut self,
        bytes: &[u8],
        rng: &mut impl catcoms_rt::CryptoRngCore,
    ) -> Result<UiStateSaved, AppError> {
        self.credit.require()?;
        if bytes.len() > crate::store::MAX_UI_STATE_BYTES || !self.host.is_open() {
            return Err(refused());
        }
        let work = self.work.take().ok_or_else(refused)?;
        work.save(bytes, rng)?;
        Ok(UiStateSaved {
            work,
            host: self.host,
            _credit: self.credit,
        })
    }
}
/// Output bytes FIRST, original physical slot/payment LAST. Native may borrow
/// the bytes through actual response publication but cannot extract an unpaid
/// Vec. This proves logical mount/session ownership, not resistance to an
/// out-of-process filesystem directory replacement during path-based IO.
pub struct UiStateRead {
    bytes: Option<Vec<u8>>,
    work: UiWork,
    host: catcoms_rt::NativeChatUiMountObservation,
    _credit: UiStateCredit,
}
impl UiStateRead {
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }
    pub fn is_live_cached(&self) -> bool {
        self.host.is_open() && self.work.is_live_cached()
    }
}
/// Actual successful IO, still bound to its original slot and metadata through
/// native's final publication check. A stale reply does not undo the disk effect.
pub struct UiStateSaved {
    work: UiWork,
    host: catcoms_rt::NativeChatUiMountObservation,
    _credit: UiStateCredit,
}
impl UiStateSaved {
    pub fn is_live_cached(&self) -> bool {
        self.host.is_open() && self.work.is_live_cached()
    }
}
macro_rules! opaque_ui_debug {
    ($($ty:ty),+) => {$(impl std::fmt::Debug for $ty {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
opaque_ui_debug!(
    UiStateBudget,
    UiStateCredit,
    UiStatePreparation,
    UiStateRead,
    UiStateSaved
);
const _: () = assert!(
    std::mem::size_of::<UiStatePreparation>()
        + std::mem::size_of::<UiStateRead>()
        + std::mem::size_of::<UiWork>()
        + 64 * std::mem::size_of::<usize>()
        < 4096
);
