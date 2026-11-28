use super::*;
use std::cell::Cell;

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

pub(super) fn cut(point: ReceiverFault) -> Result<(), AppError> {
    if NEXT.get() == Some(point) {
        NEXT.set(None);
        return Err(AppError::Io("injected recipient review write cut".into()));
    }
    Ok(())
}
