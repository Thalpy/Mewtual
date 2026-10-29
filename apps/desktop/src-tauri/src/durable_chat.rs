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
    positive: Option<&catcoms_app::durable_chat::DurableChatIdentity>,
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
    if let Some(identity) = positive {
        let store = store
            .as_ref()
            .ok_or_else(|| "vault is not mounted".to_string())?;
        if store
            .has_chat_stop_fence(server, identity)
            .map_err(|e| e.to_string())?
        {
            return Err("CHAT_SEND_STOPPED: this retired token is permanently fenced; repeat Stop to resolve its saved terminal".into());
        }
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
    let mut captured = None;
    let (token, expected_context) = match (retry_token, expected_context) {
        (Some(token), Some(context)) => (
            fixed_hex(&token).map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?,
            fixed_hex(&context).map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?,
        ),
        (None, None) => {
            let (actor, instance) = actor_instance_of(state, server)
                .await
                .map_err(|failure| op.fail(failure.code(), failure.message()))?;
            let mut token = [0; 16];
            OsCryptoRng.fill_bytes(&mut token);
            let context = actor
                .durable_send_context()
                .await
                .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
            captured = Some((actor, instance));
            (token, context)
        }
        _ => {
            return Err(op.fail(
                codes::CHAT_SEND_REJECTED,
                "A retry requires both its identity and original authoring context.",
            ))
        }
    };
    let request = DurableSendRequest {
        token,
        expected_context,
        channel,
        text,
        reply_to: reply_to.unwrap_or_default(),
    };
    request
        .validate()
        .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
    let original = DurableChatRequest::Send(request.clone());
    if let Some(terminal) = stop_request_at(state, server, original.clone(), generation, true)
        .await
        .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?
    {
        op.succeeded(if terminal.persistence == PersistOutcome::Durable {
            "CHANNEL.SEND.RETIRED_FENCE_PERSISTED"
        } else {
            "CHANNEL.SEND.RETIRED_FENCE_PENDING"
        });
        return Ok(terminal);
    }
    let (actor, instance) = match captured {
        Some(captured) => captured,
        None => actor_instance_of(state, server)
            .await
            .map_err(|failure| op.fail(failure.code(), failure.message()))?,
    };
    let actor = op.bind_actor(actor);
    let identity = original.retired_identity();
    let ready = actor
        .prepare_durable_send(request)
        .await
        .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
    let lease = match authorize(state, server, instance, generation, Some(&identity)) {
        Ok(lease) => lease,
        Err(error) if error.starts_with("CHAT_SEND_STOPPED:") => {
            // Release the original Ready and all failed final-lease custody before negative
            // recovery. The same request may only rewrite an already existing exact fence.
            drop(ready);
            return stop_request_at(state, server, original, generation, true)
                .await
                .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?
                .ok_or_else(|| op.fail(codes::CHAT_SEND_REJECTED, "retired fence disappeared"));
        }
        Err(error) => return Err(op.fail(codes::CHAT_SEND_REJECTED, error)),
    };
    let outcome = ready
        .commit(lease)
        .await
        .map_err(|e| op.fail(codes::CHAT_SEND_REJECTED, e))?;
    op.succeeded(if !outcome.durable {
        "CHANNEL.SEND.PREPARED_PENDING"
    } else if outcome.disposition != ChatDisposition::Committed {
        "CHANNEL.SEND.STOPPED"
    } else {
        "CHANNEL.SEND.PERSISTED"
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
    generation: u64,
) -> Result<SendMessageResult, AppError> {
    let channel = channel
        .parse()
        .map_err(|_| op.fail(codes::CHANNEL_BAD_ID, "bad channel id"))?;
    let request = DurableMutationRequest {
        token: fixed_hex(&retry_token).map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?,
        expected_context: fixed_hex(&expected_context)
            .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?,
        channel,
        message_id: id,
        target: fixed_hex(&target).map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?,
        action,
    };
    request
        .validate()
        .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?;
    let original = DurableChatRequest::Mutation(request.clone());
    if let Some(terminal) = stop_request_at(state, server, original.clone(), generation, true)
        .await
        .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?
    {
        return Ok(terminal);
    }
    let (actor, instance) = actor_instance_of(state, server)
        .await
        .map_err(|failure| op.fail(failure.code(), failure.message()))?;
    let actor = op.bind_actor(actor);
    let identity = original.retired_identity();
    let ready = actor
        .prepare_durable_mutation(request)
        .await
        .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?;
    let lease = match authorize(state, server, instance, generation, Some(&identity)) {
        Ok(lease) => lease,
        Err(error) if error.starts_with("CHAT_SEND_STOPPED:") => {
            drop(ready);
            return stop_request_at(state, server, original, generation, true)
                .await
                .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?
                .ok_or_else(|| op.fail(codes::CHAT_EDIT_REJECTED, "retired fence disappeared"));
        }
        Err(error) => return Err(op.fail(codes::CHAT_EDIT_REJECTED, error)),
    };
    let outcome = ready
        .commit(lease)
        .await
        .map_err(|e| op.fail(codes::CHAT_EDIT_REJECTED, e))?;
    op.succeeded(if !outcome.durable {
        "CHANNEL.MUTATION.PREPARED_PENDING"
    } else if outcome.disposition != ChatDisposition::Committed {
        "CHANNEL.MUTATION.STOPPED"
    } else {
        "CHANNEL.MUTATION.PERSISTED"
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
    stop_request_at(&state, server, original.into_request()?, generation, false)
        .await?
        .ok_or_else(|| "missing exact Stop result".to_string())
}
#[cfg(test)]
async fn stop_request(
    state: &AppState,
    server: u64,
    original: DurableChatRequest,
) -> Result<SendMessageResult, String> {
    let generation = unlocked_ui_session_generation(state).await?;
    stop_request_at(state, server, original, generation, false)
        .await?
        .ok_or_else(|| "missing exact Stop result".to_string())
}
async fn stop_request_at(
    state: &AppState,
    server: u64,
    original: DurableChatRequest,
    generation: u64,
    existing_only: bool,
) -> Result<Option<SendMessageResult>, String> {
    original.validate()?;
    // One constant serializer covers all orphan IDs. An arbitrary missing numeric ID cannot
    // create an unbounded persist_locks entry. Restores retain this same guard through register.
    let orphan = state.chat_orphan_commit.clone().lock_owned().await;
    let identity = original.retired_identity();
    let existing_fence = {
        let mounted = state.store.lock().await;
        mounted
            .as_ref()
            .ok_or_else(|| "vault is not mounted".to_string())?
            .has_chat_stop_fence(server, &identity)
            .map_err(|e| e.to_string())?
    };
    if existing_only && !existing_fence {
        return Ok(None);
    }
    let current = state
        .servers
        .lock()
        .await
        .get(&server)
        .map(|entry| (entry.actor.clone(), entry.instance));
    if let Some((actor, instance)) = current.filter(|_| !existing_fence) {
        drop(orphan);
        let ready = actor
            .prepare_durable_stop(DurableStopRequest { original })
            .await?;
        let lease = authorize(state, server, instance, generation, None)?;
        return Ok(Some(response(ready.commit(lease).await?)));
    }
    let _session = require_ui_session_generation(state, generation).await?;
    let guard = state.store.lock().await;
    let store = guard
        .as_ref()
        .ok_or_else(|| "vault is not mounted".to_string())?;
    let registry = state.servers.lock().await;
    if registry.contains_key(&server) && !existing_fence {
        return Err(
            "The conversation changed before its retired retry could be fenced; repeat Stop."
                .into(),
        );
    }
    // Decode/foreign bindings fail closed. Identical visible fences are rewritten, since a
    // previous post-rename parent-sync failure cannot acknowledge this durable terminal.
    store
        .has_chat_stop_fence(server, &identity)
        .map_err(|e| e.to_string())?;
    let durable = store
        .save_chat_stop_fence(server, &identity, &mut OsCryptoRng)
        .is_ok();
    Ok(Some(SendMessageResult {
        accepted: false,
        persistence: if durable {
            PersistOutcome::Durable
        } else {
            PersistOutcome::Pending {
                reason: PersistFailure::WriteFailed,
            }
        },
        message_id: Some(identity.message_id().to_string()),
        replayed: existing_fence,
        disposition: "stopped-retired",
    }))
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
        assert!(authorize(&state, 12, 40, generation, None).is_err());
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

    #[derive(Debug, Default)]
    struct PublicationProbe {
        chat: Arc<AtomicU64>,
    }
    #[async_trait::async_trait]
    impl MeshTransport for PublicationProbe {
        fn local_peer(&self) -> PeerId {
            PeerId::from_u64(1004)
        }
        async fn subscribe(&self, _: catcoms_rt::Topic) -> Result<(), catcoms_rt::TransportError> {
            Ok(())
        }
        async fn unsubscribe(
            &self,
            _: catcoms_rt::Topic,
        ) -> Result<(), catcoms_rt::TransportError> {
            Ok(())
        }
        async fn publish(
            &self,
            _: catcoms_rt::Topic,
            _: bytes::Bytes,
        ) -> Result<(), catcoms_rt::TransportError> {
            Ok(())
        }
        async fn publish_once(
            &self,
            _: catcoms_rt::Topic,
            bytes: bytes::Bytes,
        ) -> Result<catcoms_rt::PublishSubmission, catcoms_rt::PublishOnceError> {
            if catcoms_replication::SealedOp::decode(&bytes)
                .is_ok_and(|op| op.doc_type == catcoms_wire::DocType::Channel && op.doc_id == 5)
            {
                self.chat.fetch_add(1, Ordering::SeqCst);
            }
            Ok(catcoms_rt::PublishSubmission::Submitted)
        }
        async fn request(
            &self,
            _: PeerId,
            _: catcoms_rt::ProtocolId,
            _: bytes::Bytes,
        ) -> Result<bytes::Bytes, catcoms_rt::TransportError> {
            Err(catcoms_rt::TransportError::Closed)
        }
        async fn request_cancellable(
            &self,
            p: PeerId,
            k: catcoms_rt::ProtocolId,
            b: bytes::Bytes,
            _: catcoms_rt::RequestCancellation,
        ) -> Result<bytes::Bytes, catcoms_rt::TransportError> {
            self.request(p, k, b).await
        }
        async fn next_event(&self) -> Option<catcoms_rt::TransportEvent> {
            std::future::pending().await
        }
    }
    #[tokio::test]
    async fn native_retired_stop_fences_missing_actor_reopen_publication_and_same_id_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(dir.path(), b"test", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let (entry, task, drain) = leaving::tests::running(1001);
        let actor = entry.actor.clone();
        state.servers.lock().await.insert(12, entry);
        actor.open_channel(5).await;
        let context = actor.durable_send_context().await.unwrap();
        let request = DurableChatRequest::Send(DurableSendRequest {
            token: [193; 16],
            expected_context: context,
            channel: 5,
            text: "retained original".into(),
            reply_to: String::new(),
        });
        let op = Operation::start(
            None,
            catcoms_diagnostics::Section::Channels,
            "send_message",
            12,
            Some("5"),
        );
        assert!(
            send(
                &state,
                &op,
                12,
                "5",
                "retained original".into(),
                None,
                Some(hex::encode([193; 16])),
                Some(hex::encode(context))
            )
            .await
            .unwrap()
            .accepted
        );
        let foreign_scope = stop_request(&state, 999, request.clone()).await.unwrap();
        assert_eq!(foreign_scope.disposition, "stopped-retired");
        assert!(!state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .has_chat_stop_fence(12, &request.retired_identity())
            .unwrap());
        assert!(
            send(
                &state,
                &op,
                12,
                "5",
                "retained original".into(),
                None,
                Some(hex::encode([193; 16])),
                Some(hex::encode(context))
            )
            .await
            .unwrap()
            .accepted
        );
        // The actual stop handshake closes authoring/publication before registry retirement.
        let stopped = leaving::stop_server(&state, 12, &ManualClock::new(1_000))
            .await
            .unwrap()
            .unwrap();
        let old = stopped.remove(&mut *state.servers.lock().await).unwrap();
        drop(old);
        task.await.unwrap();
        drain.await.unwrap();
        let retained = state
            .store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_server(12)
            .unwrap()
            .to_vec();
        let terminal = stop_request(&state, 12, request.clone()).await.unwrap();
        assert_eq!(terminal.disposition, "stopped-retired");
        assert!(!terminal.accepted);
        assert_eq!(terminal.persistence, PersistOutcome::Durable);
        assert_eq!(
            state
                .store
                .lock()
                .await
                .as_ref()
                .unwrap()
                .load_server(12)
                .unwrap()
                .as_slice(),
            retained,
            "fence installation preserves retained old signed history"
        );
        assert!(state.persist_locks.lock().unwrap().len() <= 1);
        drop(state.store.lock().await.take());
        let reopened = AppState::default();
        *reopened.store.lock().await =
            Some(ServerStore::open(dir.path(), b"test", &mut OsCryptoRng).unwrap());
        *reopened.session_resumable.lock().await = true;
        let probe = PublicationProbe::default();
        let published = probe.chat.clone();
        let restored = restore_server_actor(
            &reopened,
            &retained,
            &ServerRecord {
                id: 12,
                display_name: "test".into(),
                invite: String::new(),
                is_dm: false,
            },
            probe,
            ChaCha20Rng::seed_from_u64(1004),
            Box::new(ManualClock::new(2_000)),
            &[],
            0,
            ReconnectPolicy::MemberMesh,
            vec![],
            false,
        )
        .await
        .unwrap();
        let restored_actor = restored.actor.clone();
        let mut events = restored.events;
        let events_task = tokio::spawn(async move { while events.recv().await.is_some() {} });
        assert_eq!(restored_actor.messages(5).await.len(), 1);
        for _ in 0..4 {
            let _ = restored_actor.snapshot().await.unwrap();
            tokio::task::yield_now().await;
        }
        assert_eq!(
            published.load(Ordering::SeqCst),
            0,
            "retired saved publication is stopped before Actor spawn"
        );
        restored_actor.stop_and_wait().await.unwrap();
        restored.task.await.unwrap();
        events_task.await.unwrap();
        drop(restored.chat_fence_commit);
        let (replacement, task, drain) = leaving::tests::running(1005);
        let replacement_actor = replacement.actor.clone();
        reopened.servers.lock().await.insert(12, replacement);
        replacement_actor.open_channel(5).await;
        let late = send(
            &reopened,
            &op,
            12,
            "5",
            "retained original".into(),
            None,
            Some(hex::encode([193; 16])),
            Some(hex::encode(context)),
        )
        .await
        .unwrap();
        assert!(!late.accepted);
        assert_eq!(late.disposition, "stopped-retired");
        assert_eq!(late.persistence, PersistOutcome::Durable);
        assert!(replacement_actor.messages(5).await.is_empty());
        let terminal = stop_request(&reopened, 12, request).await.unwrap();
        assert_eq!(terminal.disposition, "stopped-retired");
        assert!(terminal.replayed);
        assert_eq!(terminal.persistence, PersistOutcome::Durable);
        replacement_actor.stop_and_wait().await.unwrap();
        task.await.unwrap();
        drain.await.unwrap();
    }
    #[tokio::test]
    async fn pre_registration_chat_owner_retains_guard_until_actual_aborted_actor_completion() {
        use futures::FutureExt;
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::default();
        *state.store.lock().await =
            Some(ServerStore::open(dir.path(), b"test", &mut OsCryptoRng).unwrap());
        *state.session_resumable.lock().await = true;
        let guard = state.chat_orphan_commit.clone().lock_owned().await;
        let custody = Arc::new(std::sync::Mutex::new(Some(guard)));
        let server = Server::found(
            Hub::new().join(PeerId::from_u64(1006)),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1006),
            Box::new(ManualClock::new(1_000)),
            "test",
        )
        .unwrap();
        let (actor, mut events, task) =
            catcoms_app::spawn_with_lifetime(server, Box::new(custody.clone()));
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let owner = PreRegistrationChatOwner {
            custody,
            abort: Some(task.abort_handle()),
        };
        actor.open_channel(5).await;
        let request = DurableSendRequest {
            token: [194; 16],
            expected_context: actor.durable_send_context().await.unwrap(),
            channel: 5,
            text: "must not escape canceled restore".into(),
            reply_to: String::new(),
        };
        // A real Ready parks the actual Actor before its original storage lease. Dropping the
        // pre-registration owner aborts that task, but its lifetime still owns the absence guard.
        let ready = actor.prepare_durable_send(request.clone()).await.unwrap();
        let mut stopped = Box::pin(stop_request(&state, 12, DurableChatRequest::Send(request)));
        assert!(stopped.as_mut().now_or_never().is_none());
        drop(owner);
        assert!(state.chat_orphan_commit.try_lock().is_err());
        assert!(task.await.unwrap_err().is_cancelled());
        drop(ready);
        drain.await.unwrap();
        let result = stopped.await.unwrap();
        assert_eq!(result.persistence, PersistOutcome::Durable);
        assert_eq!(result.disposition, "stopped-retired");
        assert!(actor.messages(5).await.is_empty());
    }
}
