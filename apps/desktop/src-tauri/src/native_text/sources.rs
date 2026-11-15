//! Native canonical source owner. Root workers receive only finite captures,
//! never Arc<Sources> or an Arc to this record. All actual owners are removable.
use super::*;
use catcoms_app::store::{
    ArchiveFounderPolicy, CriticalCatalogReadPool, FreshV2TextFounderPolicy, GenesisSeedBudget,
    NativeTextSourcePolicy, NativeTextSourceRetirementPreparation,
    NativeTextSourceRetirementStatus, NativeTextSources, NativeTextVault,
    NativeTextWorkRetirementPreparation, NativeUiInvocationMetadata, V2TextHostPolicy,
};
use catcoms_sync::owner_v2::FounderSnapshotLimits;

type Sources = NativeTextSources<catcoms_net::MeshService, OsCryptoRng>;
type Retirement = NativeTextSourceRetirementPreparation<catcoms_net::MeshService, OsCryptoRng>;
type RemovedPair = (Option<Sources>, Option<MountedVault>);

/// Only this private constructor pairs a facade with the Sources issued by
/// that SAME value. Neither field can come from renderer DATA or a raw Store.
pub(super) struct ReadyBundle {
    sources: Sources,
    vault: NativeTextVault,
}
impl ReadyBundle {
    pub(super) fn new(
        vault: NativeTextVault,
        policy: &NativeTextSourcePolicy,
    ) -> Result<Self, String> {
        let budget = policy
            .prepare()
            .map_err(|_| "Chat setup is busy. Try again.")?;
        let sources = vault
            .capture_native_sources(budget)
            .finish()
            .map_err(|_| "Chat setup did not complete. Its files have been kept.")?;
        Ok(Self { vault, sources })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Booting,
    Live,
    Closing,
    Disposing,
    Disposed,
}
struct Record {
    phase: Phase,
    sources: Option<Sources>,
    ever_installed: bool,
}
pub(super) struct Controller {
    record: StdMutex<Record>,
    policy: NativeTextSourcePolicy,
    // LAST: owns the independent payment for this native record and wrappers.
    pub(super) boot: Arc<boot_lifetime::Control>,
}
impl Controller {
    /// Same guard-safe finite capture as founding, selected only by the actual
    /// native row. Caller checks its incarnation while holding the table guard.
    pub(super) fn capture_member(
        &self,
        store: &Option<MountedVault>,
        ui: &mut Option<catcoms_app::store::NativeTextUiObservation>,
        row: &server_table::NativeSelection,
        budget: &mut Option<
            catcoms_app::store::NativeTextMemberBudget<catcoms_net::MeshService, OsCryptoRng>,
        >,
        output: &mut Option<
            catcoms_app::store::NativeTextMemberPreparation<catcoms_net::MeshService, OsCryptoRng>,
        >,
    ) -> Result<(), &'static str> {
        let record = self
            .record
            .try_lock()
            .map_err(|_| "Chat is busy. Retry with the same message.")?;
        if record.phase != Phase::Live
            || self.boot.is_closed_cached()
            || output.is_some()
            || budget.is_none()
        {
            return Err("The chat session changed. Retry with the same message.");
        }
        record.require_pair(store)?;
        let vault = store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .ok_or("The original native vault is unavailable.")?;
        let sources = record
            .sources
            .as_ref()
            .ok_or("The original chat owner is unavailable.")?;
        let command = vault
            .bind_native_source_command(ui)
            .map_err(|_| "The chat session changed. Retry with the same message.")?;
        *output = Some(sources.capture_member(row, command, budget.take().unwrap()));
        Ok(())
    }
    /// Capture one finite founder job from the SAME original mounted pair.
    /// All paid inputs and owning output live in caller's outer slots; neither
    /// refusal nor controller-guard destruction disposes those resources here.
    pub(super) fn capture_creation(
        &self,
        store: &Option<MountedVault>,
        ui: &mut Option<catcoms_app::store::NativeTextUiObservation>,
        request: &mut Option<catcoms_app::store::NativeTextCreationRequest>,
        inputs: &mut Option<catcoms_app::store::NativeTextCreationInputs<OsCryptoRng>>,
        budget: &mut Option<
            catcoms_app::store::NativeTextCreationBudget<catcoms_net::MeshService, OsCryptoRng>,
        >,
        output: &mut Option<
            catcoms_app::store::NativeTextCreationPreparation<
                catcoms_net::MeshService,
                OsCryptoRng,
            >,
        >,
    ) -> Result<(), &'static str> {
        let record = self
            .record
            .try_lock()
            .map_err(|_| "Chat setup is busy. Try again.")?;
        if record.phase != Phase::Live
            || self.boot.is_closed_cached()
            || output.is_some()
            || request.is_none()
            || inputs.is_none()
            || budget.is_none()
        {
            return Err("Chat setup was interrupted. Retry with the same request.");
        }
        record.require_pair(store)?;
        let vault = store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .ok_or("The original native vault is unavailable.")?;
        let sources = record
            .sources
            .as_ref()
            .ok_or("The original chat owner is unavailable.")?;
        let command = vault
            .bind_native_source_command(ui)
            .map_err(|_| "Chat setup was interrupted. Retry with the same request.")?;
        *output = Some(sources.capture_creation(
            command,
            request.take().unwrap(),
            inputs.take().unwrap(),
            budget.take().unwrap(),
        ));
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn retains_original(&self, vault: &MountedVault) -> bool {
        let record = self.record.lock().unwrap();
        record
            .sources
            .as_ref()
            .zip(vault.as_text_v2())
            .is_some_and(|(sources, vault)| sources.is_from_original_vault_cached(vault))
    }
    pub(super) fn new(metadata: catcoms_app::store::NativeUiInvocationMetadata) -> Self {
        let founder = FreshV2TextFounderPolicy::new(
            ArchiveFounderPolicy::new(
                FounderSnapshotLimits::new(
                    8 * 1024 * 1024 + 64 * 1024,
                    8 * 1024 * 1024,
                    8 * 1024 * 1024,
                )
                .expect("fixed founder snapshot bounds"),
                3,
                1024 * 1024 * 1024,
            )
            .expect("fixed founder policy"),
            GenesisSeedBudget::new(
                8 * 1024 * 1024 + 64 * 1024,
                8 * 1024 * 1024,
                8 * 1024 * 1024,
                1024 * 1024,
                16 * 1024 * 1024,
            )
            .expect("fixed genesis policy"),
            V2TextHostPolicy::new(1024 * 1024 * 1024, 1024 * 1024).expect("fixed host policy"),
        )
        .expect("fixed owning founder policy");
        let policy = NativeTextSourcePolicy::new(
            CriticalCatalogReadPool::new(
                NativeTextSourcePolicy::required_capacity(server_table::NATIVE_ROWS)
                    .expect("fixed source capacity"),
            )
            .expect("fixed source pool"),
            server_table::NATIVE_ROWS,
            founder,
        )
        .expect("fixed source policy");
        Self {
            record: StdMutex::new(Record {
                phase: Phase::Booting,
                sources: None,
                ever_installed: false,
            }),
            boot: boot_lifetime::Control::new(metadata),
            policy,
        }
    }
    /// A resource policy clone only, not an owner/caller credential. Prepare
    /// actual Root budgets before borrowing native Store/controller guards.
    pub(super) fn policy(&self) -> NativeTextSourcePolicy {
        self.policy.clone()
    }
    pub(super) fn is_closing_cached(&self) -> bool {
        self.boot.is_closed_cached()
    }

