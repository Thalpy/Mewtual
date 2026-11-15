//! Real original MLS nodes/keys; these assert retained primitive behavior, not native authority.
use super::super::tests::{found, reserve};
use super::*;

fn counts(value: &RetainedFounderSigning) -> (usize, usize) {
    (value.profile.calls, value.header.calls)
}

#[test]
fn retained_bootstrap_each_postraw_refusal_reuses_both_exact_original_signatures() {
    for kind in [0, 1] {
        let node = found(0x3b01 + u64::from(kind));
        let mut signing = node.begin_owner_v2_founder_signing().unwrap();
        FAULT.with(|fault| fault.set(Some((kind, Fault::RefuseAfterCache))));
        assert!(node
            .prepare_owner_v2_founder_retained(&mut signing)
            .is_err());
        let profile = signing.profile.raw.unwrap();
        assert_eq!(counts(&signing), if kind == 0 { (1, 0) } else { (1, 1) });
        let prepared = node
            .prepare_owner_v2_founder_retained(&mut signing)
            .unwrap();
        assert_eq!(prepared.profile.signature(), &profile);
        assert_eq!(counts(&signing), (1, 1));
        let frozen = prepared.freeze(&node).unwrap();
        let expected = frozen
            .borrow_for_covering_save(&node)
            .unwrap()
            .header
            .to_vec();
        for _ in 0..3 {
            let again = node
                .prepare_owner_v2_founder_retained(&mut signing)
                .unwrap()
                .freeze(&node)
                .unwrap();
            assert_eq!(
                again.borrow_for_covering_save(&node).unwrap().header,
                expected
            );
        }
        assert_eq!(counts(&signing), (1, 1));
    }
}

#[test]
fn retained_bootstrap_entered_primitive_unwind_never_reenters_that_slot() {
    for kind in [0, 1] {
        let node = found(0x3b10 + u64::from(kind));
        let mut signing = node.begin_owner_v2_founder_signing().unwrap();
        FAULT.with(|fault| fault.set(Some((kind, Fault::UnwindBeforeReturn))));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = node.prepare_owner_v2_founder_retained(&mut signing);
        }))
        .is_err());
        let before = counts(&signing);
        assert_eq!(before, if kind == 0 { (1, 0) } else { (1, 1) });
        for _ in 0..3 {
            assert!(node
                .prepare_owner_v2_founder_retained(&mut signing)
                .is_err());
        }
        assert_eq!(counts(&signing), before);
        assert!(if kind == 0 {
            signing.profile.raw.is_none()
        } else {
            signing.header.raw.is_none()
        });
    }
}

