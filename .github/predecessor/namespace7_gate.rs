//! Copied alone into an UNMODIFIED a607123afbd3ebff2ef8ba10257f274fedef7eb3
//! checkout. That predecessor really understands ID3/ID5 and does not know native7.
//! Positive controls and expected verdicts are owned HERE, not in the head manifest.
use catcoms_app::store::{
    ArchiveBootstrapCatalogPolicy, CanonicalFilenameInventoryPolicy, CriticalCatalogReadPool,
    PreparedV2CurrentLayoutStore, PreparedV2GovernanceLayoutStore, V2CurrentLayoutPolicy,
    V2GovernanceLayoutPolicy, V2MemberOutboxPolicy,
};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
const SECRET: &[u8] = b"namespace7 real predecessor fixtures";
const CAPACITY: usize = 1 << 30;
const CASES: [(&str, bool); 16] = [
    ("control35", true),
    ("claim1", true),
    ("cut2", false),
    ("cut3", false),
    ("cut4", false),
    ("cut5", false),
    ("cut6", false),
    ("cut7", false),
    ("cut8", false),
    ("cut9", false),
    ("cut10", false),
    ("cut11", false),
    ("cut12", false),
    ("cut13", false),
    ("settled", false),
    ("unfenced4", true),
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
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let key = path.strip_prefix(base).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                out.insert(key, None);
                walk(base, &path, out);
            } else {
                assert!(entry.file_type().unwrap().is_file());
                out.insert(key, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
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
fn open(path: &Path, governance: bool, execution: bool, expected: bool) -> bool {
    let mut rng = ChaCha20Rng::seed_from_u64(0x77ff);
    if governance {
        let result = PreparedV2GovernanceLayoutStore::open(
            path,
            SECRET,
            &mut rng,
            V2GovernanceLayoutPolicy::new(current(execution), CAPACITY).unwrap(),
        );
        match result {
            Err(_) => false,
            Ok(mut owner) => {
                assert!(expected, "must refuse at open, not merely on retry");
                owner.retry(&mut rng).unwrap();
                assert!(owner.into_governance_layout_store().is_ok());
                true
            }
        }
    } else {
        let result = PreparedV2CurrentLayoutStore::open(path, SECRET, &mut rng, current(execution));
        match result {
            Err(_) => false,
            Ok(mut owner) => {
                assert!(expected, "must refuse at open, not merely on retry");
                owner.retry(&mut rng).unwrap();
                assert!(owner.into_current_layout_store().is_ok());
                true
            }
        }
    }
}
#[test]
fn native7_real_predecessor_fences_content_and_the_positive_controls_open() {
    let root =
        PathBuf::from(std::env::var_os("MEWTUAL_NAMESPACE7_FIXTURES").expect("head fixture root"));
    let manifest = fs::read_to_string(root.join("manifest.txt")).unwrap();
    assert_eq!(
        manifest.lines().collect::<Vec<_>>(),
        CASES.iter().map(|(name, _)| *name).collect::<Vec<_>>()
    );
    let scratch = tempfile::tempdir().unwrap();
    let mut checked = 0;
    for (name, expected) in CASES {
        for governance in [false, true] {
            for execution in [false, true] {
                let path = scratch
                    .path()
                    .join(format!("{name}-{governance}-{execution}"));
                copy(&root.join(name), &path);
                let before = tree(&path);
                assert_eq!(
                    open(&path, governance, execution, expected),
                    expected,
                    "{name}/{governance}/{execution}"
                );
                if !expected {
                    assert_eq!(tree(&path), before, "refusal changed {name}");
                }
                // New artifacts survive even on positive controls. The claim has no content;
                // unfenced4 deliberately breaks the real fence to show why rejection works.
                for name in [
                    "chat-critical-namespaces.native-registry.seed",
                    "chat-owner-v2/native-registry-layout.bin",
                    "chat-owner-v2/native-registry.bin",
                ] {
                    assert_eq!(
                        tree(&path).get(Path::new(name)),
                        before.get(Path::new(name))
                    );
                }
                if name == "claim1" && governance && !execution {
                    copy(&path, &root.join("after-governance-claim"));
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 16 * 4);
    println!("namespace7 predecessor gate: 16 fixtures x 4 openers checked");
    println!("namespace7 predecessor round trip: 1 written");
}