    /// Caller holds Store, then this short controller guard. All checks precede
    /// the infallible SAME-pair move. A losing bundle stays in caller's OUTER
    /// slot and is never destroyed under either guard.
    pub(super) fn install(
        &self,
        store: &mut Option<MountedVault>,
        ready: &mut Option<ReadyBundle>,
        original: &Arc<boot_lifetime::Control>,
    ) -> Result<(), &'static str> {
        let mut record = self
            .record
            .try_lock()
            .map_err(|_| "Chat setup is busy. Try again.")?;
        if !self.boot.same_original(original)
            || self.boot.is_closed_cached()
            || record.phase != Phase::Booting
            || record.sources.is_some()
            || store.is_some()
            || ready.is_none()
        {
            return Err("Vault startup was superseded.");
        }
        let ReadyBundle { vault, sources } = ready.take().expect("validated original ready pair");
        record.sources = Some(sources);
        *store = Some(MountedVault::TextV2(vault));
        record.ever_installed = true;
        record.phase = Phase::Live;
        Ok(())
    }

    /// Only after continuity/may_destroy admits irreversible close. Tentative
    /// close leaves this open so its documented reauthentication still works.
    pub(super) fn begin_close(&self) -> Result<(), &'static str> {
        self.boot.close(); // immediate SC install/file-work fence before writer
        let mut record = self
            .record
            .try_lock()
            .map_err(|_| "Chat is still closing. Try again.")?;
        if record.phase != Phase::Disposed && record.phase != Phase::Disposing {
            record.phase = Phase::Closing;
        }
        Ok(())
    }
    /// Pure owning capture under one short native controller guard. Root
    /// closes actual source counters/visits its book in outside-guard finish.
    pub(super) fn capture_retirement(
        &self,
        store: &Option<MountedVault>,
    ) -> Result<Option<Retirement>, &'static str> {
        let record = self
            .record
            .try_lock()
            .map_err(|_| "Chat is still closing. Try again.")?;
        if record.phase != Phase::Closing {
            return Err("Chat is not ready to finish closing.");
        }
        record.require_pair(store)?;
        match &record.sources {
            Some(original) => Ok(Some(original.capture_retirement())),
            None if !record.ever_installed => Ok(None), // typed never-installed original
            None => Err("The original chat owner is unavailable."),
        }
    }

    /// Recheck the SAME original pair at each close stage. Root's cached
    /// comparison works after closure; an empty replacement proves nothing.
    fn capture_work(
        &self,
        store: &Option<MountedVault>,
        credit: &mut Option<NativeUiInvocationMetadata>,
    ) -> Result<Option<NativeTextWorkRetirementPreparation>, &'static str> {
        let record = self
            .record
            .try_lock()
            .map_err(|_| "Chat is still closing. Try again.")?;
        record.require_pair(store)?;
        if record.phase != Phase::Closing {
            return Err("Chat is not ready to finish closing.");
        }
        Ok(store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .map(|vault| vault.capture_work_retirement(credit.take().expect("paid outside guard"))))
    }
    /// After original Root source/work proofs, caller moves this actual owner
    /// out alongside the SAME Store facade, then drops both outside guards.
    fn take_pair(&self, store: &mut Option<MountedVault>) -> Result<RemovedPair, &'static str> {
        let mut record = self
            .record
            .try_lock()
            .map_err(|_| "Chat is still closing. Try again.")?;
        if record.phase != Phase::Closing || !self.boot.is_drained_cached() {
            return Err("Chat is still closing. Try again.");
        }
        record.require_pair(store)?;
        record.phase = Phase::Disposing;
        Ok((record.sources.take(), store.take()))
    }
    /// Actual outside-guard destructors must have returned before this call.
    pub(super) fn disposed(&self) {
        let mut record = self.record.lock().unwrap_or_else(|e| e.into_inner());
        if record.phase == Phase::Disposing && record.sources.is_none() {
            record.phase = Phase::Disposed;
        }
    }
    fn is_disposed(&self, store: &Option<MountedVault>) -> Result<bool, &'static str> {
        let record = self
            .record
            .try_lock()
            .map_err(|_| "Chat is still closing. Try again.")?;
        if record.phase == Phase::Disposed {
            if record.sources.is_some() || store.is_some() {
                return Err("The original vault changed while closing.");
            }
            return Ok(true);
        }
        Ok(false)
    }
}

