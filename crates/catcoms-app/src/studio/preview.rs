//! Volatile read results. A preview is never an ordinary view or an editable epoch.
use super::*;
use crate::studio_exchange::provisional::ServerPreparedProvisionalStudioSeed;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug)]
pub enum StudioRead {
    Document(StudioView),
    AwaitingTenureReceipt(StudioPreview),
}

#[derive(Debug)]
pub struct StudioPreview {
    pub(crate) seed: Arc<ServerPreparedProvisionalStudioSeed>,
    delivery: Option<StudioPreviewDelivery>,
}

/// Keep this guard through conversion and the final native session/view checks. Dropping the
/// last copy releases the actor's bounded handoff. Timeout/cancellation revokes all copies.
#[derive(Clone, Debug)]
pub struct StudioPreviewDelivery(Arc<Delivery>);
#[derive(Debug)]
struct Delivery {
    valid: Arc<AtomicBool>,
    seed: Arc<ServerPreparedProvisionalStudioSeed>,
    _ack: oneshot::Sender<()>,
}
impl StudioPreviewDelivery {
    pub fn is_current(&self) -> bool {
        self.0.valid.load(Ordering::Acquire) && self.0.seed.unconfirmed_is_unexpired()
    }
}
impl StudioPreview {
    pub(crate) fn new(seed: Arc<ServerPreparedProvisionalStudioSeed>) -> Self {
        Self {
            seed,
            delivery: None,
        }
    }
    pub fn delivery(&self) -> StudioPreviewDelivery {
        self.delivery
            .as_ref()
            .expect("actor preview delivery")
            .clone()
    }
    /// Data remains unconfirmed even though its detached typed validation succeeded.
    pub fn inspect<O>(
        &self,
        inspect: impl FnOnce(u128, &StudioProjection) -> O,
    ) -> Result<O, String> {
        if !self.delivery().is_current() || !self.seed.unconfirmed_is_unexpired() {
            return Err("Studio preview expired; refresh".into());
        }
        Ok(inspect(
            self.seed.unconfirmed_doc_id(),
            self.seed.unconfirmed_projection(),
        ))
    }
    pub(crate) fn begin_delivery(&mut self) -> PreviewHandoff {
        let (ack, done) = oneshot::channel();
        let valid = Arc::new(AtomicBool::new(true));
        self.delivery = Some(StudioPreviewDelivery(Arc::new(Delivery {
            valid: valid.clone(),
            seed: self.seed.clone(),
            _ack: ack,
        })));
        PreviewHandoff { done, valid }
    }
}
pub(crate) struct PreviewHandoff {
    done: oneshot::Receiver<()>,
    valid: Arc<AtomicBool>,
}
impl Drop for PreviewHandoff {
    fn drop(&mut self) {
        self.valid.store(false, Ordering::Release);
    }
}
impl PreviewHandoff {
    /// No vault lease or network work here. The actor retains its already checked state until
    /// native finishes conversion, drops its result, cancels, or the fixed handoff timeout.
    pub(crate) async fn finish(
        mut self,
        clock: Arc<dyn catcoms_rt::Clock + Send>,
        mut cancellation: Option<catcoms_rt::RequestCancellation>,
    ) {
        tokio::select! {
            biased;
            _ = async { match cancellation.as_mut() { Some(c) => c.cancelled().await, None => std::future::pending::<()>().await } } => {},
            _ = clock.sleep(std::time::Duration::from_secs(5)) => {},
            _ = &mut self.done => {},
        }
    }
}
