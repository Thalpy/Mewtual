//! Paid transport graph for one queued RPC. The opaque owner is shared only by the
//! Command, original reply receiver, and actual completion. Both channel halves must be
//! destroyed before its last Drop can publish drainage. Production uses one affine Phase
//! Copy; tests replace only the destructor witness, exercising this same wrapper code.
use std::sync::Arc;
use tokio::sync::oneshot;

pub(super) struct Command<M, O> {
    pub message: M,
    // LAST: sender and every other message field retire before the actual paid owner.
    pub work: Arc<O>,
}
pub(super) struct Pending<T, O> {
    result: oneshot::Receiver<T>,
    // LAST: a dropped receiver destroys its channel/returned value before payment returns.
    _work: Arc<O>,
}
impl<T, O> Pending<T, O> {
    pub(super) async fn receive(self) -> Result<T, oneshot::error::RecvError> {
        self.result.await
    }
}
/// Acquire the original owner before constructing the oneshot/message graph. The message
/// closure is trusted fixed native construction; no callback or allocation belongs in Root's
/// final Group/Policy tail. A failed or cancelled enqueue still owns this exact Command.
pub(super) fn request<T, M, O>(
    work: O,
    message: impl FnOnce(oneshot::Sender<T>) -> M,
) -> (Command<M, O>, Pending<T, O>) {
    let work = Arc::new(work);
    let (reply, result) = oneshot::channel();
    let pending = Pending {
        result,
        _work: work.clone(),
    };
    let message = message(reply);
    (Command { message, work }, pending)
}

