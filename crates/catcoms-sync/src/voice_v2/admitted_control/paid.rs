//! Every escaping Core clone retains its SAME plain payment outside Core's Arc.
//! No raw Core Arc/Weak is exported; Vec/Box fields and final allocation die first.
use super::*;
pub(super) struct Payment(Credit);
struct Funding {
    _credit: Credit,
}
pub(super) struct Retained<T> {
    body: Arc<T>,
    _funding: Arc<Funding>,
}
impl<T> std::fmt::Debug for Retained<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RetainedControlOwner { .. }")
    }
}
impl Payment {
    pub(super) fn reserve(
        resources: &Arc<Resources>,
        group: Nonce,
        bytes: usize,
        kind: ResourceKind,
    ) -> Result<Self, VoiceError> {
        Ok(Self(resources.charge(group, bytes, kind)?))
    }
    pub(super) fn retain<T>(self, body: T) -> Retained<T> {
        let funding = Arc::new(Funding { _credit: self.0 });
        Retained {
            body: Arc::new(body),
            _funding: funding,
        }
    }
}
impl<T> Clone for Retained<T> {
    fn clone(&self) -> Self {
        Self {
            body: self.body.clone(),
            _funding: self._funding.clone(),
        }
    }
}
impl<T> Retained<T> {
    pub(super) fn get(&self) -> &T {
        &self.body
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.body, &other.body)
    }
}