impl Record {
    /// Check before any capture or owner move, under Store -> controller.
    /// A foreign or missing facade never clears an earlier owner's obligation.
    fn require_pair(&self, store: &Option<MountedVault>) -> Result<(), &'static str> {
        match (&self.sources, store) {
            (Some(sources), Some(MountedVault::TextV2(vault)))
                if self.ever_installed && sources.is_from_original_vault_cached(vault) =>
            {
                Ok(())
            }
            (None, None) if !self.ever_installed => Ok(()),
            _ => Err("The original vault changed while closing."),
        }
    }
}

/// Irreversible native close, called only after continuity permits destruction.
/// No worker owns AppState, Sources or a facade. Lost finite work is retried
/// from the SAME canonical collection, never from FAILED + idle key counters.
/// Native-only whole-close composition. Called after the real irreversible
/// close prelude. Keep row owners in their canonical table across every
/// incomplete Source/SDK/work close, without holding UI or table guards while
/// draining. This is vault shutdown, never a durable per-conversation Leave.
pub(crate) async fn close_server_rows(
    state: &AppState,
    clock: &dyn catcoms_rt::Clock,
) -> Result<(), String> {
    {
        let mut table = state.servers.lock().await;
        if table.legacy_iter().next().is_some() {
            return Err("Legacy servers still require their shutdown barrier.".into());
        }
        // Publish the global irreversible fence BEFORE per-row closure and
        // before any fallible controller lookup. Otherwise a contended close
        // could leave closed rows behind an apparently reopenable bootstrap.
        state.native_text.sources.boot.close();
        table
            .fence_native_for_shutdown()
            .expect("pure-native table validated under the same guard");
    }
    close(state, clock).await?;
    let mut removed = None;
    {
        let mut table = state.servers.lock().await;
        table
            .extract_native_for_shutdown(&mut removed)
            .map_err(str::to_owned)?;
    }
    // The actual Root owners have now drained, and every native guard has
    // left. Only here may watch cancellation/payment/weak tails be destroyed.
    drop(removed);
    Ok(())
}