/// Keep the sender's own work reference through send(), including a receiver which wakes,
/// consumes and destroys its completion before the sender has returned. Sending the last
/// reference solely inside the result would let that receiver refund the sender's graph.
pub(super) fn complete<T, O>(
    reply: oneshot::Sender<T>,
    work: Arc<O>,
    build: impl FnOnce(Arc<O>) -> T,
) {
    let value = build(work.clone());
    let _ = reply.send(value);
    drop(work);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Weak,
    };
    struct Payment(Arc<AtomicBool>);
    impl Drop for Payment {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    struct Probe {
        paid: Weak<Payment>,
        dropped: Arc<AtomicBool>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            assert!(
                self.paid.upgrade().is_some(),
                "paid graph destroyed after its owner"
            );
            self.dropped.store(true, Ordering::Release);
        }
    }
    struct Message {
        reply: oneshot::Sender<()>,
        _probe: Option<Probe>,
    }
    type TestPair = (
        Command<Message, Payment>,
        Pending<(), Payment>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    );
    fn pair() -> TestPair {
        let refunded = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let (mut command, pending) = request(Payment(refunded.clone()), |reply| Message {
            reply,
            _probe: None,
        });
        command.message._probe = Some(Probe {
            paid: Arc::downgrade(&command.work),
            dropped: dropped.clone(),
        });
        (command, pending, refunded, dropped)
    }
    #[tokio::test]
    async fn failed_send_keeps_actual_message_destructors_paid_after_receiver_drop() {
        let (command, pending, refunded, dropped) = pair();
        drop(pending);
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);
        let failed = match sender.send(command).await {
            Err(failed) => failed,
            Ok(()) => panic!("receiver stopped"),
        };
        assert!(!refunded.load(Ordering::Acquire));
        drop(failed);
        assert!(dropped.load(Ordering::Acquire));
        assert!(refunded.load(Ordering::Acquire));
    }
    #[test]
    fn closed_reply_receiver_drops_original_command_graph_before_last_payment() {
        let (command, pending, refunded, dropped) = pair();
        drop(pending);
        assert!(command.message.reply.is_closed());
        assert!(!refunded.load(Ordering::Acquire));
        drop(command); // Same whole-wrapper path as the production closed-receiver arm.
        assert!(dropped.load(Ordering::Acquire));
        assert!(refunded.load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn cancelled_send_destroys_its_queued_message_before_last_payment() {
        let (command, pending, refunded, dropped) = pair();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let (occupied, occupied_waiter, _, _) = pair();
        sender.send(occupied).await.ok().unwrap();
        let mut send = Box::pin(sender.send(command));
        // poll_fn proves this actual send is parked behind bounded capacity; no sleep.
        std::future::poll_fn(|cx| {
            assert!(send.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(pending);
        assert!(!refunded.load(Ordering::Acquire));
        drop(send);
        assert!(dropped.load(Ordering::Acquire));
        assert!(refunded.load(Ordering::Acquire));
        drop(receiver.recv().await);
        drop(occupied_waiter);
    }
    struct Completion {
        _value: Probe,
        _work: Arc<Payment>,
    }
    #[tokio::test]
    async fn actual_returned_value_stays_paid_after_pending_receive_and_until_its_drop() {
        let refunded = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let (command, pending) = request(Payment(refunded.clone()), |reply| reply);
        let Command {
            message: reply,
            work,
        } = command;
        let value = Probe {
            paid: Arc::downgrade(&work),
            dropped: dropped.clone(),
        };
        complete(reply, work, |work| Completion {
            _value: value,
            _work: work,
        });
        let completion = pending.receive().await.unwrap();
        assert!(!refunded.load(Ordering::Acquire));
        drop(completion);
        assert!(dropped.load(Ordering::Acquire));
        assert!(refunded.load(Ordering::Acquire));
    }
    #[test]
    fn panic_during_fixed_message_construction_cleans_channel_before_last_payment() {
        let refunded = Arc::new(AtomicBool::new(false));
        let result = std::panic::catch_unwind({
            let refunded = refunded.clone();
            move || {
                let _: (Command<(), Payment>, Pending<(), Payment>) =
                    request(Payment(refunded), |reply| {
                        drop(reply);
                        panic!("fixed constructor unwinds");
                    });
            }
        });
        assert!(result.is_err());
        assert!(refunded.load(Ordering::Acquire));
    }

    #[test]
    fn synchronous_receiver_finish_cannot_refund_sender_before_send_returns() {
        use std::sync::Mutex;
        use std::task::{Context, Poll, Wake, Waker};
        type Waiting = std::pin::Pin<
            Box<dyn Future<Output = Result<Completion, oneshot::error::RecvError>> + Send>,
        >;
        struct Drain {
            waiting: Mutex<Option<Waiting>>,
            paid: Weak<Payment>,
            done: AtomicBool,
        }
        impl Wake for Drain {
            fn wake(self: Arc<Self>) {
                self.wake_by_ref();
            }
            fn wake_by_ref(self: &Arc<Self>) {
                struct Noop;
                impl Wake for Noop {
                    fn wake(self: Arc<Self>) {}
                }
                let waker = Waker::from(Arc::new(Noop));
                let mut waiting = self.waiting.lock().unwrap().take().unwrap();
                let Poll::Ready(result) = waiting.as_mut().poll(&mut Context::from_waker(&waker))
                else {
                    panic!("actual oneshot wake must expose its committed value");
                };
                drop(result.unwrap());
                drop(waiting);
                // This runs synchronously INSIDE Sender::send, after the receiver and
                // completion have both finished. Only the sender's own work can pay it.
                assert!(
                    self.paid.upgrade().is_some(),
                    "receiver refunded active sender"
                );
                self.done.store(true, Ordering::Release);
            }
        }
        let refunded = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let (command, pending) = request(Payment(refunded.clone()), |reply| reply);
        let Command {
            message: reply,
            work,
        } = command;
        let drain = Arc::new(Drain {
            waiting: Mutex::new(Some(Box::pin(pending.receive()))),
            paid: Arc::downgrade(&work),
            done: AtomicBool::new(false),
        });
        let waker = Waker::from(drain.clone());
        assert!(drain
            .waiting
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending());
        let value = Probe {
            paid: Arc::downgrade(&work),
            dropped: dropped.clone(),
        };
        complete(reply, work, |work| Completion {
            _value: value,
            _work: work,
        });
        assert!(drain.done.load(Ordering::Acquire));
        assert!(dropped.load(Ordering::Acquire));
        assert!(refunded.load(Ordering::Acquire));
    }
    // No async provider or caller-owned graph is admitted by this module. Native TCP tests
    // separately pin the production Phase/Copy currency, not this destructor-only witness.
}
