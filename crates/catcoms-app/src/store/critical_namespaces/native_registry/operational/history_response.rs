//! One independent native history-response invoice. Payment only: no Host,
//! UI generation, Source, SDK, physical owner or historical authority follows.
use super::*;

const BYTES: usize = 8 * 1024 * 1024;
const POSITIONS: usize = 2;

/// Trusted native buffer policy for two overlapping bounded response owners.
/// Use a dedicated account, separate from authentication/UI/key admission and
/// the Root saved-original page's own payment. This is not an RSS/IPC/WebView
/// bound; native must enforce its capped serializer and retain this credit
/// after the actual page, serialized bytes and all temporary output buffers.
#[derive(Clone)]
pub struct NativeTextHistoryResponseBudget {
    pool: CriticalCatalogReadPool,
}
impl NativeTextHistoryResponseBudget {
    pub const fn required_capacity() -> usize {
        POSITIONS * BYTES
    }
    pub const fn response_bytes() -> usize {
        BYTES
    }
    pub fn new(pool: CriticalCatalogReadPool) -> Result<Self, AppError> {
        if pool.capacity() < Self::required_capacity() {
            return Err(refused());
        }
        Ok(Self { pool })
    }
    /// Reserve the complete quote BEFORE copies, work or native guards. A
    /// single affine payment avoids thousands of smaller metadata allocations.
    pub fn prepare(&self) -> Result<NativeTextHistoryResponseCredit, AppError> {
        Ok(NativeTextHistoryResponseCredit {
            _debit: self.pool.try_reserve(BYTES)?,
        })
    }
}
/// Unique buffer payment, deliberately not Clone/Serialize and no authority
/// accessor. Keep LAST in native's whole owning worker/response envelope.
pub struct NativeTextHistoryResponseCredit {
    _debit: CatalogReadReservation,
}
impl std::fmt::Debug for NativeTextHistoryResponseBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextHistoryResponseBudget")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for NativeTextHistoryResponseCredit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextHistoryResponseCredit")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_response_quote_rejects_short_configuration_before_any_debit() {
        let pool =
            CriticalCatalogReadPool::new(NativeTextHistoryResponseBudget::required_capacity() - 1)
                .unwrap();
        assert!(NativeTextHistoryResponseBudget::new(pool.clone()).is_err());
        assert_eq!(pool.charged_for_test(), 0);
    }
    #[test]
    fn history_response_two_actual_credits_are_affine_and_refuse_overlap_before_work() {
        let pool =
            CriticalCatalogReadPool::new(NativeTextHistoryResponseBudget::required_capacity())
                .unwrap();
        let budget = NativeTextHistoryResponseBudget::new(pool.clone()).unwrap();
        let first = budget.prepare().unwrap();
        let second = budget.prepare().unwrap();
        assert_eq!(pool.charged_for_test(), 2 * BYTES);
        assert!(budget.prepare().is_err());
        drop(first);
        let replacement = budget.prepare().unwrap();
        assert_eq!(pool.charged_for_test(), 2 * BYTES);
        drop((second, replacement, budget));
        assert_eq!(pool.charged_for_test(), 0);
    }
    #[test]
    fn history_response_original_credit_survives_policy_drop_without_other_pool_substitution() {
        let pool =
            CriticalCatalogReadPool::new(NativeTextHistoryResponseBudget::required_capacity())
                .unwrap();
        let budget = NativeTextHistoryResponseBudget::new(pool.clone()).unwrap();
        let credit = budget.prepare().unwrap();
        drop(budget);
        let other =
            CriticalCatalogReadPool::new(NativeTextHistoryResponseBudget::required_capacity())
                .unwrap();
        assert_eq!(pool.charged_for_test(), BYTES);
        assert_eq!(other.charged_for_test(), 0);
        drop(credit);
        assert_eq!(pool.charged_for_test(), 0);
    }
}
