//! Nonvisual Studio IPC. Ready first; all subsequent lock acquisition is fail-fast to avoid
//! parking the actor behind native work that itself needs that actor (lock cleanup/persistence).
use super::*;
use catcoms_app::studio::{types::*, EpochPhase, StudioRequest, StudioVaultLease, StudioView};
use serde_json::{json, Value};
pub(crate) mod recovery;
pub(crate) mod settlement;

enum InvokeRequest {
    Document(StudioRequest),
    Control(catcoms_app::studio::StudioControlRequest),
}
enum InvokeReady {
    Document(catcoms_app::studio::StudioReady),
    Control(catcoms_app::studio::StudioControlReady),
}
enum InvokeResponse {
    Document(Option<StudioView>),
    Control(catcoms_app::studio::StudioControlResponse),
}

fn id(s: &str) -> Result<[u8; 16], String> {
    if s.len() != 32
        || !s
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("Studio id must be 32 lowercase hex characters".into());
    }
    hex::decode(s)
        .map_err(|_| "invalid Studio id".to_string())?
        .try_into()
        .map_err(|_| "invalid Studio id".into())
}
fn channel_id(s: &str) -> Result<[u8; 16], String> {
    let value: u128 = s.parse().map_err(|_| "invalid channel".to_string())?;
    if value.to_string() != s {
        return Err("channel must be canonical decimal".into());
    }
    Ok(value.to_be_bytes())
}
fn apply_request(
    channel: &str,
    object: Option<&str>,
    epoch_id: &str,
    nonce: &str,
    body: String,
) -> Result<StudioRequest, String> {
    if body.len() > 64 * 1024 {
        return Err("Studio body exceeds 64 KiB".into());
    }
    let channel = channel_id(channel)?;
    let target = match object {
        Some(object) => StudioTarget::Flipnote {
            channel,
            object: id(object)?,
        },
        None => StudioTarget::Index { channel },
    };
    let request = StudioRequest::Apply {
        target,
        epoch_id: u128::from_be_bytes(id(epoch_id)?),
        nonce: id(nonce)?,
        body: body.into_bytes(),
    };
    request.validate().map_err(|e| e.to_string())?;
    Ok(request)
}

async fn invoke(
    state: &AppState,
    server: u64,
    request: StudioRequest,
) -> Result<Option<Value>, String> {
    invoke_custody(
        state,
        server,
        InvokeRequest::Document(request),
        |response| match response {
            InvokeResponse::Document(response) => response.map(view).transpose(),
            _ => Err("mismatched Studio response".into()),
        },
    )
    .await
}

/// The one Ready/lease/session fence used by both live document and historical recovery IPC.
/// Export keeps its returned bytes private until the same final generation/instance recheck.
async fn invoke_custody<V>(
    state: &AppState,
    server: u64,
    request: InvokeRequest,
    convert: impl FnOnce(InvokeResponse) -> Result<V, String>,
) -> Result<V, String> {
    if let InvokeRequest::Document(request) = &request {
        request.validate().map_err(|e| e.to_string())?;
    }
    let generation = unlocked_ui_session_generation(state).await?;
    // Reuse the existing four bounded native operation slots and cancellation-on-lock seam.
    let (slot, signal) = claim_internal_inline_download(state)?;
    let (actor, instance) = actor_instance_of(state, server).await?;
    let mut cancellation = RequestCancellation::new(signal, Some(slot.request_keepalive()));
    let clock = SystemClock;
    let ready = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err("Studio request cancelled".into()),
        _ = clock.sleep(std::time::Duration::from_secs(5)) => return Err("Studio actor busy; retry".into()),
        result = async { match request {
            InvokeRequest::Document(request) => actor.studio_begin(request).await.map(InvokeReady::Document),
            InvokeRequest::Control(request) => actor.studio_control_begin(request).await.map(InvokeReady::Control),
        }} => result?,
    };
    let lease =
        authorize(state, server, instance, generation)?.with_cancellation(cancellation.clone());
    // After lease transfer the finite worker owns ALL fences, even if this invoke is dropped.
    // Cancellation can suppress its result, not roll back a save that already began.
    let response = match ready {
        InvokeReady::Document(ready) => InvokeResponse::Document(ready.execute(lease).await?),
        InvokeReady::Control(ready) => InvokeResponse::Control(ready.execute(lease).await?),
    };
    if cancellation.is_cancelled() {
        return Err("Studio request cancelled; its local save may have completed".into());
    }
    let _commit = require_ui_session_generation(state, generation).await?;
    let servers = state.servers.lock().await;
    if servers
        .get(&server)
        .is_none_or(|entry| entry.instance != instance)
    {
        return Err("server changed during Studio operation".into());
    }
    // Conversion can be substantial (base64 and full conflict-preserving JSON). Keep the
    // completion fences until it is finished, and suppress even a lock request that arrived
    // during conversion before the actual lock task can acquire this commit guard.
    let value = convert(response)?;
    if cancellation.is_cancelled()
        || state.session_lock_requested.load(Ordering::Acquire)
        || state.ui_session_generation.load(Ordering::Acquire) != generation
    {
        return Err("Studio response belongs to a locked or changed UI session".into());
    }
    Ok(value)
}