pub(crate) async fn close(state: &AppState, clock: &dyn catcoms_rt::Clock) -> Result<(), String> {
    let controller = &state.native_text.sources;
    // All callers of this function have admitted irreversible whole-close.
    // Even the first fallible cached lookup must leave that fence published.
    controller.boot.close();
    {
        let store = state.store.lock().await;
        if controller.is_disposed(&store)? {
            return Ok(());
        }
    }
    controller.begin_close()?;
    // Do NOT join vault_mount or unlock's finalizer: the caller retains the
    // close prelude which that finalizer may await. Only actual file work ends.
    controller.boot.wait().await;
    let bootstrap = {
        let mut slot = state
            .native_text
            .bootstrap
            .lock()
            .map_err(|_| "Vault startup is unavailable.")?;
        slot.take()
    };
    drop(bootstrap); // retained partial plan / uninstalled ready pair, outside guards

    let capture = {
        let store = state.store.lock().await;
        controller.capture_retirement(&store)?
    };
    if let Some(capture) = capture {
        // Root finish takes its original entries writer and closes actual RT
        // issuers. SDK/body destruction also belongs on the blocking executor.
        let mut worker = tauri::async_runtime::spawn_blocking(move || capture.finish())
            .await
            .map_err(|_| "Chat shutdown was interrupted. Try closing again.")?
            .map_err(|_| "Chat is still closing. Try again.")?;
        loop {
            let (next, result) = tauri::async_runtime::spawn_blocking(move || {
                let result = worker.try_retire();
                (worker, result)
            })
            .await
            .map_err(|_| "Chat shutdown was interrupted. Try closing again.")?;
            worker = next;
            match result.map_err(|_| "Chat shutdown could not be verified. Try closing again.")? {
                NativeTextSourceRetirementStatus::Drained => break,
                NativeTextSourceRetirementStatus::Pending => {
                    #[cfg(test)]
                    state.native_text.source_retirement_waiting.notify_one();
                    clock.sleep(Duration::from_millis(20)).await
                }
            }
        }
        // Every actual cell/key pin must leave before final paired removal.
        drop(worker);
    }
    let mut credit = Some(
        state
            .native_text
            .ui_budget
            .prepare()
            .map_err(|_| "Chat is still closing. Try again.")?,
    );
    let capture = {
        let store = state.store.lock().await;
        controller.capture_work(&store, &mut credit)?
    };
    if let Some(capture) = capture {
        let drain = capture
            .finish()
            .map_err(|_| "Chat shutdown could not be verified.")?;
        #[cfg(test)]
        {
            let edge = state.native_text.source_work_edge.lock().unwrap().clone();
            if let Some(edge) = edge {
                edge.wait_once().await;
            }
        }
        #[cfg(test)]
        {
            // Signal ONLY after polling the real original wait to Pending.
            // A scheduling barrier before wait would not prove output custody.
            let wait = drain.wait();
            tokio::pin!(wait);
            let pending = std::future::poll_fn(|cx| {
                std::task::Poll::Ready(wait.as_mut().poll(cx).is_pending())
            })
            .await;
            if pending {
                state.native_text.source_work_waiting.notify_one();
                wait.await;
            }
        }
        #[cfg(not(test))]
        drain.wait().await;
    }
    let (sources, vault) = {
        let mut store = state.store.lock().await;
        controller.take_pair(&mut store)?
    };
    // Physical custody follows entries, SDK and transport. Never drop either
    // whole owner under Store/controller/UI/prelude's internal mutexes.
    drop(sources);
    drop(vault);
    controller.disposed(); // actual destructors have returned
    Ok(())
}

