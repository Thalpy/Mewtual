//! A reserved position in Root's ONE public/encrypted admission namespace. Packet-table
//! capacity is separate. Ordinary admissions must include these positions in their full check.
//! Root's exclusive actor borrow serializes reservation/ordinary insertion; release is atomic
//! so native custody can disappear outside Root without retaining the whole Root allocation.
use super::VoiceError;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Default)]
pub(crate) struct ReceiptNamespace {
    held: Arc<AtomicUsize>,
}
pub(crate) struct ReceiptPosition {
    held: Arc<AtomicUsize>,
    live: bool,
}
impl ReceiptNamespace {
    #[cfg(test)]
    pub(crate) fn total_for_test(&self, occupied: usize) -> usize {
        occupied + self.held.load(Ordering::Acquire)
    }
    pub(crate) fn full(&self, occupied: usize, maximum: usize) -> bool {
        occupied.saturating_add(self.held.load(Ordering::Acquire)) >= maximum
    }
    pub(crate) fn reserve(
        &mut self,
        occupied: usize,
        maximum: usize,
    ) -> Result<ReceiptPosition, VoiceError> {
        if self.full(occupied, maximum) {
            return Err(VoiceError::Capacity);
        }
        self.held.fetch_add(1, Ordering::AcqRel);
        Ok(ReceiptPosition {
            held: self.held.clone(),
            live: true,
        })
    }
    pub(crate) fn owns(&self, position: &ReceiptPosition) -> bool {
        position.live && Arc::ptr_eq(&self.held, &position.held)
    }
}
impl ReceiptPosition {
    /// Only the final native apply tail consumes this. Root's exclusive borrow ensures
    /// no ordinary insert can interleave before its corresponding receipt-map insertion.
    pub(in crate::voice_v2) fn consume(&mut self) {
        assert!(self.live, "one reserved endpoint receipt");
        self.live = false;
        assert!(
            self.held.fetch_sub(1, Ordering::AcqRel) > 0,
            "reserved receipt position"
        );
    }
}
impl Drop for ReceiptPosition {
    fn drop(&mut self) {
        if self.live {
            self.consume();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_position_blocks_ordinary_admission_and_rejects_foreign_root() {
        let mut root = ReceiptNamespace::default();
        let other = ReceiptNamespace::default();
        let mut position = root.reserve(255, 256).unwrap();
        assert!(root.full(255, 256));
        assert!(!other.owns(&position));
        assert!(root.owns(&position));
        assert!(root.reserve(255, 256).is_err());
        position.consume();
        assert!(!root.owns(&position));
        assert!(!root.full(255, 256));
        let position = root.reserve(255, 256).unwrap();
        drop(position);
        assert!(!root.full(255, 256));
    }
}