fn authorize(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
) -> Result<StudioVaultLease, String> {
    let busy = || "Studio storage busy; retry the same request".to_string();
    // NEVER await any mutex once the actor has advertised Ready. Busy releases every guard and
    // dropping Ready resumes the actor. The registry guard fences leave/reinstall, not just saves.
    let persist = persist_lock_for(state, server)
        .try_lock_owned()
        .map_err(|_| busy())?;
    let commit = state
        .ui_session_commit
        .clone()
        .try_lock_owned()
        .map_err(|_| busy())?;
    let store = state.store.clone().try_lock_owned().map_err(|_| busy())?;
    let resumable = state.session_resumable.try_lock().map_err(|_| busy())?;
    if !*resumable
        || store.is_none()
        || state.session_lock_requested.load(Ordering::Acquire)
        || state.ui_session_generation.load(Ordering::Acquire) != generation
    {
        return Err("Studio request belongs to a locked or changed UI session".into());
    }
    drop(resumable);
    let servers = state.servers.clone().try_lock_owned().map_err(|_| busy())?;
    if servers
        .get(&server)
        .is_none_or(|entry| entry.instance != instance)
    {
        return Err("server changed before Studio commit".into());
    }
    Ok(StudioVaultLease::new(
        store,
        server,
        (persist, commit, servers),
    ))
}

/// One supervised receiver per installed actor, not one task per packet or open document.
/// The actor's boolean watch coalesces inbox work without making its event consumer await the
/// actor (which could deadlock on event backpressure). No vault guard is kept between passes.
pub(crate) fn spawn_receiver(app: AppHandle, server: u64, instance: u64, actor: ServerActor) {
    let task = tokio::spawn(async move {
        drive_receiver(
            &app.state::<AppState>(),
            server,
            instance,
            &actor,
            &SystemClock,
        )
        .await;
    });
    supervise("studio_receive", server, task);
}

