//! Actual facade -> native commands -> WebCrypto/private HTTP bootstrap. There is no
//! remote peer or controlled RTC in this case: channel Ready is deliberately NOT audio.
use super::*;
use catcoms_rt::{Clock, SystemClock};
use serde::Deserialize;
use serde_json::{json, Value};
use std::future::Future;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
#[serde(
    tag = "tag",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Frame {
    Invoke {
        request: u16,
        command: Command,
        args: Value,
    },
    Ready,
    Done,
    Failed {
        phase: Phase,
        last_command: Option<Command>,
    },
}
#[derive(Deserialize, Clone, Copy, Debug)]
enum Command {
    #[serde(rename = "prepare_voice_controller")]
    PrepareController,
    #[serde(rename = "prepare_voice_owner")]
    PrepareOwner,
    #[serde(rename = "activate_voice_owner")]
    Activate,
    #[serde(rename = "end_voice_owner")]
    End,
    #[serde(rename = "admit_voice_join_key")]
    AdmitKey,
    #[serde(rename = "publish_voice_join_key")]
    PublishKey,
    #[serde(rename = "settle_voice_join_key")]
    SettleKey,
    #[serde(rename = "setup_voice_channel")]
    Setup,
    #[serde(rename = "settle_voice_channel_setup")]
    SettleSetup,
    #[serde(rename = "collect_voice_channel")]
    CollectChannel,
    #[serde(rename = "read_voice_channel_work")]
    Work,
}
#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Prepared,
    Starting,
    Uncertain,
    Ready,
    Closing,
    Closed,
}
#[derive(Default)]
struct Observations {
    controller: Option<String>,
    owner: Option<String>,
    executor: Option<String>,
    original: Option<(
        catcoms_sync::voice_v2::Nonce,
        catcoms_sync::voice_v2::CallLease,
    )>,
    activations: usize,
    publications: usize,
    work: usize,
    collected: usize,
    ready: bool,
}
struct Reply {
    value: Value,
    // Retain the real bounded serialization/projection owner through the actual pipe write.
    _setup: Option<commands::SetupReply>,
    _work: Option<commands::WorkReply>,
}
impl Reply {
    fn metadata(value: Value) -> Self {
        Self {
            value,
            _setup: None,
            _work: None,
        }
    }
}
fn text(args: &Value, field: &str) -> String {
    let value = args[field].as_str().unwrap();
    assert!(value.len() <= 256);
    value.to_string()
}
fn fields(args: &Value, expected: &[&str]) {
    let object = args.as_object().unwrap();
    assert_eq!(object.len(), expected.len());
    assert!(expected.iter().all(|field| object.contains_key(*field)));
}
async fn bounded<F: Future>(name: &str, future: F) -> F::Output {
    tokio::select! {
        value = future => value,
        _ = SystemClock.sleep(std::time::Duration::from_secs(45)) => panic!("native App bootstrap exceeded bound: {name}"),
    }
}

