//! Native custody for a chat operation's durable acceptance barrier.
use super::*;
use catcoms_app::durable_chat::{
    DurableChatRequest, DurableMutationAction, DurableMutationRequest, DurableSendLease,
    DurableSendOutcome, DurableSendRequest, DurableStopRequest,
};
use catcoms_sync::durable_chat::ChatDisposition;

fn fixed_hex<const N: usize>(text: &str) -> Result<[u8; N], String> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("Invalid message retry identity.".into());
    }
    hex::decode(text)
        .map_err(|_| "Invalid message retry identity.".to_string())?
        .try_into()
        .map_err(|_| "Invalid message retry identity.".to_string())
}

fn authorize(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
) -> Result<DurableSendLease, String> {
    let busy = || "Message storage is busy; retry this pending message.".to_string();
    // The actor already owns Ready. Waiting for a native lock here can deadlock another actor
    // transaction; fail fast and let dropping Ready resume the actor instead.
    let persist = persist_lock_for(state, server)
        .try_lock_owned()
        .map_err(|_| busy())?;
    let session = state
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
        return Err("Message request belongs to a locked or changed vault.".into());
    }
    drop(resumable);
    let registry = state.servers.clone().try_lock_owned().map_err(|_| busy())?;
    if registry
        .get(&server)
        .is_none_or(|entry| entry.instance != instance)
    {
        return Err("The conversation changed before this message could be saved.".into());
    }
    Ok(DurableSendLease::new(
        store,
        server,
        (persist, session, registry),
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn send(
    state: &AppState,
    op: &Operation,
    server: u64,
    channel: &str,
    text: String,
    reply_to: Option<String>,
    retry_token: Option<String>,
    expected_context: Option<String>,
) -> Result<SendMessageResult, AppError> {
    let generation = unlocked_ui_session_generation(state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    let channel = channel
        .parse()
        .map_err(|_| op.fail(codes::CHANNEL_BAD_ID, "bad channel id"))?;
    let (actor, instance) = actor_instance_of(state, server)
        .await
        .map_err(|failure| op.fail(failure.code(), failure.message()))?;
    let actor = op.bind_actor(actor);
    let (token, expected_context) = match (retry_token, expected_context) {
        (Some(token), Some(context)) => (
            fixed_hex(&token).map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?,
            fixed_hex(&context).map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?,
        ),
        // Older local callers still cross the same save barrier. Updated callers retain the
        // token before invocation so they can also recover an ambiguous IPC completion.
        (None, None) => {
            let mut token = [0u8; 16];
            OsCryptoRng.fill_bytes(&mut token);
            let context = actor
                .durable_send_context()
                .await
                .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
            (token, context)
        }
        _ => {
            return Err(op.fail(
                codes::CHAT_SEND_REJECTED,
                "A retry requires both its identity and original authoring context.",
            ))
        }
    };
    let ready = actor
        .prepare_durable_send(DurableSendRequest {
            token,
            expected_context,
            channel,
            text,
            reply_to: reply_to.unwrap_or_default(),
        })
        .await
        .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
    let lease = authorize(state, server, instance, generation)
        .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
    let outcome = ready
        .commit(lease)
        .await
        .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
    op.succeeded(if outcome.durable {
        "CHANNEL.SEND.PERSISTED"
    } else {
        "CHANNEL.SEND.PREPARED_PENDING"
    });
    Ok(response(outcome))
}

#[tauri::command]
pub(super) async fn durable_send_context(
    state: State<'_, AppState>,
    server: u64,
) -> Result<String, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let (actor, instance) = actor_instance_of(&state, server)
        .await
        .map_err(|failure| failure.message())?;
    let context = actor.durable_send_context().await?;
    let _session = require_ui_session_generation(&state, generation).await?;
    let registry = state.servers.lock().await;
    if registry
        .get(&server)
        .is_none_or(|entry| entry.instance != instance)
    {
        return Err("The conversation changed while preparing the message.".into());
    }
    Ok(hex::encode(context))
}

fn disposition(value: ChatDisposition) -> &'static str {
    match value {
        ChatDisposition::Committed => "committed",
        ChatDisposition::StoppedBeforeCommit => "stopped-before-commit",
        ChatDisposition::StoppedAfterCommit => "stopped-after-commit",
    }
}
fn response(outcome: DurableSendOutcome) -> SendMessageResult {
    SendMessageResult {
        accepted: outcome.durable && outcome.disposition == ChatDisposition::Committed,
        persistence: if outcome.durable {
            PersistOutcome::Durable
        } else {
            PersistOutcome::Pending {
                reason: PersistFailure::WriteFailed,
            }
        },
        message_id: Some(outcome.message_id),
        replayed: outcome.replayed,
        disposition: disposition(outcome.disposition),
    }
}
/// Mutations have no missing-token fallback: every production caller must first seal its
/// immutable request. Capturing Actor/incarnation before Ready fences replacement races.
#[allow(clippy::too_many_arguments)]
pub(super) async fn mutation(
    state: &AppState,
    op: &Operation,
    server: u64,
    channel: &str,
    id: String,
    action: DurableMutationAction,
    retry_token: String,
    expected_context: String,
    target: String,
) -> Result<SendMessageResult, AppError> {
    let generation = unlocked_ui_session_generation(state)
        .await
        .map_err(|e| op.fail(codes::SESSION_LOCKED, e))?;
    let channel = channel
        .parse()
        .map_err(|_| op.fail(codes::CHANNEL_BAD_ID, "bad channel id"))?;
    let (actor, instance) = actor_instance_of(state, server)
        .await
        .map_err(|failure| op.fail(failure.code(), failure.message()))?;
    let actor = op.bind_actor(actor);
    let request = DurableMutationRequest {
        token: fixed_hex(&retry_token).map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?,
        expected_context: fixed_hex(&expected_context)
            .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?,
        channel,
        message_id: id,
        target: fixed_hex(&target).map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?,
        action,
    };
    let ready = actor
        .prepare_durable_mutation(request)
        .await
        .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?;
    let lease = authorize(state, server, instance, generation)
        .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?;
    let outcome = ready
        .commit(lease)
        .await
        .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?;
    op.succeeded(if outcome.durable {
        "CHANNEL.MUTATION.PERSISTED"
    } else {
        "CHANNEL.MUTATION.PREPARED_PENDING"
    });
    Ok(response(outcome))
}
#[derive(serde::Serialize)]
pub(super) struct MutationContextResult {
    context: String,
    target: String,
}
#[tauri::command]
pub(super) async fn durable_mutation_context(
    state: State<'_, AppState>,
    server: u64,
    channel: String,
    id: String,
) -> Result<MutationContextResult, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let channel = channel.parse().map_err(|_| "bad channel id".to_string())?;
    let (actor, instance) = actor_instance_of(&state, server)
        .await
        .map_err(|failure| failure.message())?;
    let result = actor.durable_mutation_context(channel, id).await?;
    let _session = require_ui_session_generation(&state, generation).await?;
    let registry = state.servers.lock().await;
    if registry
        .get(&server)
        .is_none_or(|entry| entry.instance != instance)
    {
        return Err("The conversation changed while preparing the mutation.".into());
    }
    Ok(MutationContextResult {
        context: hex::encode(result.context),
        target: hex::encode(result.target),
    })
}
/// A closed, action-aware IPC request. Unknown tags/fields refuse rather than reinterpret a
/// recovered edit or deletion as a new post. Every field contributes to the original binding.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub(super) enum OriginalChatRequest {
    Send {
        token: String,
        expected_context: String,
        channel: String,
        text: String,
        reply_to: String,
    },
    Edit {
        token: String,
        expected_context: String,
        channel: String,
        message_id: String,
        target: String,
        text: String,
    },
    Delete {
        token: String,
        expected_context: String,
        channel: String,
        message_id: String,
        target: String,
    },
}
impl OriginalChatRequest {
    fn into_request(self) -> Result<DurableChatRequest, String> {
        let channel_id =
            |channel: String| channel.parse().map_err(|_| "bad channel id".to_string());
        let request = match self {
            Self::Send {
                token,
                expected_context,
                channel,
                text,
                reply_to,
            } => DurableChatRequest::Send(DurableSendRequest {
                token: fixed_hex(&token)?,
                expected_context: fixed_hex(&expected_context)?,
                channel: channel_id(channel)?,
                text,
                reply_to,
            }),
            Self::Edit {
                token,
                expected_context,
                channel,
                message_id,
                target,
                text,
            } => DurableChatRequest::Mutation(DurableMutationRequest {
                token: fixed_hex(&token)?,
                expected_context: fixed_hex(&expected_context)?,
                channel: channel_id(channel)?,
                message_id,
                target: fixed_hex(&target)?,
                action: DurableMutationAction::Edit { text },
            }),
            Self::Delete {
                token,
                expected_context,
                channel,
                message_id,
                target,
            } => DurableChatRequest::Mutation(DurableMutationRequest {
                token: fixed_hex(&token)?,
                expected_context: fixed_hex(&expected_context)?,
                channel: channel_id(channel)?,
                message_id,
                target: fixed_hex(&target)?,
                action: DurableMutationAction::Delete,
            }),
        };
        request.validate()?;
        Ok(request)
    }
}
#[tauri::command]
pub(super) async fn stop_durable_chat(
    state: State<'_, AppState>,
    server: u64,
    original: OriginalChatRequest,
) -> Result<SendMessageResult, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let (actor, instance) = actor_instance_of(&state, server)
        .await
        .map_err(|failure| failure.message())?;
    let ready = actor
        .prepare_durable_stop(DurableStopRequest {
            original: original.into_request()?,
        })
        .await?;
    let lease = authorize(&state, server, instance, generation)?;
    Ok(response(ready.commit(lease).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[tokio::test]
    async fn native_durable_send_failure_retry_restart_and_stale_session() {
        let root = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(root.path(), b"test", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let mut server = Server::found(
            Hub::new().join(PeerId::from_u64(911)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(911),
            Box::new(ManualClock::new(1000)),
            "alice",
        )
        .unwrap();
        server.open_channel(1).await.unwrap();
        let group_id = server.group_id();
        let device_id = server.device_id();
        let (actor, mut events, task) = spawn(server);
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        state.servers.lock().await.insert(
            12,
            ServerEntry {
                files: keep_session::EntryOwner::new(),
                actor: actor.clone(),
                instance: 40,
                group_id,
                device_id,
                invite: None,
                name: "test".into(),
                bootstrap: vec![],
                bootstrap_owners: HashMap::new(),
                interface_routes: None,
                rendezvous: vec![],
                mesh: None,
                is_dm: false,
                switchboard: false,
                record_seq: 0,
                voice: None,
                persist: PersistCounters::default(),
            },
        );
        let context = hex::encode(actor.durable_send_context().await.unwrap());
        let token = "11".repeat(16);
        let op = Operation::start(
            None,
            catcoms_diagnostics::Section::Channels,
            "send_message",
            12,
            Some("1"),
        );
        let blocked = root.path().join("servers").join("12.bin");
        std::fs::create_dir(&blocked).unwrap();
        let pending = send(
            &state,
            &op,
            12,
            "1",
            "one durable message".into(),
            None,
            Some(token.clone()),
            Some(context.clone()),
        )
        .await
        .unwrap();
        assert!(!pending.accepted);
        assert!(
            actor.messages(1).await.is_empty(),
            "failed save cannot appear in shared history"
        );
        assert_eq!(
            pending.persistence,
            PersistOutcome::Pending {
                reason: PersistFailure::WriteFailed
            }
        );
        std::fs::remove_dir(&blocked).unwrap();
        let accepted = send(
            &state,
            &op,
            12,
            "1",
            "one durable message".into(),
            None,
            Some(token.clone()),
            Some(context.clone()),
        )
        .await
        .unwrap();
        assert!(accepted.accepted && accepted.replayed);
        assert_eq!(accepted.message_id, pending.message_id);
        assert_eq!(actor.messages(1).await.len(), 1);
        // Cancellation at Ready must release actor custody without authoring anything.
        let ready = actor
            .prepare_durable_send(DurableSendRequest {
                token: [22; 16],
                expected_context: fixed_hex(&context).unwrap(),
                channel: 1,
                text: "stale request".into(),
                reply_to: String::new(),
            })
            .await
            .unwrap();
        let generation = state.ui_session_generation.fetch_add(1, Ordering::AcqRel);
        assert!(authorize(&state, 12, 40, generation).is_err());
        drop(ready);
        assert_eq!(actor.messages(1).await.len(), 1);
        // No clean shutdown or background persistence is available to rescue the accepted op.
        task.abort();
        let _ = task.await;
        drain.await.unwrap();
        state.servers.lock().await.clear();
        drop(state.store.lock().await.take());
        let store = ServerStore::open(root.path(), b"test", &mut OsCryptoRng).unwrap();
        let snapshot = store.load_server(12).unwrap();
        let restored = Server::restore(
            &snapshot,
            Hub::new().join(PeerId::from_u64(912)),
            ChaCha20Rng::seed_from_u64(912),
            Box::new(ManualClock::new(2000)),
            "alice",
        )
        .unwrap();
        assert_eq!(restored.messages(1).len(), 1);
        assert_eq!(
            Some(restored.messages(1)[0].id.clone()),
            accepted.message_id
        );
    }
}
