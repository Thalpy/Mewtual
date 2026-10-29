//! Capacity-only ownership of the original group and its two empty ledger
//! allocations. No raw Arc/Weak leaves these wrappers; every clone keeps the
//! SAME plain payer outside all sponsored allocations until actual destruction.
use super::*;
use std::ops::Deref;

pub(super) const GROUP_METADATA_BYTES: usize = 3 * 1024;
struct GroupFunding {
    _credit: Credit,
}
pub(super) struct FundingOwner {
    inner: Option<Arc<GroupFunding>>,
}
impl Clone for FundingOwner {
    fn clone(&self) -> Self {
        Self {
            inner: Some(self.inner.as_ref().expect("live group funding").clone()),
        }
    }
}
impl Drop for FundingOwner {
    fn drop(&mut self) {
        // Every owner uses into_inner, and no Weak/raw Arc escapes. Concurrent
        // final drops yield the plain Credit to exactly one caller AFTER the
        // funding Arc allocation/header has also been freed.
        if let Some(funding) = self.inner.take().and_then(Arc::into_inner) {
            drop(funding);
        }
    }
}
impl FundingOwner {
    pub(super) fn reserve(resources: &Arc<Resources>, group: [u8; 32]) -> Result<Self, VoiceError> {
        if inventory_bytes() > GROUP_METADATA_BYTES {
            return Err(VoiceError::Bounds);
        }
        let credit = resources.charge(group, GROUP_METADATA_BYTES, ResourceKind::Retained)?;
        Ok(Self {
            inner: Some(Arc::new(GroupFunding { _credit: credit })),
        })
    }
}

macro_rules! owner {
    ($visibility:vis $name:ident, $body:ty) => {
        /// Opaque paid alias of the original body. Clones retain the SAME
        /// outside group payment; dereference exposes the body, never its Arc.
        /// Possession/identity is not current actor, membership or route authority.
        $visibility struct $name { body: Option<Arc<$body>>, funding: FundingOwner }
        impl $name {
            pub(super) fn new(body: $body, funding: FundingOwner) -> Self {
                Self { body: Some(Arc::new(body)), funding }
            }
            /// Pure original-allocation identity, including after retirement.
            /// Actual ledger operations independently recheck the current actor.
            pub fn is_same(&self, other: &Self) -> bool {
                Arc::ptr_eq(self.body.as_ref().expect("original body"),other.body.as_ref().expect("original body"))
            }
        }
        impl Clone for $name {
            fn clone(&self) -> Self {
                Self { body: Some(self.body.as_ref().expect("original body").clone()), funding: self.funding.clone() }
            }
        }
        impl Deref for $name {
            type Target = $body;
            fn deref(&self) -> &Self::Target { self.body.as_deref().expect("original body") }
        }
        impl Drop for $name {
            fn drop(&mut self) {
                // Actual body AND Arc allocation return precede FundingOwner.
                // Remaining aliases always carry their own same-payer wrapper.
                drop(self.body.take());
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self,f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(concat!(stringify!($name)," { .. }")) }
        }
    }
}
owner!(pub(super) GroupOwner, Group);
owner!(pub HelperOwner, super::super::helper::HelperLedger);
owner!(pub SourceOwner, super::super::source::SourceLedger);

/// Must be installed BEFORE Runtime invokes any revocation waker. Its structural
/// retirement order also runs during unwind, unlike trailing statements in Drop.
pub(super) struct GroupRetirement {
    groups: BTreeMap<[u8; 32], GroupOwner>,
}
impl GroupRetirement {
    pub(super) fn take(groups: &mut BTreeMap<[u8; 32], GroupOwner>) -> Self {
        Self {
            groups: std::mem::take(groups),
        }
    }
}
impl Drop for GroupRetirement {
    fn drop(&mut self) {
        let mut last = None;
        while let Some((_, owner)) = self.groups.pop_first() {
            drop(last.replace(owner));
        }
        drop(std::mem::take(&mut self.groups));
        #[cfg(test)]
        if let Some(owner) = &last {
            tests::map_returned(
                Arc::as_ptr(owner.body.as_ref().expect("original final group")) as usize,
            );
        }
        drop(last);
    }
}

/// Test clocks must inspect the ACTUAL lock without creating Clock -> Group ->
/// ledger -> Clock cycles. The hidden Weak is paired with the same outside payer
/// through its final header deallocation; no raw Arc/Weak can escape inspection.
#[cfg(test)]
pub(super) struct GroupProbe {
    body: Option<Weak<Group>>,
    _funding: FundingOwner,
}
#[cfg(test)]
impl GroupOwner {
    pub(super) fn probe(&self) -> GroupProbe {
        GroupProbe {
            body: Some(Arc::downgrade(self.body.as_ref().expect("original group"))),
            _funding: self.funding.clone(),
        }
    }
}
#[cfg(test)]
impl GroupProbe {
    pub(super) fn inspect<T>(&self, check: impl FnOnce(&Group) -> T) -> Option<T> {
        let body = self.body.as_ref()?.upgrade()?;
        Some(check(&body))
    }
}
#[cfg(test)]
impl Drop for GroupProbe {
    fn drop(&mut self) {
        drop(self.body.take());
    }
}

#[repr(C)]
struct ArcInvoice<T> {
    strong: usize,
    weak: usize,
    value: T,
}
pub(super) fn inventory_bytes() -> usize {
    use std::mem::size_of;
    let bodies = size_of::<Group>()
        + size_of::<ArcInvoice<Group>>()
        + size_of::<super::super::helper::HelperLedger>()
        + size_of::<ArcInvoice<super::super::helper::HelperLedger>>()
        + size_of::<super::super::source::SourceLedger>()
        + size_of::<ArcInvoice<super::super::source::SourceLedger>>()
        + size_of::<GroupFunding>()
        + size_of::<ArcInvoice<GroupFunding>>();
    // Full sparse BTree leaf/internal allowance, not an amortized occupied slot.
    let tree = 32 + 11 * size_of::<([u8; 32], GroupOwner)>() + 12 * size_of::<usize>();
    // Include original/final construction and the actual collection owner copy.
    let owners = 3 * size_of::<GroupOwner>()
        + 2 * size_of::<HelperOwner>()
        + 2 * size_of::<SourceOwner>()
        + 2 * size_of::<FundingOwner>();
    // The RAII map transfer must exist before arbitrary revocation wakers. Keep
    // its prepare/move overlap in the invoice, including the unwind path.
    let retirement = 2 * size_of::<GroupRetirement>();
    bodies + tree + owners + retirement + Resources::group_index_invoice_bytes() + 64
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(in crate::voice_v2) use tests::before_return as before_return_for_test;
