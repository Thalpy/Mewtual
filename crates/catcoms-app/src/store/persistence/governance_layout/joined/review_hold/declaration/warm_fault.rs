//! One original writer checkpoint per focused physical fixture.
use super::*;
use std::cell::RefCell;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(in crate::store) enum WarmReviewCut {
    Preflight,
    Stage,
    Selected,
}

type Hook = (
    ReviewDeclarationKind,
    WarmReviewCut,
    Box<dyn FnOnce() -> Result<(), AppError>>,
);

thread_local! {
    static NEXT: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

pub(in crate::store) fn with_warm_review_hook<T>(
    kind: ReviewDeclarationKind,
    at: WarmReviewCut,
    hook: impl FnOnce() -> Result<(), AppError> + 'static,
    run: impl FnOnce() -> T,
) -> (T, bool) {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            NEXT.with(|slot| slot.take());
        }
    }
    NEXT.with(|slot| assert!(slot.replace(Some((kind, at, Box::new(hook)))).is_none()));
    let _clear = Clear;
    let result = run();
    (result, NEXT.with(|slot| slot.borrow().is_none()))
}

pub(super) fn cut(kind: ReviewDeclarationKind, at: WarmReviewCut) -> Result<(), AppError> {
    let hook = NEXT.with(|slot| {
        if slot
            .borrow()
            .as_ref()
            .is_some_and(|(role, cut, _)| *role == kind && *cut == at)
        {
            slot.take()
        } else {
            None
        }
    });
    if let Some((_, _, hook)) = hook {
        hook()?;
    }
    Ok(())
}
