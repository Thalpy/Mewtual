//! One paid synchronous native publisher on the actual actor. Unlike the body/result
//! RPCs, UI publication admission happens INSIDE Root's scoped callback, after original
//! Clock observations. No old timestamp or reusable current-authority reply crosses an await.
use super::*;
use catcoms_sync::voice_v2::chain::sdp_handoff::Copy;
use catcoms_sync::voice_v2::sdp::{InitialSdpIntent, SdpInputAdmission};
use std::sync::atomic::{AtomicBool, Ordering};

pub const VOICE_SDP_INPUT_RPC_BYTES: usize = 4096;

/// Trusted native-only publication hook; never constructed from an IPC descriptor. Its
/// concrete implementation must prepay and measure its WHOLE allocation, response backing,
/// oneshot, native input/publication/output graph and future BEFORE constructing this Box.
/// A trait-reported size is not proof. This module pays its separate actual actor envelope.
///
/// Both methods consume the hook synchronously. publish may enter the prescribed native
/// UI/Coordinator/Route/row fence and commit its scoped Root step; it may not await or call
/// Clock/Root. Native retains admitted input, actual UI/physical custody and this SAME Work
/// through real output collection. Refusal cannot create Data or retry an action factory.
pub trait VoiceSdpInputPublisher: Send {
    fn publish(self: Box<Self>, step: SdpInputAdmission<'_>, work: VoiceSdpInputWork);
    fn refused(self: Box<Self>, error: VoiceEndpointError);
}

/// Original actor-work custody, not input or publication authority. Both the
/// queue/caller and actual returned output retain it; a synchronous receiver cannot refund
/// the sender while its consuming native callback is still returning.
#[derive(Clone)]
pub struct VoiceSdpInputWork(Arc<InvocationState<Copy>>);
/// Lifecycle only, separate from actual Copy/output collection. An output intentionally
/// retains this allocation; waiting for its last Arc instead would deadlock history.
pub(super) struct InvocationState<P> {
    retired: AtomicBool,
    notify: tokio::sync::Notify,
    // LAST: notification/observer metadata stays paid through its own actual destruction.
    payment: P,
}
impl<P> InvocationState<P> {
    pub(super) fn payment(&self) -> &P {
        &self.payment
    }
    pub(super) fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }
    pub(super) fn new(payment: P) -> Self {
        Self {
            retired: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
            payment,
        }
    }
    pub(super) async fn wait_retired(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            // Register before checking the monotone predicate to close the lost-wake
            // window, including a command destroyed before its first Actor turn.
            notified.as_mut().enable();
            if self.retired.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}
pub(super) struct InvocationRetirement<P>(pub(super) Arc<InvocationState<P>>);
impl<P> Drop for InvocationRetirement<P> {
    fn drop(&mut self) {
        self.0.retired.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }
}
impl VoiceSdpInputWork {
    /// Wait for the actual scoped Root call AND consuming native callback to return,
    /// or for a never-applied command to be destroyed. This proves neither absence of
    /// publication, input/output collection nor current authority. History must wait here
    /// before invoking callback-bearing canonical row recovery outside native guards.
    pub async fn wait_retired(&self) {
        self.0.wait_retired().await;
    }
}
impl std::fmt::Debug for VoiceSdpInputWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Read only to make explicit that this wrapper retains actual payment, with no
        // exposed clone of the underlying Copy or field containing native publication.
        let _ = &self.0.payment;
        f.write_str("VoiceSdpInputWork { .. }")
    }
}
/// The same ordered aggregate is used for raw intake and queued work. Keeping it
/// intact across callbacks makes paid-last destruction hold on ordinary return AND unwind.
pub(super) struct Paid<A, P> {
    pub(super) arguments: A,
    // LAST. No Drop implementation may extract this ahead of the argument graph.
    pub(super) payment: P,
}
pub(super) struct Arguments {
    grant: Arc<CallGrant>,
    intent: Arc<InitialSdpIntent>,
    action: Arc<CryptoAction>,
    publisher: Option<Box<dyn VoiceSdpInputPublisher>>,
    // Last argument: normal return/unwind drops every original argument before the
    // lifecycle signal, while Paid still retains actual RPC payment below this graph.
    retirement: Option<InvocationRetirement<Copy>>,
}
pub(super) type Command = Paid<Arguments, VoiceSdpInputWork>;
type Preparing = Paid<Arguments, Option<Copy>>;
pub(super) fn apply<T: MeshTransport, R: CryptoRngCore>(
    mut command: Command,
    server: &mut Server<T, R>,
) {
    // Keep the ordered aggregate intact across Clock and native callbacks. Turning
    // its fields into locals would drop Work before arguments during an unwind.
    let mut publisher = command.arguments.publisher.take();
    let result = server.sync.with_voice_sdp_input_admission(
        &command.arguments.grant,
        &command.arguments.intent,
        &command.arguments.action,
        |step| {
            publisher
                .take()
                .expect("one original input publisher")
                .publish(step, command.payment.clone());
            Ok(())
        },
    );
    if let Some(publisher) = publisher {
        // Root refused before lending its callback. Keep the actual Work through this
        // native refusal and synchronous send, just as through successful publication.
        publisher.refused(VoiceEndpointError::Protocol(
            result.expect_err("Root refused before callback"),
        ));
    }
}
impl ServerActor {
    /// Admit one already-paid hook without parking a large envelope behind mailbox
    /// capacity. Full is a proven local refusal and calls this same hook's refusal path;
    /// closed is Stopped. No UI guard is acquired before the actor's Clock observations.
    /// On success the caller keeps returned Work through its actual receiver/output.
    pub fn try_queue_voice_sdp_input(
        &self,
        grant: Arc<CallGrant>,
        intent: Arc<InitialSdpIntent>,
        action: Arc<CryptoAction>,
        payment: Copy,
        publisher: Box<dyn VoiceSdpInputPublisher>,
    ) -> Result<VoiceSdpInputWork, VoiceEndpointError> {
        let mut preparing = Preparing {
            arguments: Arguments {
                grant,
                intent,
                action,
                publisher: Some(publisher),
                retirement: None,
            },
            payment: Some(payment),
        };
        let payment = preparing.payment.as_ref().expect("original RPC payment");
        let checked = if !payment.uses_intent(&preparing.arguments.intent)
            || payment.has_keepalive()
            || !preparing
                .arguments
                .action
                .uses_sdp_intent(&preparing.arguments.intent)
        {
            Err(VoiceEndpointError::Protocol(VoiceError::Unauthorized))
        } else if !payment.covers_work(VOICE_SDP_INPUT_RPC_BYTES) {
            Err(VoiceEndpointError::Protocol(VoiceError::Capacity))
        } else {
            Ok(())
        };
        if let Err(error) = checked {
            preparing
                .arguments
                .publisher
                .take()
                .expect("one original input publisher")
                .refused(error);
            return Err(error);
        }
        let permit = match self.cmd_tx.tx.try_reserve() {
            Ok(permit) => permit,
            Err(error) => {
                let error = match error {
                    mpsc::error::TrySendError::Full(()) => {
                        VoiceEndpointError::Protocol(VoiceError::Capacity)
                    }
                    mpsc::error::TrySendError::Closed(()) => VoiceEndpointError::Stopped,
                };
                preparing
                    .arguments
                    .publisher
                    .take()
                    .expect("one original input publisher")
                    .refused(error);
                return Err(error);
            }
        };
        let work = VoiceSdpInputWork(Arc::new(InvocationState::new(
            preparing.payment.take().expect("one original RPC payment"),
        )));
        preparing.arguments.retirement = Some(InvocationRetirement(work.0.clone()));
        permit.send(Envelope {
            trace: self.cmd_tx.trace,
            command: AppCommand::VoiceEndpoint(VoiceEndpointCommand(super::Command::SdpInput(
                Command {
                    arguments: preparing.arguments,
                    payment: work.clone(),
                },
            ))),
        });
        Ok(work)
    }
}

