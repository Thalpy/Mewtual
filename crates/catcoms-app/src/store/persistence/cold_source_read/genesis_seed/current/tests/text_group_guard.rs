//! Real three-family readonly namespace guards. Changing an old genesis head is NOT a
//! selected operational O and issues no result: the selected physical consumer must reject
//! it. The namespace guard must nevertheless avoid its obsolete genesis-only decoder.
#![cfg(windows)]
mod context_hash;
mod ordinary_data;
use super::*;
use crate::store::critical_namespaces::current_layout::cold_read::tests::{
    fixture as cold_fixture, fixture_at_cut, ColdFixture, ColdFixtureCut,
};
use crate::store::persistence::v2_text::PreparedV2TextParentCapture;
use crate::store::persistence::V2TextOwnedCurrent;

struct TextFixture {
    owner: V2TextOwnedCurrent<MemNetwork, ChaCha20Rng>,
    layout: V2CurrentLayoutStore,
    // LAST: original owner, selectors and all file/resource guards drain before cleanup.
    _directory: tempfile::TempDir,
}

/// No decoded family role or adopted node is accepted. Every branch crosses its actual
/// founder/restore/repair and settlement barriers, then moves the whole original wrapper.
fn text_fixture(family: usize, number: u64) -> TextFixture {
    if family == 0 {
        let mut f = fixture(number);
        f.preparation.retry(&f.layout, &mut f.rng).unwrap();
        let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
        current
            .retry_seed_settlement(&f.layout, &mut f.rng)
            .unwrap();
        return TextFixture {
            owner: V2TextOwnedCurrent::from_warm(current),
            layout: f.layout,
            _directory: f._directory,
        };
    }
    let ColdFixture {
        source,
        layout,
        mut rng,
        directory,
        ..
    } = if family == 1 {
        cold_fixture(number, false)
    } else {
        assert_eq!(family, 2);
        fixture_at_cut(number, ColdFixtureCut::ShortStage)
    };
    let transport = Arc::new(Hub::new().join(PeerId::from_u64(number)));
    let rng_input = ChaCha20Rng::seed_from_u64(number);
    let clock = Arc::new(ManualClock::new(20));
    let owner = if family == 1 {
        let mut preparation = super::super::cold::PreparedColdV2GenesisCurrent::new(
            source, transport, rng_input, clock,
        );
        preparation.retry(&layout, &mut rng).unwrap();
        let mut current = preparation.take_current(&layout).unwrap().unwrap();
        current.retry_seed_settlement(&layout, &mut rng).unwrap();
        V2TextOwnedCurrent::from_cold(current)
    } else {
        let mut preparation = super::super::initializing::PreparedColdV2GenesisInitializing::new(
            source, transport, rng_input, clock,
        );
        preparation.retry(&layout, &mut rng).unwrap();
        let mut current = preparation.take_current(&layout).unwrap().unwrap();
        current.retry_seed_settlement(&layout, &mut rng).unwrap();
        V2TextOwnedCurrent::from_repaired(current)
    };
    TextFixture {
        owner,
        layout,
        _directory: directory,
    }
}

#[test]
fn actual_three_text_families_borrow_original_context_without_old_head_decoder() {
    for family in 0..3 {
        let f = text_fixture(family, 0x7e8c0 + family as u64);
        let store = f.layout.checked_store().unwrap();
        let owner = f.owner.borrow_current().unwrap();
        let mut capture = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
        let resources = capture.resources().clone();
        let target_head = owner.source().source().capture.target.join("head.bin");
        let original_head = std::fs::read(&target_head).unwrap();
        let original_nonce = store
            .with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                let (expected, frame, nonce) = guard.original_genesis_context_data()?;
                assert_eq!(frame, original_head);
                assert_eq!(expected.source_id, owner.source().data().source);
                assert_eq!(expected.context, *owner.source().data().context);
                assert!(!guard.original_pin_data().2.is_empty());
                Ok(nonce)
            })
            .unwrap();

        // An operational selector will replace this path. Malformed successor bytes are
        // used here only to isolate the read-only guard's dependency: THEY ARE NOT O_G.
        // A physical selected-head failure may never become Empty or a saved receipt.
        std::fs::write(
            &target_head,
            b"malformed operational selector, never accepted",
        )
        .unwrap();
        store
            .with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                let (expected, frame, nonce) = guard.original_genesis_context_data()?;
                assert_eq!(frame, original_head);
                assert_eq!(nonce, original_nonce);
                assert_eq!(expected.source_id, owner.source().data().source);
                guard.validate_original_namespace()?;
                assert!(
                    capture.capture(&guard).is_err(),
                    "family {family}: bad O is not absence"
                );
                Ok(())
            })
            .unwrap();
        std::fs::write(target_head, original_head).unwrap();
    }
}

#[test]
fn actual_three_text_families_reject_equal_bytes_in_replaced_original_pin() {
    for family in 0..3 {
        let f = text_fixture(family, 0x7e8d0 + family as u64);
        let store = f.layout.checked_store().unwrap();
        let owner = f.owner.borrow_current().unwrap();
        let capture = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
        let resources = capture.resources().clone();
        let mut original = None;
        store
            .with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                let path = guard.directory_data().0.join("pin.bin");
                let moved = guard.directory_data().0.join("pin-held-by-test");
                let frame = guard.original_pin_data().2.to_vec();
                std::fs::rename(&path, &moved).unwrap();
                std::fs::write(&path, &frame).unwrap();
                // Cache test cleanup before any fallible final proof, outside retained guard
                // custody. Same bytes do not make the replacement the original selected File.
                original = Some((path, moved));
                assert!(
                    guard.validate_original_namespace().is_err(),
                    "family {family}"
                );
                assert_eq!(guard.original_pin_data().2, frame);
                Ok(())
            })
            .unwrap();
        let (path, moved) = original.unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(moved, path).unwrap();
        store
            .with_v2_text_current(&owner, |admission| {
                let guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.validate_original_namespace()
            })
            .unwrap();
    }
}

#[test]
fn actual_text_final_current_layout_proof_follows_original_source_read() {
    let f = text_fixture(0, 0x7e8e0);
    let store = f.layout.checked_store().unwrap();
    let owner = f.owner.borrow_current().unwrap();
    let capture = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
    let resources = capture.resources().clone();
    let marker = owner
        .source()
        .source()
        .capture
        .root
        .join(crate::store::critical_namespaces::current_layout::CURRENT_MARKER);
    let moved = marker.with_file_name("current-layout-original-held-by-test");
    let frame = std::fs::read(&marker).unwrap();
    let hook_marker = marker.clone();
    let hook_moved = moved.clone();
    store
        .with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.after_original_source_for_test(Box::new(move || {
                std::fs::rename(&hook_marker, &hook_moved).unwrap();
                std::fs::write(hook_marker, frame).unwrap();
            }));
            assert!(guard.validate_original_namespace().is_err());
            Ok(())
        })
        .unwrap();
    assert!(moved.is_file(), "the actual post-source hook must have run");
    std::fs::remove_file(&marker).unwrap();
    std::fs::rename(moved, marker).unwrap();
    store
        .with_v2_text_current(&owner, |admission| {
            let guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.validate_original_namespace()
        })
        .unwrap();
}

// Paired actual stage/commit and original-session admission regressions.
mod promotion;

// Actual owning first-promotion facade; no ordinary posting/native completion claim.
mod host;

// Same unique unused exclusion, not an ordinary accepted-Create/retirement claim.
mod create_lane;

// Real selected-parent capture/root pins; empty DATA never authorizes a new Create.
mod query;
