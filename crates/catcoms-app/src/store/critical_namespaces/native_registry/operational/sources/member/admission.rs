//! Final native member admission. Owning failure/success envelopes release
//! actual native custody before any runtime restoration or its notification.
use super::*;
use crate::store::{MemberSaveJob, MemberSaveJobReport, MemberSaveNativeAdmission};

/// Prepaid unentered key metadata and weak runtime association, retaining the
/// original counted finite Source work (including its physical custody). Construct
/// outside native guards. Native must TRY its actual commit acquisition: this
/// preparation still owns a counted Source End, so awaiting a commit guard
/// held by whole-close (which drains that End) would deadlock. On contention,
/// drop the whole preparation outside guards before any wait/retry. No Entry,
/// issuer or strong runtime/body is retained here.
pub struct NativeTextMemberFinalPreparation<T: MeshTransport, R: CryptoRngCore> {
    // Allocated outside guards; empty until try_admit arms the whole failure.
    // FIRST so an unrun preparation frees it before its actual payments.
    failure_frame: Option<Box<NativeTextMemberAdmissionFailure<T, R>>>,
    token: [u8; 32],
    keys: Option<NativeFounderKeys>,
    preparation: NativeTextMemberPreparation<T, R>,
}

/// Whole refusal. Native custody is FIRST, before any temporary strong cell,
/// runtime loan, key metadata or physical/preparation/payment tail. Returning
/// only an error code would destroy those resources while commit stayed held.
pub struct NativeTextMemberAdmissionFailure<T: MeshTransport, R: CryptoRngCore> {
    native: Option<MemberSaveNativeAdmission>,
    runtime: Option<NativeTextCreationJob<T, R>>,
    cell: Option<Arc<RuntimeCell<T, R>>>,
    preparation: Option<NativeTextMemberFinalPreparation<T, R>>,
    reason: V2MemberSaveRefusal,
}
impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberAdmissionFailure<T, R> {
    pub fn refusal(&self) -> V2MemberSaveRefusal {
        self.reason
    }
}

