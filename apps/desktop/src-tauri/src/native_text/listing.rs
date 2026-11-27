//! Warm session DATA from actual installed rows, shared by unlock and resume.
//! No Root loan/Source/SDK work is captured. Exact original authentication and
//! full row-set checks still gate publication through the actual resolver.
use super::*;
use catcoms_app::store::NativeUiInvocationMetadata;
use serde::Serialize;

const BUSY: &str = "Chat is busy. Try again.";
pub(super) const STALE: &str = "The chat session changed. Unlock again.";
const CREDITS: usize = 8;
const BODY_CAP: usize = 16 * 1024;

fn authorization_is_current(
    state: &AppState,
    generation: u64,
    target: &NativeTextVaultAuthenticationCheck,
) -> bool {
    if target.is_retired_mount_data() {
        retired_ui_current(state, generation)
    } else {
        !ui_session_closed(state)
            && state.ui_session_generation.load(Ordering::SeqCst) == generation
    }
}

pub(super) struct Policy {
    slots: Arc<Semaphore>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(2)),
        }
    }
}
pub(super) struct Payment {
    _permit: OwnedSemaphorePermit,
    _credits: Vec<NativeUiInvocationMetadata>,
}
impl Policy {
    /// Independent of the global authoring permit: unlock already owns that
    /// permit. Both unlock and resume count toward these SAME two output slots.
    pub(super) fn prepare(
        &self,
        budget: &NativeUiInvocationBudget,
    ) -> Result<Arc<Payment>, String> {
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| BUSY)?;
        let first = budget.prepare().map_err(|_| BUSY)?;
        let mut credits = Vec::with_capacity(CREDITS);
        credits.push(first); // invoice precedes even this fixed vector allocation
        for _ in 1..CREDITS {
            credits.push(budget.prepare().map_err(|_| BUSY)?);
        }
        Ok(Arc::new(Payment {
            _permit: permit,
            _credits: credits,
        }))
    }
}
struct Names {
    name: String,
    display: String,
}
impl Names {
    fn prepared() -> Self {
        Self {
            name: String::with_capacity(256),
            display: String::with_capacity(256),
        }
    }
}
struct Row {
    server: u64,
    selection: server_table::NativeSelection,
    instance: u64,
    scope: RequestCancellation,
    context: [u8; 32],
    channel: u128,
    is_dm: bool,
    names: Names,
}
/// Entire owning snapshot remains OUTSIDE final guards, even if the real IPC
/// callback panics. Move only its serialized DATA body into the resolver.
pub(super) struct Output {
    target: Arc<NativeTextVaultAuthenticationCheck>,
    generation: u64,
    account: RequestCancellation,
    rows: Vec<Row>,
    pub(super) body: Option<String>,
    #[cfg(test)]
    _notice: Option<DropNotice>, // actual proof/plaintext fields have dropped
    _payment: Arc<Payment>, // LAST after actual proofs/plaintext on every path
}
impl Output {
    pub(super) fn current(
        &self,
        state: &AppState,
        store: &Option<MountedVault>,
        table: &server_table::ServerTable,
    ) -> bool {
        if !authorization_is_current(state, self.generation, &self.target)
            || self.account.is_cancelled()
            || state.native_text.sources.is_closing_cached()
            || !store
                .as_ref()
                .is_some_and(|vault| vault.matches_text_v2_authentication(&self.target))
            || table.len() != self.rows.len()
            || (self.target.is_retired_mount_data() && !table.is_empty())
        {
            return false;
        }
        // Cardinality matters even for the EMPTY snapshot. Every row must be
        // the original native association; an inserted/replaced Legacy row can
        // never satisfy these checks or be omitted as if it did not exist.
        self.rows.iter().all(|proof| {
            table.native(&proof.server).is_some_and(|row| {
                let data = row.data();
                row.matches(&proof.selection, proof.instance, &self.target)
                    && !proof.scope.is_cancelled()
                    && data.request == proof.context
                    && data.channel == proof.channel
                    && data.is_dm == proof.is_dm
                    && data.name == proof.names.name
                    && data.display_name == proof.names.display
            })
        })
    }
}
pub(super) async fn snapshot(
    state: &AppState,
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    payment: Arc<Payment>,
) -> Result<Output, String> {
    let retired = target.is_retired_mount_data();
    let account = if retired {
        keep_session::current_retired(state, generation)
    } else {
        keep_session::current(state, generation)
    }
    .ok_or(STALE)?;
    let mut names = [Some(Names::prepared()), Some(Names::prepared())];
    let mut output = Output {
        target,
        generation,
        account,
        rows: Vec::with_capacity(server_table::NATIVE_ROWS),
        body: None,
        #[cfg(test)]
        _notice: None,
        _payment: payment,
    };
    let captured = {
        let _commit = if retired {
            state.ui_session_commit.lock().await
        } else {
            require_ui_session_generation(state, generation).await?
        };
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        (|| {
            if !authorization_is_current(state, generation, &output.target)
                || state.native_text.sources.is_closing_cached()
                || output.account.is_cancelled()
                || !store
                    .as_ref()
                    .is_some_and(|vault| vault.matches_text_v2_authentication(&output.target))
                || table.len() > server_table::NATIVE_ROWS
                || table.native_iter().count() != table.len()
                || (output.target.is_retired_mount_data() && !table.is_empty())
            {
                return Err(STALE);
            }
            for (index, (id, row)) in table.native_iter().enumerate() {
                let data = row.data();
                if data.name.len() > 256
                    || data.display_name.len() > 256
                    || !data.target.same_original(&output.target)
                {
                    return Err(STALE);
                }
                let scope = row.scope(output.account.clone()).ok_or(STALE)?;
                let mut names = names[index].take().unwrap();
                // Both buffers and the two map positions were paid/preallocated
                // BEFORE these guards. Their bounded copies cannot grow them.
                names.name.push_str(&data.name);
                names.display.push_str(&data.display_name);
                output.rows.push(Row {
                    server: *id,
                    selection: data.selection.clone(),
                    instance: data.instance,
                    scope,
                    context: data.request,
                    channel: data.channel,
                    is_dm: data.is_dm,
                    names,
                });
            }
            if !output.current(state, &store, &table) {
                return Err(STALE);
            }
            Ok(())
        })()
    };
    captured.map_err(str::to_owned)?;
    output.rows.sort_unstable_by_key(|row| row.server);
    let rows: Vec<_> = output
        .rows
        .iter()
        .map(|row| Record {
            backend: "text-v2",
            server: row.server,
            context: hex::encode(row.context),
            channel: row.channel.to_string(),
            name: &row.names.name,
            display_name: &row.names.display,
            is_dm: row.is_dm,
        })
        .collect();
    output.body = Some(history::encode(
        &Envelope {
            backend: "text-v2",
            servers: rows,
            recovery: output.target.is_retired_mount_data().then_some("removed"),
        },
        BODY_CAP,
    )?);
    #[cfg(test)]
    {
        output._notice = state
            .native_text
            .listing_drop_watch
            .lock()
            .unwrap()
            .clone()
            .map(|watch| DropNotice {
                watch,
                payment: Arc::downgrade(&output._payment),
            });
    }
    Ok(output)
}
#[cfg(test)]
pub(super) struct DropWatch {
    pub(super) commit: Arc<Mutex<()>>,
    pub(super) store: Arc<Mutex<Option<MountedVault>>>,
    pub(super) table: Arc<Mutex<server_table::ServerTable>>,
    pub(super) observed: std::sync::atomic::AtomicUsize,
    pub(super) outside: AtomicBool,
}
#[cfg(test)]
struct DropNotice {
    watch: Arc<DropWatch>,
    payment: std::sync::Weak<Payment>,
}
#[cfg(test)]
impl Drop for DropNotice {
    fn drop(&mut self) {
        if self.watch.commit.try_lock().is_err()
            || self.watch.store.try_lock().is_err()
            || self.watch.table.try_lock().is_err()
            || self.payment.upgrade().is_none()
        {
            self.watch.outside.store(false, Ordering::SeqCst);
        }
        self.watch.observed.fetch_add(1, Ordering::SeqCst);
    }
}
#[derive(Serialize)]
struct Record<'a> {
    backend: &'static str,
    server: u64,
    context: String,
    channel: String,
    name: &'a str,
    display_name: &'a str,
    is_dm: bool,
}
#[derive(Serialize)]
struct Envelope<'a> {
    backend: &'static str,
    servers: Vec<Record<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<&'static str>,
}

