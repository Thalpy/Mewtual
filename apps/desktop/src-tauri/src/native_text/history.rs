//! Bounded own receipts and separately guarded live and archived projections.
//! Root owns the reads. Native pays its response copies and rechecks the
//! installed objects before resolving an invocation.
use super::*;
use catcoms_app::store::{
    NativeTextHistoryResponseCredit, NativeTextJoinedMemberProjection,
    NativeTextJoinedRowSelection, NativeTextJoinedSelectionPreparation,
    NativeTextMemberHistoryPage, NativeTextMemberHistoryStatus, NativeTextMemberPreparation,
};
use catcoms_replication::chat::{
    ActionId, Body, GuardedProjection, ProjectionCertainty, SignedAction,
};
use serde::Serialize;
use std::io::Write;
use tauri::ipc::InvokeResponseBody;
use tokio::sync::OwnedMutexGuard;

enum Preparation {
    Founder(NativeTextMemberPreparation<catcoms_net::MeshService, OsCryptoRng>),
    Joined(
        NativeTextJoinedSelectionPreparation<OsCryptoRng>,
        NativeTextJoinedRowSelection,
    ),
}
const STALE: &str = "The chat session changed. Refresh this chat.";
const BUSY: &str = "Chat history is busy. Try again.";
// Eight maximum-size texts, even with sixfold JSON escaping, fit below this
// cap. Another MiB of the invoice covers bounded metadata and semantic-ID
// scratch. IPC, WebView and allocator overhead are not claimed as bounded RSS.
const SERIALIZED_LIMIT: usize = 4 * 1024 * 1024;

