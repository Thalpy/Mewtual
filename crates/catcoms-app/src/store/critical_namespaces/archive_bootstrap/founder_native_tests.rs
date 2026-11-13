//! Genuine Root debit/SDK owners with synthetic host atomics. These establish
//! exact original-source/key ordering, not a checked AppState/native command.
use super::*;
use catcoms_rt::{
    NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatSourceOwner, NativeChatUiBinding,
    NativeChatUiSessionOwner,
};
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc,
};
use std::task::{Context, Poll, Waker};

struct NativeFixture {
    binding: NativeChatUiBinding,
    ui: Option<NativeChatUiSessionOwner>,
    source: Option<NativeChatSourceOwner>,
    generation: Arc<AtomicU64>,
    authorized: Arc<AtomicU64>,
}
impl NativeFixture {
    fn new() -> Self {
        let generation = Arc::new(AtomicU64::new(7));
        let authorized = Arc::new(AtomicU64::new(7));
        let binding = NativeChatUiBinding::from_trusted_host(
            generation.clone(),
            authorized.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        let ui = binding.open_session(7).unwrap();
        let source =
            NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(())).unwrap();
        Self {
            binding,
            ui: Some(ui),
            source: Some(source),
            generation,
            authorized,
        }
    }
    fn begin(&self, store: &ServerStore, invocation: u8) -> Creation {
        let source = self.source.as_ref().unwrap().observe();
        match store
            .prepare_archive_founder_with_origin(
                [invocation; 32],
                "native original",
                configured(),
                inputs(MlsDevice::generate().unwrap(), u64::from(invocation)),
                FounderOrigin::NativeRequired(source),
            )
            .unwrap()
        {
            ArchiveFounderBegin::Original(owner) => owner,
            ArchiveFounderBegin::Existing { .. } => panic!("expected genuine original"),
        }
    }
    fn cut(&self) -> catcoms_rt::NativeChatPreparedKeyCut {
        self.source
            .as_ref()
            .unwrap()
            .prepare_cut(
                &self.ui.as_ref().unwrap().observe(),
                NativeChatKeyPurpose::FounderSdk,
            )
            .unwrap()
    }
    fn reauthenticate(&mut self) {
        let generation = self.generation.load(Ordering::SeqCst);
        self.authorized.store(generation, Ordering::SeqCst);
        self.ui = Some(self.binding.open_session(generation).unwrap());
    }
}

#[test]
fn native_sdk_final_generation_refusal_preserves_saved_unit_and_retries_exact_original_once() {
    let (_path, store, mut rng, _pool) = fixture(0xac01);
    let mut native = NativeFixture::new();
    let mut owner = native.begin(&store, 61);
    let original = native.source.as_ref().unwrap().observe();
    let mut cut = native.cut();
    let mut ends = NativeChatKeyEnds::new();
    let generation = native.generation.clone();
    *store.founder_before_sdk_consume.lock().unwrap() = Some(Box::new(move || {
        generation.fetch_add(1, Ordering::SeqCst);
    }));
    assert!(owner
        .advance_native(
            &store,
            &mut rng,
            &mut NativeFounderSdkEntry {
                original: &original,
                cut: &mut cut,
                ends: &mut ends,
            }
        )
        .is_err());
    assert_eq!(entries(), 0);
    assert!(!owner.sdk_started);
    assert!(owner.saved_debit.is_some());
    assert!(owner.inputs.is_some());
    let saved = fs::read(debit_path(&store, &owner)).unwrap();
    let mut unchanged = rng.clone();
    drop(ends);
    native.reauthenticate();
    let mut fresh = native.cut();
    let mut ends = NativeChatKeyEnds::new();
    assert_eq!(
        owner
            .advance_native(
                &store,
                &mut rng,
                &mut NativeFounderSdkEntry {
                    original: &original,
                    cut: &mut fresh,
                    ends: &mut ends,
                }
            )
            .unwrap(),
        ArchiveFounderProgress::GroupCreatedUnexposed
    );
    assert_eq!(entries(), 1);
    assert!(owner.server.is_some());
    assert!(owner.saved_debit.is_none());
    assert_eq!(fs::read(debit_path(&store, &owner)).unwrap(), saved);
    assert_eq!(
        rng.next_u64(),
        unchanged.next_u64(),
        "native refusal must not reseal"
    );
    // A native cached result cannot fall through the public library entrypoint
    // or conversion to the library planning/signing path.
    assert!(owner.advance(&store, &mut rng).is_err());
    let returned = owner.into_ram_plan(&store).unwrap_err();
    let (mut owner, _) = returned.into_parts();
    assert_eq!(
        owner
            .advance_native(
                &store,
                &mut rng,
                &mut NativeFounderSdkEntry {
                    original: &original,
                    cut: &mut fresh,
                    ends: &mut ends,
                }
            )
            .unwrap(),
        ArchiveFounderProgress::GroupCreatedUnexposed
    );
    assert_eq!(entries(), 1);
}

