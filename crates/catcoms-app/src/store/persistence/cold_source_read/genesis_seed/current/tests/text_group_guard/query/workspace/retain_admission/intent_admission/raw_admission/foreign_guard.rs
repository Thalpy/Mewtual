//! Cross-vault guards must refuse BEFORE spending original paid proof scratch or
//! opening its payload. Exclusive native handles distinguish early identity refusal
//! (Invalid) from the former attempted original-file open (Io/sharing violation).
use super::*;
use crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard;
use crate::store::persistence::v2_text::V2TextReadResources;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

fn exclusive(path: &Path) -> std::fs::File {
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}
fn foreign_guard(
    foreign: &TextFixture,
    resources: &V2TextReadResources,
    check: impl FnOnce(&mut V2TextCurrentGroupGuard<'_, '_>) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let store = foreign.layout.checked_store()?;
    let owner = foreign.owner.borrow_current()?;
    store.with_v2_text_current(&owner, |a| {
        let mut guard = store.lock_v2_text_current_group(a, resources)?;
        check(&mut guard)
    })
}
fn identity_refusal(result: Result<(), AppError>) {
    assert!(matches!(result,Err(AppError::Invalid(_))),"wrong Runtime must refuse identity before original payload open, not sharing Io: {result:?}");
}

#[test]
fn actual_other_vault_guard_refuses_ready_a_i_s_and_saved_s_before_original_payload_open() {
    // This is a real separately mounted/settled/promoted original owner, with its
    // own actual Source/read account/Current. No fake guard or resource is constructed.
    let foreign =
        super::super::super::super::super::super::promotion::promoted_text_fixture(0, 0x7fed0);
    let store = foreign.layout.checked_store().unwrap();
    let owner = foreign.owner.borrow_current().unwrap();
    let paid = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
    let foreign_resources = paid.resources().clone();
    drop(owner);
    {
        let mut f = retain_fixture(0, 0x7fee0);
        let stage = stage_admission(&f);
        f.transaction.run_stage(stage).unwrap();
        let target = f
            .original
            .owner
            .borrow_current()
            .unwrap()
            .source()
            .source()
            .capture
            .target
            .to_path_buf();
        let held = exclusive(&target.join("text.wal"));
        identity_refusal(foreign_guard(&foreign, &foreign_resources, |guard| {
            f.transaction
                .ready_selection()?
                .validate_ready_under_current_guard(guard)
        }));
        drop(held);
        let commit = commit_admission(&f);
        f.transaction.run_commit(commit).unwrap();
    }
    {
        let mut f = intent_fixture(0, 0x7fef0);
        let stage = stage_i(&f);
        f.intent.run_stage(stage).unwrap();
        let held = exclusive(&i_target(&f).join("text.wal"));
        identity_refusal(foreign_guard(&foreign, &foreign_resources, |guard| {
            f.intent
                .ready_selection()?
                .validate_ready_under_current_guard(guard)
        }));
        drop(held);
        let commit = commit_i(&f);
        f.intent.run_commit(commit).unwrap();
        validate_saved_i(&f).unwrap();
    }
    {
        let mut f = raw_fixture(0, 0x7ff00);
        let stage = stage_s(&f);
        f.signature.run_stage(stage).unwrap();
        let target = s_target(&f);
        let held = exclusive(&target.join("text.wal"));
        identity_refusal(foreign_guard(&foreign, &foreign_resources, |guard| {
            f.signature
                .ready_selection()?
                .validate_ready_under_current_guard(guard)
        }));
        drop(held);
        let commit = commit_s(&f);
        f.signature.run_commit(commit).unwrap();
        let signature = named_artifact(&target, "s-");
        let held = exclusive(&signature);
        identity_refusal(foreign_guard(&foreign, &foreign_resources, |guard| {
            f.signature
                .saved_raw_signature()?
                .validate_selected_under_current_guard(guard)
        }));
        drop(held);
        validate_saved_s(&f).unwrap();
    }
}
