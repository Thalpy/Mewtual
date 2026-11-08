//! Genuine source/Current/lane/lease regressions for the new recovery roles. Raw hostile
//! bytes are TEST INPUT only; no scalar flag or parsed value fabricates an admission.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextPromotionScratch, AdmittedV2TextPromotionSeedRepair,
};
use crate::store::persistence::v2_text::V2TextPromotionRecoveryClassData;

fn target(f: &PromotionFixture) -> std::path::PathBuf {
    f.original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .to_path_buf()
}
fn admit_repair(
    f: &PromotionFixture,
    repair: &mut PreparedV2TextPromotionSeedRepair,
) -> AdmittedV2TextPromotionSeedRepair {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = repair.resources().clone();
    let mut capture = Some(
        PreparedV2TextPromotionSeedRepairCapture::prepare(store, owner.source(), repair).unwrap(),
    );
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_seed_repair_into(
                &f.lane,
                repair,
                &mut capture,
                &mut admitted,
            )
        })
        .unwrap();
    assert!(capture.is_none());
    admitted.unwrap()
}
fn admit_scratch(
    f: &PromotionFixture,
    scratch: &mut PreparedV2TextPromotionScratch,
) -> AdmittedV2TextPromotionScratch {
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let resources = scratch.resources().clone();
    let mut capture = Some(
        PreparedV2TextPromotionScratchCapture::prepare(store, owner.source(), scratch).unwrap(),
    );
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_scratch_into(&f.lane, scratch, &mut capture, &mut admitted)
        })
        .unwrap();
    assert!(capture.is_none());
    admitted.unwrap()
}
fn classify(f: &PromotionFixture) -> Result<V2TextPromotionRecoveryClassData, AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = f.prepared.resources().clone();
    store.with_v2_text_current(&owner, |admission| {
        let guard = store.lock_v2_text_current_group(admission, &resources)?;
        f.prepared.classify_recovery_under_current_guard(&guard)
    })
}
fn encrypt_plain(store: &ServerStore, kind: &str, plain: &[u8], salt: u64) -> Vec<u8> {
    // Test-only construction of untrusted bytes using the ACTUAL current vault key and
    // frozen format KDF. No production helper/accessor or opaque proof is widened for this.
    let db = store.keys.db_key().unwrap();
    let key = blake3::derive_key(&format!("catcoms/private-chat-text-{kind}-key/v1"), &db);
    crate::store::frame(
        &catcoms_crypto::seal(&key, plain, &mut ChaCha20Rng::seed_from_u64(salt)).unwrap(),
    )
}
fn foreign_control(store: &ServerStore, kind: &str, version: u16, salt: u64) -> Vec<u8> {
    let mut e = catcoms_wire::Encoder::new();
    e.put_bytes(format!("catcoms/private-chat-text-{kind}/v1\0").as_bytes())
        .unwrap();
    e.put_u16(version);
    encrypt_plain(store, kind, &e.finish(), salt)
}
fn decrypt_seed(store: &ServerStore, raw: &[u8]) -> Vec<u8> {
    let db = store.keys.db_key().unwrap();
    let key = blake3::derive_key("catcoms/private-chat-text-seed-key/v1", &db);
    catcoms_crypto::unseal(&key, &crate::store::unframe(raw).unwrap()).unwrap()
}
fn directory_from_seed(plain: &[u8]) -> Vec<u8> {
    let mut d = catcoms_wire::Decoder::new(plain);
    d.get_bytes().unwrap();
    d.get_u16().unwrap();
    for _ in 0..5 {
        d.get_bytes().unwrap();
    } // C, original-ID, original-frame, H, O
    let directory = d.get_bytes().unwrap().to_vec();
    d.finish().unwrap();
    directory
}

