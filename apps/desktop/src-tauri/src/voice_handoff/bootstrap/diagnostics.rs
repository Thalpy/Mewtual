//! Primitive try-lock snapshots only. Never reuse ordinary routing/checks here:
//! a blocked native producer is precisely when a timeout report must not wait.
use super::*;
use catcoms_sync::voice_v2::presence::PairSelector;

impl Registry {
    pub(in crate::voice_handoff) fn fixture_native_diagnostic(
        &self,
        selected: &SelectedHandoffOwner,
        expected: PairSelector,
    ) -> String {
        let stopping = self.stopping.load(Ordering::Acquire);
        let observed: Result<_, &'static str> = (|| {
            let guard = self.index.try_lock().map_err(|_| "registry contended")?;
            let index = guard.as_ref().ok_or("registry absent")?;
            let row = index
                .rows
                .iter()
                .flatten()
                .find(|row| std::ptr::eq(row.selected.as_ref(), selected))
                .ok_or("selected channel absent")?;
            let state = row.state.try_lock().map_err(|_| "channel row contended")?;
            let flags = (
                state.pending,
                state.effects_closed,
                state.closed,
                state.browser_setup.is_some(),
            );
            let native = state
                .session
                .as_ref()
                .map(|session| session.fixture_native_snapshot());
            Ok((flags, native))
        })();
        // Every guard, including error-path guards, is gone before formatting. The native
        // observation is Copy. Resources getters are deliberately absent: they may block.
        match observed {
            Err(reason) => format!("{reason}; stopping={stopping}"),
            Ok((flags, native)) => {
                let detail = native.map(|observation| observation.describe(expected));
                format!("stopping={stopping}; pending/effects_closed/closed/browser_setup={flags:?}; work={detail:?}; pool counts not sampled (blocking getter)")
            }
        }
    }
}
