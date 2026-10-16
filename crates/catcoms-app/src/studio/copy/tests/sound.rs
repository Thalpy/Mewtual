use super::*;
use crate::studio::{
    StudioControlAction, StudioControlRequest, StudioControlResponse, StudioVaultLease,
};
use std::sync::Arc;
use tokio::sync::Mutex;

fn package() -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/tests/fixtures/studio-export-v1.json"
    )))
    .unwrap();
    hex::decode(fixture["pixaHex"].as_str().unwrap()).unwrap()
}

async fn control(
    actor: &crate::ServerActor,
    store: &Arc<Mutex<Option<ServerStore>>>,
    source: StudioTarget,
    action: StudioControlAction,
) -> Result<StudioControlResponse, String> {
    actor
        .studio_control_begin(StudioControlRequest {
            target: source,
            action,
        })
        .await?
        .execute(StudioVaultLease::new(
            store.clone().try_lock_owned().unwrap(),
            SERVER,
            (),
        ))
        .await
}

#[tokio::test]
async fn studio_sound_draft_copy_actor_export_media_gate_and_exact_retry_after_disposal() {
    let root = tempfile::tempdir().unwrap();
    let hub = Hub::new();
    let mut store = ServerStore::open(root.path(), b"sound-copy", &mut rng()).unwrap();
    let mut server = Server::found(
        hub.join(PeerId::from_u64(1)),
        MlsDevice::generate().unwrap(),
        rng(),
        Box::new(ManualClock::new(1000)),
        "copy-sound",
    )
    .unwrap();
    let group = server.group_id();
    server.set_blob_store(store.blob_store(&hex::encode(&group)).unwrap());
    let source = StudioTarget::Flipnote {
        channel: crate::channel_id("general").to_be_bytes(),
        object: [0x51; 16],
    };
    let destination = StudioTarget::Flipnote {
        channel: source.channel(),
        object: [0x52; 16],
    };
    let bytes = package();
    let cid = crate::Cid::of(&bytes);
    server.publish_pixa(&bytes).unwrap();
    let export = FlipnoteOp::SetExport {
        export: [3; 16],
        cid: *cid.as_bytes(),
        bytes: bytes.len() as u64,
        expiry: StudioExpiry::Never,
    }
    .encode()
    .unwrap();
    let close = closing_branch(
        &mut server,
        &mut store,
        source,
        FlipnoteOp::SetHeader(FlipnoteHeader::Title("source".into()))
            .encode()
            .unwrap(),
        FlipnoteOp::SetHeader(FlipnoteHeader::Fps(12))
            .encode()
            .unwrap(),
        export.clone(),
    );
    install_successor(&mut server, &mut store, source, &close);
    server
        .studio_transaction(
            &mut store,
            SERVER,
            StudioRequest::Create {
                channel: destination.channel(),
                object: [0x52; 16],
                nonce: [4; 16],
                title: "destination".into(),
                ts: 1000,
            },
        )
        .unwrap();
    let logical = source.document(&group).unwrap();
    let operation = DomainOp {
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
        nonce: [3; 16],
        body: export,
    };
    let item = StudioRecoveryItem::SoundRecord {
        value: operation.id(&server.device_id()),
    };
    let pool = Arc::new(tokio::sync::Semaphore::new(4));
    let prepared = server
        .begin_copy_with_pool(
            &store,
            SERVER,
            source,
            StudioOverlayCopyChoice {
                destination,
                item,
                mode: StudioRecoveryMode::Restore,
            },
            &pool,
        )
        .unwrap()
        .plan()
        .await
        .unwrap();
    let preview = server
        .finish_studio_copy_preview(&mut store, SERVER, source, prepared)
        .unwrap();
    assert_eq!(
        preview.value().disposition,
        StudioRecoveryDisposition::Ready
    );
    assert_eq!(
        preview.value().source_ops,
        [operation.id(&server.device_id())]
    );
    let epoch_id = preview.value().epoch_id;
    let expected_projection = preview.value().expected_projection;
    let body = preview.value().body.clone().unwrap();
    drop(preview);
    let echo = || StudioOverlayCopyApply {
        destination,
        item,
        mode: StudioRecoveryMode::Restore,
        epoch_id,
        expected_projection,
        nonce: [5; 16],
        body: body.clone(),
    };
    let destination_logical = destination.document(&group).unwrap();
    let store = Arc::new(Mutex::new(Some(store)));
    let (actor, mut events, task) = crate::spawn(server);
    let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let blob = root
        .path()
        .join("blobs")
        .join(hex::encode(&group))
        .join(cid.to_hex());
    std::fs::remove_file(&blob).unwrap();
    let before = store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .load_epoch_intents(SERVER, &destination_logical)
        .unwrap()
        .pending()
        .len();
    assert!(control(
        &actor,
        &store,
        source,
        StudioControlAction::ApplyOverlayCopy(Box::new(echo()))
    )
    .await
    .is_err());
    assert_eq!(
        store
            .lock()
            .await
            .as_ref()
            .unwrap()
            .load_epoch_intents(SERVER, &destination_logical)
            .unwrap()
            .pending()
            .len(),
        before
    );
    actor.publish_pixa(bytes, None).await.unwrap();
    assert!(matches!(
        control(
            &actor,
            &store,
            source,
            StudioControlAction::ApplyOverlayCopy(Box::new(echo()))
        )
        .await
        .unwrap(),
        StudioControlResponse::OverlayCopyApplied {
            already_saved: false,
            ..
        }
    ));
    let (branch, content, accepted, _) = store
        .lock()
        .await
        .as_ref()
        .unwrap()
        .studio_branch_identity_for_test(SERVER, &logical);
    control(
        &actor,
        &store,
        source,
        StudioControlAction::DisposeOverlay(Box::new(StudioOverlayDisposalRequest {
            branch,
            content,
            accepted,
            mode: StudioDisposalRequestMode::Discard(
                StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap(),
            ),
        })),
    )
    .await
    .unwrap();
    std::fs::remove_file(blob).unwrap();
    assert!(matches!(
        control(
            &actor,
            &store,
            source,
            StudioControlAction::ApplyOverlayCopy(Box::new(echo()))
        )
        .await
        .unwrap(),
        StudioControlResponse::OverlayCopyApplied {
            already_saved: true,
            ..
        }
    ));
    actor.shutdown().await;
    task.await.unwrap();
    drain.await.unwrap();
}
