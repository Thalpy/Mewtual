//! Copied alone into the pinned ID8-aware predecessor checkout.
use catcoms_app::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    NativeTextJoinedStorage, V2CurrentLayoutPolicy, V2MemberOutboxPolicy,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const SECRET: &[u8] = b"private native registry preparation";
const CAPACITY: usize = 1 << 30;

fn policy() -> V2CurrentLayoutPolicy {
    V2CurrentLayoutPolicy::new(
        ArchiveBootstrapCatalogPolicy::new(
            CriticalCatalogReadPool::new(CAPACITY).unwrap(),
            CAPACITY,
        )
        .unwrap()
        .with_bootstrap_source_data_read(),
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

fn inventory(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn collect(root: &Path, dir: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                result.insert(relative, None);
                collect(root, &path, result);
            } else {
                assert!(kind.is_file());
                result.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    collect(root, root, &mut result);
    result
}

#[test]
fn predecessor_opens_id8_and_preserves_indexed_states_on_refusal() {
    let root = PathBuf::from(
        std::env::var_os("MEWTUAL_INDEXED_HISTORY_FIXTURES").expect("head fixture root"),
    );
    let mut rng = ChaCha20Rng::seed_from_u64(0xe506);
    let control = NativeTextJoinedStorage::open_for_native_startup(
        &root.join("id8"),
        SECRET,
        &mut rng,
        policy(),
    )
    .unwrap();
    assert_eq!(control.row_count_data().unwrap(), 0);
    drop(control);
    for state in ["claim", "stage", "id13"] {
        let path = root.join(state);
        let before = inventory(&path);
        let has_file = |name| before.get(Path::new(name)).is_some_and(Option::is_some);
        assert!(has_file("chat-critical-namespaces.bin"));
        assert_eq!(
            has_file("chat-critical-namespaces.governance.seed"),
            state != "id13"
        );
        assert_eq!(
            has_file("chat-critical-namespaces.replace.tmp"),
            state == "stage"
        );
        assert_eq!(
            has_file("chat-critical-namespaces.indexed-shared-history.marker"),
            state != "claim"
        );
        assert_eq!(
            before
                .keys()
                .any(|name| name.ends_with("archive-indexed.head")),
            state != "claim"
        );
        assert!(
            NativeTextJoinedStorage::open_for_native_startup(&path, SECRET, &mut rng, policy())
                .is_err(),
            "predecessor accepted {state}",
        );
        assert_eq!(inventory(&path), before, "predecessor changed {state}");
    }
    println!("indexed history predecessor gate: 4 fixtures checked");
}
