//! Member save through the actual whole-owner founder-as-member host: the real member lane and
//! signer, the real namespace5 mount on the fixture's own vault and the real member outbox.
//!
//! The native admission here is a TEST custody box with a test session. These root fixtures
//! establish no native authorization; that needs the native consumer's checked factory, its
//! generation/registry paths and its own barrier tests. Write boundaries are injected through
//! the outbox's thread-local cut hooks, so every job here runs on the test thread.
use super::super::super::super::member_sign_dispatches_for_test;
use super::*;
use crate::store::persistence::v2_member_outbox::{
    member_outbox_call_at_for_test, member_outbox_crash_at_for_test, member_outbox_disarm_for_test,
    MemberOutboxTestCut,
};
use crate::store::{
    MemberSaveJobReport, MemberSaveNativeAdmission, V2MemberSaveOutcome, V2MemberSaveRefusal,
    V2MemberSignRequest,
};
use catcoms_replication::chat::Body;
use catcoms_rt::RequestCancellation;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

/// Field order is Drop order: the host (and the whole original owner) first, then the layout,
/// then the temporary vault LAST.
struct SaveFixture {
    host: PreparedV2TextHost<MemNetwork, ChaCha20Rng>,
    layout: V2CurrentLayoutStore,
    vault: PathBuf,
    rng: ChaCha20Rng,
    _directory: tempfile::TempDir,
}

/// The actual warm family with namespace5 declared and mounted, after its real empty
/// promotion. Nothing has been signed or saved yet.
fn saving(number: u64) -> SaveFixture {
    let mut f = fixture_with(number, true);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    let owner = V2TextOwnedCurrent::from_warm(current);
    let mut host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_original_owner(owner),
        host_policy(),
    );
    host.retry_empty_promotion(&f.layout, &mut ChaCha20Rng::seed_from_u64(number ^ 0x55))
        .unwrap();
    SaveFixture {
        host,
        vault: f._directory.path().to_path_buf(),
        layout: f.layout,
        rng: f.rng,
        _directory: f._directory,
    }
}

/// A test session. The sender is kept, so the receiver never reads "publisher gone".
struct Session {
    tx: watch::Sender<bool>,
    cancellation: RequestCancellation,
}
fn session() -> Session {
    let (tx, rx) = watch::channel(false);
    Session {
        tx,
        cancellation: RequestCancellation::new(rx, None),
    }
}
fn cancelled() -> Session {
    let session = session();
    session.tx.send(true).unwrap();
    session
}

/// TEST native custody: records when the job releases it. Not native authorization.
struct Released(Arc<AtomicBool>);
impl Drop for Released {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
fn native(session: &Session) -> (MemberSaveNativeAdmission, Arc<AtomicBool>) {
    let released = Arc::new(AtomicBool::new(false));
    (
        MemberSaveNativeAdmission::new(Released(released.clone()), session.cancellation.clone()),
        released,
    )
}

fn create(text: &str) -> Body {
    Body::Create {
        text: text.to_owned(),
        reply_to: None,
    }
}
fn plain(token: [u8; 32], body: &Body) -> V2MemberSignRequest<'_> {
    V2MemberSignRequest {
        token,
        channel: 7,
        body,
        parents: &[],
        dependencies: &[],
    }
}

