//! Latest native request for a logical view. Entries hold weak tokens, never projection data.
use super::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

type Key = (usize, u64, StudioTarget);
type Requests = BTreeMap<Key, Weak<()>>;
fn requests() -> &'static Mutex<Requests> {
    static REQUESTS: OnceLock<Mutex<Requests>> = OnceLock::new();
    REQUESTS.get_or_init(Mutex::default)
}
pub(super) struct ViewRequest {
    key: Key,
    generation: Arc<()>,
}
impl ViewRequest {
    pub(super) fn begin(state: &AppState, server: u64, target: StudioTarget) -> Self {
        let key = (std::ptr::from_ref(state) as usize, server, target);
        let generation = Arc::new(());
        let mut requests = requests().lock().expect("Studio request map");
        // Native operation slots bound live requests. Dead entries disappear on every admission.
        requests.retain(|_, value| value.strong_count() != 0);
        requests.insert(key, Arc::downgrade(&generation));
        Self { key, generation }
    }
    pub(super) fn is_current(&self) -> bool {
        requests()
            .lock()
            .expect("Studio request map")
            .get(&self.key)
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, &self.generation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacing_one_view_does_not_revoke_another() {
        let state = AppState::default();
        let target = StudioTarget::Index { channel: [3; 16] };
        let other = StudioTarget::Index { channel: [4; 16] };
        let old = ViewRequest::begin(&state, 7, target);
        let separate = ViewRequest::begin(&state, 7, other);
        assert!(old.is_current());
        let new = ViewRequest::begin(&state, 7, target);
        assert!(!old.is_current());
        assert!(new.is_current());
        assert!(separate.is_current());
        drop(new);
        assert!(
            !old.is_current(),
            "dropping a newer request cannot revive its predecessor"
        );
    }
}
