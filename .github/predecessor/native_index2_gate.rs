//! Copied alone into unmodified382a30a047426228be82f4bb35fb7a87a5ea2e24.
//! This predecessor genuinely understands7/v1. It owns this verdict table;
//! the head supplies fixtures, never acceptance flags or modified handlers.
use catcoms_app::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    NativeRegistryPolicy, NativeRegistryStore, PreparedV2CurrentLayoutStore, V2CurrentLayoutPolicy,
    V2MemberOutboxPolicy,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
const SECRET: &[u8] = b"native index2 real predecessor fixtures";
const CAPACITY: usize = 1 << 30;
// (fixture, native1-aware constructor accepts, Current3/member5 accepts).
// Positive controls make unsupported-reader refusal and fence removal observable.
const CASES: [(&str, bool, bool); 18] = [
    ("control35", false, true),
    ("control7v1", true, false),
    ("claim2", true, false),
    ("fence", false, false),
    ("partial-seed", false, false),
    ("seed", false, false),
    ("partial-index", false, false),
    ("index-temp", false, false),
    ("index-renamed", false, false),
    ("partial-marker", false, false),
    ("marker-temp", false, false),
    ("marker-renamed", false, false),
    ("partial-catalog", false, false),
    ("catalog-renamed", false, false),
    ("covered", false, false),
    ("seed-unlinked", false, false),
    ("settled2", false, false),
    ("unfenced-seed", true, false),
];
fn current(execution: bool) -> V2CurrentLayoutPolicy {
    let archive = ArchiveBootstrapCatalogPolicy::new(
        CriticalCatalogReadPool::new(CAPACITY).unwrap(),
        CAPACITY,
    )
    .unwrap()
    .with_bootstrap_source_data_read();
    let archive = if execution {
        archive.with_checkpoint_join_execution(2, 10_000).unwrap()
    } else {
        archive
    };
    V2CurrentLayoutPolicy::new(
        archive,
        CanonicalFilenameInventoryPolicy::new(16, 144 * 1024 * 1024 + 1024 + 40).unwrap(),
        CAPACITY,
    )
    .unwrap()
    .with_member_outbox(
        V2MemberOutboxPolicy::new(
            CriticalCatalogReadPool::new(CAPACITY).unwrap(),
            256,
            256 * 1024 * 1024,
            1 << 20,
            1,
            CAPACITY,
        )
        .unwrap(),
    )
    .unwrap()
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(base: &Path, at: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(base).unwrap().to_owned();
            if entry.file_type().unwrap().is_dir() {
                result.insert(relative, None);
                walk(base, &path, result);
            } else {
                assert!(entry.file_type().unwrap().is_file());
                result.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}
fn copy(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            assert!(entry.file_type().unwrap().is_file());
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
fn open(path: &Path, native: bool, execution: bool, expected: bool) -> bool {
    let mut rng = ChaCha20Rng::seed_from_u64(0x7bff);
    if native {
        let policy = NativeRegistryPolicy::new(
            CriticalCatalogReadPool::new(NativeRegistryPolicy::required_capacity()).unwrap(),
            NativeRegistryPolicy::required_persistent_capacity(),
        )
        .unwrap();
        match NativeRegistryStore::open(path, SECRET, &mut rng, current(execution), policy) {
            Err(_) => false,
            Ok(owner) => {
                assert!(
                    expected,
                    "unsupported data must refuse at open, not later read"
                );
                assert_eq!(owner.row_count_data().unwrap(), 1);
                true
            }
        }
    } else {
        match PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, current(execution)) {
            Err(_) => false,
            Ok(mut owner) => {
                assert!(
                    expected,
                    "unsupported data must refuse at open, not later retry"
                );
                owner.retry(&mut rng).unwrap();
                assert!(owner.into_current_layout_store().is_ok());
                true
            }
        }
    }
}
#[test]
fn native2_real_native1_predecessor_requires_the_actual_fence_and_preserves_every_fixture() {
    let root = PathBuf::from(
        std::env::var_os("MEWTUAL_NATIVE2_FIXTURES").expect("explicit head fixture destination"),
    );
    let scratch = tempfile::tempdir().unwrap();
    let mut checked = 0;
    for (name, native_expected, current_expected) in CASES {
        assert!(root.join(name).is_dir(), "fixture missing: {name}");
        for native in [false, true] {
            for execution in [false, true] {
                let target = scratch.path().join(format!("{name}-{native}-{execution}"));
                copy(&root.join(name), &target);
                let before = tree(&target);
                let expected = if native {
                    native_expected
                } else {
                    current_expected
                };
                assert_eq!(
                    open(&target, native, execution, expected),
                    expected,
                    "{name}/{native}/{execution}"
                );
                assert_eq!(
                    tree(&target),
                    before,
                    "predecessor changed {name}/{native}/{execution}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 18 * 4);
    println!("native2 predecessor gate: 18 fixtures x 4 openers checked");
}