impl SaveFixture {
    fn prepare(
        &mut self,
        token: [u8; 32],
        body: &Body,
        session: &Session,
    ) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        self.host.prepare_member_save(
            &self.layout,
            plain(token, body),
            &session.cancellation,
            &mut self.rng,
        )
    }
    fn custody(&self) -> Option<String> {
        self.host.member_save_custody_for_test(&self.layout)
    }
    /// Prepare, admit, run and confirm one save end to end; returns its exact receipt.
    fn save(&mut self, token: [u8; 32], body: &Body) -> V2MemberSaveOutcome {
        let live = session();
        assert_eq!(
            self.prepare(token, body, &live),
            Ok(V2MemberSaveOutcome::Ready)
        );
        let (admission, released) = native(&live);
        let job = self
            .host
            .admit_member_save(&self.layout, token, admission)
            .unwrap();
        assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
        assert!(released.load(Ordering::SeqCst));
        let saved = self.prepare(token, body, &live).unwrap();
        assert!(matches!(saved, V2MemberSaveOutcome::Saved { .. }));
        saved
    }
    /// Every object of every member namespace, byte for byte.
    fn outbox_files(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let parent = self.vault.join("chat-owner-v2").join("member-outbox-v1");
        let mut files = BTreeMap::new();
        for namespace in std::fs::read_dir(parent).unwrap() {
            for entry in std::fs::read_dir(namespace.unwrap().path()).unwrap() {
                let path = entry.unwrap().path();
                files.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
        files
    }
}

#[test]
fn cancellation_during_namespace_initialization_refuses_before_fresh_signing() {
    struct CancelOnDraw<'a> {
        rng: &'a mut ChaCha20Rng,
        signal: &'a watch::Sender<bool>,
        fired: bool,
    }
    impl CancelOnDraw<'_> {
        fn cancel_once(&mut self) {
            if !self.fired {
                self.fired = true;
                self.signal.send(true).unwrap();
            }
        }
    }
    impl RngCore for CancelOnDraw<'_> {
        fn next_u32(&mut self) -> u32 {
            self.cancel_once();
            self.rng.next_u32()
        }
        fn next_u64(&mut self) -> u64 {
            self.cancel_once();
            self.rng.next_u64()
        }
        fn fill_bytes(&mut self, bytes: &mut [u8]) {
            self.cancel_once();
            self.rng.fill_bytes(bytes);
        }
        fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
            self.fill_bytes(bytes);
            Ok(())
        }
    }
    impl CryptoRng for CancelOnDraw<'_> {}

    let mut f = saving(0x7fbd0);
    let stale = session();
    let body = create("cancel before authoring after a slow namespace open");
    let dispatches = member_sign_dispatches_for_test();
    let result = {
        let mut rng = CancelOnDraw {
            rng: &mut f.rng,
            signal: &stale.tx,
            fired: false,
        };
        let result = f.host.prepare_member_save(
            &f.layout,
            plain([0xf1; 32], &body),
            &stale.cancellation,
            &mut rng,
        );
        assert!(rng.fired, "exercise the real empty-head sealing seam");
        result
    };
    assert_eq!(result, Err(V2MemberSaveRefusal::Cancelled));
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
    assert_eq!(f.custody(), None);
    assert!(f.host.take_member_signed_original([0xf1; 32]).is_err());
    // Initial namespace metadata is harmless; a fresh invocation can author/save normally.
    f.save([0xf1; 32], &body);
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
}