struct Payment {
    _permit: OwnedSemaphorePermit,
    _credit: NativeTextHistoryResponseCredit,
}
#[derive(Clone, Copy)]
struct Read {
    server: u64,
    channel: u128,
    context: [u8; 32],
    request: ReadRequest,
}
#[derive(Clone, Copy)]
enum ReadRequest {
    Own(OwnRead),
    Live(ActionId),
    Archived(ArchiveRead),
}
#[derive(Clone, Copy)]
struct OwnRead {
    start: u64,
    end: Option<u64>,
    maximum: usize,
}
#[derive(Clone, Copy)]
enum ArchiveRead {
    Page {
        after: Option<ActionId>,
        limit: usize,
    },
    Message(ActionId),
}
struct Invocation {
    generation: u64,
    target: Arc<NativeTextVaultAuthenticationCheck>,
    selection: server_table::NativeSelection,
    instance: u64,
    scope: RequestCancellation,
    member_work: Option<Arc<server_table::JoinedMemberWork>>,
    read: Read,
    payment: Arc<Payment>, // LAST, including unpolled/cancelled reply futures
}
fn decimal(value: &str) -> Result<u64, &'static str> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "Invalid history cursor.")?;
    if parsed.to_string() != value {
        return Err("Invalid history cursor.");
    }
    Ok(parsed)
}
fn parse(payload: &InvokeBody) -> Result<Read, &'static str> {
    let InvokeBody::Json(value) = payload else {
        return Err("Invalid history request.");
    };
    let object = value.as_object().ok_or("Invalid history request.")?;
    if object.len() > 7
        || object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "server"
                    | "channel"
                    | "expectedContext"
                    | "start"
                    | "endExclusive"
                    | "limit"
                    | "receivedAction"
                    | "archivedAction"
                    | "archiveAfter"
                    | "trace"
            )
        })
    {
        return Err("Invalid history request.");
    }
    let string = |key: &str, max| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| s.len() <= max)
            .ok_or("Invalid history request.")
    };
    let server = object
        .get("server")
        .and_then(serde_json::Value::as_u64)
        .ok_or("Invalid server.")?;
    let channel_string = string("channel", 39)?;
    let channel = channel_string
        .parse::<u128>()
        .map_err(|_| "Invalid channel.")?;
    if channel.to_string() != channel_string {
        return Err("Invalid channel.");
    }
    let encoded = string("expectedContext", 64)?;
    if encoded.len() != 64 {
        return Err("Invalid chat context.");
    }
    let mut context = [0; 32];
    hex::decode_to_slice(encoded, &mut context).map_err(|_| "Invalid chat context.")?;
    if context == [0; 32] {
        return Err("Invalid chat context.");
    }
    let action_id = |encoded: &str| -> Result<ActionId, &'static str> {
        let mut id = [0; 32];
        hex::decode_to_slice(encoded, &mut id).map_err(|_| "Invalid message.")?;
        Ok(ActionId::from_bytes(id))
    };
    let limit = || {
        object
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .filter(|n| (1..=8).contains(n))
            .map(|n| n as usize)
            .ok_or("Invalid history page size.")
    };
    let own = object.contains_key("start") || object.contains_key("endExclusive");
    let request = match (
        object.contains_key("receivedAction"),
        object.contains_key("archivedAction"),
        object.get("archiveAfter"),
        own,
    ) {
        (true, false, None, false) if !object.contains_key("limit") => {
            ReadRequest::Live(action_id(string("receivedAction", 64)?)?)
        }
        (false, true, None, false) if !object.contains_key("limit") => ReadRequest::Archived(
            ArchiveRead::Message(action_id(string("archivedAction", 64)?)?),
        ),
        (false, false, Some(after), false) => {
            let after = match after {
                serde_json::Value::Null => None,
                serde_json::Value::String(value)
                    if value.len() == 64 && !value.bytes().any(|c| c.is_ascii_uppercase()) =>
                {
                    Some(action_id(value)?)
                }
                _ => return Err("Invalid history cursor."),
            };
            ReadRequest::Archived(ArchiveRead::Page {
                after,
                limit: limit()?,
            })
        }
        (false, false, None, true) => {
            let start = decimal(string("start", 20)?)?;
            let end = match object.get("endExclusive") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(value)) if value.len() <= 20 => {
                    Some(decimal(value)?)
                }
                _ => return Err("Invalid history cursor."),
            };
            if end.is_some_and(|end| start > end) {
                return Err("Invalid history cursor.");
            }
            ReadRequest::Own(OwnRead {
                start,
                end,
                maximum: limit()?,
            })
        }
        _ => return Err("Invalid history request."),
    };
    if object
        .get("trace")
        .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| s.len() > 128))
    {
        return Err("Invalid history request.");
    }
    Ok(Read {
        server,
        channel,
        context,
        request,
    })
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
    let capture = (|| {
        let state = app.state::<AppState>();
        let generation = state.ui_session_generation.load(Ordering::SeqCst);
        if ui_session_closed(&state) || state.native_text.sources.is_closing_cached() {
            return Err(STALE);
        }
        let target = state.native_text.ui_state.original_target().ok_or(STALE)?;
        let permit = state
            .native_text
            .inflight
            .clone()
            .try_acquire_owned()
            .map_err(|_| BUSY)?;
        // Full native response payment precedes parsing copies and Root work.
        let credit = state
            .native_text
            .history_budget
            .prepare()
            .map_err(|_| BUSY)?;
        let payment = Arc::new(Payment {
            _permit: permit,
            _credit: credit,
        });
        let read = parse(invoke.message.payload())?;
        let account = keep_session::current(&state, generation).ok_or(STALE)?;
        let (selection, instance, scope, member_work) = {
            let table = state.servers.try_lock().map_err(|_| BUSY)?;
            let row = table.native(&read.server).ok_or(STALE)?;
            if row.data().request != read.context
                || row.data().channel != read.channel
                || !row.data().target.same_original(&target)
            {
                return Err(STALE);
            }
            (
                row.data().selection.clone(),
                row.data().instance,
                row.scope(account).ok_or(STALE)?,
                row.data().member_work.clone(),
            )
        };
        Ok(Invocation {
            generation,
            target,
            selection,
            instance,
            scope,
            member_work,
            read,
            payment,
        })
    })();
    let invocation = match capture {
        Ok(value) => value,
        Err(error) => {
            invoke.resolver.reject(error);
            return true;
        }
    };
    let resolver = invoke.resolver;
    drop(invoke.message);
    spawn(Box::pin(async move {
        // Owning result is outside ALL final guards. Resolver unwind must not
        // destroy the paid Root page or its invoice while those guards exist.
        let state = app.state::<AppState>();
        let mut result = match invocation.read.request {
            ReadRequest::Live(action) => execute_received(&invocation, action)
                .await
                .map(Response::Received),
            ReadRequest::Own(read) => execute(&state, &invocation, read)
                .await
                .map(Response::Saved),
            ReadRequest::Archived(read) => execute_archive(&state, &invocation, read).await,
        };
        #[cfg(test)]
        {
            let edge = state.native_text.text_reply_edge.lock().unwrap().clone();
            if let Some(edge) = edge {
                edge.wait_once().await;
            }
        }
        let _commit = state.ui_session_commit.lock().await;
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        if !current(&state, &invocation, &store, &table) {
            resolver.reject(STALE);
        } else {
            match &mut result {
                Ok(Response::Saved(output)) => {
                    resolver.resolve(InvokeResponseBody::Json(output.body.take().unwrap()))
                }
                Ok(Response::Received(output)) => output.resolve(resolver),
                Ok(Response::ArchivePage(output)) => {
                    resolver.resolve(InvokeResponseBody::Json(output.body.take().unwrap()))
                }
                Err(error) => resolver.reject(&*error),
            }
        }
    }));
    true
}
fn current(
    state: &AppState,
    call: &Invocation,
    store: &Option<MountedVault>,
    table: &server_table::ServerTable,
) -> bool {
    state.ui_session_generation.load(Ordering::SeqCst) == call.generation
        && !ui_session_closed(state)
        && !call.scope.is_cancelled()
        && !state.native_text.sources.is_closing_cached()
        && store
            .as_ref()
            .and_then(MountedVault::as_text_v2)
            .is_some_and(|vault| call.target.is_for_original_vault(vault))
        && table.native(&call.read.server).is_some_and(|row| {
            row.matches(&call.selection, call.instance, &call.target)
                && row.data().request == call.read.context
                && row.data().channel == call.read.channel
        })
}
async fn capture(state: &AppState, call: &Invocation) -> Result<Preparation, String> {
    let mut budget = if call.selection.founder().is_some() {
        Some(
            state
                .native_text
                .sources
                .policy()
                .prepare_member()
                .map_err(|_| BUSY)?,
        )
    } else {
        None
    };
    let mut ui = Some(founding::ui_capture(state, call.generation, &call.target).await?);
    let mut founder = None;
    let mut joined = None;
    let result = {
        let _commit = require_ui_session_generation(state, call.generation).await?;
        let store = state.store.lock().await;
        let table = state.servers.lock().await;
        if !current(state, call, &store, &table) {
            Err(STALE)
        } else {
            match &call.selection {
                server_table::NativeSelection::Founder(_) => state
                    .native_text
                    .sources
                    .capture_member(&store, &mut ui, &call.selection, &mut budget, &mut founder),
                server_table::NativeSelection::Joined(selection) => state
                    .native_text
                    .sources
                    .capture_joined_member(&store, &mut ui, selection, &mut joined),
            }
        }
    };
    result.map_err(str::to_owned)?;
    match (&call.selection, founder, joined) {
        (server_table::NativeSelection::Founder(_), Some(original), _) => {
            Ok(Preparation::Founder(original))
        }
        (server_table::NativeSelection::Joined(row), _, Some(original)) => {
            Ok(Preparation::Joined(original, row.clone()))
        }
        _ => Err(STALE.to_owned()),
    }
}
struct Work {
    preparation: Preparation,
    scope: RequestCancellation,
    read: OwnRead,
    channel: u128,
    _local: Option<OwnedMutexGuard<()>>,
    #[cfg(test)]
    watch: Option<Arc<DropWatch>>,
    payment: Arc<Payment>, // LAST during unrun/unwind/returned response
}
struct Output {
    _page: NativeTextMemberHistoryPage,
    body: Option<String>,
    #[cfg(test)]
    _notice: Option<DropNotice>, // actual page/body have dropped, credit is live
    _payment: Arc<Payment>,
}

