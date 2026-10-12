//! Per-mount, one-shot I/O failure for actor restart regressions. Never compiled in production.
//! The actor must reach the real rotation writer; this creates no receipt or installed source.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StudioRotationBoundary {
    Recovery,
    Successor,
}

#[derive(Clone)]
pub(crate) struct StudioRotationInterruption {
    target: StudioTarget,
    boundary: StudioRotationBoundary,
    after_write: bool,
    hit: Arc<AtomicBool>,
}

impl StudioRotationInterruption {
    /// Whether this interruption is aimed at the write about to happen. The one-shot flag is
    /// deliberately not consulted here: it is claimed by whichever side actually fires.
    fn aimed_at(&self, target: StudioTarget, step: WriteTag) -> bool {
        target == self.target
            && matches!(
                (self.boundary, step),
                (StudioRotationBoundary::Recovery, WriteTag::Recovery)
                    | (StudioRotationBoundary::Successor, WriteTag::Successor)
            )
    }

    /// Fail before the replacement, so no record is left behind.
    pub(super) fn before(&self, target: StudioTarget, step: WriteTag) -> Intercept {
        if !self.after_write && self.claim(target, step) {
            return Intercept::Fail(AppError::Io("injected Studio rotation interruption".into()));
        }
        Intercept::Continue
    }

    /// Fail once the bytes are in place, so a restart finds a durable but unaccounted record.
    ///
    /// This half used to be expressed by the injected writer performing the write itself and
    /// then returning an error. It no longer writes: the capability does, and this only decides
    /// afterwards, which is the same observable without a second path to disk.
    pub(super) fn after(&self, target: StudioTarget, step: WriteTag) -> AfterIntercept {
        if self.after_write && self.claim(target, step) {
            return AfterIntercept::Fail(AppError::Io(
                "injected Studio rotation interruption".into(),
            ));
        }
        AfterIntercept::Continue
    }

    /// Aimed here, and the one shot is still unspent.
    fn claim(&self, target: StudioTarget, step: WriteTag) -> bool {
        self.aimed_at(target, step) && !self.hit.swap(true, Ordering::SeqCst)
    }
}

impl ServerStore {
    pub(crate) fn interrupt_studio_rotation_for_test(
        &mut self,
        target: StudioTarget,
        boundary: StudioRotationBoundary,
        after_write: bool,
    ) -> Arc<AtomicBool> {
        let hit = Arc::new(AtomicBool::new(false));
        self.studio_rotation_interruption = Some(StudioRotationInterruption {
            target,
            boundary,
            after_write,
            hit: hit.clone(),
        });
        hit
    }
}