/// Actual file work FIRST. Its consuming run or Drop releases the real native
/// commit guard before restoring the whole runtime and waking its waiters.
/// Source retirement remains behind both this running loan and the member job's
/// actual save-source End. Native must fund its own spawn/return envelopes too.
pub struct NativeTextMemberSaveJob<T: MeshTransport, R: CryptoRngCore> {
    member: Option<MemberSaveJob>,
    runtime: Option<NativeTextCreationJob<T, R>>,
    failure_frame: Option<Box<NativeTextMemberAdmissionFailure<T, R>>>,
    preparation: Option<NativeTextMemberFinalPreparation<T, R>>,
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberPreparation<T, R> {
    /// Blocking lookup/key-metadata preparation BEFORE acquiring UI commit.
    /// Entered phase-one Ends must already have retired. No new key is entered
    /// here; actual membership/source/session admission remains the final edge.
    pub fn finish_admission(
        self,
        token: [u8; 32],
    ) -> Result<NativeTextMemberFinalPreparation<T, R>, V2MemberSaveRefusal> {
        if token == [0; 32] {
            return Err(V2MemberSaveRefusal::Stale);
        }
        let (cell, keys) = self.prepare_keys()?;
        drop(cell); // only Weak survives the native commit await
        let failure_frame = Box::new(NativeTextMemberAdmissionFailure {
            native: None,
            runtime: None,
            cell: None,
            preparation: None,
            reason: V2MemberSaveRefusal::Unavailable,
        });
        Ok(NativeTextMemberFinalPreparation {
            failure_frame: Some(failure_frame),
            token,
            keys: Some(keys),
            preparation: self,
        })
    }
}

fn borrow_runtime<T: MeshTransport, R: CryptoRngCore>(
    cell: &RuntimeCell<T, R>,
    scope: &Scope,
) -> Result<Runtime<T, R>, V2MemberSaveRefusal> {
    let mut state = cell
        .state
        .try_lock()
        .map_err(|_| V2MemberSaveRefusal::Busy)?;
    if state.retiring || !scope.is_open() {
        return Err(V2MemberSaveRefusal::Unavailable);
    }
    if state.running || state.poisoned {
        return Err(V2MemberSaveRefusal::Busy);
    }
    let body = state.body.take().ok_or(V2MemberSaveRefusal::Unavailable)?;
    state.running = true;
    Ok(body) // guard leaves before any native-aware Host call
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberFinalPreparation<T, R> {
    /// Native supplies its ACTUAL checked owned guard, row/persistence custody
    /// and full original cancellation. An arbitrary erased box authenticates
    /// nothing. This consumes no IO, pool admission or issuer preparation.
    /// Every post-borrow refusal retains the whole loan behind native custody.
    pub fn try_admit(
        mut self,
        native: MemberSaveNativeAdmission,
    ) -> Result<NativeTextMemberSaveJob<T, R>, Box<NativeTextMemberAdmissionFailure<T, R>>> {
        // Arm ownership BEFORE any fallible check/upgrade. Temporary strong
        // cells must live here, never in a local that can Drop under commit.
        let mut failure = self
            .failure_frame
            .take()
            .expect("original prepaid failure frame");
        failure.native = Some(native);
        failure.preparation = Some(self);
        if let Err(reason) = failure
            .preparation
            .as_ref()
            .unwrap()
            .preparation
            .require_current()
        {
            failure.reason = reason;
            return Err(failure);
        }
        let cell = failure
            .preparation
            .as_ref()
            .unwrap()
            .preparation
            .row
            .original
            .runtime
            .upgrade();
        failure.cell = cell;
        if failure.cell.is_none() {
            return Err(failure);
        }
        let body = match borrow_runtime(
            failure.cell.as_ref().unwrap(),
            &failure.preparation.as_ref().unwrap().preparation.scope,
        ) {
            Ok(body) => body,
            Err(reason) => {
                failure.reason = reason;
                return Err(failure);
            }
        };
        // All moves below are infallible on this private, validated state.
        // Arm restoration immediately, outside the runtime writer.
        let preparation = failure.preparation.as_mut().unwrap();
        let keys = preparation
            .keys
            .take()
            .expect("one original prepared key set");
        let command = preparation
            .preparation
            .command
            .take()
            .expect("original command retained");
        failure.runtime = Some(NativeTextCreationJob {
            loan: RuntimeLoan {
                body: Some(body),
                cell: failure.cell.take().unwrap(),
                recorded: false,
            },
            keys,
            command,
        });
        let token = failure.preparation.as_ref().unwrap().token;
        let mut job = NativeTextMemberSaveJob {
            member: None,
            runtime: failure.runtime.take(),
            failure_frame: None,
            preparation: failure.preparation.take(),
        };
        let native = failure.native.take().unwrap();
        let result = admit(job.runtime.as_mut().unwrap(), token, native);
        match result {
            Ok(member) => {
                job.member = Some(member);
                job.failure_frame = Some(failure); // no allocator/free under commit
                Ok(job)
            }
            Err((native, reason)) => {
                failure.native = Some(native);
                failure.runtime = job.runtime.take();
                failure.preparation = job.preparation.take();
                failure.reason = reason;
                Err(failure)
            }
        }
    }
}

fn admit<T: MeshTransport, R: CryptoRngCore>(
    runtime: &mut NativeTextCreationJob<T, R>,
    token: [u8; 32],
    native: MemberSaveNativeAdmission,
) -> Result<MemberSaveJob, (MemberSaveNativeAdmission, V2MemberSaveRefusal)> {
    if runtime.require_current().is_err() {
        return Err((native, V2MemberSaveRefusal::Cancelled));
    }
    let body = match runtime.loan.body.as_mut() {
        Some(body) => body,
        None => return Err((native, V2MemberSaveRefusal::Unavailable)),
    };
    let driver = match body.driver.as_mut() {
        Some(driver) => driver,
        None => return Err((native, V2MemberSaveRefusal::Unavailable)),
    };
    let result = body
        .work
        .admit_native_member(driver, token, native, &mut runtime.keys);
    runtime.loan.recorded = true; // known Host result; never an attempt refund
    result
}

impl<T: MeshTransport, R: CryptoRngCore> NativeTextMemberSaveJob<T, R> {
    /// Actual blocking commit, not an IPC-waiter lifetime. The inner result is
    /// deposited before its native custody leaves; runtime notification follows.
    pub fn run(mut self) -> MemberSaveJobReport {
        let report = self
            .member
            .take()
            .expect("actual admitted member job")
            .run();
        #[cfg(all(test, any(windows, target_os = "linux")))]
        super::tests::after_commit();
        drop(self.runtime.take());
        drop(self.failure_frame.take());
        drop(self.preparation.take());
        report
    }
}

pub(super) fn invoice<T: MeshTransport, R: CryptoRngCore>() -> usize {
    2 * std::mem::size_of::<NativeTextMemberFinalPreparation<T, R>>()
        + 2 * std::mem::size_of::<NativeTextMemberAdmissionFailure<T, R>>()
        + 2 * std::mem::size_of::<NativeTextMemberSaveJob<T, R>>()
        + 4096
}

macro_rules! opaque {
    ($($name:ident),*)=> {$(impl<T:MeshTransport,R:CryptoRngCore> std::fmt::Debug for $name<T,R> {
        fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($name)," { .. }"))}
    })*};
}
opaque!(
    NativeTextMemberFinalPreparation,
    NativeTextMemberAdmissionFailure,
    NativeTextMemberSaveJob
);