enum Response {
    Saved(Output),
    Received(ReceivedOutput),
    ArchivePage(ArchivePageOutput),
}

struct ArchivePageOutput {
    body: Option<String>,
    _payment: Arc<Payment>,
}

struct ReceivedOutput {
    scope: &'static str,
    projection: Option<NativeTextJoinedMemberProjection>,
    _payment: Arc<Payment>,
}

impl ReceivedOutput {
    fn resolve<R: tauri::Runtime>(&self, resolver: tauri::ipc::InvokeResolver<R>) {
        let Some(projection) = &self.projection else {
            resolver.resolve(InvokeResponseBody::Json(received_body(self.scope, None)));
            return;
        };
        let mut resolver = Some(resolver);
        let _ = projection.with_publication(|result| {
            resolver
                .take()
                .unwrap()
                .resolve(InvokeResponseBody::Json(received_body(
                    self.scope,
                    Some(result),
                )));
        });
        if let Some(resolver) = resolver {
            resolver.reject(STALE);
        }
    }
}

fn received_body(scope: &'static str, projection: Option<&GuardedProjection>) -> String {
    let item = match projection {
        Some(GuardedProjection::Visible {
            create,
            text,
            certainty,
            ..
        }) => serde_json::json!({
            "action": hex::encode(create.as_bytes()),
            "text": text,
            "certainty": match certainty {
                ProjectionCertainty::Proven => "proven",
                ProjectionCertainty::Provisional => "provisional",
            }
        }),
        _ => serde_json::Value::Null,
    };
    serde_json::json!({"backend":"text-v2","scope":scope,"item":item}).to_string()
}

