// Appended only to the pinned predecessor's existing cfg(test) cell/tests.rs.
// This reader owns the verdict table; the head supplies no verdict manifest.
mod native4_predecessor {
    use super::*;
    use std::collections::BTreeMap;

    const CASES: [(&str, bool, usize); 6] = [
        ("selected3", true, 1),
        ("data-pending", true, 2),
        ("data-active", true, 2),
        ("native-pending", false, 2),
        ("native-stage", false, 2),
        ("native-active", false, 2),
    ];
    fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        fn walk(base: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
            for item in fs::read_dir(path).unwrap() {
                let item = item.unwrap();
                let path = item.path();
                let name = path.strip_prefix(base).unwrap().to_owned();
                if item.file_type().unwrap().is_dir() {
                    out.insert(name, None);
                    walk(base, &path, out);
                } else {
                    assert!(item.file_type().unwrap().is_file());
                    out.insert(name, Some(fs::read(path).unwrap()));
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(root, root, &mut files);
        files
    }
    #[test]
    fn native4_predecessor_gate() {
        use crate::store::{
            CriticalCatalogReadPool, NativeIndexPolicy, NativeRegistryPolicy, NativeRegistryStore,
        };
        let root = PathBuf::from(
            std::env::var_os("MEWTUAL_NATIVE4_FIXTURES")
                .expect("explicit fresh Native4 fixture destination"),
        );
        let names: std::collections::BTreeSet<_> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                assert!(
                    entry.file_type().unwrap().is_dir(),
                    "fixture must be a real directory"
                );
                entry.file_name().into_string().unwrap()
            })
            .collect();
        assert_eq!(
            names,
            CASES.iter().map(|case| case.0.to_owned()).collect(),
            "every fixture must have a pinned verdict"
        );
        for (name, lifecycle_ok, rows) in CASES {
            let path = root.join(name);
            assert!(path.is_dir(), "missing fixture {name}");
            for opener in 0..3 {
                let before = tree(&path);
                let result = match opener {
                    0 => NativeRegistryStore::open(
                        &path,
                        SECRET,
                        &mut NeverRng,
                        current_policy(),
                        NativeRegistryPolicy::new(
                            CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity())
                                .unwrap(),
                            NativeRegistryPolicy::required_persistent_capacity(),
                        )
                        .unwrap(),
                    )
                    .and_then(|owner| owner.row_count_data()),
                    1 => NativeIndexStore::open(
                        &path,
                        SECRET,
                        &mut NeverRng,
                        current_policy(),
                        NativeIndexPolicy::new(
                            CriticalCatalogReadPool::new(NativeIndexPolicy::required_capacity())
                                .unwrap(),
                            NativeIndexPolicy::required_persistent_capacity(),
                        )
                        .unwrap(),
                    )
                    .and_then(|owner| owner.row_count_data()),
                    _ => NativeIndexStore::open_lifecycle3_for_test(
                        &path,
                        SECRET,
                        &mut NeverRng,
                        current_policy(),
                    )
                    .and_then(|mut owner| owner.row_count_data()),
                };
                let accepted = opener == 2 && lifecycle_ok;
                assert_eq!(
                    result.is_ok(),
                    accepted,
                    "{name}, opener {opener}: {result:?}"
                );
                if accepted {
                    assert_eq!(result.unwrap(), rows);
                } else {
                    assert_eq!(
                        tree(&path),
                        before,
                        "refusal changed {name}, opener {opener}"
                    );
                }
            }
        }
        println!("native4 predecessor gate: 6 fixtures x 3 openers checked");
    }
}
