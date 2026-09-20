use super::*;
use crate::store::epoch_studio::handoff::{HandoffSync, HandoffWrite};
use catcoms_replication::studio::{StudioHandoffOutcome, StudioOverlaySave};
use catcoms_replication::ReplError;

mod eligibility;
mod evidence;
mod fences;
mod inspection;
mod metadata;
mod performance;
mod references;

fn transfer(f: &Fixture, store: &mut ServerStore, basis: [u8; 32]) -> StudioHandoffOutcome {
    let mut b = budget(store, f);
    store
        .handoff_studio_overlay(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
        )
        .unwrap()
}
fn flush(
    m: &EpochMutation<'_>,
    step: HandoffSync,
    path: &Path,
    bytes: u64,
) -> Result<(), AppError> {
    match step {
        HandoffSync::Source => sync_studio(m, path, bytes),
        HandoffSync::Intents => sync_intent(m, path, bytes),
    }
}
fn prepare(f: &Fixture, store: &mut ServerStore) -> (CloseRecord, [u8; 32], StudioProjection) {
    let (close, basis) = closing(f, store);
    let expected = save(f, store, &close, basis.fingerprint(), f.title(), 123)
        .projection()
        .clone();
    let mut source = f.load(store).unwrap();
    let receipt = source.unit.receipt_head().unwrap().cloned().unwrap();
    let decision = source
        .unit
        .resume_owner_decision(&f.group, &f.device, 0, &receipt, &close)
        .unwrap();
    let mut b = budget(store, f);
    store
        .prepare_studio_owner_decision_with_writer(
            SERVER,
            &decision,
            &f.group,
            0,
            &mut rng(),
            &mut b.storage,
            |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
        )
        .unwrap();
    warm(f, store);
    let (_, installed) = rotate(f, store);
    assert_eq!((installed.epoch(), installed.op_count()), (1, 0));
    store.retain_studio_source(&f.group, &f.device, installed);
    let mut b = budget(store, f);
    store
        .complete_studio_installed_head(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            0,
            &mut rng(),
            &mut b,
        )
        .unwrap();
    (close, basis.fingerprint(), expected)
}

/// A clock that advances a fixed step on every read, so a slice deadline is crossed by
/// construction rather than by hoping cheap and expensive operations fall either side of a
/// wall-clock threshold. Design 7.3 requires the injected `Clock`, never `SystemClock`.
#[derive(Debug)]
struct SteppingClock {
    ms: std::sync::atomic::AtomicU64,
    step: u64,
}

impl catcoms_rt::Clock for SteppingClock {
    fn now_ms(&self) -> u64 {
        self.monotonic_ms()
    }
    fn monotonic_ms(&self) -> u64 {
        self.ms
            .fetch_add(self.step, std::sync::atomic::Ordering::SeqCst)
            + self.step
    }
    fn sleep(
        &self,
        _: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(std::future::ready(()))
    }
}