async fn execute_received(call: &Invocation, action: ActionId) -> Result<ReceivedOutput, String> {
    let Some(local) = call.member_work.clone() else {
        return Ok(ReceivedOutput {
            scope: "received-live",
            projection: None,
            _payment: call.payment.clone(),
        });
    };
    let guard = local.foreground(call.scope.clone()).await.ok_or(STALE)?;
    let payment = call.payment.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let slot = local.received.lock().map_err(|_| BUSY.to_owned())?;
        let projection = slot
            .as_ref()
            .map(|ledger| ledger.read_message_projection(action))
            .transpose()
            .map_err(|_| STALE.to_owned())?
            .flatten();
        Ok(ReceivedOutput {
            scope: "received-live",
            projection,
            _payment: payment,
        })
    })
    .await
    .map_err(|_| STALE.to_owned())?
}

async fn execute_archive(
    state: &AppState,
    call: &Invocation,
    read: ArchiveRead,
) -> Result<Response, String> {
    let channel = call.read.channel;
    let scope = call.scope.clone();
    let payment = call.payment.clone();
    let original = if let Some(local) = &call.member_work {
        let guard = local.foreground(scope.clone()).await.ok_or(STALE)?;
        let Preparation::Joined(original, selection) = capture(state, call).await? else {
            return Err(STALE.to_owned());
        };
        Some((original, selection, guard))
    } else {
        None
    };
    tauri::async_runtime::spawn_blocking(move || match read {
        ArchiveRead::Page { after, limit } => {
            let page = match original {
                Some((original, row, _guard)) => original
                    .archived_action_page(&row, channel, after, limit, &scope)
                    .map_err(|_| "Saved received messages could not be read. Try again.")?,
                None => None,
            };
            let ids = page.as_ref().map_or_else(Vec::new, |page| {
                page.actions()
                    .iter()
                    .map(|id| hex::encode(id.as_bytes()))
                    .collect()
            });
            let body = serde_json::json!({
                "backend": "text-v2",
                "scope": "received-archive-index",
                "available": page.is_some(),
                "ids": ids,
                "more": page.as_ref().is_some_and(|page| page.has_more()),
            })
            .to_string();
            Ok(Response::ArchivePage(ArchivePageOutput {
                body: Some(body),
                _payment: payment,
            }))
        }
        ArchiveRead::Message(action) => {
            let projection = match original {
                Some((original, row, _guard)) => original
                    .archived_message_projection(&row, channel, action, scope, &mut OsCryptoRng)
                    .map_err(|_| "Saved received messages could not be read. Try again.")?,
                None => None,
            };
            Ok(Response::Received(ReceivedOutput {
                scope: "received-archive",
                projection,
                _payment: payment,
            }))
        }
    })
    .await
    .map_err(|_| STALE.to_owned())?
}
impl Work {
    fn run(self) -> Result<Output, String> {
        let read = self.read;
        let page = match self.preparation {
            Preparation::Founder(original) => original
                .finish()
                .map_err(|_| STALE)?
                .prepare_member_history(read.start, read.end, read.maximum, &self.scope),
            Preparation::Joined(original, row) => original.prepare_member_history(
                &row,
                read.start,
                read.end,
                read.maximum,
                &self.scope,
                &mut OsCryptoRng,
            ),
        }
        .map_err(|_| "Chat history could not be read. Try again.")?;
        // Consuming the Root job restores its runtime before any native await.
        let body = serialize_page(&page, self.channel, SERIALIZED_LIMIT)?;
        Ok(Output {
            _page: page,
            body: Some(body),
            #[cfg(test)]
            _notice: self.watch.map(|watch| DropNotice {
                watch,
                payment: Arc::downgrade(&self.payment),
            }),
            _payment: self.payment,
        })
    }
}
async fn execute(state: &AppState, call: &Invocation, read: OwnRead) -> Result<Output, String> {
    let local = match &call.member_work {
        Some(work) => Some(work.foreground(call.scope.clone()).await.ok_or(STALE)?),
        None => None,
    };
    let work = Work {
        preparation: capture(state, call).await?,
        scope: call.scope.clone(),
        read,
        channel: call.read.channel,
        _local: local,
        #[cfg(test)]
        watch: state.native_text.history_drop_watch.lock().unwrap().clone(),
        payment: call.payment.clone(),
    };
    tauri::async_runtime::spawn_blocking(move || work.run())
        .await
        .map_err(|_| STALE.to_owned())?
}

