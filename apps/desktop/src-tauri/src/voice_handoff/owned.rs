//! Private paired allocation custody for native channel metadata and Sessions.
//! No raw Arc/Weak escapes these wrappers. Every alias carries the SAME plain
//! payer outside actual body/header deallocation, including Registry teardown.
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Weak};

/// Extract the final plain payment AFTER its own Arc allocation is freed. The
/// into_inner single-winner guarantee also covers concurrent final aliases.
pub(crate) struct PlainOwner<T>(Option<Arc<T>>);
impl<T> PlainOwner<T> {
    pub(in crate::voice_handoff) fn new(value: T) -> Self {
        Self(Some(Arc::new(value)))
    }
    pub(in crate::voice_handoff) fn is_same(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    fn release_with(&mut self, after_deallocation: impl FnOnce()) {
        if let Some(original) = self.0.take() {
            if let Some(value) = Arc::into_inner(original) {
                after_deallocation();
                drop(value);
            }
        }
    }
}
impl<T> Clone for PlainOwner<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Drop for PlainOwner<T> {
    fn drop(&mut self) {
        self.release_with(|| {});
    }
}

pub(crate) struct OwnedArc<T, P> {
    body: Option<Arc<T>>,
    payer: P,
}
impl<T, P: Clone> OwnedArc<T, P> {
    /// Caller already owns the full original prepaid graph before constructing
    /// `value`. Only the closed handoff implementation can create an allocation.
    pub(in crate::voice_handoff) fn new(value: T, payer: P) -> Self {
        Self {
            body: Some(Arc::new(value)),
            payer,
        }
    }
    pub(in crate::voice_handoff) fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(
            self.body.as_ref().expect("original body"),
            other.body.as_ref().expect("original body"),
        )
    }
    pub(in crate::voice_handoff) fn strong_count(&self) -> usize {
        Arc::strong_count(self.body.as_ref().expect("original body"))
    }
    pub(in crate::voice_handoff) fn weak_count(&self) -> usize {
        Arc::weak_count(self.body.as_ref().expect("original body"))
    }
    pub(in crate::voice_handoff) fn downgrade(&self) -> OwnedWeak<T, P> {
        OwnedWeak {
            body: Some(Arc::downgrade(self.body.as_ref().expect("original body"))),
            payer: self.payer.clone(),
        }
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn get_mut(&mut self) -> Option<&mut T> {
        Arc::get_mut(self.body.as_mut().expect("original body"))
    }
}
impl<T, P: Clone> Clone for OwnedArc<T, P> {
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
            payer: self.payer.clone(),
        }
    }
}
impl<T, P> Deref for OwnedArc<T, P> {
    type Target = T;
    fn deref(&self) -> &T {
        self.body.as_deref().expect("original body")
    }
}
impl<T, P> Drop for OwnedArc<T, P> {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}

/// Legacy weak witnesses remain separately classified history, not actual-return
/// proof. Their payer nevertheless survives the final weak allocation too.
pub(crate) struct OwnedWeak<T, P> {
    body: Option<Weak<T>>,
    payer: P,
}
impl<T, P: Clone> OwnedWeak<T, P> {
    pub(in crate::voice_handoff) fn strong_count(&self) -> usize {
        self.body.as_ref().expect("original weak").strong_count()
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn upgrade(&self) -> Option<OwnedArc<T, P>> {
        self.body
            .as_ref()
            .expect("original weak")
            .upgrade()
            .map(|body| OwnedArc {
                body: Some(body),
                payer: self.payer.clone(),
            })
    }
}
impl<T, P: Clone> Clone for OwnedWeak<T, P> {
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
            payer: self.payer.clone(),
        }
    }
}
impl<T, P> Drop for OwnedWeak<T, P> {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}