#[test]
fn native_sdk_foreign_source_and_retired_source_never_spend_the_sdk_unit() {
    let (_path, store, mut rng, _pool) = fixture(0xac02);
    let mut native = NativeFixture::new();
    let other = NativeFixture::new();
    let mut owner = native.begin(&store, 62);
    let original = native.source.as_ref().unwrap().observe();
    let foreign = other.source.as_ref().unwrap().observe();
    let mut wrong_cut = other.cut();
    let mut ends = NativeChatKeyEnds::new();
    assert!(owner
        .advance_native(
            &store,
            &mut rng,
            &mut NativeFounderSdkEntry {
                original: &foreign,
                cut: &mut wrong_cut,
                ends: &mut ends,
            }
        )
        .is_err());
    assert_eq!(entries(), 0);
    assert!(!debit_path(&store, &owner).exists());
    assert!(owner
        .advance_native(
            &store,
            &mut rng,
            &mut NativeFounderSdkEntry {
                original: &original,
                cut: &mut wrong_cut,
                ends: &mut ends,
            }
        )
        .is_err());
    assert!(!debit_path(&store, &owner).exists());
    assert!(owner.advance(&store, &mut rng).is_err());
    let mut wrong_purpose = native
        .source
        .as_ref()
        .unwrap()
        .prepare_cut(
            &native.ui.as_ref().unwrap().observe(),
            NativeChatKeyPurpose::BootstrapHeader,
        )
        .unwrap();
    let mut unchanged = rng.clone();
    assert!(owner
        .advance_native(
            &store,
            &mut rng,
            &mut NativeFounderSdkEntry {
                original: &original,
                cut: &mut wrong_purpose,
                ends: &mut ends,
            }
        )
        .is_err());
    assert!(!debit_path(&store, &owner).exists());
    assert_eq!(entries(), 0);
    assert_eq!(
        rng.next_u64(),
        unchanged.next_u64(),
        "wrong purpose must precede RNG/IO"
    );
    let mut cut = native.cut();
    let source = native.source.take().unwrap();
    *store.founder_before_sdk_consume.lock().unwrap() = Some(Box::new(move || drop(source)));
    assert!(owner
        .advance_native(
            &store,
            &mut rng,
            &mut NativeFounderSdkEntry {
                original: &original,
                cut: &mut cut,
                ends: &mut ends,
            }
        )
        .is_err());
    assert_eq!(entries(), 0);
    assert!(owner.saved_debit.is_some());
    assert!(!owner.sdk_started);
    // Equal host/generation labels cannot replace the retired source allocation.
    let replacement = NativeChatSourceOwner::for_trusted_source(
        &native.ui.as_ref().unwrap().observe(),
        Arc::new(()),
    )
    .unwrap();
    let replacement_original = replacement.observe();
    let mut replacement_cut = replacement
        .prepare_cut(
            &native.ui.as_ref().unwrap().observe(),
            NativeChatKeyPurpose::FounderSdk,
        )
        .unwrap();
    assert!(owner
        .advance_native(
            &store,
            &mut rng,
            &mut NativeFounderSdkEntry {
                original: &replacement_original,
                cut: &mut replacement_cut,
                ends: &mut ends,
            }
        )
        .is_err());
    assert_eq!(entries(), 0);
}

#[test]
fn native_sdk_cached_success_refuses_stale_source_session_or_host_before_bookkeeping() {
    for action in 0..5 {
        let (_path, store, mut rng, _pool) = fixture(0xac20 + action);
        let mut native = NativeFixture::new();
        let mut owner = native.begin(&store, 64);
        let original = native.source.as_ref().unwrap().observe();
        let mut cut = native.cut();
        let mut ends = NativeChatKeyEnds::new();
        owner
            .advance_native(
                &store,
                &mut rng,
                &mut NativeFounderSdkEntry {
                    original: &original,
                    cut: &mut cut,
                    ends: &mut ends,
                },
            )
            .unwrap();
        drop(ends);
        let bytes = fs::read(debit_path(&store, &owner)).unwrap();
        let mut unchanged = rng.clone();
        match action {
            0 => {
                native.generation.fetch_add(1, Ordering::SeqCst);
            }
            1 => {
                native.authorized.store(9, Ordering::SeqCst);
            }
            2 => {
                drop(native.source.take());
            }
            3 => {
                drop(native.ui.take());
            }
            4 => {
                drop(native.binding);
            }
            _ => unreachable!(),
        }
        let mut ends = NativeChatKeyEnds::new();
        assert!(owner
            .advance_native(
                &store,
                &mut rng,
                &mut NativeFounderSdkEntry {
                    original: &original,
                    cut: &mut cut,
                    ends: &mut ends,
                }
            )
            .is_err());
        assert_eq!(entries(), 1);
        assert_eq!(fs::read(debit_path(&store, &owner)).unwrap(), bytes);
        assert_eq!(rng.next_u64(), unchanged.next_u64());
    }
}