#[test]
fn retained_bootstrap_foreign_node_refuses_after_profile_without_touching_either_key() {
    let node = found(0x3b20);
    let foreign = found(0x3b21);
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    FAULT.with(|fault| fault.set(Some((0, Fault::RefuseAfterCache))));
    assert!(node
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    assert!(foreign
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    assert_eq!(counts(&signing), (1, 0));
    node.prepare_owner_v2_founder_retained(&mut signing)
        .unwrap();
    assert_eq!(counts(&signing), (1, 1));
}

#[test]
fn retained_bootstrap_equal_scope_labels_do_not_adopt_a_replacement_reservation() {
    let mut node = found(0x3b30);
    let _original = reserve(&mut node);
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    FAULT.with(|fault| fault.set(Some((0, Fault::RefuseAfterCache))));
    assert!(node
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    node.local_chat_scope = Default::default();
    let _replacement = reserve(&mut node);
    assert!(node
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    assert_eq!(counts(&signing), (1, 0));
}

#[test]
fn retained_bootstrap_invalid_cached_signature_refuses_without_resigning() {
    let node = found(0x3b40);
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    FAULT.with(|fault| fault.set(Some((0, Fault::RefuseAfterCache))));
    assert!(node
        .prepare_owner_v2_founder_retained(&mut signing)
        .is_err());
    signing.profile.raw.as_mut().unwrap()[0] ^= 1;
    for _ in 0..3 {
        assert!(node
            .prepare_owner_v2_founder_retained(&mut signing)
            .is_err());
    }
    assert_eq!(counts(&signing), (1, 0));
    assert!(signing.profile.raw.is_some());
}

#[test]
fn retained_bootstrap_does_not_change_the_stateless_library_contract() {
    let node = found(0x3b50);
    let first = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    let second = node
        .prepare_owner_v2_founder()
        .unwrap()
        .freeze(&node)
        .unwrap();
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    let retained = node
        .prepare_owner_v2_founder_retained(&mut signing)
        .unwrap()
        .freeze(&node)
        .unwrap();
    let one = first.borrow_for_covering_save(&node).unwrap();
    for other in [&second, &retained] {
        let two = other.borrow_for_covering_save(&node).unwrap();
        assert_eq!(one.profile, two.profile);
        assert_eq!(one.header, two.header);
        assert_eq!(one.state, two.state);
    }
    assert_eq!(counts(&signing), (1, 1));
}

// Actual MLS primitives and concrete lifecycle cuts, but synthetic host atomics:
// these are not evidence for the absent native checked factory/Root origin.
struct NativeFixture {
    binding: catcoms_rt::NativeChatUiBinding,
    ui: Option<catcoms_rt::NativeChatUiSessionOwner>,
    source: Option<catcoms_rt::NativeChatSourceOwner>,
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    authorized: std::sync::Arc<std::sync::atomic::AtomicU64>,
    requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl NativeFixture {
    fn new() -> Self {
        use std::sync::{
            atomic::{AtomicBool, AtomicU64},
            Arc,
        };
        let generation = Arc::new(AtomicU64::new(7));
        let authorized = Arc::new(AtomicU64::new(7));
        let requested = Arc::new(AtomicBool::new(false));
        let binding = catcoms_rt::NativeChatUiBinding::from_trusted_host(
            generation.clone(),
            authorized.clone(),
            requested.clone(),
            Arc::new(()),
        );
        let ui = binding.open_session(7, Arc::new(())).unwrap();
        let source =
            catcoms_rt::NativeChatSourceOwner::for_trusted_source(&ui.observe(), Arc::new(()))
                .unwrap();
        Self {
            binding,
            ui: Some(ui),
            source: Some(source),
            generation,
            authorized,
            requested,
        }
    }
    fn cuts(
        &self,
    ) -> (
        catcoms_rt::NativeChatPreparedKeyCut,
        catcoms_rt::NativeChatPreparedKeyCut,
    ) {
        use catcoms_rt::NativeChatKeyPurpose::{BootstrapHeader, BootstrapProfile};
        let source = self.source.as_ref().unwrap();
        let command = self.ui.as_ref().unwrap().observe();
        (
            source.prepare_cut(&command, BootstrapProfile).unwrap(),
            source.prepare_cut(&command, BootstrapHeader).unwrap(),
        )
    }
    fn reauthenticate(&mut self) {
        use std::sync::atomic::Ordering::SeqCst;
        let generation = self.generation.load(SeqCst);
        self.authorized.store(generation, SeqCst);
        self.requested.store(false, SeqCst);
        self.ui = Some(self.binding.open_session(generation, Arc::new(())).unwrap());
    }
}

#[test]
fn native_retained_bootstrap_lock_at_each_actual_primitive_leaves_it_unentered() {
    for kind in [0, 1] {
        use std::sync::atomic::Ordering::SeqCst;
        let node = found(0x3c00 + u64::from(kind));
        let mut signing = node.begin_owner_v2_founder_signing().unwrap();
        let mut native = NativeFixture::new();
        let original = native.source.as_ref().unwrap().observe();
        let (mut profile, mut header) = native.cuts();
        let generation = native.generation.clone();
        NATIVE_KEY_BOUNDARY.with(|slot| {
            *slot.borrow_mut() = Some((
                kind,
                Box::new(move || {
                    // Actual final native entry sees generation publication AFTER the
                    // node/SDK source checks and canonical full-key hashing.
                    generation.fetch_add(1, SeqCst);
                }),
            ))
        });
        let mut ends = catcoms_rt::NativeChatKeyEnds::new();
        assert!(node
            .prepare_owner_v2_founder_native_retained(
                &mut signing,
                &original,
                &mut profile,
                &mut header,
                &mut ends
            )
            .is_err());
        assert!(NATIVE_KEY_BOUNDARY.with(|slot| slot.borrow().is_none()));
        assert_eq!(counts(&signing), if kind == 0 { (0, 0) } else { (1, 0) });
        assert!(if kind == 0 {
            signing.profile.entered.is_none()
        } else {
            signing.header.entered.is_none()
        });
        drop(ends);
        native.reauthenticate();
        let (mut profile, mut header) = native.cuts();
        let mut ends = catcoms_rt::NativeChatKeyEnds::new();
        let prepared = node
            .prepare_owner_v2_founder_native_retained(
                &mut signing,
                &original,
                &mut profile,
                &mut header,
                &mut ends,
            )
            .unwrap();
        prepared.freeze(&node).unwrap();
        assert_eq!(counts(&signing), (1, 1));
    }
}

#[test]
fn native_retained_bootstrap_source_close_at_each_primitive_does_not_enter_the_key() {
    for kind in [0, 1] {
        let node = found(0x3c10 + u64::from(kind));
        let mut signing = node.begin_owner_v2_founder_signing().unwrap();
        let mut native = NativeFixture::new();
        let original = native.source.as_ref().unwrap().observe();
        let (mut profile, mut header) = native.cuts();
        let source = native.source.take().unwrap();
        NATIVE_KEY_BOUNDARY
            .with(|slot| *slot.borrow_mut() = Some((kind, Box::new(move || drop(source)))));
        let mut ends = catcoms_rt::NativeChatKeyEnds::new();
        assert!(node
            .prepare_owner_v2_founder_native_retained(
                &mut signing,
                &original,
                &mut profile,
                &mut header,
                &mut ends
            )
            .is_err());
        assert!(NATIVE_KEY_BOUNDARY.with(|slot| slot.borrow().is_none()));
        assert_eq!(counts(&signing), if kind == 0 { (0, 0) } else { (1, 0) });
        for _ in 0..3 {
            assert!(node
                .prepare_owner_v2_founder_native_retained(
                    &mut signing,
                    &original,
                    &mut profile,
                    &mut header,
                    &mut ends
                )
                .is_err());
        }
        assert_eq!(counts(&signing), if kind == 0 { (0, 0) } else { (1, 0) });
    }
}

#[test]
fn native_retained_bootstrap_wrong_source_or_purpose_never_reaches_the_primitive() {
    let node = found(0x3c20);
    let mut signing = node.begin_owner_v2_founder_signing().unwrap();
    let native = NativeFixture::new();
    let other = NativeFixture::new();
    let original = native.source.as_ref().unwrap().observe();
    let foreign = other.source.as_ref().unwrap().observe();
    let (mut profile, mut header) = native.cuts();
    let mut ends = catcoms_rt::NativeChatKeyEnds::new();
    assert!(node
        .prepare_owner_v2_founder_native_retained(
            &mut signing,
            &foreign,
            &mut profile,
            &mut header,
            &mut ends
        )
        .is_err());
    assert_eq!(counts(&signing), (0, 0));
    assert!(node
        .prepare_owner_v2_founder_native_retained(
            &mut signing,
            &original,
            &mut header,
            &mut profile,
            &mut ends
        )
        .is_err());
    assert_eq!(counts(&signing), (0, 0));
}

#[test]
fn native_retained_bootstrap_primitive_unwind_keeps_end_until_outer_return_and_never_resigns() {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};
    for kind in [0, 1] {
        let node = found(0x3c30 + u64::from(kind));
        let mut signing = node.begin_owner_v2_founder_signing().unwrap();
        let mut native = NativeFixture::new();
        let original = native.source.as_ref().unwrap().observe();
        let (mut profile, mut header) = native.cuts();
        let mut ends = catcoms_rt::NativeChatKeyEnds::new();
        FAULT.with(|fault| fault.set(Some((kind, Fault::UnwindBeforeReturn))));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = node.prepare_owner_v2_founder_native_retained(
                &mut signing,
                &original,
                &mut profile,
                &mut header,
                &mut ends,
            );
        }));
        assert!(result.is_err());
        let expected = if kind == 0 { (1, 0) } else { (1, 1) };
        assert_eq!(counts(&signing), expected);
        let drain = native.ui.take().unwrap().retire();
        let mut waiting = Box::pin(drain.wait());
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(waiting.as_mut().poll(&mut context), Poll::Pending));
        for _ in 0..3 {
            assert!(node
                .prepare_owner_v2_founder_native_retained(
                    &mut signing,
                    &original,
                    &mut profile,
                    &mut header,
                    &mut ends
                )
                .is_err());
        }
        assert_eq!(counts(&signing), expected);
        drop(ends);
        assert!(matches!(
            waiting.as_mut().poll(&mut context),
            Poll::Ready(())
        ));
    }
}
