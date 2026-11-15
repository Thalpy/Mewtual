// Appended ONLY to the pinned predecessor's existing cfg(test) cell/tests.rs.
// Production files and the predecessor's existing tests remain byte-identical.
// This table belongs to the predecessor harness, never to a head manifest.
mod normal3_predecessor {
    use super::*;
    use std::{collections::BTreeMap, path::PathBuf};

    const CASES: [(&str, [bool; 3]); 11] = [
        ("control1", [true, false, false]),
        ("control2", [false, true, false]),
        ("selected3", [false, false, true]),
        ("renamed3", [false, false, true]),
        ("refreshed3", [false, false, true]),
        ("refresh-stage3", [false, false, false]),
        ("partial3", [false, false, true]),
        ("rename-stage3", [false, false, true]),
        ("duplicate3", [false, false, true]),
        ("bad-mac3", [false, false, true]),
        ("authenticated-invalid3", [false, false, false]),
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
    fn index3_predecessor_gate() {
        let root = PathBuf::from(
            std::env::var_os("MEWTUAL_NATIVE3_FIXTURES")
                .expect("explicit fresh fixture destination"),
        );
        for (name, expected) in CASES {
            let path = root.join(name);
            assert!(path.is_dir(), "missing positive or negative fixture {name}");
            for (opener, accepted) in expected.into_iter().enumerate() {
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
                    1 => mount::NativeIndexStore::open(
                        &path,
                        SECRET,
                        &mut NeverRng,
                        current_policy(),
                        policy(),
                    )
                    .and_then(|owner| owner.row_count_data()),
                    _ => mount::NativeIndexStore::open_lifecycle3_for_test(
                        &path,
                        SECRET,
                        &mut NeverRng,
                        current_policy(),
                    )
                    .and_then(|mut owner| owner.row_count_data()),
                };
                assert_eq!(
                    result.is_ok(),
                    accepted,
                    "{name}, actual opener {opener}: {result:?}"
                );
                if accepted {
                    assert_eq!(result.unwrap(), 1);
                } else {
                    assert_eq!(
                        tree(&path),
                        before,
                        "refusal must preserve {name}, opener {opener}"
                    );
                }
            }
        }
        println!("native3 predecessor gate: 11 fixtures x 3 openers checked");
    }
}
