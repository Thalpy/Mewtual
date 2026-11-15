//! The one exact v2 invoice shared by its handler, gate and retained claims.
//! A pool handle, receipt, claim label or cloned Arc creates no additional debit.
use super::*;

pub(super) struct IndexPayment {
    #[cfg(test)]
    before_refund: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    refunded: Arc<std::sync::atomic::AtomicBool>,
    // Keep the actual debit last: payment metadata/observations retire first.
    pub(super) debit: CatalogReadReservation,
}
impl IndexPayment {
    pub(super) fn new(debit: CatalogReadReservation) -> Self {
        #[cfg(test)]
        {
            let mut debit = debit;
            let refunded = Arc::new(std::sync::atomic::AtomicBool::new(false));
            debit.observe_drop_for_test(refunded.clone());
            Self {
                before_refund: std::sync::Mutex::new(None),
                refunded,
                debit,
            }
        }
        #[cfg(not(test))]
        Self { debit }
    }
    #[cfg(test)]
    pub(super) fn observe_before_refund(&self, check: Box<dyn FnOnce() + Send>) {
        *self.before_refund.lock().unwrap() = Some(check);
    }
    #[cfg(test)]
    pub(super) fn refund_notice(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.refunded.clone()
    }
}
impl Drop for IndexPayment {
    fn drop(&mut self) {
        // This test observation runs outside the account mutex, immediately
        // before the exact non-Clone reservation's destructor refunds it.
        #[cfg(test)]
        if let Some(check) = self
            .before_refund
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            check();
        }
    }
}
