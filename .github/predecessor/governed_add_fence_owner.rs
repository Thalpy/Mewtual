// Appended only to the pinned predecessor's Windows member-owner test module.
#[test]
fn governed_add_old_reader_preserves_the_stage_before_first_member_signing() {
    let raw = std::fs::read(
        std::env::var_os("MEWTUAL_GOVERNED_ADD_FENCE_FIXTURE").expect("explicit head fixture"),
    )
    .unwrap();
    let mut f = promoted(0, 0x7fed5);
    let head = f.head();
    let dispatches = member_sign_dispatches_for_test();
    let path = f.target.join("text.wal.stage.v1");
    std::fs::write(&path, &raw).unwrap();
    let body = create("first member signing on the older owner", None);
    let token = [0x55; 32];
    assert!(f
        .host
        .retry_sign_member_request(&f.layout, plain(token, &body))
        .is_err());
    assert_eq!(member_sign_dispatches_for_test(), dispatches);
    assert_eq!(std::fs::read(&path).unwrap(), raw);
    assert_eq!(f.head(), head);

    // Remove only this fixture's marker, then require the same request and
    // actual retained predecessor owner to pass the positive control.
    std::fs::remove_file(path).unwrap();
    f.send(token, &body, &[], &[]);
    assert_eq!(member_sign_dispatches_for_test(), dispatches + 1);
}