#[test]
fn actual_scratch_preserves_authenticated_foreign_kind_version_context_and_private_page() {
    let mut f = promotion_fixture(0, 0x7ed00);
    fail_actual_stage(&mut f, V2TextPromotionTestCut::SeedStaged);
    let path = target(&f).join("text.seed.stage.v1");
    let store = f.original.layout.checked_store().unwrap();
    let original_seed = std::fs::read(&path).unwrap();
    let mut foreign_context = decrypt_seed(store, &original_seed);
    let nonce_offset = {
        let mut seed = catcoms_wire::Decoder::new(&foreign_context);
        seed.get_bytes().unwrap();
        seed.get_u16().unwrap();
        let context = seed.get_bytes().unwrap();
        let mut d = catcoms_wire::Decoder::new(context);
        d.get_bytes().unwrap();
        d.get_u16().unwrap();
        for _ in 0..9 {
            d.get_bytes().unwrap();
        }
        let nonce = d.get_bytes().unwrap();
        nonce.as_ptr() as usize - foreign_context.as_ptr() as usize
    };
    foreign_context[nonce_offset] ^= 1;
    let private_page = directory_from_seed(&decrypt_seed(store, &original_seed));
    let cases = [
        foreign_control(store, "head", 1, 0x7ed01),
        foreign_control(store, "archive", 1, 0x7ed02),
        foreign_control(store, "seed", 2, 0x7ed03),
        encrypt_plain(store, "seed", &foreign_context, 0x7ed04),
        private_page,
    ];
    let head = std::fs::read(target(&f).join("head.bin")).unwrap();
    for bytes in cases {
        std::fs::write(&path, &bytes).unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let mut scratch =
            PreparedV2TextPromotionScratch::prepare(store, owner.source(), &f.prepared).unwrap();
        let resources = scratch.resources().clone();
        let mut capture = Some(
            PreparedV2TextPromotionScratchCapture::prepare(store, owner.source(), &scratch)
                .unwrap(),
        );
        let mut admitted = None;
        assert!(store
            .with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_promotion_scratch_into(
                    &f.lane,
                    &mut scratch,
                    &mut capture,
                    &mut admitted,
                )
            })
            .is_err());
        assert!(admitted.is_none());
        assert!(
            classify(&f).is_err(),
            "foreign/authenticated data must not dispatch scratch"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(target(&f).join("head.bin")).unwrap(), head);
        assert!(!target(&f).join("text.seed").exists());
        drop(capture);
    }
}

