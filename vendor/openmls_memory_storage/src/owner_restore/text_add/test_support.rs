//! Negative-only test seams at REAL candidate storage/serializer edges. This
//! module is absent from the ordinary library build. No callback, SDK/provider
//! extractor, output adopter, Source grant or counter setter is exposed.
use super::*;
use std::cell::RefCell;
use std::sync::atomic::AtomicUsize;

#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAddTestEvent {
    PendingSerialize,
    SerializeEntered,
    PendingWritten,
    OldRatchetsWritten,
    MergeStateWritten,
    MergeTreeWritten,
    MergeEpochRead,
    MergeEpochWritten,
    MergeEpochDeleted,
}
const EVENTS: usize = 9;
struct Registration {
    counts: [AtomicUsize; EVENTS],
    fail: Option<(TextAddTestEvent, usize)>,
    pending_credit: Option<usize>,
}
thread_local! {
    static ACTIVE: RefCell<Option<Arc<Registration>>> = const { RefCell::new(None) };
}

/// One thread's fixed storage fault plan. Counts observe actual entries; a
/// pending credit override can only LOWER the real typed remaining account.
pub struct TextAddTestObservation {
    registration: Arc<Registration>,
    // Drop must clear the installing thread's TLS, never another thread's slot.
    _same_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl fmt::Debug for TextAddTestObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextAddTestObservation")
            .finish_non_exhaustive()
    }
}
impl TextAddTestObservation {
    pub fn install(fail: Option<(TextAddTestEvent, usize)>, pending_credit: Option<usize>) -> Self {
        let registration = Arc::new(Registration {
            counts: std::array::from_fn(|_| AtomicUsize::new(0)),
            fail,
            pending_credit,
        });
        ACTIVE.with(|active| {
            assert!(
                active.borrow().is_none(),
                "one private Add test plan per thread"
            );
            *active.borrow_mut() = Some(registration.clone());
        });
        Self {
            registration,
            _same_thread: std::marker::PhantomData,
        }
    }
    pub fn count(&self, event: TextAddTestEvent) -> usize {
        self.registration.counts[event as usize].load(Ordering::Relaxed)
    }
    pub fn serializer_temporary_bytes() -> usize {
        account::SERIALIZE_TEMPORARY_REQUESTS
    }
}
impl Drop for TextAddTestObservation {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            let mut slot = active.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|r| Arc::ptr_eq(r, &self.registration))
            {
                *slot = None;
            }
        });
    }
}
pub(super) fn event(event: TextAddTestEvent) -> Result<(), MemoryStorageError> {
    ACTIVE.with(|active| {
        let slot = active.borrow();
        let Some(registration) = slot.as_ref() else {
            return Ok(());
        };
        let count = registration.counts[event as usize].fetch_add(1, Ordering::Relaxed) + 1;
        if registration.fail == Some((event, count)) {
            Err(Error.into())
        } else {
            Ok(())
        }
    })
}
pub(super) fn pending_serialize(a: &mut OperationAccount) -> Result<(), MemoryStorageError> {
    ACTIVE.with(|active| {
        if let Some(credit) = active.borrow().as_ref().and_then(|r| r.pending_credit) {
            a.typed.bytes = a.typed.bytes.min(credit);
        }
    });
    event(TextAddTestEvent::PendingSerialize)
}