async fn invoke(
    state: &AppState,
    port: u16,
    command: Command,
    args: Value,
    seen: &mut Observations,
) -> Result<Reply, String> {
    match command {
        Command::PrepareController => {
            fields(&args, &[]);
            assert!(seen.controller.is_none());
            let id = crate::prepare_voice_controller_inner(state).await?;
            seen.controller = Some(id.clone());
            Ok(Reply::metadata(json!(id)))
        }
        Command::PrepareOwner => {
            fields(&args, &["controller", "server", "room"]);
            assert!(seen.owner.is_none());
            let controller = text(&args, "controller");
            assert_eq!(Some(&controller), seen.controller.as_ref());
            assert_eq!(args["server"], 7);
            assert_eq!(args["room"], crate::channel_id("general").to_string());
            let id =
                crate::prepare_voice_owner_inner(state, controller, 7, text(&args, "room")).await?;
            seen.owner = Some(id.clone());
            Ok(Reply::metadata(json!(id)))
        }
        Command::Activate => {
            fields(&args, &["candidate"]);
            let candidate = text(&args, "candidate");
            assert!(
                seen.controller.as_ref() == Some(&candidate)
                    || seen.owner.as_ref() == Some(&candidate)
            );
            let result = crate::activate_voice_owner_inner(state, candidate.clone()).await?;
            if result.group.is_some() {
                seen.activations += 1;
                let owner = state
                    .voice_authority
                    .lock()
                    .unwrap()
                    .owner(&candidate)
                    .unwrap();
                if let Some((join, lease)) = &seen.original {
                    assert_eq!(*join, owner.join);
                    assert!(lease.is_same(&owner.lease));
                } else {
                    seen.original = Some((owner.join, owner.lease));
                }
            }
            Ok(Reply::metadata(serde_json::to_value(result).unwrap()))
        }
        Command::End => {
            fields(&args, &["owner"]);
            let owner = text(&args, "owner");
            assert!(
                seen.controller.as_ref() == Some(&owner) || seen.owner.as_ref() == Some(&owner)
            );
            crate::end_voice_owner_inner(state, &owner).await;
            Ok(Reply::metadata(Value::Null))
        }
        command => {
            let expected = match command {
                Command::PublishKey => &["owner", "executor", "permit", "publicKey"][..],
                Command::SettleKey => &["owner", "executorId"][..],
                Command::Work => &["owner", "executor", "fromSlot"][..],
                _ => &["owner", "executor"][..],
            };
            fields(&args, expected);
            let owner = text(&args, "owner");
            assert_eq!(Some(&owner), seen.owner.as_ref());
            let executor = text(
                &args,
                if matches!(command, Command::SettleKey) {
                    "executorId"
                } else {
                    "executor"
                },
            );
            let raw = hex::decode(&executor).unwrap().try_into().unwrap();
            if let Some(original) = &seen.executor {
                assert_eq!(*original, executor);
            } else {
                seen.executor = Some(executor.clone());
            }
            let value = match command {
                Command::AdmitKey => serde_json::to_value(
                    crate::voice_join_key::admit_inner(state, &owner, &executor).await?,
                )
                .unwrap(),
                Command::PublishKey => {
                    seen.publications += 1;
                    let point = hex::decode(text(&args, "publicKey"))
                        .unwrap()
                        .try_into()
                        .unwrap();
                    serde_json::to_value(
                        crate::voice_join_key::publish_inner(
                            state,
                            &owner,
                            &executor,
                            &text(&args, "permit"),
                            point,
                        )
                        .await?,
                    )
                    .unwrap()
                }
                Command::SettleKey => {
                    crate::voice_join_key::settle_inner(state, &owner, raw).await?;
                    Value::Null
                }
                Command::Setup => {
                    let status =
                        bootstrap::ensure(state, &owner, raw, &mut catcoms_rt::OsCryptoRng)
                            .await
                            .map_err(|_| "native bootstrap refused")?;
                    let projection = state
                        .voice_channels
                        .project(&owner, raw)
                        .map_err(|_| "projection refused")?;
                    let reply =
                        commands::SetupReply::new(status, owner, executor, port, projection)?;
                    return Ok(Reply {
                        value: serde_json::from_str(
                            &reply.json().map_err(|_| "serialization refused")?,
                        )
                        .unwrap(),
                        _setup: Some(reply),
                        _work: None,
                    });
                }
                Command::SettleSetup => {
                    commands::settle_inner(state, &owner, &executor).await?;
                    Value::Null
                }
                Command::CollectChannel => {
                    commands::collect_inner(state, &owner, &executor).await?;
                    seen.collected += 1;
                    Value::Null
                }
                Command::Work => {
                    let reply = commands::read_work_inner(
                        state,
                        owner,
                        executor,
                        args["fromSlot"].as_u64().unwrap().try_into().unwrap(),
                    )?;
                    seen.work += 1;
                    return Ok(Reply {
                        value: serde_json::from_str(
                            &reply
                                .fixture_json()
                                .map_err(|_| "work serialization refused")?,
                        )
                        .unwrap(),
                        _setup: None,
                        _work: Some(reply),
                    });
                }
                _ => unreachable!("handled native bootstrap commands"),
            };
            Ok(Reply::metadata(value))
        }
    }
}