#[test]
fn actual_seed_repair_preserves_wrong_and_oversized_child_prefix_without_any_write() {
    let mut f = promotion_fixture(0, 0x7ed10);
    fail_actual_stage(&mut f, V2TextPromotionTestCut::DirectoryCreated);
    let root = target(&f);
    let page = std::fs::read_dir(root.join("text.pages"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("p-"))
        .unwrap();
    let seed = std::fs::read(root.join("text.seed")).unwrap();
    let head = std::fs::read(root.join("head.bin")).unwrap();
    for bytes in [vec![0xff], vec![0; 16471]] {
        std::fs::write(&page, &bytes).unwrap();
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let mut repair =
            PreparedV2TextPromotionSeedRepair::prepare(store, owner.source(), &f.prepared).unwrap();
        let resources = repair.resources().clone();
        let mut capture = Some(
            PreparedV2TextPromotionSeedRepairCapture::prepare(store, owner.source(), &repair)
                .unwrap(),
        );
        let mut admitted = None;
        assert!(store
            .with_v2_text_current(&owner, |admission| {
                let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
                guard.admit_text_promotion_seed_repair_into(
                    &f.lane,
                    &mut repair,
                    &mut capture,
                    &mut admitted,
                )
            })
            .is_err());
        assert!(admitted.is_none());
        assert!(classify(&f).is_err());
        assert_eq!(std::fs::read(&page).unwrap(), bytes);
        assert_eq!(std::fs::read(root.join("text.seed")).unwrap(), seed);
        assert_eq!(std::fs::read(root.join("head.bin")).unwrap(), head);
        assert!(!root.join("head.stage").exists());
        drop(capture);
    }
}

#[test]
fn actual_post_unlink_barrier_retry_requires_cached_unlink_and_install_is_idempotent() {
    let mut f = promotion_fixture(0, 0x7ed20);
    fail_actual_stage(&mut f, V2TextPromotionTestCut::SeedPartial);
    let root = target(&f);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let mut scratch =
        PreparedV2TextPromotionScratch::prepare(store, owner.source(), &f.prepared).unwrap();
    scratch.fail_after_unlink_for_test();
    let admitted = admit_scratch(&f, &mut scratch);
    assert!(scratch.run(admitted).is_err());
    assert!(!root.join("text.seed.stage.v1").exists());
    assert!(
        scratch.install_cleanup_into(&mut f.prepared).is_err(),
        "unlink alone is not a completed parent barrier"
    );
    // A brand-new preparation has no actual unlink evidence and cannot reinterpret a
    // missing stage as successful cleanup. Its real capture/admission refuses.
    let mut fresh =
        PreparedV2TextPromotionScratch::prepare(store, owner.source(), &f.prepared).unwrap();
    let resources = fresh.resources().clone();
    let mut capture = Some(
        PreparedV2TextPromotionScratchCapture::prepare(store, owner.source(), &fresh).unwrap(),
    );
    let mut marker = None;
    assert!(store
        .with_v2_text_current(&owner, |admission| {
            let mut guard = store.lock_v2_text_current_group(admission, &resources)?;
            guard.admit_text_promotion_scratch_into(&f.lane, &mut fresh, &mut capture, &mut marker)
        })
        .is_err());
    assert!(marker.is_none());
    drop(capture);
    drop(resources);
    drop(fresh);
    drop(owner);
    let admitted = admit_scratch(&f, &mut scratch);
    scratch.run(admitted).unwrap();
    scratch.install_cleanup_into(&mut f.prepared).unwrap();
    scratch.install_cleanup_into(&mut f.prepared).unwrap();
    assert_eq!(
        classify(&f).unwrap(),
        V2TextPromotionRecoveryClassData::CleanUnstarted
    );
    drop(scratch);
    stage_and_commit(&mut f);
    let mut lane = Some(f.lane);
    finish_promotion_lane(&f.original, &f.prepared, &mut lane);
}

#[test]
fn actual_repair_lost_ready_return_survives_and_install_does_not_repeat_io() {
    let mut f = promotion_fixture(0, 0x7ed30);
    fail_actual_stage(&mut f, V2TextPromotionTestCut::DirectoryCreated);
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    let mut repair =
        PreparedV2TextPromotionSeedRepair::prepare(store, owner.source(), &f.prepared).unwrap();
    repair.fail_after_ready_for_test();
    drop(owner);
    let admitted = admit_repair(&f, &mut repair);
    assert!(repair.run(admitted).is_err());
    assert!(f.prepared.ready_selection().is_err());
    repair.install_ready_into(&mut f.prepared).unwrap();
    repair.install_ready_into(&mut f.prepared).unwrap();
    assert!(f.prepared.ready_selection().is_ok());
    assert!(f.prepared.saved_promotion().is_err());
    drop(repair);
    commit_ready(&mut f);
    let mut lane = Some(f.lane);
    finish_promotion_lane(&f.original, &f.prepared, &mut lane);
}

#[test]
fn actual_scope_cancellation_after_repair_and_scratch_admission_keeps_finite_results() {
    {
        let mut f = promotion_fixture(0, 0x7ed40);
        fail_actual_stage(&mut f, V2TextPromotionTestCut::DirectoryCreated);
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let revocation = owner.warm_scope().unwrap().revocation();
        let mut repair =
            PreparedV2TextPromotionSeedRepair::prepare(store, owner.source(), &f.prepared).unwrap();
        drop(owner);
        let admitted = admit_repair(&f, &mut repair);
        revocation.revoke();
        repair.run(admitted).unwrap();
        repair.install_ready_into(&mut f.prepared).unwrap();
        assert!(f.prepared.ready_selection().is_ok());
        assert!(
            f.prepared.saved_promotion().is_err(),
            "finite repair cannot select O"
        );
        if let Ok(owner) = f.original.owner.borrow_current() {
            assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
        };
    }
    {
        let mut f = promotion_fixture(0, 0x7ed41);
        fail_actual_stage(&mut f, V2TextPromotionTestCut::SeedPartial);
        let root = target(&f);
        let store = f.original.layout.checked_store().unwrap();
        let owner = f.original.owner.borrow_current().unwrap();
        let revocation = owner.warm_scope().unwrap().revocation();
        let mut scratch =
            PreparedV2TextPromotionScratch::prepare(store, owner.source(), &f.prepared).unwrap();
        drop(owner);
        let admitted = admit_scratch(&f, &mut scratch);
        revocation.revoke();
        scratch.run(admitted).unwrap();
        scratch.install_cleanup_into(&mut f.prepared).unwrap();
        assert!(!root.join("text.seed.stage.v1").exists());
        assert!(!root.join("text.seed").exists());
        assert!(f.prepared.ready_selection().is_err());
        if let Ok(owner) = f.original.owner.borrow_current() {
            assert!(store.with_v2_text_current(&owner, |_| Ok(())).is_err());
        };
    }
}

#[test]
fn actual_recovery_dispatch_distinguishes_clean_complete_stage_scratch_and_main() {
    let mut f = promotion_fixture(0, 0x7ed50);
    assert_eq!(
        classify(&f).unwrap(),
        V2TextPromotionRecoveryClassData::CleanUnstarted
    );
    fail_actual_stage(&mut f, V2TextPromotionTestCut::SeedStaged);
    assert_eq!(
        classify(&f).unwrap(),
        V2TextPromotionRecoveryClassData::CompleteSeedStage
    );
    let path = target(&f).join("text.seed.stage.v1");
    let bytes = std::fs::read(&path).unwrap();
    let mut damaged = bytes.clone();
    *damaged.last_mut().unwrap() ^= 1;
    std::fs::write(&path, &damaged).unwrap();
    assert_eq!(
        classify(&f).unwrap(),
        V2TextPromotionRecoveryClassData::FirstSeedScratch
    );
    std::fs::write(&path, &bytes).unwrap();
    fail_actual_stage(&mut f, V2TextPromotionTestCut::SeedSaved);
    assert_eq!(
        classify(&f).unwrap(),
        V2TextPromotionRecoveryClassData::CompleteMain
    );
    let path = target(&f).join("text.seed");
    let mut bad = std::fs::read(&path).unwrap();
    *bad.last_mut().unwrap() ^= 1;
    std::fs::write(&path, bad).unwrap();
    assert!(
        classify(&f).is_err(),
        "corrupt MAIN must never fall back to scratch"
    );
}