const _: () = assert!(
    std::mem::size_of::<Controller>()
        + 2 * std::mem::size_of::<ReadyBundle>()
        + 2 * std::mem::size_of::<super::BootOutput>()
        + 2 * std::mem::size_of::<boot_lifetime::Control>()
        + 512
        <= 4096
);

#[cfg(test)]
mod close_tests {
    use super::*;

    #[tokio::test]
    async fn native_close_controller_contention_still_publishes_irreversible_fence() {
        let state = Arc::new(AppState::default());
        let controller = &state.native_text.sources;
        let held_state = state.clone();
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            let _guard = held_state.native_text.sources.record.lock().unwrap();
            entered.send(()).unwrap();
            // Sender destruction also releases this real lock on test unwind.
            let _ = held.recv();
        });
        ready.recv().unwrap();
        assert!(close_server_rows(&state, &SystemClock).await.is_err());
        assert!(controller.boot.is_closed_cached());
        assert!(
            controller.boot.enter().is_err(),
            "a refused close must not reopen bootstrap admission"
        );
        assert!(state.servers.lock().await.is_empty());
        drop(release);
        worker.join().unwrap();
        close_server_rows(&state, &SystemClock).await.unwrap();
        close_server_rows(&state, &SystemClock).await.unwrap();
        assert!(controller.is_disposed(&*state.store.lock().await).unwrap());
    }

    #[tokio::test]
    async fn native_close_refuses_legacy_before_any_global_or_row_fence() {
        let state = AppState::default();
        let (entry, task, drain) = leaving::tests::running(13);
        let (account, signal) = watch::channel(false);
        let scope = entry.files.scope(RequestCancellation::new(signal, None));
        state.servers.lock().await.install_legacy_fixture(1, entry);
        assert!(close_server_rows(&state, &SystemClock).await.is_err());
        assert!(!state.native_text.sources.boot.is_closed_cached());
        assert!(!scope.is_cancelled());
        assert_eq!(state.servers.lock().await.legacy(&1).unwrap().instance, 13);
        drop(state);
        drop(account);
        task.abort();
        drain.abort();
    }
}
