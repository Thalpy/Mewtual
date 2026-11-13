//! Real general-verifier Plans and retained MLS keys with synthetic UI atomics.
//! These do not establish native command capture or Root NativeRequired binding.
use super::*;
use catcoms_rt::{
    NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatKeyRefusal, NativeChatPreparedKeyCut,
    NativeChatSourceOwner, NativeChatUiBinding, NativeChatUiSessionOwner,
};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::task::{Context, Poll, Waker};

struct NativeFixture {
    binding: NativeChatUiBinding,
    ui: Option<NativeChatUiSessionOwner>,
    source: NativeChatSourceOwner,
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
            source,
            generation,
            authorized,
        }
    }
    fn cut(&self, purpose: NativeChatKeyPurpose) -> NativeChatPreparedKeyCut {
        self.source
            .prepare_cut(&self.ui.as_ref().unwrap().observe(), purpose)
            .unwrap()
    }
    fn reauthenticate(&mut self) {
        let generation = self.generation.load(SeqCst);
        self.authorized.store(generation, SeqCst);
        self.ui = Some(self.binding.open_session(generation).unwrap());
    }
}

#[test]
fn native_member_late_generation_refusal_keeps_the_same_core_plan_prepared() {
    let hub = Hub::new();
    let (mut node, _clock, g) = member_node(&hub, 0x9c01, 1);
    let instance = node.registry_instance();
    let signer = node.ordinary_member_signer_data(&instance).unwrap();
    let mut inputs = V2MemberDraftInputs::new();
    let drawn = node
        .draw_ordinary_draft_inputs(&instance, &mut inputs)
        .unwrap();
    let draft = create_draft(&g, &g.founder, signer, drawn, "native ordinary action");
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
        .unwrap();
    let original_plan = plan.observe_original();
    let mut native = NativeFixture::new();
    let original = native.source.observe();
    let mut cut = native.cut(NativeChatKeyPurpose::MemberAction);
    let generation = native.generation.clone();
    BEFORE_NATIVE_MEMBER_KEY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            generation.fetch_add(1, SeqCst);
        }))
    });
    let mut ends = NativeChatKeyEnds::new();
    let calls = key_calls_for_test();
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    assert_eq!(
        node.sign_ordinary_admitted_native(&inputs, guard, &original, &mut cut, &mut ends)
            .unwrap_err(),
        V2MemberSignRefusal::Native(NativeChatKeyRefusal::Unavailable)
    );
    assert!(BEFORE_NATIVE_MEMBER_KEY.with(|slot| slot.borrow().is_none()));
    assert_eq!(key_calls_for_test(), calls);
    assert_eq!(inputs.drawn_data(), Some(drawn));
    native.reauthenticate();
    let mut current = native.cut(NativeChatKeyPurpose::MemberAction);
    // This succeeds only if native refusal happened BEFORE Core's KeyEntered.
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    drop(
        node.sign_ordinary_admitted_native(&inputs, guard, &original, &mut current, &mut ends)
            .unwrap(),
    );
    assert_eq!(key_calls_for_test(), calls + 1);
    assert!(plan
        .complete_cached_raw64_data(&original_plan, &draft)
        .unwrap()
        .verify());
}

#[test]
fn native_member_wrong_purpose_or_original_source_never_enters_core_key() {
    let hub = Hub::new();
    let (mut node, _clock, g) = member_node(&hub, 0x9c02, 1);
    let instance = node.registry_instance();
    let signer = node.ordinary_member_signer_data(&instance).unwrap();
    let mut inputs = V2MemberDraftInputs::new();
    let drawn = node
        .draw_ordinary_draft_inputs(&instance, &mut inputs)
        .unwrap();
    let draft = create_draft(&g, &g.founder, signer, drawn, "bound native action");
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
        .unwrap();
    let native = NativeFixture::new();
    let foreign = NativeFixture::new();
    let original = native.source.observe();
    let mut cut = native.cut(NativeChatKeyPurpose::MemberAction);
    let mut wrong = native.cut(NativeChatKeyPurpose::BootstrapHeader);
    let mut ends = NativeChatKeyEnds::new();
    let calls = key_calls_for_test();
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    assert_eq!(
        node.sign_ordinary_admitted_native(
            &inputs,
            guard,
            &foreign.source.observe(),
            &mut cut,
            &mut ends
        )
        .unwrap_err(),
        V2MemberSignRefusal::Native(NativeChatKeyRefusal::Foreign)
    );
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    assert_eq!(
        node.sign_ordinary_admitted_native(&inputs, guard, &original, &mut wrong, &mut ends)
            .unwrap_err(),
        V2MemberSignRefusal::Native(NativeChatKeyRefusal::WrongPurpose)
    );
    assert_eq!(key_calls_for_test(), calls);
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    drop(
        node.sign_ordinary_admitted_native(&inputs, guard, &original, &mut cut, &mut ends)
            .unwrap(),
    );
    assert_eq!(key_calls_for_test(), calls + 1);
}

#[test]
fn native_member_raw_is_cached_before_reader_and_outer_native_end_retire() {
    let hub = Hub::new();
    let (mut node, _clock, g) = member_node(&hub, 0x9c03, 1);
    let instance = node.registry_instance();
    let signer = node.ordinary_member_signer_data(&instance).unwrap();
    let mut inputs = V2MemberDraftInputs::new();
    let drawn = node
        .draw_ordinary_draft_inputs(&instance, &mut inputs)
        .unwrap();
    let draft = create_draft(&g, &g.founder, signer, drawn, "native cached action");
    let mut plan = g
        .verifier
        .prepare_ordinary_signing_plan(&draft, &g.founder, &[], &g.view)
        .unwrap();
    let original_plan = plan.observe_original();
    let mut native = NativeFixture::new();
    let original = native.source.observe();
    let mut cut = native.cut(NativeChatKeyPurpose::MemberAction);
    let mut ends = NativeChatKeyEnds::new();
    let calls = key_calls_for_test();
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    let guard = node
        .sign_ordinary_admitted_native(&inputs, guard, &original, &mut cut, &mut ends)
        .unwrap();
    assert!(guard.cached_raw64_data().is_some());
    let drain = native.ui.take().unwrap().retire();
    let mut waiting = Box::pin(drain.wait());
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    drop(guard);
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    let guard = plan.admit_signing(&draft, &signer).unwrap();
    // Lower-layer cached DATA reuse takes no fresh native/key entry. Root's
    // NativeRequired preflight must still refuse stale command/save authority.
    drop(
        node.sign_ordinary_admitted_native(&inputs, guard, &original, &mut cut, &mut ends)
            .unwrap(),
    );
    assert_eq!(key_calls_for_test(), calls + 1);
    assert!(plan
        .complete_cached_raw64_data(&original_plan, &draft)
        .unwrap()
        .verify());
    drop(ends);
    assert!(matches!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(())
    ));
}