async fn drive_receiver(
    state: &AppState,
    server: u64,
    instance: u64,
    actor: &ServerActor,
    clock: &dyn Clock,
) {
    let mut pending = actor.studio_pending();
    loop {
        if pending.has_changed().is_err() {
            break;
        }
        if !*pending.borrow_and_update() {
            // A disconnected peer can miss the LAST edit: no later gossip edge is guaranteed.
            // Poll the existing coordinator occasionally without adding an actor select timer
            // that would cancel legacy sync_once/outbox work. No vault guard survives this wait.
            if !idle_receiver_wake(&mut pending, clock).await {
                break;
            }
        }
        if state
            .servers
            .lock()
            .await
            .get(&server)
            .is_none_or(|e| e.instance != instance)
        {
            break;
        }
        // Busy, locked or failed storage leaves no success claim. Retry at most once per
        // second while this bounded inbox is nonempty; no packet is pulled before custody.
        let _ = receive_once(state, server, instance, actor).await;
        // Even repeated false/true edges cannot reset the throttle. No locks/slots remain
        // held during this delay; actor closure is noticed on its next bounded iteration.
        clock.sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// An idle catch-up check is needed even without another gossip edge. The peer's last edit
/// may have happened while disconnected; actor closure still stops the native worker promptly.
async fn idle_receiver_wake(
    pending: &mut tokio::sync::watch::Receiver<bool>,
    clock: &dyn Clock,
) -> bool {
    tokio::select! {
        result = pending.changed() => result.is_ok(),
        _ = clock.sleep(std::time::Duration::from_secs(5)) => true,
    }
}

async fn receive_once(
    state: &AppState,
    server: u64,
    instance: u64,
    actor: &ServerActor,
) -> Result<(), String> {
    let generation = unlocked_ui_session_generation(state).await?;
    let (slot, signal) = claim_internal_inline_download(state)?;
    let mut cancellation = RequestCancellation::new(signal, Some(slot.request_keepalive()));
    let ready = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err("Studio receive cancelled".into()),
        _ = SystemClock.sleep(std::time::Duration::from_secs(5)) => return Err("Studio receiver busy".into()),
        ready = actor.studio_receive_begin() => ready?,
    };
    // Uses the ORIGINAL worker incarnation, never the actor currently occupying this number.
    let lease = authorize(state, server, instance, generation)?.with_cancellation(cancellation);
    ready.execute(lease).await?;
    Ok(())
}

/// Retain exact UI/incarnation custody through synchronous emission, after source leases have
/// dropped. Delayed old-actor events cannot invalidate a replacement server's Studio views.
pub(crate) async fn forward_if_current(
    state: &AppState,
    server: u64,
    instance: u64,
    emit: impl FnOnce(),
) -> bool {
    let Ok(generation) = unlocked_ui_session_generation(state).await else {
        return false;
    };
    let Ok(_commit) = require_ui_session_generation(state, generation).await else {
        return false;
    };
    let entries = state.servers.lock().await;
    if entries.get(&server).is_none_or(|e| e.instance != instance) {
        return false;
    }
    emit();
    true
}

#[tauri::command]
pub(crate) async fn studio_list(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
) -> Result<Option<Value>, String> {
    invoke(
        &state,
        server,
        StudioRequest::Read {
            target: StudioTarget::Index {
                channel: channel_id(&channel)?,
            },
        },
    )
    .await
}
#[tauri::command]
pub(crate) async fn studio_read(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: String,
) -> Result<Option<Value>, String> {
    invoke(
        &state,
        server,
        StudioRequest::Read {
            target: StudioTarget::Flipnote {
                channel: channel_id(&channel)?,
                object: id(&object)?,
            },
        },
    )
    .await
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn studio_create(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: String,
    nonce: String,
    title: String,
    created_at_ms: u64,
) -> Result<Option<Value>, String> {
    invoke(
        &state,
        server,
        StudioRequest::Create {
            channel: channel_id(&channel)?,
            object: id(&object)?,
            nonce: id(&nonce)?,
            title,
            ts: created_at_ms,
        },
    )
    .await
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn studio_apply(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    object: String,
    epoch_id: String,
    nonce: String,
    body: String,
) -> Result<Option<Value>, String> {
    invoke(
        &state,
        server,
        apply_request(&channel, Some(&object), &epoch_id, &nonce, body)?,
    )
    .await
}
#[tauri::command]
pub(crate) async fn studio_apply_index(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    epoch_id: String,
    nonce: String,
    body: String,
) -> Result<Option<Value>, String> {
    invoke(
        &state,
        server,
        apply_request(&channel, None, &epoch_id, &nonce, body)?,
    )
    .await
}

// This bridge representation intentionally retains all conflict/deletion/overflow evidence.
// It is not the old fixture's lossy root or a synthetic "settled" label. Expiry is discriminated:
// absent, never and timestamp zero remain three different values at this boundary.
fn expiry(v: StudioExpiry) -> Value {
    match v {
        StudioExpiry::Unrecorded => json!({"kind":"unrecorded"}),
        StudioExpiry::Never => json!({"kind":"never"}),
        StudioExpiry::At(ms) => json!({"kind":"at", "ms":ms}),
    }
}
fn isource(v: &IndexSource) -> Value {
    json!({"opId":hex::encode(v.op_id),"author":v.author.to_string(),"nonce":hex::encode(v.nonce)})
}
fn fsource(v: &FrameSource) -> Value {
    json!({"opId":hex::encode(v.op_id),"author":v.author.to_string(),"nonce":hex::encode(v.nonce),"ts":v.ts})
}
fn ireg<T>(v: &IndexRegister<T>, f: impl Fn(&T) -> Value) -> Value {
    let value = |v: &IndexValue<T>| json!({"value":f(&v.value),"source":isource(&v.source)});
    json!({"selected":value(&v.selected),"conflicts":v.conflicts.iter().map(value).collect::<Vec<_>>()})
}
fn freg<T>(v: &FrameRegister<T>, f: impl Fn(&T) -> Value) -> Value {
    let value = |v: &FrameValue<T>| json!({"value":f(&v.value),"source":fsource(&v.source)});
    json!({"selected":value(&v.selected),"conflicts":v.conflicts.iter().map(value).collect::<Vec<_>>()})
}
fn objects(v: &std::collections::BTreeMap<[u8; 16], IndexEntry>) -> Value {
    Value::Object(v.iter().map(|(id,e)| (hex::encode(id), json!({
        "creations":e.creations.iter().map(|v| json!({"source":isource(&v.source),"value":{
            "kind":match v.value.kind { StudioKind::Flipnote=>"flipnote",StudioKind::Score=>"score" },
            "title":v.value.title,"createdBy":v.value.created_by.to_string(),"ts":v.value.ts,"expiry":expiry(v.value.expiry)
        }})).collect::<Vec<_>>(),"title":ireg(&e.title,|v|json!(v)),"expiry":ireg(&e.expiry,|v|expiry(*v))
    }))).collect())
}
fn blob(v: &FrameBlob) -> Value {
    json!({"cid":hex::encode(v.cid),"bytes":v.bytes})
}
fn view(v: StudioView) -> Result<Value, String> {
    let content = projection_content(&v.projection);
    let value = json!({"v":1,"epochId":format!("{:032x}",v.epoch_id),"epoch":v.epoch.to_string(),
        "channel":u128::from_be_bytes(v.projection.channel()).to_string(),"publication":"local", "provisional":true,
        "phase":match v.phase {EpochPhase::Open=>"open",EpochPhase::Closing=>"closing",EpochPhase::Settled=>"settled",EpochPhase::Fault=>"fault"},"content":content});
    bounded_view(value)
}
fn projection_content(projection: &StudioProjection) -> Value {
    match projection {
        StudioProjection::Index(p) => {
            json!({"kind":"index","objects":objects(&p.objects),"overflow":objects(&p.overflow),
            "deletedObjects":objects(&p.deleted_objects),"tombstones":p.tombstones.iter().map(|(id,vs)|(hex::encode(id),vs.iter().map(isource).collect::<Vec<_>>())).collect::<std::collections::BTreeMap<_,_>>() })
        }
        StudioProjection::Flipnote(p) => {
            json!({"kind":"flipnote","title":p.title.as_ref().map(|v|freg(v,|v|json!(v))),"fps":p.fps.as_ref().map(|v|freg(v,|v|json!(v))),
            "timeline":p.timeline.iter().map(hex::encode).collect::<Vec<_>>(),"declaredFrameBytes":p.declared_frame_bytes,
            "overCap":p.over_cap.iter().map(|(id,v)|(hex::encode(id),json!({"count":v.count,"bytes":v.bytes}))).collect::<std::collections::BTreeMap<_,_>>(),
            "tombstones":p.tombstones.iter().map(|(id,vs)|(hex::encode(id),vs.iter().map(fsource).collect::<Vec<_>>())).collect::<std::collections::BTreeMap<_,_>>(),
            "frames":p.frames.iter().map(|(id,e)|(hex::encode(id),json!({"pixels":freg(&e.pixels,blob),"insertions":e.insertions.iter().map(|v| json!({
                "source":fsource(&v.source),"value":{"checkpoint":v.value.checkpoint,"after":v.value.after.map(hex::encode),"anchor":v.value.anchor.map(hex::encode),"before":v.value.before.map(hex::encode),"blob":blob(&v.value.blob)}
            })).collect::<Vec<_>>()}))).collect::<std::collections::BTreeMap<_,_>>() })
        }
    }
}
fn bounded_view(value: Value) -> Result<Value, String> {
    // Typed source/recovery caps bound construction; also refuse an oversized IPC encoding rather
    // than silently dropping conflict evidence to fit a UI payload.
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
        return Err("Studio view exceeds IPC limit".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