/// The process index has one owner and no escaping references. Move its
/// original credit outside Box destruction rather than refunding inside it.
pub(in crate::voice_handoff) struct OwnedBox<T, P> {
    body: Option<Box<T>>,
    _payer: P,
}
impl<T, P> OwnedBox<T, P> {
    pub(in crate::voice_handoff) fn new(value: T, payer: P) -> Self {
        Self {
            body: Some(Box::new(value)),
            _payer: payer,
        }
    }
}
impl<T, P> Deref for OwnedBox<T, P> {
    type Target = T;
    fn deref(&self) -> &T {
        self.body.as_deref().expect("original index")
    }
}
impl<T, P> DerefMut for OwnedBox<T, P> {
    fn deref_mut(&mut self) -> &mut T {
        self.body.as_deref_mut().expect("original index")
    }
}
impl<T, P> Drop for OwnedBox<T, P> {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::Clock;
    use catcoms_sync::voice_v2::resources::{Credit, ResourceKind, Resources};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Resume(Option<std::sync::mpsc::Sender<()>>);
    impl Drop for Resume {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::select! { value = future => value,
        _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => panic!("paired allocation return exceeded bound"), }
    }
    fn payer(resources: &Arc<Resources>) -> PlainOwner<Credit> {
        PlainOwner::new(
            resources
                .charge([93; 32], 1024, ResourceKind::Retained)
                .unwrap(),
        )
    }

    #[test]
    fn paired_metadata_weak_keeps_the_actual_empty_allocation_paid_after_body_return() {
        struct Body(Arc<AtomicUsize>);
        impl Drop for Body {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::AcqRel);
            }
        }
        let resources = Arc::new(Resources::default());
        let destroyed = Arc::new(AtomicUsize::new(0));
        let body = OwnedArc::new(Body(destroyed.clone()), payer(&resources));
        let weak = body.downgrade();
        let alias = weak.upgrade().unwrap();
        assert!(alias.is_same(&body));
        drop((body, alias));
        assert_eq!(destroyed.load(Ordering::Acquire), 1);
        assert!(weak.upgrade().is_none());
        assert_eq!(resources.counts(), (1024, 0));
        drop(weak);
        assert_eq!(resources.counts(), (0, 0));
    }

    #[tokio::test]
    async fn paired_plain_payer_refunds_after_its_actual_arc_deallocation() {
        let resources = Arc::new(Resources::default());
        let mut first = payer(&resources);
        let mut last = first.clone();
        let extractions = Arc::new(AtomicUsize::new(0));
        let (entered, at_deallocation) = tokio::sync::oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let release = Resume(Some(resume));
        let (done, finished) = tokio::sync::oneshot::channel();
        let observed = extractions.clone();
        // Retire a nonlast alias first, then park the known actual last winner
        // AFTER allocation deallocation, with its original value still paid.
        let (start, started) = std::sync::mpsc::channel();
        let start_release = Resume(Some(start));
        let (other_done, other_finished) = tokio::sync::oneshot::channel();
        let first_observed = extractions.clone();
        drop(std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = started.recv();
                first.release_with(|| {
                    first_observed.fetch_add(1, Ordering::AcqRel);
                });
            }));
            let _ = other_done.send(result);
        }));
        drop(start_release);
        bounded(other_finished).await.unwrap().unwrap();
        drop(std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                last.release_with(|| {
                    observed.fetch_add(1, Ordering::AcqRel);
                    let _ = entered.send(());
                    let _ = resumed.recv();
                });
            }));
            let _ = done.send(result);
        }));
        bounded(at_deallocation).await.unwrap();
        assert_eq!(resources.counts(), (1024, 0));
        assert_eq!(extractions.load(Ordering::Acquire), 1);
        drop(release);
        bounded(finished).await.unwrap().unwrap();
        assert_eq!(resources.counts(), (0, 0));
    }

    #[tokio::test]
    async fn paired_plain_payer_concurrent_final_aliases_have_one_extraction_and_refund() {
        let resources = Arc::new(Resources::default());
        let original = payer(&resources);
        let other = original.clone();
        let extractions = Arc::new(AtomicUsize::new(0));
        let mut releases = Vec::with_capacity(2);
        let mut finished = Vec::with_capacity(2);
        for mut owner in [original, other] {
            let (resume, resumed) = std::sync::mpsc::channel();
            releases.push(Resume(Some(resume)));
            let (done, returned) = tokio::sync::oneshot::channel();
            finished.push(returned);
            let observed = extractions.clone();
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
        drop(releases);
        for returned in finished {
            bounded(returned).await.unwrap().unwrap();
        }
        assert_eq!(extractions.load(Ordering::Acquire), 1);
        assert_eq!(resources.counts(), (0, 0));
    }
}