/// N31. Design 7.3's two distinct events, proven apart rather than inferred from "work remains".
///
/// A visit that returns with work left proves nothing on its own, because it may have deferred
/// before signing anything. The discriminator is the remaining count at slice entry and exit: the
/// core decrements it by exactly one per successful `sign_next`, so the difference is a count of
/// signatures actually produced. Each of the four outcomes is asserted on that pair, and each
/// limiter is exercised with the **other** one disabled so neither can stand in for it.
#[test]
fn studio_overlay_handoff_signing_slice_reports_yield_bound_and_completion_apart() {
    use crate::store::epoch_studio::handoff_capture::{
        MAX_SIGNING_TURNS_PER_VISIT, SIGNING_SLICE_BUDGET_MS,
    };
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let (basis, _) = performance::fixture(&f, &mut store, 40);
    let records = canonical(&store);

    let mut b = budget(&mut store, &f);
    let capture = match store
        .start_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut |m: &EpochMutation<'_>, _, p: &Path, bytes: &[u8]| m.write(p, bytes),
            &mut flush,
        )
        .unwrap()
    {
        crate::store::epoch_studio::handoff::StudioHandoffStart::Captured(capture) => capture,
        crate::store::epoch_studio::handoff::StudioHandoffStart::Settled(_) => {
            panic!("the fixture branch was classified as already transferred")
        }
    };
    let mut plan = capture.prepare().expect("H2 reconstructs the candidate");
    assert_eq!(plan.remaining(), 40);

    // 1. Priority yield. Signs ZERO and leaves the count untouched, and says so itself rather
    //    than leaving the caller to infer a yield from the fact that work remains.
    let ticking = SteppingClock {
        ms: std::sync::atomic::AtomicU64::new(0),
        step: 1,
    };
    let slice = plan
        .sign_slice(
            &f.device,
            &f.group,
            0,
            true,
            MAX_SIGNING_TURNS_PER_VISIT,
            Some((&ticking, SIGNING_SLICE_BUDGET_MS)),
        )
        .unwrap();
    assert!(slice.yielded(), "a priority yield was not reported as one");
    assert_eq!(slice.signed(), 0, "a priority yield signed something");
    assert_eq!(slice.remaining(), 40);
    assert!(!slice.complete());

    // 2. Count-bounded slice, with the time limiter DISABLED so only the turn cap can stop it.
    let slice = plan
        .sign_slice(
            &f.device,
            &f.group,
            0,
            false,
            MAX_SIGNING_TURNS_PER_VISIT,
            None,
        )
        .unwrap();
    assert!(!slice.yielded());
    assert_eq!(
        slice.signed(),
        MAX_SIGNING_TURNS_PER_VISIT,
        "the turn cap did not bound the slice"
    );
    assert_eq!(slice.remaining(), 40 - MAX_SIGNING_TURNS_PER_VISIT);
    assert!(!slice.complete());

    // 3. Time-bounded slice, with the count limiter DISABLED so only the deadline can stop it.
    //    The clock advances 200 ms per read against a 250 ms budget: the entry read sets the
    //    deadline, the first signature's check is under it, the second crosses. A slice may
    //    overrun by one whole operation, which is exactly the two signatures observed here.
    let stepping = SteppingClock {
        ms: std::sync::atomic::AtomicU64::new(0),
        step: 200,
    };
    let slice = plan
        .sign_slice(
            &f.device,
            &f.group,
            0,
            false,
            usize::MAX,
            Some((&stepping, SIGNING_SLICE_BUDGET_MS)),
        )
        .unwrap();
    assert!(!slice.yielded());
    assert_eq!(
        slice.signed(),
        2,
        "the slice budget did not bound the slice"
    );
    assert_eq!(slice.remaining(), 40 - MAX_SIGNING_TURNS_PER_VISIT - 2);
    assert!(!slice.complete());

    // 4. Completion, with both limiters disabled.
    let slice = plan
        .sign_slice(&f.device, &f.group, 0, false, usize::MAX, None)
        .unwrap();
    assert_eq!(slice.signed(), 40 - MAX_SIGNING_TURNS_PER_VISIT - 2);
    assert_eq!(slice.remaining(), 0);
    assert!(slice.complete());

    // No signature became durable at any point: H3 signs privately and H5 alone writes.
    assert_eq!(
        canonical(&store),
        records,
        "signing exposed a durable prefix"
    );
}

/// H4 refuses a batch that is not finished signing. `finish` would fail anyway, but it would fail
/// somewhere inside manifest construction; refusing here names the actual mistake, and a scheduled
/// caller that assembles a plan it has only partly signed is exactly the mistake worth naming.
#[test]
fn studio_overlay_handoff_assembly_refuses_a_partly_signed_batch() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let (basis, _) = performance::fixture(&f, &mut store, 4);
    let mut b = budget(&mut store, &f);
    let capture = match store
        .start_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut |m: &EpochMutation<'_>, _, p: &Path, bytes: &[u8]| m.write(p, bytes),
            &mut flush,
        )
        .unwrap()
    {
        crate::store::epoch_studio::handoff::StudioHandoffStart::Captured(capture) => capture,
        crate::store::epoch_studio::handoff::StudioHandoffStart::Settled(_) => {
            panic!("the fixture branch was classified as already transferred")
        }
    };
    let mut plan = capture.prepare().unwrap();
    // One bounded slice, deliberately short of the whole branch.
    let slice = plan
        .sign_slice(&f.device, &f.group, 0, false, 2, None)
        .unwrap();
    assert_eq!(slice.signed(), 2);
    assert_eq!(slice.remaining(), 2);
    match plan.assemble() {
        Err(error) => assert!(
            error.to_string().contains("signing did not complete"),
            "a partly signed batch was refused for an unrelated reason: {error}"
        ),
        Ok(_) => panic!("a partly signed batch was assembled"),
    }
}

