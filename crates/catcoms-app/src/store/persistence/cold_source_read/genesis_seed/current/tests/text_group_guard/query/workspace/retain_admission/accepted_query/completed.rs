//! Genuine completed-directory DATA loans retain exact native query provenance.
use super::*;

#[test]
fn actual_completed_directory_loan_refuses_incomplete_and_other_retained_a_and_reports_real_frame_size(
) {
    let f = saved_fixture(0, 0x7fe10);
    let other = saved_fixture(0, 0x7fe20);
    let (mut query, mut reader, mut capture) = prepare_accepted(&f);
    assert!(query
        .borrow_completed_directory_path_data(&f.transaction)
        .is_err());
    assert!(query
        .borrow_completed_directory_path_data(&other.transaction)
        .is_err());
    let admitted = admit_accepted(&f, &mut query, &mut reader, &mut capture);
    reader.drain_unused_outside_writers();
    // Actual admission alone is still not a complete result/path.
    assert!(query
        .borrow_completed_directory_path_data(&f.transaction)
        .is_err());
    query.run(admitted).unwrap();
    validate_accepted(&f, &query).unwrap();
    assert!(query
        .borrow_completed_directory_path_data(&other.transaction)
        .is_err());
    let path = query
        .borrow_completed_directory_path_data(&f.transaction)
        .unwrap();
    assert!(path.next_page_data().unwrap().is_none());
    assert!(path.row_data().unwrap().is_some());
    let owner = f.original.owner.borrow_current().unwrap();
    let pages = owner.source().source().capture.target.join("text.pages");
    let observed: Vec<_> = path.original_frame_sizes_data().collect();
    assert_eq!(observed.len(), 1, "the genuine first Directory is one leaf");
    for (reference, length) in observed {
        let id: String = reference
            .page
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let actual = std::fs::read(pages.join(format!("p-{id}.bin"))).unwrap();
        assert_eq!(length, actual.len());
    }
    // Original DATA refusal never replaces or invalidates either actual A/query.
    validate_accepted(&f, &query).unwrap();
    assert!(other.transaction.saved_retained_action().is_ok());
}
