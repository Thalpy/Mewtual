use super::*;
use catcoms_rt::MeshTransport;
use futures::{poll, StreamExt};

fn topic() -> Topic {
    Topic::new("one-shot-test")
}

fn take(receiver: &mut mpsc::Receiver<Command>) -> Publication {
    match receiver.try_recv().expect("one command") {
        Command::PublishOnce(publication) => publication,
        _ => panic!("must not enter the legacy publish path"),
    }
}

#[tokio::test]
async fn publication_service_and_original_custody_outlive_input_and_cancelled_waiter() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[derive(Debug)]
    struct Credit(Arc<AtomicUsize>);
    impl Drop for Credit {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct Input {
        bytes: Vec<u8>,
        retired: Arc<AtomicBool>,
    }
    impl AsRef<[u8]> for Input {
        fn as_ref(&self) -> &[u8] {
            &self.bytes
        }
    }
    impl Drop for Input {
        fn drop(&mut self) {
            self.retired.store(true, Ordering::SeqCst);
        }
    }
    let retired = Arc::new(AtomicUsize::new(0));
    let input_retired = Arc::new(AtomicBool::new(false));
    let (owner, receiver) = tokio::sync::watch::channel(false);
    let authority = RequestCancellation::new(receiver, Some(Arc::new(Credit(retired.clone()))))
        .retain_custody(Arc::new(Credit(retired.clone())));
    let (tx, mut rx) = mpsc::channel(1);
    let slots = Arc::new(Semaphore::new(1));
    let mut future = Box::pin(publish_once_with_authority(
        &tx,
        &slots,
        topic(),
        Bytes::from_owner(Input {
            bytes: vec![7; 1024],
            retired: input_retired.clone(),
        }),
        Some(authority),
    ));
    assert!(poll!(&mut future).is_pending());
    let queued = take(&mut rx);
    assert!(
        input_retired.load(Ordering::SeqCst),
        "driver compacted its actual lower copy"
    );
    drop(future);
    drop(owner);
    assert_eq!(
        retired.load(Ordering::SeqCst),
        0,
        "queued lower bytes retain both owners"
    );
    queued.run_with(|_, _| panic!("abandoned original wait must not enter gossip"));
    assert_eq!(retired.load(Ordering::SeqCst), 2);
    assert_eq!(slots.available_permits(), 1);
}