/// Observation only, after actual page/body destruction. The callback records
/// atomics rather than panicking during another panic's cleanup.
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
#[serde(tag = "kind", rename_all = "lowercase")]
enum BodyData<'a> {
    Create {
        text: &'a str,
        reply_to: Option<String>,
    },
    Edit {
        create: String,
        text: &'a str,
    },
    Delete {
        create: String,
    },
    Join {
        create: String,
    },
}
#[derive(Serialize)]
struct Record<'a> {
    ordinal: String,
    action: String,
    signer: String,
    channel: String,
    presented_ms: String,
    status: &'static str,
    body: BodyData<'a>,
}
fn record<'a>(
    action: &'a SignedAction,
    ordinal: u64,
    status: NativeTextMemberHistoryStatus,
) -> Record<'a> {
    let body = match &action.spec().body {
        Body::Create { text, reply_to } => BodyData::Create {
            text,
            reply_to: reply_to.map(|id| hex::encode(id.as_bytes())),
        },
        Body::Edit { create, text } => BodyData::Edit {
            create: hex::encode(create.as_bytes()),
            text,
        },
        Body::Delete { create } => BodyData::Delete {
            create: hex::encode(create.as_bytes()),
        },
        Body::Join { create } => BodyData::Join {
            create: hex::encode(create.as_bytes()),
        },
    };
    Record {
        ordinal: ordinal.to_string(),
        action: hex::encode(action.id().as_bytes()),
        signer: hex::encode(action.signer()),
        channel: action.spec().realm.channel().to_string(),
        presented_ms: action.spec().presented_ms.to_string(),
        status: match status {
            NativeTextMemberHistoryStatus::Pending => "pending",
            NativeTextMemberHistoryStatus::Submitted => "submitted",
            NativeTextMemberHistoryStatus::Held => "held",
        },
        body,
    }
}
#[derive(Serialize)]
struct Page<'a> {
    backend: &'static str,
    scope: &'static str,
    next: String,
    end_exclusive: String,
    items: Vec<Record<'a>>,
}
fn serialize_page(
    page: &NativeTextMemberHistoryPage,
    channel: u128,
    cap: usize,
) -> Result<String, String> {
    // The Root cursor is CONTEXT-wide. Empty channel-filtered pages still carry
    // its next/end; never infer group completeness/unread totals from items.
    let borrowed: Vec<_> = page.items().collect();
    let items = borrowed
        .iter()
        .map(|item| record(item.action(), item.ordinal(), item.status()))
        .collect();
    encode(
        &channel_page(items, page.next(), page.end_exclusive(), channel),
        cap,
    )
}
fn channel_page(items: Vec<Record<'_>>, next: u64, end: u64, channel: u128) -> Page<'_> {
    let channel = channel.to_string();
    Page {
        backend: "text-v2",
        scope: "own-saved-originals",
        next: next.to_string(),
        end_exclusive: end.to_string(),
        items: items
            .into_iter()
            .filter(|item| item.channel == channel)
            .collect(),
    }
}
struct Capped {
    bytes: Vec<u8>,
    cap: usize,
}
impl Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|len| len > self.cap)
        {
            return Err(std::io::Error::other("History response limit reached"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode(value: &impl Serialize, cap: usize) -> Result<String, String> {
    // No serde_json::Value/whole-page clone and no Vec growth past fixed cap.
    if cap > SERIALIZED_LIMIT {
        return Err("Invalid history response limit.".into());
    }
    let mut writer = Capped {
        bytes: Vec::with_capacity(cap),
        cap,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| "Chat history response is too large.")?;
    String::from_utf8(writer.bytes).map_err(|_| "Invalid history response.".into())
}
const _: () =
    assert!(SERIALIZED_LIMIT + 1024 * 1024 <= NativeTextHistoryResponseBudget::response_bytes());

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_cursor_is_lossless_canonical_and_bounded() {
        assert_eq!(decimal("18446744073709551615"), Ok(u64::MAX));
        for bad in ["", "01", "-1", "+1", "18446744073709551616"] {
            assert!(decimal(bad).is_err());
        }
    }
    #[test]
    fn history_empty_channel_page_still_advances_context_cursor() {
        let key = catcoms_crypto::DeviceKeypair::from_seed(&[7; 32]);
        let action = SignedAction::sign(
            &key,
            catcoms_replication::chat::ActionSpec {
                realm: catcoms_replication::chat::Realm::new(b"history-context", 2).unwrap(),
                basis: catcoms_replication::chat::BasisId::from_bytes([4; 32]),
                grant: catcoms_replication::chat::GrantId::from_bytes([5; 32]),
                nonce: [3; 16],
                presented_ms: u64::MAX,
                parents: vec![],
                body: Body::Create {
                    text: "another channel".into(),
                    reply_to: None,
                },
            },
        )
        .unwrap();
        let page = channel_page(
            vec![record(&action, 0, NativeTextMemberHistoryStatus::Submitted)],
            1,
            3,
            1,
        );
        let json: serde_json::Value =
            serde_json::from_str(&encode(&page, SERIALIZED_LIMIT).unwrap()).unwrap();
        assert!(json["items"].as_array().unwrap().is_empty());
        assert_eq!(json["next"], "1");
        assert_eq!(json["end_exclusive"], "3");
        let page = channel_page(
            vec![record(&action, 0, NativeTextMemberHistoryStatus::Held)],
            1,
            3,
            2,
        );
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].status, "held");
    }
    #[test]
    fn history_serializer_handles_eight_maximum_escaped_texts_without_copying_them() {
        let text = "\0".repeat(catcoms_replication::chat::MAX_TEXT_BYTES);
        let items = (0..8)
            .map(|i| Record {
                ordinal: i.to_string(),
                action: "f".repeat(64),
                signer: "e".repeat(64),
                channel: u128::MAX.to_string(),
                presented_ms: u64::MAX.to_string(),
                status: "pending",
                body: BodyData::Create {
                    text: &text,
                    reply_to: Some("d".repeat(64)),
                },
            })
            .collect();
        let page = Page {
            backend: "text-v2",
            scope: "own-saved-originals",
            next: "8".into(),
            end_exclusive: "8".into(),
            items,
        };
        let encoded = encode(&page, SERIALIZED_LIMIT).unwrap();
        assert!(encoded.len() > 3 * 1024 * 1024);
        assert!(encode(&page, encoded.len() - 1).is_err());
        assert_eq!(encode(&page, encoded.len()).unwrap(), encoded);
        assert!(encode(&page, SERIALIZED_LIMIT + 1).is_err());
    }
}
