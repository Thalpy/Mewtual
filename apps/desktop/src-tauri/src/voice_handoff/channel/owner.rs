//! Original Session allocation custody, including raw-header lifetime after the
//! last strong owner. No Arc/Weak escapes this closed wrapper.
use super::*;
use crate::voice_handoff::owned::PlainOwner;
use std::ops::Deref;

pub(in crate::voice_handoff) struct SessionFunding {
    _control: Credit,
    _browser_control: Credit,
}
/// Every strong alias pairs the original body with SAME outside payments. The
/// original2KiB bootstrap fee independently covers the emptied allocation while
/// private weak witnesses remain; it is not a new reservation or synthetic refund.
pub(in crate::voice_handoff) struct SessionRef {
    body: Option<Arc<Session>>,
    funding: PlainOwner<SessionFunding>,
    tail: PlainOwner<Credit>,
}
pub(in crate::voice_handoff) struct SessionWeak {
    body: Option<std::sync::Weak<Session>>,
    tail: PlainOwner<Credit>,
}
impl SessionRef {
    pub(super) fn new(
        mut body: Session,
        control: Credit,
        bootstrap: Credit,
        browser_control: Credit,
    ) -> Self {
        let funding = PlainOwner::new(SessionFunding {
            _control: control,
            _browser_control: browser_control,
        });
        // The body clone allows only an existing live-body weak upgrade to
        // recover the SAME payment. Funding contains no Session/Actor backedge.
        body.funding = Some(funding.clone());
        Self {
            body: Some(Arc::new(body)),
            funding,
            tail: PlainOwner::new(bootstrap),
        }
    }
    pub(in crate::voice_handoff) fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(
            self.body.as_ref().expect("original Session"),
            other.body.as_ref().expect("original Session"),
        )
    }
    pub(in crate::voice_handoff) fn strong_count(&self) -> usize {
        Arc::strong_count(self.body.as_ref().expect("original Session"))
    }
    pub(in crate::voice_handoff) fn weak_count(&self) -> usize {
        Arc::weak_count(self.body.as_ref().expect("original Session"))
    }
    pub(in crate::voice_handoff) fn downgrade(&self) -> SessionWeak {
        SessionWeak {
            body: Some(Arc::downgrade(
                self.body.as_ref().expect("original Session"),
            )),
            tail: self.tail.clone(),
        }
    }
    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_pause_final_drop(
        &self,
    ) -> (tokio::sync::oneshot::Receiver<()>, ReturnRelease) {
        let (entered, observed) = tokio::sync::oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let mut boundary = self.return_boundary.lock().unwrap();
        assert!(boundary.is_none(), "one original destruction barrier");
        *boundary = Some(ReturnBoundary {
            entered: Some(entered),
            resume: resumed,
        });
        (observed, ReturnRelease(Some(resume)))
    }
}
impl Clone for SessionRef {
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
            funding: self.funding.clone(),
            tail: self.tail.clone(),
        }
    }
}
impl Deref for SessionRef {
    type Target = Session;
    fn deref(&self) -> &Session {
        self.body.as_deref().expect("original Session")
    }
}
impl Drop for SessionRef {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}
impl SessionWeak {
    pub(in crate::voice_handoff) fn strong_count(&self) -> usize {
        self.body
            .as_ref()
            .expect("original Session witness")
            .strong_count()
    }
    pub(in crate::voice_handoff) fn upgrade(&self) -> Option<SessionRef> {
        let body = self
            .body
            .as_ref()
            .expect("original Session witness")
            .upgrade()?;
        let funding = body
            .funding
            .as_ref()
            .expect("original outside payment")
            .clone();
        Some(SessionRef {
            body: Some(body),
            funding,
            tail: self.tail.clone(),
        })
    }
}
impl Clone for SessionWeak {
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
            tail: self.tail.clone(),
        }
    }
}
impl Drop for SessionWeak {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}

