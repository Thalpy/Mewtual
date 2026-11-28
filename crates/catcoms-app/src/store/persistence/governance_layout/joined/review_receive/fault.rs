use super::*;
use std::cell::{Cell, RefCell};

type Hook = (ReceiverFault, Box<dyn FnOnce()>);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum ReceiverFault {
    CatalogClaim,
    CatalogStage,
    CatalogSelected,
    AdmittedHeader,
    ManifestSelected,
}

thread_local! {
    static NEXT: Cell<Option<ReceiverFault>> = const { Cell::new(None) };
    static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

pub(in crate::store) fn with_receiver_fault<T>(
    point: ReceiverFault,
    run: impl FnOnce() -> T,
) -> (T, bool) {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            NEXT.set(None);
        }
    }
    assert!(NEXT.replace(Some(point)).is_none());
    let _clear = Clear;
    let result = run();
    (result, NEXT.get().is_none())
}

pub(in crate::store) fn with_receiver_hook<T>(
    point: ReceiverFault,
    hook: impl FnOnce() + 'static,
    run: impl FnOnce() -> T,
) -> (T, bool) {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            HOOK.with(|slot| {
                slot.take();
            });
        }
    }
    HOOK.with(|slot| assert!(slot.replace(Some((point, Box::new(hook)))).is_none()));
    let _clear = Clear;
    let result = run();
    (result, HOOK.with(|slot| slot.borrow().is_none()))
}

pub(super) fn cut(point: ReceiverFault) -> Result<(), AppError> {
    let hook = HOOK.with(|slot| {
        if slot
            .borrow()
            .as_ref()
            .is_some_and(|(expected, _)| *expected == point)
        {
            slot.take()
        } else {
            None
        }
    });
    if let Some((_, hook)) = hook {
        hook();
    }
    if NEXT.get() == Some(point) {
        NEXT.set(None);
        return Err(AppError::Io("injected recipient review write cut".into()));
    }
    Ok(())
}
