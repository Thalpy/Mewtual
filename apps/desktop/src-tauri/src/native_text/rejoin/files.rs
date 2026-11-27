//! Each blocking rejoin phase borrows the affine retired vault. Return the
//! actual owner before acknowledging its file work or starting network I/O.
use super::super::*;
use catcoms_app::store::{
    NativeTextRemovedJoinedVault, NativeTextUiObservation, NativeUiInvocationMetadata,
};

#[derive(Default)]
pub(in crate::native_text) struct Lane {
    active: StdMutex<Option<Arc<CallLife>>>,
}
struct CallLife {
    generation: u64,
    original: Arc<NativeTextVaultAuthenticationCheck>,
    restored: AtomicBool,
    returned: StdMutex<Option<Box<NativeTextRemovedJoinedVault>>>,
    changed: tokio::sync::Notify,
    _credit: NativeUiInvocationMetadata,
}
struct Loan {
    original: Option<Box<NativeTextRemovedJoinedVault>>,
    life: Arc<CallLife>,
}
impl Drop for Loan {
    fn drop(&mut self) {
        if let Some(original) = self.original.take() {
            // This original shelf survives a lost or unpolled result. Only
            // the affine borrower publishes here. No native mutex is held.
            let mut returned = self.life.returned.lock().unwrap_or_else(|e| e.into_inner());
            *returned = Some(original);
        } else {
            // The generation fence overtook admission before Store lent
            // custody. There is no physical return to wait for.
            self.life.restored.store(true, Ordering::Release);
        }
        self.life.changed.notify_waiters();
    }
}

impl Lane {
    fn enter(
        &self,
        candidate: &mut Option<Arc<CallLife>>,
        previous: &mut Option<Arc<CallLife>>,
    ) -> Result<Loan, &'static str> {
        let mut active = self.active.try_lock().map_err(|_| super::BUSY)?;
        if active
            .as_ref()
            .is_some_and(|call| !call.restored.load(Ordering::Acquire))
        {
            return Err(super::BUSY);
        }
        let original = candidate.take().expect("prepaid file work");
        *previous = active.replace(original.clone());
        Ok(Loan {
            original: None,
            life: original,
        })
    }

    pub(in crate::native_text) fn has_loan(&self) -> Result<bool, &'static str> {
        let active = self.active.lock().map_err(|_| super::FAILED)?;
        Ok(active
            .as_ref()
            .is_some_and(|call| !call.restored.load(Ordering::Acquire)))
    }

    /// The caller published a generation fence before joining this wait.
    /// When an older file end leaves, the same retired owner is in Store.
    pub(in crate::native_text) async fn wait_before(
        &self,
        state: &AppState,
        generation: Option<u64>,
    ) -> Result<(), String> {
        let waiting = {
            let active = self
                .active
                .lock()
                .map_err(|_| "Rejoin storage is unavailable.")?;
            active
                .as_ref()
                .filter(|call| generation.is_none_or(|cut| call.generation < cut))
                .cloned()
        };
        if let Some(original) = waiting {
            restore(state, &original).await?;
        }
        Ok(())
    }
}

async fn restore(state: &AppState, life: &CallLife) -> Result<(), String> {
    loop {
        let ready = life.changed.notified();
        tokio::pin!(ready);
        ready.as_mut().enable();
        {
            let mut store = state.store.lock().await;
            if life.restored.load(Ordering::Acquire) {
                return Ok(());
            }
            let mut returned = life.returned.lock().map_err(|_| super::FAILED)?;
            if let Some(owner) = returned.as_ref() {
                if store.is_some() || !owner.matches_native_original_mount_cached(&life.original) {
                    return Err(listing::STALE.into());
                }
                *store = returned.take().map(MountedVault::TextV2Retired);
                life.restored.store(true, Ordering::Release);
            }
        }
        if life.restored.load(Ordering::Acquire) {
            life.changed.notify_waiters();
            return Ok(());
        }
        ready.await;
    }
}

pub(super) async fn run<T: Send + 'static>(
    state: &AppState,
    call: &super::Invocation,
    perform: impl FnOnce(&mut NativeTextRemovedJoinedVault, NativeTextUiObservation) -> Result<T, String>
        + Send
        + 'static,
) -> Result<T, String> {
    let ui_credit = state
        .native_text
        .ui_budget
        .prepare()
        .map_err(|_| super::BUSY)?;
    let file_credit = state
        .native_text
        .ui_budget
        .prepare()
        .map_err(|_| super::BUSY)?;
    let mut candidate = Some(Arc::new(CallLife {
        generation: call.generation,
        original: call.target.clone(),
        restored: AtomicBool::new(false),
        returned: StdMutex::new(None),
        changed: tokio::sync::Notify::new(),
        _credit: file_credit,
    }));
    let preparation = {
        let store = state.store.lock().await;
        call.original(&store)?
            .prepare_rejoin_ui(call.generation, ui_credit)
    };
    let mut ui = preparation.finish().map_err(|_| listing::STALE)?;
    // The previously installed observation and a refused work capsule both
    // retire outside the native mutexes.
    let mut previous = None;
    let mut loan = None;
    let observation;
    {
        let _commit = state.ui_session_commit.lock().await;
        let mut store = state.store.lock().await;
        if !call.is_current(state) {
            return Err(listing::STALE.into());
        }
        observation = call
            .original(&store)?
            .admit_rejoin_ui(&mut ui)
            .map_err(|_| listing::STALE)?;
        let work = loan.insert(
            state
                .native_text
                .rejoin_files
                .enter(&mut candidate, &mut previous)?,
        );
        // Publish the named loan before this last recheck. A lock that saw
        // no loan has already invalidated the old generation before we lend.
        if !call.is_current(state) {
            return Err(listing::STALE.into());
        }
        let Some(MountedVault::TextV2Retired(vault)) = store.take() else {
            unreachable!("original retired owner proved under this guard")
        };
        work.original = Some(vault);
    }
    drop(previous);
    let mut loan = loan.expect("one admitted file loan");
    let ui = observation;
    let life = loan.life.clone();
    #[cfg(test)]
    {
        let edge = state.native_text.rejoin_file_edge.lock().unwrap().clone();
        if let Some(edge) = edge {
            edge.wait_once().await;
        }
    }
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let result = perform(loan.original.as_mut().expect("admitted retired vault"), ui);
        drop(loan); // return the original on success, or via the same Drop on unwind
        result
    })
    .await;
    restore(state, &life).await?;
    outcome.map_err(|_| super::FAILED)?
}
