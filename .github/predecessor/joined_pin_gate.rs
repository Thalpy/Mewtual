#[test]
fn joined_pin_predecessor_checks_control_and_refuses_both_new_states() {
    let root = std::path::PathBuf::from(std::env::var_os("MEWTUAL_JOINED_PIN_FIXTURES").unwrap());
    let control = NativeTextGovernedStorage::open(
        root.join("governed"),
        SECRET,
        &mut NeverRng,
        current_policy(),
    )
    .unwrap();
    assert_eq!(control.row_count_data().unwrap(), 0);
    drop(control);
    for kind in ["staged", "selected"] {
        assert!(
            NativeTextGovernedStorage::open(
                root.join(kind),
                SECRET,
                &mut NeverRng,
                current_policy()
            )
            .is_err(),
            "predecessor accepted {kind}"
        );
    }
}
