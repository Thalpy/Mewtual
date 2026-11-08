//! Negative destructor observation of an EXISTING actual read body. No new read,
//! source upgrade, fee replacement or owning SDK/capture loan is possible here.
use super::*;
impl V2TextReadResources {
    pub(in crate::store) fn original_body_liveness_for_test(&self) -> impl Fn() -> bool + 'static {
        let body = Arc::downgrade(&self.body);
        move || body.strong_count() != 0
    }
}