/// Actual fixed App queue/callback graph. Native's concrete publisher/oneshot/future and
/// response bytes have SEPARATE original payments; no trait-reported subtraction occurs.
pub const fn voice_sdp_input_rpc_bytes() -> usize {
    std::mem::size_of::<Envelope>()
        + std::mem::size_of::<Command>()
        + std::mem::size_of::<Preparing>()
        + std::mem::size_of::<(usize, usize, InvocationState<Copy>)>()
        + 3 * std::mem::size_of::<VoiceSdpInputWork>()
        + catcoms_sync::voice_v2::sdp::sdp_input_admission_bytes()
        + 256 // bounded queue permit, closure, result and argument bookkeeping
}
const _: () = assert!(voice_sdp_input_rpc_bytes() <= VOICE_SDP_INPUT_RPC_BYTES);

#[cfg(test)]
mod tests {
    use super::*;
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
    struct ArgumentsProbe {
        payment: Weak<Payment>,
        dropped: Arc<AtomicBool>,
    }
    impl Drop for ArgumentsProbe {
        fn drop(&mut self) {
            assert!(
                self.payment.upgrade().is_some(),
                "argument destructor outlived its actual payment"
            );
            self.dropped.store(true, Ordering::Release);
        }
    }
    type Probe = (
        Paid<ArgumentsProbe, Arc<Payment>>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    );
    fn probe() -> Probe {
        let refunded = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let payment = Arc::new(Payment(refunded.clone()));
        let arguments = ArgumentsProbe {
            payment: Arc::downgrade(&payment),
            dropped: dropped.clone(),
        };
        (Paid { arguments, payment }, refunded, dropped)
    }
    #[test]
    fn scoped_input_command_arguments_remain_paid_on_clock_or_publisher_unwind() {
        let (command, refunded, dropped) = probe();
        let result = std::panic::catch_unwind(move || {
            // This is the SAME aggregate used by actual apply; its fields must stay
            // together across the callback, including a callback which never returns.
            let command = command;
            assert!(command.arguments.payment.upgrade().is_some());
            panic!("injected publication callback unwind");
        });
        assert!(result.is_err());
        assert!(dropped.load(Ordering::Acquire));
        assert!(refunded.load(Ordering::Acquire));
    }
    #[test]
    fn scoped_input_raw_intake_arguments_remain_paid_on_synchronous_refusal_unwind() {
        let (command, refunded, dropped) = probe();
        let intake = Paid {
            arguments: command.arguments,
            payment: Some(command.payment),
        };
        let result = std::panic::catch_unwind(move || {
            let intake = intake;
            assert!(intake.payment.is_some());
            panic!("injected synchronous refusal unwind");
        });
        assert!(result.is_err());
        assert!(dropped.load(Ordering::Acquire));
        assert!(refunded.load(Ordering::Acquire));
    }
    #[test]
    fn scoped_input_abandoned_waiter_and_immediate_output_collection_cannot_refund_sender() {
        for immediate_output in [false, true] {
            let (command, refunded, dropped) = probe();
            let waiter = command.payment.clone();
            drop(waiter); // real work remains admitted independently of its observer
            let output = command.payment.clone();
            if immediate_output {
                drop(output);
            } else {
                drop(command);
                assert!(!refunded.load(Ordering::Acquire));
                drop(output);
                assert!(dropped.load(Ordering::Acquire));
                assert!(refunded.load(Ordering::Acquire));
                continue;
            }
            assert!(!refunded.load(Ordering::Acquire));
            assert!(!dropped.load(Ordering::Acquire));
            drop(command); // the SAME sender-local aggregate protects callback return
            assert!(dropped.load(Ordering::Acquire));
            assert!(refunded.load(Ordering::Acquire));
        }
    }