#[test]
fn append_payment_pressure_retains_the_original_across_lane_replacement() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut f = saving(0x7fbd1);
    let live = session();
    let body = create("keep the signed original when preseal credit is busy");
    let pool = f
        .layout
        .checked_store()
        .unwrap()
        .member_outbox_mount()
        .unwrap()
        .pool_for_test();
    let held = Rc::new(RefCell::new(None));
    let hold_in_hook = held.clone();
    let dispatches = member_sign_dispatches_for_test();
    let result = PreparedV2TextHost::<MemNetwork, ChaCha20Rng>::with_member_preseal_hook_for_test(
        Box::new(move || {
            let free = pool.capacity() - pool.charged_for_test();
            *hold_in_hook.borrow_mut() = Some(pool.try_reserve(free).unwrap());
        }),
        || f.prepare([0xf2; 32], &body, &live),
    );
    assert_eq!(result, Err(V2MemberSaveRefusal::Busy));
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
    let original = f.host.member_save_original_for_test(&f.layout).unwrap();
    assert!(f.custody().unwrap().starts_with("Unresolved"));
    f.host.drop_member_lane_for_test();
    assert_eq!(
        f.host.member_save_original_for_test(&f.layout).unwrap(),
        original
    );
    drop(held.borrow_mut().take());
    assert_eq!(
        f.prepare([0xf2; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
    assert_eq!(
        f.host.member_save_original_for_test(&f.layout).unwrap(),
        original
    );
    f.save([0xf2; 32], &body);
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
}

#[test]
fn dropping_a_save_end_does_not_acquire_the_issuance_mutex() {
    let mut f = saving(0x7fbd2);
    let live = session();
    let body = create("retire an admitted end without waiting on its issuer");
    assert_eq!(
        f.prepare([0xf3; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let completed = f.host.member_save_ends_completed_for_test().unwrap();
    let (admission, released) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0xf3; 32], admission)
        .unwrap();
    assert!(f.host.cancel_and_drain_admitted_promotion().is_err());
    // Hold the actual issuer mutex on this thread. The previous Drop would deadlock here;
    // the focused CI subprocess deadline bounds that regression without native authority.
    f.host.with_member_save_issue_lock_for_test(|| drop(job));
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(
        f.host.member_save_ends_completed_for_test(),
        Some(completed + 1)
    );
    f.host.cancel_and_drain_admitted_promotion().unwrap();
}

#[test]
fn a_member_save_reaches_saved_local_and_exact_retries_never_sign_again() {
    let mut f = saving(0x7fb00);
    let live = session();
    let first = create("saved through the member outbox");
    let dispatches = member_sign_dispatches_for_test();
    assert_eq!(
        f.prepare([0x11; 32], &first, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
    // An exact phase-1 retry before admission keeps the SAME presealed work.
    assert_eq!(
        f.prepare([0x11; 32], &first, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    assert_eq!(f.custody().as_deref(), Some("Prepared"));
    let (admission, released) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x11; 32], admission)
        .unwrap();
    assert_eq!(f.custody().as_deref(), Some("Running"));
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(f.custody().as_deref(), Some("Saved"));
    let saved = f.prepare([0x11; 32], &first, &live).unwrap();
    assert!(matches!(
        saved,
        V2MemberSaveOutcome::Saved { ordinal: 0, .. }
    ));
    assert_eq!(f.custody(), None);
    // Exact retries after the outcome is settled: an exact lookup Hit, no authoring.
    assert_eq!(f.prepare([0x11; 32], &first, &live), Ok(saved));
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
    // The next request is the next ordinal; the first is unchanged.
    let second = f.save([0x12; 32], &create("the next request"));
    assert!(matches!(
        second,
        V2MemberSaveOutcome::Saved { ordinal: 1, .. }
    ));
    assert_eq!(f.prepare([0x11; 32], &first, &live), Ok(saved));
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 2);
}

#[test]
fn the_final_edge_refuses_without_presealed_work_and_returns_the_native_custody_whole() {
    let mut f = saving(0x7fb05);
    let live = session();
    let dispatches = member_sign_dispatches_for_test();
    let files = f.outbox_files();
    assert!(f
        .layout
        .checked_store()
        .unwrap()
        .installed_member_outbox_mount()
        .unwrap()
        .is_none());
    for install_mount in [false, true] {
        if install_mount {
            // Installing the Store's lazy metadata alone supplies no prepared request.
            f.layout
                .checked_store()
                .unwrap()
                .member_outbox_mount()
                .unwrap();
        }
        let (admission, released) = native(&live);
        let (returned, refusal) = f
            .host
            .admit_member_save(&f.layout, [0x15; 32], admission)
            .unwrap_err();
        assert_eq!(refusal, V2MemberSaveRefusal::Stale);
        assert!(!released.load(Ordering::SeqCst));
        drop(returned);
        assert!(released.load(Ordering::SeqCst));
        assert_eq!(member_sign_dispatches_for_test(), dispatches);
        assert_eq!(f.outbox_files(), files);
        assert_eq!(f.custody(), None);
    }
}

#[test]
fn a_cancelled_session_refuses_at_the_linearization_point_and_keeps_the_presealed_original() {
    let mut f = saving(0x7fb10);
    let live = session();
    let body = create("cancelled before the edge");
    assert_eq!(
        f.prepare([0x21; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let dispatches = member_sign_dispatches_for_test();
    let files = f.outbox_files();
    let stale = cancelled();
    let (admission, released) = native(&stale);
    let (returned, refusal) = f
        .host
        .admit_member_save(&f.layout, [0x21; 32], admission)
        .unwrap_err();
    assert_eq!(refusal, V2MemberSaveRefusal::Cancelled);
    assert!(!released.load(Ordering::SeqCst));
    drop(returned);
    assert_eq!(f.custody().as_deref(), Some("Prepared"));
    assert_eq!(
        f.host.member_save_owner_idle_for_test(&f.layout),
        Some(true)
    );
    assert_eq!(f.outbox_files(), files);
    // A fresh final edge with a live session admits the SAME original: no new signature.
    let (admission, released) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x21; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert!(released.load(Ordering::SeqCst));
    assert!(matches!(
        f.prepare([0x21; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 0, .. })
    ));
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
}

#[test]
fn a_close_before_the_edge_refuses_and_a_close_after_admission_waits_for_the_job() {
    // Close wins: the lane's serialized lifecycle refuses every later issuance.
    let mut f = saving(0x7fb20);
    let live = session();
    let body = create("close races admission");
    assert_eq!(
        f.prepare([0x31; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    f.host.cancel_and_drain_admitted_promotion().unwrap();
    let (admission, released) = native(&live);
    let (returned, refusal) = f
        .host
        .admit_member_save(&f.layout, [0x31; 32], admission)
        .unwrap_err();
    assert_eq!(refusal, V2MemberSaveRefusal::Stale);
    assert!(!released.load(Ordering::SeqCst));
    drop(returned);
    // The exact presealed DATA is retained, and the closed lane can never issue again: phase
    // 1 may report the retained DATA, but no later final edge admits it.
    assert_eq!(f.custody().as_deref(), Some("Prepared"));
    let _ = f.prepare([0x31; 32], &body, &live);
    let (admission, _) = native(&live);
    let refusal = f
        .host
        .admit_member_save(&f.layout, [0x31; 32], admission)
        .unwrap_err()
        .1;
    assert_eq!(refusal, V2MemberSaveRefusal::Stale);
    assert_eq!(f.custody().as_deref(), Some("Prepared"));

    // Admission wins: the close stays pending until the job finishes its IO and cleanup.
    let mut g = saving(0x7fb21);
    assert_eq!(
        g.prepare([0x32; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let (admission, released) = native(&live);
    let job = g
        .host
        .admit_member_save(&g.layout, [0x32; 32], admission)
        .unwrap();
    assert!(g.host.cancel_and_drain_admitted_promotion().is_err());
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert!(released.load(Ordering::SeqCst));
    g.host.cancel_and_drain_admitted_promotion().unwrap();
    assert_eq!(g.custody().as_deref(), Some("Saved"));
}

#[test]
fn an_unrun_job_de_admits_without_io_and_needs_a_fresh_final_edge() {
    let mut f = saving(0x7fb30);
    let live = session();
    let body = create("admitted, then dropped unrun");
    assert_eq!(
        f.prepare([0x41; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let dispatches = member_sign_dispatches_for_test();
    let files = f.outbox_files();
    let completed = f.host.member_save_ends_completed_for_test().unwrap();
    let (admission, released) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x41; 32], admission)
        .unwrap();
    drop(job);
    // Native custody, the source end and the lease were released at once; nothing was written.
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(
        f.host.member_save_ends_completed_for_test(),
        Some(completed + 1)
    );
    assert_eq!(f.custody().as_deref(), Some("Prepared"));
    assert_eq!(
        f.host.member_save_owner_idle_for_test(&f.layout),
        Some(true)
    );
    assert_eq!(f.outbox_files(), files);
    // The de-admitted work needs a FRESH edge: a session cancelled meanwhile refuses.
    let (admission, _) = native(&cancelled());
    let (_, refusal) = f
        .host
        .admit_member_save(&f.layout, [0x41; 32], admission)
        .unwrap_err();
    assert_eq!(refusal, V2MemberSaveRefusal::Cancelled);
    let (admission, _) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x41; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
}

#[test]
fn an_uncertain_commit_keeps_the_exact_original_and_the_next_call_resolves_it_without_signing() {
    let mut f = saving(0x7fb40);
    let live = session();
    let body = create("stopped after the commit point");
    assert_eq!(
        f.prepare([0x51; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let dispatches = member_sign_dispatches_for_test();
    member_outbox_crash_at_for_test(MemberOutboxTestCut::FirstOutput);
    let (admission, released) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x51; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::Uncertain);
    member_outbox_disarm_for_test();
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(f.custody().as_deref(), Some("Unresolved { lost: false }"));
    // Recovery rolls the committed WAL forward; the exact lookup with the original's own
    // request then finds it. Nothing is signed again.
    assert!(matches!(
        f.prepare([0x51; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 0, .. })
    ));
    assert_eq!(f.custody(), None);
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
}

#[test]
fn a_job_that_unwinds_mid_commit_leaves_its_original_recoverable_and_never_signs_again() {
    let mut f = saving(0x7fb50);
    let live = session();
    let body = create("unwound inside the admitted commit");
    assert_eq!(
        f.prepare([0x61; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let dispatches = member_sign_dispatches_for_test();
    member_outbox_call_at_for_test(
        MemberOutboxTestCut::FirstOutput,
        Box::new(|| panic!("injected unwind inside the admitted commit")),
    );
    let (admission, released) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x61; 32], admission)
        .unwrap();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()));
    member_outbox_disarm_for_test();
    assert!(unwound.is_err());
    assert!(released.load(Ordering::SeqCst));
    // The owner died with the job; the exact original stayed in the Store's cell.
    assert_eq!(f.custody().as_deref(), Some("Unresolved { lost: true }"));
    assert_eq!(
        f.host.member_save_owner_idle_for_test(&f.layout),
        Some(false)
    );
    assert!(matches!(
        f.prepare([0x61; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 0, .. })
    ));
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
}

#[test]
fn cancellation_and_receiver_loss_during_paused_io_cannot_cancel_admitted_work() {
    let mut f = saving(0x7fb60);
    let body = create("paused in the selector stage");
    let live = session();
    assert_eq!(
        f.prepare([0x71; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let (admission, released) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x71; 32], admission)
        .unwrap();
    let Session { tx, cancellation } = live;
    let held_at_pause = Arc::new(AtomicBool::new(false));
    let (observed, seen) = (held_at_pause.clone(), released.clone());
    member_outbox_call_at_for_test(
        MemberOutboxTestCut::HeadStage,
        Box::new(move || {
            // A lock or replacement publishes cancellation and the reply receiver vanishes
            // while the admitted write is paused between its head stage and the CAS.
            tx.send(true).unwrap();
            drop(cancellation);
            drop(tx);
            observed.store(!seen.load(Ordering::SeqCst), Ordering::SeqCst);
        }),
    );
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    member_outbox_disarm_for_test();
    // The native custody was still held at the pause and released only after the outcome.
    assert!(held_at_pause.load(Ordering::SeqCst));
    assert!(released.load(Ordering::SeqCst));
    assert!(matches!(
        f.prepare([0x71; 32], &body, &session()),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 0, .. })
    ));
}

#[test]
fn a_replacement_lane_adopts_a_retained_original_instead_of_signing_again() {
    let mut f = saving(0x7fb70);
    let live = session();
    let body = create("retained across a lane replacement");
    assert_eq!(
        f.prepare([0x81; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let dispatches = member_sign_dispatches_for_test();
    // The lane (and its lent request) is gone; the Store still holds the presealed original.
    f.host.drop_member_lane_for_test();
    assert_eq!(
        f.prepare([0x81; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let (admission, _) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x81; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert!(matches!(
        f.prepare([0x81; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 0, .. })
    ));
    assert_eq!(member_sign_dispatches_for_test(), dispatches);

    // An Unresolved original across a lane replacement: recovery and the exact lookup settle
    // it; the replacement lane never authors it.
    let next = create("uncertain, then the lane is replaced");
    assert_eq!(
        f.prepare([0x82; 32], &next, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let dispatches = member_sign_dispatches_for_test();
    member_outbox_crash_at_for_test(MemberOutboxTestCut::FirstOutput);
    let (admission, _) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0x82; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::Uncertain);
    member_outbox_disarm_for_test();
    f.host.drop_member_lane_for_test();
    assert!(matches!(
        f.prepare([0x82; 32], &next, &live),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 1, .. })
    ));
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
}

#[test]
fn an_exact_outbox_hit_settles_every_lane_state_and_never_touches_another_token() {
    let mut f = saving(0x7fb80);
    let live = session();
    let body = create("saved exactly once");
    let saved = f.save([0xa1; 32], &body);
    let V2MemberSaveOutcome::Saved { action, .. } = saved else {
        panic!("expected a saved receipt");
    };
    // No request: a replacement lane holds nothing for the token; the receipt stands alone.
    f.host.drop_member_lane_for_test();
    assert_eq!(f.prepare([0xa1; 32], &body, &live), Ok(saved));
    // Complete: a replacement lane authored the same request again (another original) and
    // holds it in memory. The stored entry wins and the in-memory original is retired.
    f.host.drop_member_lane_for_test();
    f.host
        .retry_sign_member_request(&f.layout, plain([0xa1; 32], &body))
        .unwrap();
    assert_eq!(f.prepare([0xa1; 32], &body, &live), Ok(saved));
    assert!(f.host.take_member_signed_original([0xa1; 32]).is_err());
    // Lent: another original of the same request was taken out. The hit retires the request;
    // the extra copy is never the saved one and must never be published.
    f.host.drop_member_lane_for_test();
    f.host
        .retry_sign_member_request(&f.layout, plain([0xa1; 32], &body))
        .unwrap();
    let extra = f.host.take_member_signed_original([0xa1; 32]).unwrap();
    assert!(extra.action().id() != action);
    assert_eq!(f.prepare([0xa1; 32], &body, &live), Ok(saved));
    drop(extra);
    // Unrelated pending work under another token is untouched by this token's hit.
    f.host.drop_member_lane_for_test();
    let other = create("unrelated pending request");
    f.host
        .retry_sign_member_request(&f.layout, plain([0xa2; 32], &other))
        .unwrap();
    assert_eq!(f.prepare([0xa1; 32], &body, &live), Ok(saved));
    let pending = f.host.take_member_signed_original([0xa2; 32]).unwrap();
    assert!(pending.action().spec().body == other);
}

#[test]
fn a_lookup_read_failure_refuses_before_signing_and_is_never_absence() {
    let mut f = saving(0x7fb90);
    let live = session();
    f.save([0xb1; 32], &create("an entry the index must read"));
    // Every index node is damaged in place; the next request's exact lookup must read them.
    for (path, mut bytes) in f.outbox_files() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("n-") {
            let at = bytes.len() / 2;
            bytes[at] ^= 0x5a;
            std::fs::write(&path, bytes).unwrap();
        }
    }
    let files = f.outbox_files();
    let dispatches = member_sign_dispatches_for_test();
    let refused = f.prepare([0xb2; 32], &create("must not be signed"), &live);
    assert!(
        matches!(
            refused,
            Err(V2MemberSaveRefusal::Failed | V2MemberSaveRefusal::Incomplete)
        ),
        "a damaged index is an error, never Absent: {refused:?}"
    );
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
    assert_eq!(f.custody(), None);
    assert_eq!(f.outbox_files(), files);
}

#[test]
fn a_same_token_retry_with_changed_data_is_a_conflict_in_every_custody_state() {
    let mut f = saving(0x7fba0);
    let live = session();
    let first = create("the request as first sent");
    let edited = create("the request edited before a retry");
    // Presealed: the edited retry conflicts and the presealed original is untouched.
    assert_eq!(
        f.prepare([0xc1; 32], &first, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    assert_eq!(
        f.prepare([0xc1; 32], &edited, &live),
        Err(V2MemberSaveRefusal::Conflict)
    );
    assert_eq!(f.custody().as_deref(), Some("Prepared"));
    let (admission, _) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0xc1; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    // A cached SavedLocal of the first request never reports the edited one as saved.
    assert_eq!(
        f.prepare([0xc1; 32], &edited, &live),
        Err(V2MemberSaveRefusal::Conflict)
    );
    assert!(matches!(
        f.prepare([0xc1; 32], &first, &live),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 0, .. })
    ));
    // Durable: the exact lookup conflicts too.
    assert_eq!(
        f.prepare([0xc1; 32], &edited, &live),
        Err(V2MemberSaveRefusal::Conflict)
    );

    // Unresolved: an uncertain commit of the first request, then an edited retry.
    let second = create("second request as first sent");
    assert_eq!(
        f.prepare([0xc2; 32], &second, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    member_outbox_crash_at_for_test(MemberOutboxTestCut::FirstOutput);
    let (admission, _) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0xc2; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::Uncertain);
    member_outbox_disarm_for_test();
    assert_eq!(
        f.prepare([0xc2; 32], &create("second request, edited"), &live),
        Err(V2MemberSaveRefusal::Conflict)
    );
    assert!(matches!(
        f.prepare([0xc2; 32], &second, &live),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 1, .. })
    ));
}

#[test]
fn another_tokens_retained_original_is_named_rather_than_reported_busy() {
    let mut f = saving(0x7fbb0);
    let live = session();
    let first = create("retained while another request arrives");
    assert_eq!(
        f.prepare([0xd1; 32], &first, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let dispatches = member_sign_dispatches_for_test();
    let other = create("the other request");
    assert_eq!(
        f.prepare([0xd2; 32], &other, &live),
        Err(V2MemberSaveRefusal::Retained { token: [0xd1; 32] })
    );
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
    let (admission, _) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0xd1; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    // The cached outcome settles the lane on the other token's call, which then proceeds.
    assert_eq!(
        f.prepare([0xd2; 32], &other, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    // Retained is NOT terminal: even an exact retry of the already saved token names the
    // retained other token until that one is finished (composer contract).
    assert_eq!(
        f.prepare([0xd1; 32], &first, &live),
        Err(V2MemberSaveRefusal::Retained { token: [0xd2; 32] })
    );
}

#[test]
fn an_unwind_before_the_commit_point_re_presealing_the_same_original_after_an_absent_lookup() {
    let mut f = saving(0x7fbc0);
    let live = session();
    let body = create("torn before it became the WAL");
    assert_eq!(
        f.prepare([0xe1; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    let dispatches = member_sign_dispatches_for_test();
    let parent = f.vault.join("chat-owner-v2").join("member-outbox-v1");
    member_outbox_call_at_for_test(
        MemberOutboxTestCut::WalStage,
        Box::new(move || {
            // Tear the pre-commit WAL stage as an interrupted write would, then die before it
            // is renamed onto the WAL: nothing was ever committed.
            let namespace = std::fs::read_dir(&parent)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let stage = namespace.join("outbox.wal.stage.v1");
            let length = std::fs::metadata(&stage).unwrap().len();
            std::fs::OpenOptions::new()
                .write(true)
                .open(&stage)
                .unwrap()
                .set_len(length / 2)
                .unwrap();
            panic!("injected unwind before the commit point");
        }),
    );
    let (admission, released) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0xe1; 32], admission)
        .unwrap();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()));
    member_outbox_disarm_for_test();
    assert!(unwound.is_err());
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(f.custody().as_deref(), Some("Unresolved { lost: true }"));
    // Recovery removes the torn stage, the exact lookup is Absent, and THE SAME original is
    // presealed again: no new signature.
    assert_eq!(
        f.prepare([0xe1; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Ready)
    );
    assert_eq!(f.custody().as_deref(), Some("Prepared"));
    let (admission, _) = native(&live);
    let job = f
        .host
        .admit_member_save(&f.layout, [0xe1; 32], admission)
        .unwrap();
    assert_eq!(job.run(), MemberSaveJobReport::SavedLocal);
    assert!(matches!(
        f.prepare([0xe1; 32], &body, &live),
        Ok(V2MemberSaveOutcome::Saved { ordinal: 0, .. })
    ));
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
}

/// Every regular file named `pin.bin` below `root`.
fn pin_files(root: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() && entry.file_name() == "pin.bin" {
                found.push(entry.path());
            }
        }
    }
    found
}

#[test]
fn the_member_context_pin_identity_is_the_on_disk_pin_ciphertext_in_every_family() {
    use crate::store::critical_namespaces::archive_current_codec::V2GroupPinId;
    // The stable context's pin component comes from the EXACT authenticated pin ciphertext,
    // which every family (warm, cold, repaired) retains from the same file a restart reads.
    // This pins only that component; a full warm-save then cold-reopen Hit is not covered.
    for family in 0..3 {
        let original = text_fixture(family, 0x7fbd0 + family as u64);
        let owner = original.owner.borrow_current().unwrap();
        let id = owner.original_pin_id_data().unwrap();
        let pins = pin_files(original._directory.path());
        assert!(
            pins.iter()
                .any(|pin| V2GroupPinId::of(&std::fs::read(pin).unwrap()) == id),
            "family {family}: the pin identity must name an on-disk pin ciphertext"
        );
    }
}