#[test]
fn native_sdk_same_keys_follow_the_actual_root_chain_through_private_source_saved() {
    let (_path, store, mut rng, _pool) = fixture(0xac30);
    let mut native = NativeFixture::new();
    let original = native.source.as_ref().unwrap().observe();
    let mut keys = NativeFounderKeys::prepare(
        native.source.as_ref().unwrap(),
        &native.ui.as_ref().unwrap().observe(),
    )
    .unwrap();
    // Actual private SDK identity creation, cached immediately in owned inputs.
    // This is Root composition with synthetic UI atomics, NOT a native command.
    keys.enter_device().unwrap();
    let device = MlsDevice::generate().unwrap();
    let input = inputs(device, 65);
    let mut creator = match store
        .prepare_archive_founder_with_origin(
            [65; 32],
            "native source chain",
            configured(),
            input,
            FounderOrigin::NativeRequired(original),
        )
        .unwrap()
    {
        ArchiveFounderBegin::Original(owner) => owner,
        ArchiveFounderBegin::Existing { .. } => panic!("expected actual original"),
    };
    creator
        .advance_native(&store, &mut rng, &mut keys.sdk_entry())
        .unwrap();
    let debit = creator.original.data().clone();
    let debit_id = creator.debit_id.unwrap();
    let name = debit.invocation.name();
    let intent = store
        .dir
        .join(ARCHIVE_BOOTSTRAP_ROOT)
        .join(format!("{}.intent", std::str::from_utf8(&name).unwrap()));
    let mut planning = creator
        .into_ram_plan_with_access(&store, FounderAccess::Native(&mut keys))
        .unwrap();
    assert!(planning.prepare_ram_plan(&store, &mut rng).is_err());
    planning
        .save_founding_intent_with_access(&store, &mut rng, FounderAccess::Native(&mut keys))
        .unwrap();
    let mut source = planning
        .into_source_preparation_with_access(&store, FounderAccess::Native(&mut keys))
        .unwrap();
    assert!(source.prepare_private_source(&store, &mut rng).is_err());
    assert_eq!(
        source
            .save_source_terminal_with_access(&store, &mut rng, FounderAccess::Native(&mut keys))
            .unwrap(),
        ArchiveFounderSourceProgress::SourceSavedUnexposed
    );
    let exact = fs::read(&intent).unwrap();
    let data =
        codec::open_intent_data(&store.keys.db_key().unwrap(), &debit, debit_id, &exact).unwrap();
    assert!(matches!(data.kind, codec::IntentKind::SourceSaved { .. }));
    assert_eq!(data.io_spent, 1);
    assert_eq!(entries(), 1);
    assert!(source.save_source_terminal(&store, &mut rng).is_err());
    let drain = native.ui.take().unwrap().retire();
    assert!(source
        .save_source_terminal_with_access(&store, &mut rng, FounderAccess::Native(&mut keys))
        .is_err());
    assert_eq!(fs::read(&intent).unwrap(), exact);
    let mut waiting = Box::pin(drain.wait());
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    // Device/SDK/Profile/Header accounting is retained outside every real Root
    // guard through the saved result; no public library path can advance it.
    drop(keys);
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(())
    ));
}

#[test]
fn native_sdk_return_is_cached_before_outer_end_drains_even_when_the_caller_unwinds() {
    for unwind in [false, true] {
        let (_path, store, mut rng, _pool) = fixture(0xac10 + u64::from(unwind));
        let mut native = NativeFixture::new();
        let mut owner = native.begin(&store, 63);
        let original = native.source.as_ref().unwrap().observe();
        let mut cut = native.cut();
        let mut ends = NativeChatKeyEnds::new();
        let ui = native.ui.take().unwrap();
        let retained = Arc::new(std::sync::Mutex::new(None));
        let at_return = retained.clone();
        let generation = native.generation.clone();
        owner.after_sdk_cache = Some(Box::new(move || {
            generation.fetch_add(1, Ordering::SeqCst);
            let drain = ui.retire();
            let mut waiting = Box::pin(drain.wait());
            assert!(matches!(
                waiting
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
            drop(waiting);
            *at_return.lock().unwrap() = Some(drain);
            if unwind {
                panic!("injected native caller loss after SDK cache");
            }
        }));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            owner.advance_native(
                &store,
                &mut rng,
                &mut NativeFounderSdkEntry {
                    original: &original,
                    cut: &mut cut,
                    ends: &mut ends,
                },
            )
        }));
        assert_eq!(result.is_err(), unwind);
        if let Ok(result) = result {
            result.unwrap();
        }
        assert_eq!(entries(), 1);
        assert!(owner.sdk_started);
        assert!(owner.server.is_some());
        assert!(owner.inputs.is_none());
        let drain = retained.lock().unwrap().take().unwrap();
        let mut waiting = Box::pin(drain.wait());
        assert!(matches!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        drop(ends);
        assert!(matches!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(())
        ));
    }
}