    struct LifecycleArguments {
        probe: ArgumentsProbe,
        // Match actual Arguments: all original arguments die before retirement.
        retirement: InvocationRetirement<Arc<Payment>>,
    }
    type LifecycleProbe = (
        Paid<LifecycleArguments, Arc<InvocationState<Arc<Payment>>>>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    );
    fn lifecycle_probe() -> LifecycleProbe {
        let (probe, refunded, dropped) = probe();
        let state = Arc::new(InvocationState::new(probe.payment));
        let arguments = LifecycleArguments {
            probe: probe.arguments,
            retirement: InvocationRetirement(state.clone()),
        };
        (
            Paid {
                arguments,
                payment: state,
            },
            refunded,
            dropped,
        )
    }

    #[test]
    fn scoped_input_retirement_is_callback_completion_not_output_collection() {
        use std::{future::Future, task::Context};
        for unwind in [false, true] {
            let (command, refunded, dropped) = lifecycle_probe();
            let output = command.payment.clone();
            let mut waiting = Box::pin(output.wait_retired());
            let mut context = Context::from_waker(std::task::Waker::noop());
            assert!(waiting.as_mut().poll(&mut context).is_pending());
            assert!(!dropped.load(Ordering::Acquire));
            if unwind {
                let result = std::panic::catch_unwind(move || {
                    let command = command;
                    assert!(!command
                        .arguments
                        .retirement
                        .0
                        .retired
                        .load(Ordering::Acquire));
                    assert!(command.arguments.probe.payment.upgrade().is_some());
                    panic!("injected scoped callback unwind");
                });
                assert!(result.is_err());
            } else {
                // Also models an admitted command destroyed without an Actor turn.
                drop(command);
            }
            assert!(dropped.load(Ordering::Acquire));
            assert!(waiting.as_mut().poll(&mut context).is_ready());
            assert!(!refunded.load(Ordering::Acquire));
            drop(waiting);
            drop(output);
            assert!(refunded.load(Ordering::Acquire));
        }
    }