/// H1/H2. A plan built from records that have since moved on is a stale proposal, however well
/// formed it is, and the commit visit must refuse it before any signature becomes durable.
///
/// This is the handoff analogue of `studio_overlay_detached_plan_is_refused_when_the_record
/// _changed`, and it is what makes the H5 stamp recheck load bearing: the synchronous adapter
/// never leaves a gap, so nothing else in this suite can exercise it.
#[test]
fn studio_overlay_handoff_plan_is_refused_when_its_records_changed() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(false);
    let mut store = open(root.path());
    let (close, basis, _) = prepare(&f, &mut store);
    let records = canonical(&store);

    // H1 under custody, then H2 detached. Nothing durable exists yet.
    let mut b = budget(&mut store, &f);
    let capture = match store
        .start_studio_handoff_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            basis,
            Some(0),
            &mut rng(),
            &mut b,
            &mut |m: &EpochMutation<'_>, _, p: &Path, bytes: &[u8]| m.write(p, bytes),
            &mut flush,
        )
        .unwrap()
    {
        crate::store::epoch_studio::handoff::StudioHandoffStart::Captured(capture) => capture,
        crate::store::epoch_studio::handoff::StudioHandoffStart::Settled(_) => {
            panic!("the fixture branch was classified as already transferred")
        }
    };
    let mut plan = capture.prepare().expect("H2 reconstructs the candidate");
    assert!(
        plan.sign_slice(&f.device, &f.group, 0, false, usize::MAX, None)
            .unwrap()
            .complete(),
        "H3 did not sign the whole branch"
    );
    let commit = plan.assemble().expect("H4 assembles the candidate");
    assert_eq!(
        canonical(&store),
        records,
        "H1 to H4 wrote something durable"
    );

    // The vault is closed and reopened while the plan is detached, which is what a crash between
    // H2 and H5 looks like. The plan still holds the previous mount, so the context it was built
    // against no longer exists even though every byte on disk is identical.
    let _ = close;
    drop(store);
    let mut store = open(root.path());
    assert_eq!(canonical(&store), records);

    let mut b = budget(&mut store, &f);
    let refused = store.commit_studio_handoff_with_io(
        SERVER,
        &f.group,
        f.target,
        &f.device,
        commit,
        Some(0),
        &mut rng(),
        &mut b,
        &mut |m: &EpochMutation<'_>, _, p: &Path, bytes: &[u8]| m.write(p, bytes),
        &mut flush,
    );
    match refused {
        Err(error) => assert!(
            error
                .to_string()
                .contains("overlay records or context changed"),
            "a stale handoff plan was refused for an unrelated reason: {error}"
        ),
        Ok(_) => panic!("a handoff plan built from superseded records was committed"),
    }
    assert_eq!(
        canonical(&store),
        records,
        "a refused handoff plan changed durable records"
    );
}

#[test]
fn studio_overlay_handoff_signs_the_whole_branch_once_and_keeps_pending_intents() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let (close, basis, expected) = prepare(&f, &mut store);
        let before_intents = store
            .load_epoch_intents(SERVER, &f.logical)
            .unwrap()
            .ledger
            .encode()
            .unwrap();
        let mut independent = f.load(&store).unwrap().unit;
        let signed = independent
            .edit_or_reseal(&f.device, &f.group, &mut rng(), &f.title(), 123)
            .unwrap();
        assert_eq!(independent.projection().unwrap(), expected);
        let mut writes = Vec::new();
        let mut b = budget(&mut store, &f);
        let outcome = store
            .handoff_studio_overlay_with_io(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                basis,
                Some(0),
                &mut rng(),
                &mut b,
                &mut |_m, step, p, bytes| {
                    writes.push(step);
                    atomic_write(p, bytes)
                },
                &mut flush,
            )
            .unwrap();
        assert_eq!(
            writes,
            [
                HandoffWrite::Prepared,
                HandoffWrite::Source,
                HandoffWrite::Completed
            ]
        );
        let saved = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert!(
            saved.overlay().is_none(),
            "handoff retained the redundant base"
        );
        assert_eq!(
            saved.ledger.encode().unwrap(),
            before_intents,
            "handoff retired pending envelopes"
        );
        assert!(!saved.is_overlay(&f.title().id(&f.device.device_id())));
        let mut actual = f.load(&store).unwrap().unit;
        assert_eq!((actual.epoch(), actual.op_count()), (1, 1));
        assert_eq!(actual.projection().unwrap(), expected);
        let retry = actual
            .edit_or_reseal(&f.device, &f.group, &mut rng(), &f.title(), 999)
            .unwrap();
        assert_eq!(
            f.signed(&signed),
            f.signed(&retry),
            "handoff changed original timestamp or signed delta"
        );
        let source_bytes = fs::read(f.path(&store)).unwrap();
        drop(store);
        let mut store = open(root.path());
        assert_eq!(transfer(&f, &mut store, basis), outcome);
        assert_eq!(fs::read(f.path(&store)).unwrap(), source_bytes);
        let mut b = budget(&mut store, &f);
        let retry = store
            .save_studio_closing_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                &close,
                None,
                basis,
                f.title(),
                999,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        assert!(matches!(retry, StudioOverlaySave::HandedOff(ref value) if value == &outcome));
    }
}

