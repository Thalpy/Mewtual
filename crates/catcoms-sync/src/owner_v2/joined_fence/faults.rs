//! Deterministic one-shot faults at the actual commit observer. Test-only, bound to one
//! original node and exact commit digest; no production layout or global timing changes.
use crate::*;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Cut {
    RefuseCommit,
    PanicAfterSettlement,
}
struct Fault {
    instance: RegistrySyncInstance,
    digest: [u8; 32],
    cut: Cut,
}
thread_local! {
    static NEXT: RefCell<Option<Fault>> = const { RefCell::new(None) };
}

/// Scoped and not Send: cleanup must occur on the thread where the fault was armed.
pub(super) struct Armed(PhantomData<Rc<()>>);
impl Drop for Armed {
    fn drop(&mut self) {
        NEXT.with(|next| *next.borrow_mut() = None);
    }
}
pub(super) fn arm<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
    commit: &[u8],
    cut: Cut,
) -> Armed {
    NEXT.with(|next| {
        let mut next = next.borrow_mut();
        assert!(next.is_none(), "nested commit fault");
        *next = Some(Fault {
            instance: node.registry_instance(),
            digest: *blake3::hash(commit).as_bytes(),
            cut,
        });
    });
    Armed(PhantomData)
}
fn take<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
    digest: &[u8; 32],
    cut: Cut,
) -> bool {
    NEXT.with(|next| {
        let mut next = next.borrow_mut();
        let hit = next.as_ref().is_some_and(|fault| {
            node.matches_registry_instance(&fault.instance)
                && &fault.digest == digest
                && fault.cut == cut
        });
        if hit {
            *next = None;
        }
        hit
    })
}
pub(crate) fn refuse_commit<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
    digest: &[u8; 32],
) -> bool {
    take(node, digest, Cut::RefuseCommit)
}
pub(crate) fn after_settlement<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
    digest: Option<[u8; 32]>,
) {
    if digest.is_some_and(|digest| take(node, &digest, Cut::PanicAfterSettlement)) {
        panic!("test-only observer hook panic after settlement");
    }
}
