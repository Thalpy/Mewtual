// Appended only to the pinned predecessor's control/tests.rs.
#[test]
fn governed_add_old_reader_preserves_authentication_with_unsupported_wal_version() {
    let key = [71; 32];
    let raw = std::fs::read(
        std::env::var_os("MEWTUAL_GOVERNED_ADD_FENCE_FIXTURE").expect("explicit head fixture"),
    )
    .unwrap();
    let kind = TextControlKindData::Wal;
    let original = plain(kind, 96);
    let mut rng = ChaCha20Rng::seed_from_u64(447);
    let accepted =
        PreparedTextControlFrameData::prepare_data(kind, &key, &original, &mut rng).unwrap();
    assert_eq!(
        open_control_data(&key, accepted.id_data(), accepted.frame_data())
            .unwrap()
            .as_slice(),
        original.as_slice(),
    );

    // The predecessor sees a complete authentic frame under its known KDF.
    // Inspect the logical version before asserting that its old parser refuses.
    assert!(matches!(
        inspect_seed_stage_mac_data(&key, &raw).unwrap(),
        TextControlMacInspectionData::Authenticated
    ));
    let old_wal_key = blake3::derive_key(kind.domains().1, &key);
    let opened = unseal(&old_wal_key, &unframe(&raw).unwrap()).unwrap();
    let mut decoder = Decoder::new(&opened);
    assert_eq!(decoder.get_bytes().unwrap(), kind.domains().0);
    assert_eq!(decoder.get_u16().unwrap(), 2);
    assert!(open_control_data(&key, kind.id_data(&raw), &raw).is_err());
}