/// Real native outcomes, selected-key crypto and private HTTP; no mock Ready/actor grant.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_app_facade_recovers_lost_activation_and_joins_actual_channel_shutdown() {
    let fixture = crate::voice_activation_tests::Fixture::new().await;
    let bound = service::BoundServer::bind_application(
        fixture.state.voice_channels.clone(),
        crate::voice_runtime(&fixture.state),
        service::Origin::PackagedHttp,
        context::Context::new(&fixture.state),
    )
    .unwrap();
    let port = bound.port();
    let server = bound.start().unwrap();
    let mut command = tokio::process::Command::new("node");
    command
        .arg("--experimental-strip-types")
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scripts/voice-app-interop.ts"),
        )
        .arg(crate::channel_id("general").to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // Hidden test helper, not a user interactive window.
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let errors = child.stderr.take().unwrap();
    let mut stderr = tokio::spawn(async move {
        let mut bytes = Vec::new();
        errors
            .take(16 * 1024)
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        bytes
    });
    let mut seen = Observations::default();
    let mut serial = 0;
    let observed = webcrypto::catch_fixture(async {
        loop {
            let frame = bounded("public metadata frame", async {
                let size = output.read_u32().await.unwrap() as usize;
                assert!((1..=4096).contains(&size));
                let mut bytes = vec![0; size];
                output.read_exact(&mut bytes).await.unwrap();
                serde_json::from_slice::<Frame>(&bytes).unwrap()
            })
            .await;
            match frame {
                Frame::Invoke {
                    request,
                    command,
                    args,
                } => {
                    assert!(request > serial && request < 128);
                    serial = request;
                    let reply = bounded(
                        "actual native command",
                        invoke(&fixture.state, port, command, args, &mut seen),
                    )
                    .await;
                    let value = match &reply {
                        Ok(reply) => {
                            json!({"tag":"reply","request":request,"ok":true,"value":reply.value})
                        }
                        Err(_) => json!({"tag":"reply","request":request,"ok":false,"value":null}),
                    };
                    let bytes = serde_json::to_vec(&value).unwrap();
                    assert!(bytes.len() <= 4096);
                    bounded("public metadata write", async {
                        input.write_u32(bytes.len() as u32).await.unwrap();
                        input.write_all(&bytes).await.unwrap();
                        input.flush().await.unwrap();
                    })
                    .await;
                    drop(bytes);
                    drop(reply); // Actual public-copy write, then original projection.
                }
                Frame::Ready => {
                    assert!(!seen.ready);
                    seen.ready = true;
                    assert_eq!(seen.activations, 2);
                    assert_eq!(seen.publications, 1);
                    assert!(seen.work >= 1);
                    assert!(seen.original.as_ref().unwrap().1.is_live());
                }
                Frame::Done => {
                    assert!(seen.ready);
                    assert!(!seen.original.as_ref().unwrap().1.is_live());
                    assert!(
                        seen.collected >= 1,
                        "normal facade cleanup must collect before any forced fixture shutdown"
                    );
                    break;
                }
                Frame::Failed {
                    phase,
                    last_command,
                } => panic!(
                    "production facade child failed: phase={phase:?}; command={last_command:?}"
                ),
            }
        }
    })
    .await;
    drop(input);
    // Every failure still passes through child, native, service and Actor cleanup. A
    // watchdog is a failed observation, never proof of physical return or an excuse
    // to skip the remaining owned cleanup. Only this TEST subprocess/reader is killed.
    let mut failure = observed.err();
    if failure.is_some() {
        let _ = child.start_kill();
    }
    let exited =
        webcrypto::catch_fixture(bounded("child exit", async { child.wait().await.unwrap() }))
            .await;
    match exited {
        Ok(status) if status.success() => {}
        Ok(_) => {
            if failure.is_none() {
                failure = Some(Box::new("bootstrap child exited unsuccessfully"));
            }
        }
        Err(error) => {
            if failure.is_none() {
                failure = Some(error);
            }
            let _ = child.start_kill();
            let _ = webcrypto::catch_fixture(bounded("killed child return", child.wait())).await;
        }
    }
    let diagnostic = webcrypto::catch_fixture(bounded("child stderr return", &mut stderr)).await;
    match diagnostic {
        Ok(Ok(bytes)) => {
            if !bytes.is_empty() {
                eprintln!(
                    "bounded facade fixture Node diagnostics: {}",
                    String::from_utf8_lossy(&bytes)
                );
            }
        }
        Ok(Err(error)) => {
            // This reader has actually returned; never poll its consumed JoinHandle again.
            if failure.is_none() {
                failure = Some(Box::new(format!("bootstrap stderr reader failed: {error}")));
            }
        }
        Err(error) => {
            stderr.abort(); // Test-owned bounded reader only, AFTER child termination attempt.
            let _ =
                webcrypto::catch_fixture(bounded("stderr reader cancellation", &mut stderr)).await;
            if failure.is_none() {
                failure = Some(error);
            }
        }
    }
    if failure.is_some() {
        if let Some(owner) = &seen.owner {
            let _ = webcrypto::catch_fixture(bounded(
                "failed original owner End",
                crate::end_voice_owner_inner(&fixture.state, owner),
            ))
            .await;
        }
        if let Some(controller) = &seen.controller {
            let _ = webcrypto::catch_fixture(bounded(
                "failed original controller End",
                crate::end_voice_owner_inner(&fixture.state, controller),
            ))
            .await;
        }
        if let (Some(owner), Some(executor)) = (&seen.owner, &seen.executor) {
            let raw = hex::decode(executor).unwrap().try_into().unwrap();
            let _ = webcrypto::catch_fixture(bounded(
                "failed selected-key settlement",
                crate::voice_join_key::settle_inner(&fixture.state, owner, raw),
            ))
            .await;
            let _ = webcrypto::catch_fixture(bounded(
                "failed original setup settlement",
                commands::settle_inner(&fixture.state, owner, executor),
            ))
            .await;
            let _ = webcrypto::catch_fixture(bounded(
                "failed original channel collection",
                commands::collect_inner(&fixture.state, owner, executor),
            ))
            .await;
        }
    }
    if let Err(error) = webcrypto::catch_fixture(bounded(
        "native physical channel return",
        fixture.state.voice_channels.stop_and_wait_for_physical(),
    ))
    .await
    {
        if failure.is_none() {
            failure = Some(error);
        }
    }
    if let Err(error) = webcrypto::catch_fixture(bounded("private HTTP service return", async {
        assert!(!server.finish().await.unwrap().io_failed);
    }))
    .await
    {
        if failure.is_none() {
            failure = Some(error);
        }
    }
    if let Err(error) =
        webcrypto::catch_fixture(bounded("original Actor return", fixture.close())).await
    {
        if failure.is_none() {
            failure = Some(error);
        }
    }
    if let Some(error) = failure {
        std::panic::resume_unwind(error);
    }
}
