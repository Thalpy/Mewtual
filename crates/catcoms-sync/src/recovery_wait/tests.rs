use super::*;
use catcoms_rt::ManualClock;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

async fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let mut future = future;
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}

#[derive(Debug)]
struct PausedPublisher {
    peer: PeerId,
    allow: Arc<AtomicBool>,
    submitted: Arc<Mutex<Vec<Vec<u8>>>>,
}

#[async_trait::async_trait]
impl MeshTransport for PausedPublisher {
    fn local_peer(&self) -> PeerId {
        self.peer
    }
    async fn subscribe(&self, _: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn unsubscribe(&self, _: Topic) -> Result<(), TransportError> {
        Ok(())
    }
    async fn publish(&self, _: Topic, data: Bytes) -> Result<(), TransportError> {
        if !self.allow.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.submitted.lock().unwrap().push(data.to_vec());
        Ok(())
    }
    async fn request(&self, _: PeerId, _: ProtocolId, _: Bytes) -> Result<Bytes, TransportError> {
        std::future::pending().await
    }
    async fn next_event(&self) -> Option<TransportEvent> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancelled_owner_polls_retain_the_original_signed_control_publication() {
    let (_, mut members, ids) = crate::tests::build_members(2).await;
    let mut original = members.remove(0);
    let saved = original.snapshot().unwrap();
    let allow = Arc::new(AtomicBool::new(false));
    let submitted = Arc::new(Mutex::new(Vec::new()));
    let mut owner = ChannelSync::restore(
        &saved,
        PausedPublisher {
            peer: original.local_peer(),
            allow: Arc::clone(&allow),
            submitted: Arc::clone(&submitted),
        },
        ChaCha20Rng::seed_from_u64(90),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    // Actual MLS removal creates the accepted, originally signed control frame.
    owner.commit_remove_now(&ids[1]);
    let expected = owner.outbox.clone();
    assert!(!expected.is_empty());
    for _ in 0..64 {
        let mut tick = Box::pin(owner.run_once());
        assert!(poll_once(tick.as_mut()).await.is_pending());
        drop(tick); // exactly the cancellation caused by a ready actor command
        assert_eq!(
            owner.outbox, expected,
            "a pending publisher cannot lose admitted signed bytes"
        );
    }
    assert!(submitted.lock().unwrap().is_empty());
    allow.store(true, Ordering::SeqCst);
    let mut tick = Box::pin(owner.run_once());
    assert!(poll_once(tick.as_mut()).await.is_pending());
    drop(tick);
    assert!(owner.outbox.is_empty());
    assert_eq!(
        *submitted.lock().unwrap(),
        expected
            .into_iter()
            .map(|(_, bytes)| bytes)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn removed_member_discards_owned_response_without_retiring_its_recovery_obligation() {
    use automerge::{transaction::Transactable, ROOT};
    let (_, mut members, ids) = crate::tests::build_members(2).await;
    let mut b = members.pop().unwrap();
    let mut a = members.pop().unwrap();
    a.open_channel(DocType::Channel, 901).await.unwrap();
    b.open_channel(DocType::Channel, 901).await.unwrap();
    a.post(DocType::Channel, 901, |doc| {
        doc.put(ROOT, "held", "signed history")
    })
    .await
    .unwrap();
    // Discard live delivery so only the held request can supply this history.
    assert!(matches!(
        b.transport.next_event().await,
        Some(TransportEvent::Gossip { .. })
    ));
    b.catchup_queue.clear();
    b.enqueue_doc_catchup(DocType::Channel, 901);
    let before = b.doc_version(DocType::Channel, 901);
    let checkpoint = b.snapshot().unwrap();
    b.start_queued_catchup();
    let request = b.pending_catchup.as_mut().expect("queued request");
    assert!(
        poll_fn(|cx| Poll::Ready(request.response.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let Some(TransportEvent::Request {
        from,
        data,
        responder,
        ..
    }) = a.transport.next_event().await
    else {
        panic!("real catch-up request");
    };
    let bytes = a.handle_request(from, &data);
    assert!(!bytes.is_empty());
    let control_hub = catcoms_rt::Hub::new();
    let mut control = ChannelSync::restore(
        &checkpoint,
        control_hub.join(b.local_peer()),
        ChaCha20Rng::seed_from_u64(91),
        Box::new(ManualClock::new(1_000)),
    )
    .unwrap();
    let admitted = control
        .apply_catchup_since_response(
            a.local_peer(),
            DocType::Channel,
            901,
            b.pending_catchup.as_ref().unwrap().auth,
            &bytes,
        )
        .unwrap()
        .unwrap();
    assert!(
        admitted > 0,
        "the exact answer passes request binding and admission before removal"
    );
    assert!(control.doc_version(DocType::Channel, 901) > before);
    // The response is valid for its original request, but a real signed removal now retires us.
    a.commit_remove_now(&ids[1]);
    let removal = a.commit_log.back().unwrap().clone();
    assert!(b.apply_commit_in_order(&removal));
    assert!(!b.group.is_active());
    responder.respond(Bytes::from(bytes.clone()));
    let response = b
        .pending_catchup
        .as_mut()
        .unwrap()
        .response
        .as_mut()
        .await
        .unwrap();
    assert_eq!(
        response.as_ref(),
        bytes.as_slice(),
        "the owned wait returned the valid response"
    );
    b.complete_queued_catchup(Ok(response));
    assert_eq!(b.doc_version(DocType::Channel, 901), before);
    assert!(b.catchup_queue.contains(&CatchupTask::Doc {
        doc_type: DocType::Channel,
        doc_id: 901
    }));
    b.start_queued_catchup();
    assert!(
        b.pending_catchup.is_none(),
        "removed local membership cannot start a replacement request"
    );
}
