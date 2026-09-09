use super::*;

#[test]
fn studio_detached_preparation_rechecks_full_record_membership_and_mount() {
    for mode in 0..5 {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut f = Fixture::new(true);
        let mut b = budget(&mut store, &f);
        f.edit(&mut store, &mut b, f.insert());
        let capture = store
            .capture_studio_source(SERVER, &f.group, f.target, &f.device)
            .unwrap()
            .unwrap();
        let prepared = std::thread::spawn(move || capture.rebuild().unwrap())
            .join()
            .unwrap();
        match mode {
            1 => {
                let mut b = budget(&mut store, &f);
                f.edit(&mut store, &mut b, f.title());
            }
            2 => {
                drop(store);
                store = open(root.path());
            }
            3 => {
                let other = MlsDevice::generate().unwrap();
                f.group
                    .add_member(&f.device, other.key_package().unwrap())
                    .unwrap();
            }
            4 => {
                let mut bytes = fs::read(f.path(&store)).unwrap();
                bytes[0] ^= 1;
                fs::write(f.path(&store), bytes).unwrap();
            }
            _ => {}
        }
        let result = store.install_prepared_studio_source(&f.group, &f.device, prepared);
        if mode == 0 {
            assert!(result.unwrap());
            assert!(store.studio_source_is_warm(SERVER, &f.group, f.target, &f.device));
        } else if mode == 4 {
            assert!(result.is_err(), "stale result mode {mode} attached");
        } else {
            assert!(!result.unwrap(), "healthy supersession is not a disk error");
        }
    }
}