#[tokio::test]
async fn original_publication_authority_is_checked_at_driver_handoff() {
    let (tx, mut rx) = mpsc::channel(1);
    let slots = Arc::new(Semaphore::new(1));
    let (owner, observer) = tokio::sync::watch::channel(false);
    let authority = RequestCancellation::new(observer, None);
    let mut future = Box::pin(publish_once_with_authority(
        &tx,
        &slots,
        topic(),
        Bytes::from_static(b"original"),
        Some(authority),
    ));
    assert!(poll!(&mut future).is_pending());
    let queued = take(&mut rx);
    owner.send_replace(true);
    assert_eq!(
        slots.available_permits(),
        0,
        "escaped queue custody is still charged"
    );
    queued.run_with(|_, _| panic!("retired mount must not enter gossip"));
    assert_eq!(future.await, Err(PublishOnceError::Revoked));
    assert_eq!(slots.available_permits(), 1);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn retirement_after_publication_admission_does_not_retract_submission() {
    let (tx, mut rx) = mpsc::channel(1);
    let slots = Arc::new(Semaphore::new(1));
    let (owner, observer) = tokio::sync::watch::channel(false);
    let mut future = Box::pin(publish_once_with_authority(
        &tx,
        &slots,
        topic(),
        Bytes::from_static(b"admitted"),
        Some(RequestCancellation::new(observer, None)),
    ));
    assert!(poll!(&mut future).is_pending());
    take(&mut rx).run_with(|_, bytes| {
        // This is after the final original-authority check. A later retirement cannot turn
        // an admitted send into a clean refusal or authorize refunding live transport work.
        owner.send_replace(true);
        assert_eq!(bytes.as_ref(), b"admitted");
        assert_eq!(slots.available_permits(), 0);
        Ok(PublishSubmission::Submitted)
    });
    assert_eq!(future.await, Ok(PublishSubmission::Submitted));
    assert_eq!(slots.available_permits(), 1);
}

#[tokio::test]
async fn publish_once_waits_for_driver_result_and_never_requeues_failure() {
    let (tx, mut rx) = mpsc::channel(4);
    let slots = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    for outcome in [
        Ok(PublishSubmission::Submitted),
        Ok(PublishSubmission::Duplicate),
        Err(PublishOnceError::NoPeers),
        Err(PublishOnceError::QueuesFull),
        Err(PublishOnceError::TooLarge),
        Err(PublishOnceError::Failed),
    ] {
        let mut future = Box::pin(publish_once(
            &tx,
            &slots,
            topic(),
            Bytes::from_static(b"op"),
        ));
        assert!(
            poll!(&mut future).is_pending(),
            "enqueue is not acknowledgement"
        );
        assert_eq!(slots.available_permits(), MAX_IN_FLIGHT - 1);
        take(&mut rx).run_with(|t, data| {
            assert_eq!(slots.available_permits(), MAX_IN_FLIGHT - 1);
            assert_eq!(t, topic());
            assert_eq!(data.as_ref(), b"op");
            outcome
        });
        assert_eq!(future.await, outcome);
        assert_eq!(slots.available_permits(), MAX_IN_FLIGHT);
        assert!(rx.try_recv().is_err(), "no automatic retry command");
    }
}

#[tokio::test]
async fn publish_once_driver_unwind_releases_capacity_without_acknowledging_or_retrying() {
    let (tx, mut rx) = mpsc::channel(1);
    let slots = Arc::new(Semaphore::new(1));
    let mut future = Box::pin(publish_once(&tx, &slots, topic(), Bytes::new()));
    assert!(poll!(&mut future).is_pending());
    let command = take(&mut rx);
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        command.run_with(|_, _| {
            assert_eq!(
                slots.available_permits(),
                0,
                "hold capacity through the attempt"
            );
            panic!("injected driver-attempt failure");
        });
    }));
    assert!(unwind.is_err());
    assert_eq!(slots.available_permits(), 1);
    assert_eq!(future.await, Err(PublishOnceError::Closed));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn publish_once_cancellation_keeps_queued_capacity_until_driver_drains() {
    let (tx, mut rx) = mpsc::channel(32);
    let slots = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    drop(publish_once(&tx, &slots, topic(), Bytes::new()));
    assert!(rx.try_recv().is_err(), "unpolled future does nothing");
    for _ in 0..MAX_IN_FLIGHT {
        let mut future = Box::pin(publish_once(&tx, &slots, topic(), Bytes::new()));
        assert!(poll!(&mut future).is_pending());
        drop(future);
    }
    assert_eq!(
        slots.available_permits(),
        0,
        "cancel must not refund escaped work"
    );
    assert_eq!(
        publish_once(&tx, &slots, topic(), Bytes::new()).await,
        Err(PublishOnceError::Busy)
    );
    for _ in 0..MAX_IN_FLIGHT {
        take(&mut rx).run_with(|_, _| panic!("cancelled before admission"));
    }
    assert_eq!(slots.available_permits(), MAX_IN_FLIGHT);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn publish_once_ack_loss_after_admission_does_not_retract_or_retry() {
    let (tx, mut rx) = mpsc::channel(1);
    let slots = Arc::new(Semaphore::new(1));
    let mut future = Box::pin(publish_once(&tx, &slots, topic(), Bytes::new()));
    assert!(poll!(&mut future).is_pending());
    let mut attempted = false;
    take(&mut rx).run_with(|_, _| {
        // Exact race seam: the final closed check has passed, but the result is not sent yet.
        drop(future);
        attempted = true;
        Ok(PublishSubmission::Submitted)
    });
    assert!(attempted);
    assert_eq!(slots.available_permits(), 1);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn publish_once_bounds_inputs_compacts_slices_and_handles_full_or_closed_command_queue() {
    let (tx, mut rx) = mpsc::channel(1);
    let slots = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    assert_eq!(
        publish_once(
            &tx,
            &slots,
            topic(),
            Bytes::from(vec![0; MAX_PUBLISH_ONCE_BYTES + 1])
        )
        .await,
        Err(PublishOnceError::TooLarge)
    );
    assert_eq!(
        publish_once(
            &tx,
            &slots,
            Topic::new(vec![0; MAX_PUBLISH_ONCE_TOPIC_BYTES + 1]),
            Bytes::new()
        )
        .await,
        Err(PublishOnceError::TooLarge)
    );
    assert_eq!(slots.available_permits(), MAX_IN_FLIGHT);
    assert!(rx.try_recv().is_err());

    let backing = Bytes::from(vec![7; 2 * MAX_PUBLISH_ONCE_BYTES]);
    let data = backing.slice(..MAX_PUBLISH_ONCE_BYTES);
    let label = backing.slice(..MAX_PUBLISH_ONCE_TOPIC_BYTES);
    let mut future = Box::pin(publish_once(
        &tx,
        &slots,
        Topic::new(label.clone()),
        data.clone(),
    ));
    assert!(poll!(&mut future).is_pending());
    // Shared command queue saturation is independent of the one-shot semaphore.
    assert_eq!(
        publish_once(&tx, &slots, topic(), Bytes::new()).await,
        Err(PublishOnceError::Busy)
    );
    assert_eq!(slots.available_permits(), MAX_IN_FLIGHT - 1);
    let queued = take(&mut rx);
    assert_ne!(
        queued.data.as_ptr(),
        data.as_ptr(),
        "compact the retained payload"
    );
    assert_ne!(
        queued.topic.as_bytes().as_ptr(),
        label.as_ptr(),
        "compact the topic too"
    );
    assert_eq!(queued.data, data);
    assert_eq!(queued.topic.as_bytes(), label.as_ref());
    drop(queued); // Driver shutdown before an acknowledgement.
    assert_eq!(future.await, Err(PublishOnceError::Closed));
    assert_eq!(slots.available_permits(), MAX_IN_FLIGHT);
    drop(rx);
    assert_eq!(
        publish_once(&tx, &slots, topic(), Bytes::new()).await,
        Err(PublishOnceError::Closed)
    );
    assert_eq!(slots.available_permits(), MAX_IN_FLIGHT);
}

#[test]
fn publish_once_classification_does_not_confuse_duplicate_with_submission() {
    use gossipsub::PublishError as E;
    assert_eq!(
        classify(Ok(gossipsub::MessageId::from(vec![1]))),
        Ok(PublishSubmission::Submitted)
    );
    assert_eq!(
        classify(Err(E::Duplicate)),
        Ok(PublishSubmission::Duplicate)
    );
    assert_eq!(
        classify(Err(E::NoPeersSubscribedToTopic)),
        Err(PublishOnceError::NoPeers)
    );
    assert_eq!(
        classify(Err(E::AllQueuesFull(3))),
        Err(PublishOnceError::QueuesFull)
    );
    assert_eq!(
        classify(Err(E::MessageTooLarge)),
        Err(PublishOnceError::TooLarge)
    );
    assert_eq!(
        classify(Err(E::TransformFailed(std::io::Error::other(
            "private detail"
        )))),
        Err(PublishOnceError::Failed)
    );
}

#[tokio::test]
async fn publish_once_real_service_returns_gossip_refusal_not_enqueue_success() {
    let service = crate::MeshService::new_memory(None, &[]).unwrap();
    assert_eq!(
        service
            .publish_once(topic(), Bytes::from_static(b"op"))
            .await,
        Err(PublishOnceError::NoPeers)
    );
    // The gossip configuration can have a smaller cap than the bounded transport admission.
    assert_eq!(
        service
            .publish_once(topic(), Bytes::from(vec![0; MAX_PUBLISH_ONCE_BYTES]))
            .await,
        Err(PublishOnceError::TooLarge)
    );
}

#[tokio::test]
async fn publish_once_real_gossip_submits_after_subscription_and_receiver_gets_exact_bytes() {
    use libp2p::swarm::SwarmEvent;
    let mut sender = crate::build_memory_swarm();
    let mut receiver = crate::build_memory_swarm();
    sender.listen_on("/memory/0".parse().unwrap()).unwrap();
    let address = loop {
        if let SwarmEvent::NewListenAddr { address, .. } = sender.select_next_some().await {
            break address;
        }
    };
    receiver.dial(address).unwrap();
    let label = topic();
    receiver
        .behaviour_mut()
        .gossipsub
        .subscribe(&to_ident(&label))
        .unwrap();
    let drive = async {
        loop {
            tokio::select! {
                event = sender.select_next_some() => {
                    if matches!(event, SwarmEvent::Behaviour(crate::MeshBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { .. }))) { break; }
                }
                _ = receiver.select_next_some() => {}
            }
        }
        let (tx, mut rx) = mpsc::channel(1);
        let slots = Arc::new(Semaphore::new(1));
        let mut future = Box::pin(publish_once(
            &tx,
            &slots,
            label.clone(),
            Bytes::from_static(b"exact sealed bytes"),
        ));
        assert!(poll!(&mut future).is_pending());
        take(&mut rx).run(&mut sender.behaviour_mut().gossipsub);
        assert_eq!(future.await, Ok(PublishSubmission::Submitted));
        loop {
            tokio::select! {
                _ = sender.select_next_some() => {}
                event = receiver.select_next_some() => {
                    if let SwarmEvent::Behaviour(crate::MeshBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) = event {
                        assert_eq!(message.data, b"exact sealed bytes");
                        assert_eq!(message.topic, to_ident(&label).hash());
                        break;
                    }
                }
            }
        }
    };
    // Timeout is a test watchdog, never production scheduling or a readiness sleep.
    tokio::time::timeout(std::time::Duration::from_secs(10), drive)
        .await
        .unwrap();
}
