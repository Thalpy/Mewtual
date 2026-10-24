//! Actual Node/WebCrypto endpoint fixture. Only bounded public metadata uses the process
//! control pipe. Input/Result/key material uses the actual authenticated HTTP service.
use super::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::future::Future;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const FRAME_BYTES: usize = 4096;
pub(crate) type Failure = Box<dyn std::any::Any + Send>;

/// Test-only panic collection, so a failed native assertion still goes through explicit
/// subprocess/service collection. This neither aborts nor refunds an application task.
pub(crate) async fn catch_fixture<F: Future>(future: F) -> Result<F::Output, Failure> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(std::task::Poll::Ready(value)) => std::task::Poll::Ready(Ok(value)),
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(error) => std::task::Poll::Ready(Err(error)),
        }
    })
    .await
}

#[derive(Deserialize)]
#[serde(tag = "tag", rename_all = "camelCase", deny_unknown_fields)]
enum Frame {
    Call { method: Method, args: Metadata },
    Done { value: Value },
    Failed { reason: String },
    Blocked,
}
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum Method {
    AdmitKey,
    PublishKey,
    SettleKey,
    Setup,
    SettleSetup,
    CollectChannel,
    ReadWork,
    ObserveTerminal,
}
#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Metadata {
    owner: String,
    executor: String,
    permit: Option<String>,
    point: Option<String>,
    #[serde(rename = "fromSlot")]
    from_slot: Option<u8>,
}

struct Callbacks {
    publication: Option<Metadata>,
    publish_calls: usize,
    collect_calls: usize,
    refuse_publish: bool,
    refuse_collect: bool,
}
impl Default for Callbacks {
    fn default() -> Self {
        Self {
            publication: None,
            publish_calls: 0,
            collect_calls: 0,
            refuse_publish: true,
            refuse_collect: true,
        }
    }
}