fn parse(payload: &InvokeBody) -> Result<(), &'static str> {
    let InvokeBody::Json(value) = payload else {
        return Err("Invalid resume request.");
    };
    let object = value.as_object().ok_or("Invalid resume request.")?;
    if object.len() > 1
        || object.keys().any(|key| key != "trace")
        || object
            .get("trace")
            .is_some_and(|value| !value.is_null() && value.as_str().is_none_or(|s| s.len() > 128))
    {
        return Err("Invalid resume request.");
    }
    Ok(())
}
struct Resume {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    payment: Arc<Payment>,
}
pub(super) fn route<R: tauri::Runtime>(invoke: Invoke<R>, spawn: &impl Fn(Task)) -> bool {
    if invoke.acl.is_none()
        || invoke.message.webview_ref().label() != "main"
        || invoke.message.webview_ref().window().label() != "main"
    {
        invoke
            .resolver
            .reject("Chat is not available from this window.");
        return true;
    }
    let app = invoke.message.webview_ref().app_handle().clone();
    let captured = (|| {
        let state = app.state::<AppState>();
        let generation = state.ui_session_generation.load(Ordering::SeqCst);
        // Locked resume stays null, independently of whether the vault has rows.
        if ui_session_closed(&state) && !retired_ui_current(&state, generation) {
            return Ok(None);
        }
        if state.native_text.sources.is_closing_cached() {
            return Err(STALE.to_owned());
        }
        let target = state.native_text.ui_state.original_target().ok_or(STALE)?;
        let payment = state
            .native_text
            .listing
            .prepare(&state.native_text.ui_budget)?;
        parse(invoke.message.payload()).map_err(str::to_owned)?;
        Ok(Some(Resume {
            generation,
            target,
            payment,
        }))
    })();
    let captured = match captured {
        Ok(Some(value)) => value,
        Ok(None) => {
            invoke.resolver.resolve(Option::<()>::None);
            return true;
        }
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        let state = app.state::<AppState>();
        let mut result = snapshot(
            &state,
            captured.generation,
            captured.target.clone(),
            captured.payment.clone(),
        )
        .await;
        #[cfg(test)]
        {
            let edge = state.native_text.listing_reply_edge.lock().unwrap().clone();
            if let Some(edge) = edge {
                edge.wait_once().await;
            }
        }
        let _commit = state.ui_session_commit.lock().await;
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        let current = authorization_is_current(&state, captured.generation, &captured.target)
            && !state.native_text.sources.is_closing_cached()
            && store
                .as_ref()
                .is_some_and(|vault| vault.matches_text_v2_authentication(&captured.target));
        match &mut result {
            Ok(output) if current && output.current(&state, &store, &table) => resolver.resolve(
                tauri::ipc::InvokeResponseBody::Json(output.body.take().unwrap()),
            ),
            Ok(_) => resolver.reject(STALE),
            Err(error) if current => resolver.reject(&*error),
            Err(_) => resolver.reject(STALE),
        }
    }));
    true
}
// The Root credit quote is process DATA, not signing authority. Keep this
// invoice in sync with NativeUiInvocationBudget's public metadata quote.
const _: () = assert!(
    BODY_CAP + 2 * 2 * 256 + std::mem::size_of::<Output>() + 4096
        <= CREDITS * NativeUiInvocationBudget::metadata_bytes()
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_listing_two_paid_outputs_fit_existing_control_generation_and_authentication_overlap()
    {
        let budget = NativeUiInvocationBudget::new(
            catcoms_app::store::CriticalCatalogReadPool::new(
                NativeUiInvocationBudget::required_capacity(),
            )
            .unwrap(),
        )
        .unwrap();
        // Real debits model the documented permanent7, generation32 and auth2
        // overlap. They are payment only, not native authentication fixtures.
        let controls: Vec<_> = (0..41).map(|_| budget.prepare().unwrap()).collect();
        let policy = Policy::default();
        let first = policy.prepare(&budget).unwrap();
        let second = policy.prepare(&budget).unwrap();
        assert!(policy.prepare(&budget).is_err());
        drop(first);
        let replacement = policy.prepare(&budget).unwrap();
        drop((second, replacement, controls));
    }
    #[test]
    fn native_listing_two_maximum_escaped_names_fit_fixed_native_body_without_plaintext_clone() {
        let name = "\0".repeat(256);
        let records = (1..=2)
            .map(|server| Record {
                backend: "text-v2",
                server,
                context: "a".repeat(64),
                channel: u128::MAX.to_string(),
                name: &name,
                display_name: &name,
                is_dm: false,
            })
            .collect();
        let output = history::encode(
            &Envelope {
                backend: "text-v2",
                servers: records,
                recovery: None,
            },
            BODY_CAP,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["servers"].as_array().unwrap().len(), 2);
        assert_eq!(value["servers"][0]["name"], name);
    }
}