fn copy_vault(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for item in fs::read_dir(from).unwrap() {
        let item = item.unwrap();
        let next = to.join(item.file_name());
        if item.file_type().unwrap().is_dir() {
            copy_vault(&item.path(), &next);
        } else {
            fs::copy(item.path(), next).unwrap();
        }
    }
}

#[test]
fn studio_overlay_handoff_crash_barriers_reopen_without_signed_prefixes_or_duplicates() {
    for art in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut template = open(root.path());
        let (_, basis, expected) = prepare(&f, &mut template);
        drop(template);
        for step in [
            HandoffWrite::Prepared,
            HandoffWrite::Source,
            HandoffWrite::Completed,
        ] {
            for after in [false, true] {
                let attempt = tempfile::tempdir().unwrap();
                copy_vault(root.path(), attempt.path());
                let mut store = open(attempt.path());
                let mut b = budget(&mut store, &f);
                let mut hit = false;
                let error = store
                    .handoff_studio_overlay_with_io(
                        SERVER,
                        &f.group,
                        f.target,
                        &f.device,
                        basis,
                        Some(0),
                        &mut rng(),
                        &mut b,
                        &mut |_m, at, p, bytes| {
                            if at == step {
                                hit = true;
                                if after {
                                    atomic_write(p, bytes)?;
                                }
                                return Err(invalid("injected handoff write"));
                            }
                            atomic_write(p, bytes)
                        },
                        &mut flush,
                    )
                    .unwrap_err();
                assert!(hit && error.to_string().contains("injected handoff write"));
                assert!(b.requires_reconciliation());
                drop(store);
                let mut store = open(attempt.path());
                let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
                let source = f.load(&store).unwrap();
                assert!(source.op_count() <= 1);
                if step == HandoffWrite::Completed && after {
                    assert!(state.overlay().is_none());
                } else {
                    assert_eq!(
                        state.local_draft().unwrap().unwrap().projection(),
                        &expected
                    );
                }
                transfer(&f, &mut store, basis);
                assert_eq!(f.load(&store).unwrap().op_count(), 1);
                assert_eq!(f.load(&store).unwrap().projection().unwrap(), expected);
                assert_eq!(f.intents(&store), 1);
            }
        }
        for after in [false, true] {
            let attempt = tempfile::tempdir().unwrap();
            copy_vault(root.path(), attempt.path());
            let mut store = open(attempt.path());
            let mut b = budget(&mut store, &f);
            let mut hit = false;
            let error = store
                .handoff_studio_overlay_with_io(
                    SERVER,
                    &f.group,
                    f.target,
                    &f.device,
                    basis,
                    Some(0),
                    &mut rng(),
                    &mut b,
                    &mut |m: &EpochMutation<'_>, _, p: &Path, bytes: &[u8]| m.write(p, bytes),
                    &mut |m, step, p, bytes| {
                        assert_eq!(step, HandoffSync::Source);
                        hit = true;
                        if after {
                            flush(m, step, p, bytes)?;
                        }
                        Err(invalid("injected handoff sync"))
                    },
                )
                .unwrap_err();
            assert!(hit && error.to_string().contains("injected handoff sync"));
            drop(store);
            let mut store = open(attempt.path());
            assert!(store
                .load_epoch_intents(SERVER, &f.logical)
                .unwrap()
                .handoff_prepared());
            assert_eq!(f.load(&store).unwrap().op_count(), 1);
            transfer(&f, &mut store, basis);
            assert_eq!(f.load(&store).unwrap().projection().unwrap(), expected);
        }
    }
}