    #[test]
    fn scoped_input_retirement_wakes_registered_observers_and_survives_cancelled_waiter() {
        use std::{
            future::Future,
            sync::atomic::AtomicUsize,
            task::{Context, Wake, Waker},
        };
        struct Wakes(AtomicUsize);
        impl Wake for Wakes {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
            fn wake_by_ref(self: &Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let (command, refunded, dropped) = lifecycle_probe();
        let output = command.payment.clone();
        let wakes = Arc::new(Wakes(AtomicUsize::new(0)));
        let waker = Waker::from(wakes.clone());
        let mut context = Context::from_waker(&waker);
        let mut cancelled = Box::pin(output.wait_retired());
        assert!(cancelled.as_mut().poll(&mut context).is_pending());
        drop(cancelled);
        let mut first = Box::pin(output.wait_retired());
        let mut second = Box::pin(output.wait_retired());
        assert!(first.as_mut().poll(&mut context).is_pending());
        assert!(second.as_mut().poll(&mut context).is_pending());
        drop(command);
        assert_eq!(wakes.0.load(Ordering::Relaxed), 2);
        assert!(first.as_mut().poll(&mut context).is_ready());
        assert!(second.as_mut().poll(&mut context).is_ready());
        // Completion before a fresh subscription is equally observable: Notify is
        // only a wakeup, while the monotone predicate is the source of truth.
        let mut late = Box::pin(output.wait_retired());
        assert!(late.as_mut().poll(&mut context).is_ready());
        assert!(dropped.load(Ordering::Acquire));
        assert!(!refunded.load(Ordering::Acquire));
        drop((first, second, late));
        drop(output);
        assert!(refunded.load(Ordering::Acquire));
    }
}
