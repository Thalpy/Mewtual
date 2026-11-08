//! Actual separate vaults: a rejected cross-original DATA read must not detach the
//! captured Files from the exact native marker that still keeps their vault busy.
use super::*;

#[test]
fn actual_workspace_read_rejects_another_vault_without_detaching_its_source_or_lease() {
    use std::os::windows::fs::OpenOptionsExt;
    let first = super::super::super::promotion::promoted_text_fixture(0, 0x7f960);
    let selected_a = selected_capture(&first);
    let query_a = completed_query(&first, &selected_a, [31; 32]);
    let lane_a = create_lane(&first, query_a.resources());
    let (producer_a, mut prepared_a) = workspace(&first, &selected_a, &query_a, "first vault");
    let mut admitted_a = read_workspace(&first, &lane_a, &mut prepared_a, &query_a);
    let owner = first.owner.borrow_current().unwrap();
    let head_a = owner.source().source().capture.target.join("head.bin");
    drop(owner);

    let second = super::super::super::promotion::promoted_text_fixture(0, 0x7f961);
    let selected_b = selected_capture(&second);
    let query_b = completed_query(&second, &selected_b, [31; 32]);
    let lane_b = create_lane(&second, query_b.resources());
    let (producer_b, mut prepared_b) = workspace(&second, &selected_b, &query_b, "second vault");
    let mut admitted_b = read_workspace(&second, &lane_b, &mut prepared_b, &query_b);
    assert!(admitted_a
        .prove_clear_data(&producer_b.observe_original())
        .is_err());
    assert!(admitted_b
        .prove_clear_data(&producer_a.observe_original())
        .is_err());
    // The API exposes no &mut Captured owner to swap. A second real marker can drain
    // independently; its destruction cannot release the first marker's vault custody.
    drop(admitted_b);
    drop(selected_a);
    drop(query_a);
    drop(producer_a);
    drop(lane_a);
    let TextFixture {
        owner,
        layout,
        _directory: directory_a,
    } = first;
    drop(owner);
    drop(layout);
    assert!(matches!(
        catcoms_storage::acquire_vault_session(directory_a.path()),
        Err(catcoms_storage::StorageError::VaultBusy)
    ));
    assert!(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&head_a)
        .is_err());
    // Already-admitted finite work finishes after the original owner was dropped. Its
    // Source Files close before the SAME lease/End, despite both cross-vault refusals.
    prepared_a.run_read(admitted_a).unwrap();
    assert!(prepared_a.has_ready_data());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(head_a)
        .unwrap();
    drop(file);
    let session = catcoms_storage::acquire_vault_session(directory_a.path()).unwrap();
    drop(session);
}