/// Reach a real subsequent receipt, preserving the transferred operation in its signed closure.
fn grow(f: &Fixture, store: &mut ServerStore) {
    let mut b = budget(store, f);
    let mut state = f.load(store).unwrap();
    let before = state.unit.snapshot().unwrap();
    for n in 140..150 {
        let mut op = f.title();
        op.nonce = [n; 16];
        let mut copy = StudioEpoch::restore(
            &state.unit.snapshot().unwrap(),
            &f.group,
            f.target,
            f.device.device_id(),
        )
        .unwrap();
        let packet = copy
            .edit_or_reseal(&f.device, &f.group, &mut rng(), &op, 400)
            .unwrap();
        let mut expanded = automerge::Change::from_bytes(f.signed(&packet).delta)
            .unwrap()
            .decode();
        expanded.message = Some("x".repeat(220_000));
        let change = automerge::Change::from(expanded);
        let signed = SignedOp::sign_domain(
            &f.device,
            f.logical.doc_type,
            state.doc_id(),
            change.raw_bytes().to_vec(),
            &op,
        )
        .unwrap();
        let packet = SealedOp::seal(&signed, &f.group, &f.device, &mut rng()).unwrap();
        assert_eq!(
            state.unit.ingest(&packet, &f.group, &f.device).unwrap(),
            Admission::Accepted
        );
    }
    let observed = state.source.as_ref().map(SourceVersion::record);
    let state = store
        .save_studio_source(
            SERVER,
            state.unit,
            observed,
            &before,
            WritePurpose::Ordinary,
            &mut rng(),
            &mut b.storage,
            |m: &EpochMutation<'_>, p: &Path, b: &[u8]| m.write(p, b),
            |m: &EpochMutation<'_>, p: &Path, b: u64| m.sync_studio(p, b),
        )
        .unwrap();
    store.retain_studio_source(&f.group, &f.device, state);
}

#[test]
fn studio_overlay_handoff_completed_retry_keeps_channel_after_real_receipt_retirement() {
    let root = tempfile::tempdir().unwrap();
    let f = Fixture::new(true);
    let mut store = open(root.path());
    let (close, basis, _) = prepare(&f, &mut store);
    let outcome = transfer(&f, &mut store, basis);
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .overlay()
        .is_none());
    grow(&f, &mut store);
    let (_, next) = rotate(&f, &mut store);
    assert_eq!(next.epoch(), 2);
    assert_eq!(
        f.intents(&store),
        0,
        "fixture did not legitimately retire the transferred operation"
    );
    drop(store);
    let mut store = open(root.path());
    let saved = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    assert!(saved.overlay().is_none() && !saved.handoff_prepared() && saved.pending().len() == 0);
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let path = store.epoch_intent_path(&scope);
    let original = fs::read(&path).unwrap();
    let mut b = budget(&mut store, &f);
    let mut syncs = 0;
    let positive = store
        .save_studio_closing_overlay_with_io(
            SERVER,
            &f.group,
            f.target,
            &f.device,
            &close,
            None,
            basis,
            f.title(),
            999,
            &mut rng(),
            &mut b,
            |_, _, _| panic!("completed retry rewrote its record"),
            |m, p, bytes| {
                syncs += 1;
                sync_intent(m, p, bytes)
            },
        )
        .unwrap();
    assert!(matches!(positive, StudioOverlaySave::HandedOff(ref value) if value == &outcome));
    assert_eq!(syncs, 1);
    let StudioTarget::Flipnote { object, .. } = f.target else {
        unreachable!()
    };
    let wrong = StudioTarget::Flipnote {
        channel: [88; 16],
        object,
    };
    assert_eq!(wrong.document(&f.group.group_id()).unwrap(), f.logical);
    let mut b = budget(&mut store, &f);
    syncs = 0;
    let negative = store.save_studio_closing_overlay_with_io(
        SERVER,
        &f.group,
        wrong,
        &f.device,
        &close,
        None,
        basis,
        f.title(),
        999,
        &mut rng(),
        &mut b,
        |_, _, _| panic!("wrong-channel retry wrote"),
        |m, p, bytes| {
            syncs += 1;
            sync_intent(m, p, bytes)
        },
    );
    assert!(
        matches!(negative, Err(AppError::Invalid(ref s)) if s == &format!("epoch studio: {}",ReplError::EpochScope)),
        "completed retry acknowledged a different channel: {negative:?}"
    );
    assert_eq!(syncs, 0, "wrong-channel retry reached its sync callback");
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(store
        .load_epoch_intents(SERVER, &f.logical)
        .unwrap()
        .overlay()
        .is_none());
}