/// Added to existing invoices; no old Session/header/future term is removed
/// merely because credits moved outside the body.
pub(in crate::voice_handoff) fn session_owner_layout_bytes() -> usize {
    #[repr(C)]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }
    std::mem::size_of::<ArcAllocation<SessionFunding>>()
        + std::mem::size_of::<ArcAllocation<Credit>>()
        + 4 * std::mem::size_of::<SessionRef>()
        + 2 * std::mem::size_of::<SessionWeak>()
}

#[cfg(test)]
pub(super) struct ReturnBoundary {
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    resume: std::sync::mpsc::Receiver<()>,
}
#[cfg(test)]
impl ReturnBoundary {
    pub(super) fn wait(mut self) {
        let _ = self.entered.take().expect("one actual destructor").send(());
        let _ = self.resume.recv();
    }
}
#[cfg(test)]
pub(in crate::voice_handoff) struct ReturnRelease(Option<std::sync::mpsc::Sender<()>>);
#[cfg(test)]
impl Drop for ReturnRelease {
    fn drop(&mut self) {
        if let Some(resume) = self.0.take() {
            let _ = resume.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::Clock;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::select! {
            value = future => value,
            _ = catcoms_rt::SystemClock.sleep(std::time::Duration::from_secs(30)) => {
                panic!("actual Session destruction exceeded its observation bound")
            }
        }
    }

    #[tokio::test]
    async fn final_session_body_and_weak_allocation_keep_their_original_payments_until_actual_return(
    ) {
        let resources = Arc::new(Resources::default());
        let session = Session::new(
            resources.clone(),
            [91; 32],
            1,
            &mut ChaCha20Rng::from_seed([11; 32]),
        )
        .unwrap();
        let weak = session.downgrade();
        let paid = resources.counts();
        assert_eq!(
            paid,
            (CONTROL_BYTES + BOOTSTRAP_BYTES + BROWSER_CONTROL_BYTES, 1)
        );
        let (entered, release) = session.fixture_pause_final_drop();
        let (finished, returned) = tokio::sync::oneshot::channel();
        drop(std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(session)));
            let _ = finished.send(result);
        }));
        bounded(entered).await.unwrap();
        assert_eq!(weak.strong_count(), 0);
        assert!(weak.upgrade().is_none());
        assert_eq!(
            resources.counts(),
            paid,
            "strong0 is not actual destructor return"
        );
        // RAII resumes the producer even if an assertion above unwinds.
        drop(release);
        bounded(returned).await.unwrap().unwrap();
        assert_eq!(resources.counts(), (BOOTSTRAP_BYTES, 0));
        drop(weak);
        assert_eq!(resources.counts(), (0, 0));
    }

    #[tokio::test]
    async fn concurrent_final_session_aliases_keep_payment_through_the_single_body_destructor() {
        let resources = Arc::new(Resources::default());
        let session = Session::new(
            resources.clone(),
            [92; 32],
            1,
            &mut ChaCha20Rng::from_seed([12; 32]),
        )
        .unwrap();
        let (entered, release) = session.fixture_pause_final_drop();
        let alias = session.clone();
        let paid = resources.counts();
        let mut starts = Vec::with_capacity(2);
        let mut returns = Vec::with_capacity(2);
        for original in [session, alias] {
            let (start, started) = std::sync::mpsc::channel();
            starts.push(ReturnRelease(Some(start)));
            let (finished, returned) = tokio::sync::oneshot::channel();
            returns.push(returned);
            drop(std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = started.recv();
                    drop(original);
                }));
                let _ = finished.send(result);
            }));
        }
        drop(starts);
        bounded(entered).await.unwrap();
        assert_eq!(resources.counts(), paid);
        drop(release);
        for returned in returns {
            bounded(returned).await.unwrap().unwrap();
        }
        assert_eq!(resources.counts(), (0, 0));
    }
}