pub(crate) struct Endpoint {
    state: Arc<AppState>,
    owner: String,
    child: Child,
    input: Option<ChildStdin>,
    output: Option<ChildStdout>,
    stderr: Option<tokio::task::JoinHandle<Vec<u8>>>,
    server: Option<service::RunningServer>,
    port: u16,
    // Resources are owned before the first callback, but a selected grant exists only
    // after actual publication. None is preparation state, never a synthetic admission.
    selected: Option<Arc<SelectedHandoffOwner>>,
    callbacks: Callbacks,
}
impl Endpoint {
    /// Complete the actual selected-key publication and channel Status before starting a
    /// protected pair. A slow process launch/setup cannot consume the original pair deadline.
    pub(crate) async fn prepare(state: Arc<AppState>, owner: String) -> Result<Self, Failure> {
        let runtime = crate::voice_runtime(&state);
        let bound = match service::BoundServer::bind_application(
            state.voice_channels.clone(),
            runtime,
            service::Origin::PackagedHttp,
            context::Context::new(&state),
        ) {
            Ok(bound) => bound,
            Err(error) => {
                return Err(preparation_failed(
                    &state,
                    &owner,
                    None,
                    format!("effect service bind refused: {error:?}"),
                )
                .await)
            }
        };
        let port = bound.port();
        let server = match bound.start() {
            Ok(server) => server,
            Err(error) => {
                return Err(preparation_failed(
                    &state,
                    &owner,
                    None,
                    format!("complete effect service start/layout refused: {error:?}"),
                )
                .await)
            }
        };
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/voice-native-interop.ts");
        let mut command = Command::new("node");
        command
            .arg("--experimental-strip-types")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: this is a hidden test helper.
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return Err(preparation_failed(
                    &state,
                    &owner,
                    Some(server),
                    format!("Node is required for actual WebCrypto acceptance: {error}"),
                )
                .await);
            }
        };
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let mut errors = child.stderr.take().unwrap();
        let stderr = tokio::spawn(async move {
            let mut retained = Vec::new();
            let mut scratch = [0; 1024];
            loop {
                let count = errors.read(&mut scratch).await.unwrap();
                if count == 0 {
                    return retained;
                }
                assert!(
                    retained.len() + count <= 16 * 1024,
                    "endpoint stderr exceeded fixture bound"
                );
                retained.extend_from_slice(&scratch[..count]);
            }
        });
        let mut endpoint = Self {
            state,
            owner,
            child,
            input: Some(input),
            output: Some(output),
            stderr: Some(stderr),
            server: Some(server),
            port,
            selected: None,
            callbacks: Callbacks::default(),
        };
        let prepared = catch_fixture(async {
        let group = endpoint.state.voice_authority.lock().unwrap().owner(&endpoint.owner).unwrap().binding.scope.group;
        let (ready, executor) = drive_streams(
            &endpoint.state,
            &endpoint.owner,
            port,
            endpoint.input.as_mut().unwrap(),
            endpoint.output.as_mut().unwrap(),
            &mut endpoint.callbacks,
            json!({
                "tag": "step", "name": "prepare", "owner": endpoint.owner, "group": hex::encode(group),
            }),
        )
        .await;
        assert_eq!(ready, json!({"kind": "ready"}));
        endpoint.state
            .voice_authority
            .lock()
            .unwrap()
            .selected_handoff_owner(&endpoint.owner, executor.expect("actual selected-key publication"))
            .unwrap()
        }).await;
        match prepared {
            Ok(selected) => {
                endpoint.selected = Some(selected);
                Ok(endpoint)
            }
            Err(error) => {
                let _ = endpoint.collect().await;
                Err(error)
            }
        }
    }

    pub(crate) fn selected(&self) -> Arc<SelectedHandoffOwner> {
        self.selected
            .as_ref()
            .expect("only a completed native publication supplies a selected owner")
            .clone()
    }

    async fn step(&mut self, value: Value) -> Value {
        // Borrow the actual process streams without cloning or creating another reader.
        drive_streams(
            &self.state,
            &self.owner,
            self.port,
            self.input.as_mut().unwrap(),
            self.output.as_mut().unwrap(),
            &mut self.callbacks,
            value,
        )
        .await
        .0
    }
    /// No public row/purpose/role is supplied. Keep servicing the one metadata callback
    /// until actual native history independently establishes all four completed actions.
    pub(crate) async fn automatic_confirmation(&mut self) {
        let answer = self.step(json!({"tag":"step", "name":"pump"})).await;
        assert_eq!(answer, json!({"kind":"observed"}));
        assert!(automatic_terminal(&self.state, &self.selected()));
    }
    pub(crate) async fn derive(&mut self, selector: PairSelector) -> ledger::ResolvedRow {
        let selected = self.selected();
        let bootstrap::Status::Ready(public) = bootstrap::ensure(
            &self.state,
            selected.id(),
            selected.executor(),
            &mut catcoms_rt::OsCryptoRng,
        )
        .await
        .unwrap() else {
            panic!("original browser channel")
        };
        let route = self
            .state
            .voice_channels
            .route(public.public().slot)
            .unwrap();
        let row = self.wait_native_derivation(selector).await;
        // The fixture can only make the next authenticated Input fail locally; it cannot
        // synthesize a receipt or renew this original parent/action's authority.
        route.session().fixture_arm_input_pressure();
        let answer = self
            .step(json!({"tag":"step", "name":"derive",
                "purpose": {"peer":selector.peer.to_string(), "join":hex::encode(selector.join),
                    "keyRef":hex::encode(selector.key_ref), "epoch":selector.epoch.to_string()},
            }))
            .await;
        assert_eq!(answer, json!({"kind":"accepted"}));
        assert!(!route.session().fixture_input_pressure_pending());
        assert!(matches!(
            row.status().unwrap().engine.receipt.unwrap().outcome,
            catcoms_sync::voice_v2::crypto_actions::CryptoOutcome::Derived
        ));
        row
    }

    /// Observe original automatic work without delivering a descriptor/Input to Node.
    /// Used by the real-HTTP End-before-first-browser-work regression.
    pub(crate) async fn wait_native_derivation(
        &self,
        selector: PairSelector,
    ) -> ledger::ResolvedRow {
        let selected = self.selected();
        let clock = crate::voice_runtime(&self.state).clock();
        // No fixture factory call: the ordinary application service must independently
        // discover this proven counterpart and attach the original native Derive. The
        // browser then reads its descriptor through the actual metadata IPC handler.
        let row = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(row) = self
                    .state
                    .voice_handoff
                    .lookup_original(&selected, ledger::Purpose::Derive(selector))
                {
                    if row.status().unwrap().action.is_some() {
                        break row;
                    }
                }
                clock.sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("automatic native producer did not attach the original Derive");
        assert!(row.status().unwrap().action.is_some());
        assert!(
            row.status().unwrap().engine.receipt.is_none(),
            "genuine uncommitted native Derive"
        );
        row
    }
    pub(crate) async fn start(&mut self, parent: &ledger::ResolvedRow) -> ledger::RowId {
        self.child(parent, ledger::ConfirmationPurpose::Start(None))
            .await
    }
    pub(crate) async fn incoming(
        &mut self,
        parent: &ledger::ResolvedRow,
        role: u8,
    ) -> ledger::RowId {
        self.child(parent, ledger::ConfirmationPurpose::Incoming(role))
            .await
    }
    pub(crate) async fn respond(
        &mut self,
        parent: &ledger::ResolvedRow,
        trigger: ledger::RowId,
    ) -> ledger::RowId {
        self.child(parent, ledger::ConfirmationPurpose::Respond(trigger))
            .await
    }
    async fn child(
        &mut self,
        parent: &ledger::ResolvedRow,
        request: ledger::ConfirmationPurpose,
    ) -> ledger::RowId {
        let selected = self.selected();
        // No fixture or browser factory call may make this attachment. The actual normal
        // service must independently choose Start/Respond or the authenticated incoming
        // action, under the original parent and real native worker/shutdown position.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(row) = self
                    .state
                    .voice_handoff
                    .lookup_child(&selected, parent, request, None)
                {
                    if row.status().unwrap().action.is_some() {
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("automatic native confirmation attachment exceeded its bound");
        let metadata = match request {
            ledger::ConfirmationPurpose::Start(predecessor) => {
                json!({"kind":"start", "predecessor":predecessor.map(hex::encode)})
            }
            ledger::ConfirmationPurpose::Respond(trigger) => {
                json!({"kind":"respond", "triggerRow":hex::encode(trigger.logical())})
            }
            ledger::ConfirmationPurpose::Incoming(role) => json!({"kind":"incoming", "role":role}),
        };
        let answer = self
            .step(json!({"tag":"step", "name":"child", "request":metadata}))
            .await;
        assert_eq!(answer["kind"], "accepted");
        let id = ledger::RowId::from_logical(
            hex::decode(answer["row"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        let row = self
            .state
            .voice_handoff
            .resolve(
                &self.selected(),
                ledger::Purpose::Confirmation {
                    parent: parent.id(),
                    request,
                },
                id,
            )
            .unwrap();
        let status = row.status().unwrap();
        let receipt = status
            .engine
            .receipt
            .expect("actual native terminal result, not browser acceptance alone");
        match request {
            ledger::ConfirmationPurpose::Incoming(_) => assert_eq!(
                receipt.outcome,
                catcoms_sync::voice_v2::crypto_actions::CryptoOutcome::Opened
            ),
            _ => assert!(matches!(
                receipt.outcome,
                catcoms_sync::voice_v2::crypto_actions::CryptoOutcome::Sealed { .. }
            )),
        }
        assert!(
            status.acknowledged && status.drained && status.engine.settled,
            "actual native ACK/settlement/Return must precede next role"
        );
        id
    }
    pub(crate) async fn close(mut self) -> Result<(), Failure> {
        let closed =
            catch_fixture(async {
                crate::end_voice_owner_inner(&self.state, &self.owner).await;
                assert_eq!(
                    self.step(json!({"tag":"step", "name":"close"})).await,
                    json!({"kind":"closed", "bytes":0})
                );
                assert!(self.callbacks.publish_calls >= 2 && !self.callbacks.refuse_publish,
            "one deliberately refused publication must succeed on the same original metadata");
                assert!(
                    self.callbacks.collect_calls >= 2 && !self.callbacks.refuse_collect,
                    "one deliberately refused collection must succeed on the same original channel"
                );
            })
            .await;
        let success = self.collect().await;
        if closed.is_ok() && !success {
            return Err(Box::new(
                "Node exited unsuccessfully after reported collection".to_string(),
            ));
        }
        closed
    }

    /// Failure collection deliberately does not claim browser final ACK or native ledger
    /// drainage. It revokes this original native owner, then observes actual process/IO
    /// retirement before returning a test failure. Original application history is retained.
    pub(crate) async fn collect(&mut self) -> bool {
        let ended = catch_fixture(tokio::time::timeout(
            std::time::Duration::from_secs(10),
            crate::end_voice_owner_inner(&self.state, &self.owner),
        ))
        .await;
        drop(self.input.take());
        drop(self.output.take());
        let (exited, success) = match tokio::time::timeout(
            std::time::Duration::from_secs(1),
            self.child.wait(),
        )
        .await
        {
            Ok(Ok(status)) => (true, status.success()),
            Ok(Err(_)) => (false, false),
            Err(_) => {
                let _ = self.child.start_kill();
                (
                    matches!(
                        tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait())
                            .await,
                        Ok(Ok(_))
                    ),
                    false,
                )
            }
        };
        let stderr = if let Some(mut errors) = self.stderr.take() {
            match tokio::time::timeout(std::time::Duration::from_secs(5), &mut errors).await {
                Ok(Ok(bytes)) => {
                    if !success && !bytes.is_empty() {
                        eprintln!("bounded Node stderr: {}", String::from_utf8_lossy(&bytes));
                    }
                    true
                }
                Ok(Err(_)) => false,
                Err(_) => {
                    errors.abort();
                    let _ = errors.await;
                    false
                }
            }
        } else {
            true
        };
        let served = if let Some(server) = self.server.take() {
            matches!(tokio::time::timeout(std::time::Duration::from_secs(5), server.finish()).await, Ok(Ok(completion)) if !completion.io_failed)
        } else {
            true
        };
        assert!(matches!(ended, Ok(Ok(()))) && exited && stderr && served,
            "fixture collection failed: native End {}, child {exited}, stderr {stderr}, service {served}", matches!(ended, Ok(Ok(()))));
        success
    }
}

/// Failure before the full Endpoint exists still owns an activated native join. End that
/// exact owner and collect any actual service before exposing the failed preparation result.
async fn preparation_failed(
    state: &AppState,
    owner: &str,
    server: Option<service::RunningServer>,
    reason: String,
) -> Failure {
    let ended = catch_fixture(tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::end_voice_owner_inner(state, owner),
    ))
    .await;
    let served = match server {
        Some(server) => {
            matches!(tokio::time::timeout(std::time::Duration::from_secs(5), server.finish()).await, Ok(Ok(completion)) if !completion.io_failed)
        }
        None => true,
    };
    assert!(
        matches!(ended, Ok(Ok(()))) && served,
        "preparation failure must collect original native owner/service: {reason}"
    );
    Box::new(reason)
}
async fn write(input: &mut ChildStdin, value: Value) {
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(
        bytes.len() <= FRAME_BYTES,
        "only bounded public metadata belongs on the control pipe"
    );
    input
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    input.write_all(&bytes).await.unwrap();
    input.flush().await.unwrap();
}
async fn drive_streams(
    state: &AppState,
    owner: &str,
    port: u16,
    input: &mut ChildStdin,
    output: &mut ChildStdout,
    callbacks: &mut Callbacks,
    step: Value,
) -> (Value, Option<[u8; 32]>) {
    // This bounds a broken subprocess/harness, never a protocol deadline. Cancelling this
    // waiter drops its original pipe/process owners; admitted native work keeps its own
    // custody. No timeout creates a replacement key, action, Phase or channel.
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        drive_streams_inner(state, owner, port, input, output, callbacks, step),
    )
    .await
    .expect("native/WebCrypto control step exceeded its harness bound")
}
async fn drive_streams_inner(
    state: &AppState,
    owner: &str,
    port: u16,
    input: &mut ChildStdin,
    output: &mut ChildStdout,
    callbacks: &mut Callbacks,
    step: Value,
) -> (Value, Option<[u8; 32]>) {
    let automatic = step["name"] == "pump";
    write(input, step).await;
    let mut published_executor = None;
    // One callback owner spans the automatic step, rather than returning while a
    // background Node readWork waits forever. The bound follows the pump's minimum50ms
    // cadence, maximum32-slot scan and unchanged30s outer harness bound. Ordinary steps
    // retain32. No budget, timeout or protocol attempt is renewed by this fixture.
    let callbacks_bound = if automatic { 32 * (30_000 / 50) } else { 32 };
    for _ in 0..callbacks_bound {
        let length = output
            .read_u32()
            .await
            .expect("endpoint pipe closed before step completion") as usize;
        assert!(
            (1..=FRAME_BYTES).contains(&length),
            "endpoint exceeded fixed control frame"
        );
        let mut bytes = [0; FRAME_BYTES];
        output.read_exact(&mut bytes[..length]).await.unwrap();
        match serde_json::from_slice::<Frame>(&bytes[..length]).expect("closed metadata protocol") {
            Frame::Done { value } => return (value, published_executor),
            Frame::Failed { reason } => panic!("actual native/WebCrypto step failed: {reason}"),
            Frame::Blocked => panic!("unexpected blocked-step acknowledgement"),
            Frame::Call { method, args } => {
                let stop_pump = automatic
                    && matches!(method, Method::ObserveTerminal)
                    && hex::decode(&args.executor)
                        .ok()
                        .and_then(|bytes| bytes.try_into().ok())
                        .and_then(|executor| {
                            state
                                .voice_authority
                                .lock()
                                .unwrap()
                                .selected_handoff_owner(owner, executor)
                                .ok()
                        })
                        .is_some_and(|selected| automatic_terminal(state, &selected));
                let refuse = match method {
                    Method::PublishKey => {
                        callbacks.publish_calls += 1;
                        if let Some(original) = &callbacks.publication {
                            assert!(
                                original == &args,
                                "publication retry changed original owner/key/executor/permit"
                            );
                        } else {
                            callbacks.publication = Some(args.clone());
                        }
                        std::mem::replace(&mut callbacks.refuse_publish, false)
                    }
                    Method::CollectChannel => {
                        callbacks.collect_calls += 1;
                        let original = callbacks
                            .publication
                            .as_ref()
                            .expect("original selected publication");
                        assert!(
                            original.owner == args.owner && original.executor == args.executor,
                            "collection retry changed original selected channel identity"
                        );
                        std::mem::replace(&mut callbacks.refuse_collect, false)
                    }
                    _ => false,
                };
                let reply = if refuse {
                    Err(())
                } else {
                    metadata(state, owner, port, method, args).await
                };
                if let Ok(reply) = &reply {
                    if let Some(executor) = reply.executor {
                        published_executor = Some(executor);
                    }
                }
                // The actual public projection/payment survives serialization AND actual
                // control-pipe write, rather than being dropped at reply construction.
                let envelope = match &reply {
                    Ok(reply) => {
                        json!({"tag":"reply", "ok":true, "value":reply.value, "stopPump":stop_pump})
                    }
                    // Retryable native pressure/closure is a real adapter rejection. Let
                    // the SAME retained broker decide recovery; never panic or dump input.
                    Err(()) => json!({"tag":"reply", "ok":false}),
                };
                write(input, envelope).await;
                drop(reply);
            }
        }
    }
    panic!("endpoint exceeded its bounded callback count");
}

/// Independent original native history, not the pump's public metadata or JS progress.
/// Both endpoints must retain exactly one Derived parent and three original confirmation
/// actions, with actual terminal receipt, exact ACK, settlement and ordinary Return.
fn automatic_terminal(state: &AppState, selected: &SelectedHandoffOwner) -> bool {
    use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
    let mut from = 0;
    let mut parent = None;
    let mut children = 0;
    while from < 32 {
        let projection = state.voice_handoff.project_work(selected, from).unwrap();
        from = projection.next_slot;
        let Some(metadata) = projection.metadata.as_ref() else {
            continue;
        };
        let row = state
            .voice_handoff
            .resolve(selected, metadata.purpose, metadata.row)
            .unwrap();
        let status = row.status().unwrap();
        if !status.acknowledged || !status.drained || !status.engine.settled {
            return false;
        }
        let Some(receipt) = &status.engine.receipt else {
            return false;
        };
        match metadata.purpose {
            ledger::Purpose::Derive(_) => {
                if parent.replace(metadata.row).is_some()
                    || receipt.outcome != CryptoOutcome::Derived
                    || !row.has_transition_claim()
                    || !row.has_established_key_claim()
                {
                    return false;
                }
            }
            ledger::Purpose::Confirmation {
                parent: original,
                request,
            } => {
                if parent != Some(original) {
                    return false;
                }
                let valid = match request {
                    ledger::ConfirmationPurpose::Incoming(_) => {
                        receipt.outcome == CryptoOutcome::Opened
                    }
                    _ => matches!(receipt.outcome, CryptoOutcome::Sealed { .. }),
                };
                if !valid {
                    return false;
                }
                children += 1;
            }
        }
    }
    parent.is_some() && children == 3
}

struct MetadataReply {
    value: Value,
    executor: Option<[u8; 32]>,
    _projection: Option<commands::SetupReply>,
    _work_projection: Option<commands::WorkReply>,
}
async fn metadata(
    state: &AppState,
    owner: &str,
    port: u16,
    method: Method,
    args: Metadata,
) -> Result<MetadataReply, ()> {
    if args.owner != owner {
        return Err(());
    }
    if !matches!(method, Method::PublishKey) && (args.permit.is_some() || args.point.is_some()) {
        return Err(());
    }
    if !matches!(method, Method::ReadWork) && args.from_slot.is_some() {
        return Err(());
    }
    let executor = hex::decode(&args.executor)
        .map_err(|_| ())?
        .try_into()
        .map_err(|_| ())?;
    let mut projection = None;
    let mut work_projection = None;
    let mut published = None;
    let value = match method {
        Method::AdmitKey => serde_json::to_value(
            crate::voice_join_key::admit_inner(state, owner, &args.executor)
                .await
                .map_err(|_| ())?,
        )
        .map_err(|_| ())?,
        Method::PublishKey => {
            let point = hex::decode(args.point.as_deref().ok_or(())?)
                .map_err(|_| ())?
                .try_into()
                .map_err(|_| ())?;
            let value = crate::voice_join_key::publish_inner(
                state,
                owner,
                &args.executor,
                args.permit.as_deref().ok_or(())?,
                point,
            )
            .await
            .map_err(|_| ())?;
            published = Some(executor);
            serde_json::to_value(value).map_err(|_| ())?
        }
        Method::SettleKey => {
            crate::voice_join_key::settle_inner(state, owner, executor)
                .await
                .map_err(|_| ())?;
            Value::Null
        }
        Method::Setup => {
            let status = bootstrap::ensure(state, owner, executor, &mut catcoms_rt::OsCryptoRng)
                .await
                .map_err(|_| ())?;
            let metadata = state
                .voice_channels
                .project(owner, executor)
                .map_err(|_| ())?;
            let reply =
                commands::SetupReply::new(status, owner.to_string(), args.executor, port, metadata)
                    .map_err(|_| ())?;
            let value = serde_json::from_str(&reply.json().map_err(|_| ())?).map_err(|_| ())?;
            projection = Some(reply);
            value
        }
        Method::SettleSetup => {
            commands::settle_inner(state, owner, &args.executor)
                .await
                .map_err(|_| ())?;
            Value::Null
        }
        Method::ReadWork => {
            let reply = commands::read_work_inner(
                state,
                owner.to_string(),
                args.executor,
                args.from_slot.ok_or(())?,
            )
            .map_err(|_| ())?;
            let value =
                serde_json::from_str(&reply.fixture_json().map_err(|_| ())?).map_err(|_| ())?;
            work_projection = Some(reply);
            value
        }
        Method::CollectChannel => {
            commands::collect_inner(state, owner, &args.executor)
                .await
                .map_err(|_| ())?;
            Value::Null
        }
        // Test-only bounded observation. Its exact selected owner is validated above and
        // the caller independently inspects actual native histories + original Claim.
        // It creates no work, grant, token, key, adoption or SDK/RTC authority.
        Method::ObserveTerminal => Value::Null,
    };
    Ok(MetadataReply {
        value,
        executor: published,
        _projection: projection,
        _work_projection: work_projection,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wedged_node_step_is_bounded_and_actual_child_stderr_and_service_are_collected() {
    let mut fixture = crate::voice_activation_tests::Fixture::new().await;
    let owner = fixture.candidate().await;
    crate::activate_voice_owner_inner(&fixture.state, owner.clone())
        .await
        .unwrap();
    // Move the exact existing state into shared fixture ownership. All native Arc/grant/
    // registry allocations remain the originals; Fixture::close uses its Actor handles.
    let state = Arc::new(std::mem::take(&mut fixture.state));
    let mut endpoint = match Endpoint::prepare(state, owner).await {
        Ok(endpoint) => endpoint,
        Err(error) => std::panic::resume_unwind(error),
    };
    let selected = endpoint.selected();
    let port = endpoint.port;
    let wedged = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        catch_fixture(async {
            write(
                endpoint.input.as_mut().unwrap(),
                json!({"tag":"step", "name":"wedge"}),
            )
            .await;
            let output = endpoint.output.as_mut().unwrap();
            let length = output.read_u32().await.unwrap() as usize;
            assert!((1..=FRAME_BYTES).contains(&length));
            let mut bytes = [0; FRAME_BYTES];
            output.read_exact(&mut bytes[..length]).await.unwrap();
            assert!(matches!(
                serde_json::from_slice::<Frame>(&bytes[..length]).unwrap(),
                Frame::Blocked
            ));
            // The child acknowledges its blocked state before this timer starts. It never
            // completes the step; this bounds external fixture IO, not protocol time.
            tokio::time::timeout(std::time::Duration::from_millis(100), output.read_u32()).await
        }),
    )
    .await;
    let success = endpoint.collect().await;
    assert!(matches!(wedged, Ok(Ok(Err(_)))) && !success,
        "only the acknowledged wedged step's timeout is success; EOF/read errors or pre-ACK timeout are failures");
    assert!(
        endpoint.child.try_wait().unwrap().is_some(),
        "the actual child was reaped"
    );
    assert!(
        endpoint.input.is_none()
            && endpoint.output.is_none()
            && endpoint.stderr.is_none()
            && endpoint.server.is_none()
    );
    assert!(
        selected.grant().is_err(),
        "failure collection Ends the original native owner"
    );
    assert!(
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .is_err(),
        "the actual original service listener is closed"
    );
    drop((selected, endpoint));
    fixture.close().await;
}

#[tokio::test]
async fn bind_capacity_refusal_ends_the_activated_original_owner_before_returning() {
    for warm_registry in [false, true] {
        let mut fixture = crate::voice_activation_tests::Fixture::new().await;
        let owner = fixture.candidate().await;
        crate::activate_voice_owner_inner(&fixture.state, owner.clone())
            .await
            .unwrap();
        let lease = fixture
            .state
            .voice_authority
            .lock()
            .unwrap()
            .owner(&owner)
            .unwrap()
            .lease;
        let resources = crate::voice_runtime(&fixture.state).resources();
        // Production setup binds/pays the empty registry before call activation. Otherwise
        // full pressure refuses that earlier allocation (mapped to Binding), and never
        // exercises the intended fixed service-payment refusal checked below.
        if warm_registry {
            fixture.state.voice_channels.bind_pool(&resources).unwrap();
        }
        let bytes = catcoms_sync::voice_v2::resources::PROCESS_BYTES - resources.counts().0;
        let pressure = resources.charge_process_retained(bytes).unwrap();
        let state = Arc::new(std::mem::take(&mut fixture.state));
        let failed = Endpoint::prepare(state, owner).await;
        let error = match failed {
            Ok(mut endpoint) => {
                endpoint.collect().await;
                panic!("full process pool must refuse before listener/process creation")
            }
            Err(error) => error,
        };
        let stage = if warm_registry { "Capacity" } else { "Binding" };
        assert!(
            error
                .downcast_ref::<String>()
                .is_some_and(|reason| reason == &format!("effect service bind refused: {stage}")),
            "must exercise the exact constructor allocation refusal, not a later script failure"
        );
        assert!(
            !lease.is_live(),
            "construction refusal Ends the same activated owner"
        );
        drop((pressure, lease));
        fixture.close().await;
    }
}
