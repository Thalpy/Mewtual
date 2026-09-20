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
    pub(super) fn before_write(
        &self,
        target: StudioTarget,
        step: RotationWrite,
        path: &Path,
        bytes: &[u8],
    ) -> Result<(), AppError> {
        let matches = matches!(
            (self.boundary, step),
            (StudioRotationBoundary::Recovery, RotationWrite::Recovery)
                | (StudioRotationBoundary::Successor, RotationWrite::Successor)
        );
        if target == self.target && matches && !self.hit.swap(true, Ordering::SeqCst) {
            if self.after_write {
                write_for_test(path, bytes)?;
            }
            return Err(AppError::Io("injected Studio rotation interruption".into()));
        }
        Ok(())
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
